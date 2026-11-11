use crate::request_gate::RequestGate;
use anyhow::{Context, Result, ensure};
use clash_mobile_runtime::{NativeHost, Session, Snapshot, core};
use core::{api, config, proxy_order, rule_manager};
use futures_util::{StreamExt, stream};
use jni::{
    EnvUnowned, JValue, jni_sig, jni_str,
    objects::{JClass, JObject, JString},
    refs::Global,
    strings::JNIStr,
    vm::JavaVM,
};
use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

#[path = "application_policies.rs"]
mod application_policies;
#[path = "application_routing.rs"]
mod application_routing;
#[path = "../../../../crates/client-ui/src/fonts.rs"]
mod fonts;

slint::include_modules!();
static EVENTS: Mutex<Option<mpsc::Sender<String>>> = Mutex::new(None);
static AUTHORIZATION_REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
// Also included in exported diagnostics. Contains view counts/status, never URLs or secrets.
static PRESENTATION: Mutex<String> = Mutex::new(String::new());
static FIRST_APPEARANCE: Mutex<Option<(bool, i32)>> = Mutex::new(None);
static UI_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static UI_LIFETIME: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

// NativeActivity can recreate an Activity before the previous native thread
// has dropped Slint's thread-local renderer and released its EGL surface.
struct UiLifetime;
impl Drop for UiLifetime {
    fn drop(&mut self) {
        let mut active = UI_LIFETIME
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *active = false;
        UI_LIFETIME.1.notify_one();
    }
}
thread_local! {
    static UI_THREAD_LIFETIME: UiLifetime = const { UiLifetime };
    static DIAGNOSTIC_WINDOW: std::cell::RefCell<Option<slint::Weak<AndroidWindow>>> = const { std::cell::RefCell::new(None) };
    static DIAGNOSTIC_GEOMETRY: std::cell::RefCell<BTreeMap<String, Vec<f32>>> = const { std::cell::RefCell::new(BTreeMap::new()) };
}

fn await_previous_ui() {
    let active = UI_LIFETIME
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut active = UI_LIFETIME
        .1
        .wait_while(active, |active| *active)
        .unwrap_or_else(|error| error.into_inner());
    *active = true;
    // Register before Slint initializes its thread-local context, so its
    // renderer is dropped before the next Activity is allowed to initialize.
    UI_THREAD_LIFETIME.with(|_| {});
}

struct AndroidHost {
    vm: JavaVM,
    activity: Global<JObject<'static>>,
}
impl AndroidHost {
    fn new(app: &slint::android::AndroidApp) -> Result<Self> {
        // android-activity owns the VM and Activity for this process.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
        let activity = vm.attach_current_thread(|env| {
            let object = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
            env.new_global_ref(object)
        })?;
        Ok(Self { vm, activity })
    }
    fn directory(&self, method: &JNIStr) -> Result<PathBuf> {
        self.vm.attach_current_thread(|env| -> Result<_> {
            let directory = env
                .call_method(
                    self.activity.as_obj(),
                    method,
                    jni_sig!("()Ljava/io/File;"),
                    &[],
                )?
                .l()?;
            let path = env
                .call_method(
                    directory,
                    jni_str!("getAbsolutePath"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            let path = env.cast_local::<JString>(path)?;
            Ok(PathBuf::from(path.try_to_string(env)?))
        })
    }
}
impl NativeHost for AndroidHost {
    fn request(&self, value: Value) -> Result<Value> {
        let result: Value = self.vm.attach_current_thread(|env| -> Result<_> {
            let input = env.new_string(value.to_string())?;
            let response = env
                .call_method(
                    self.activity.as_obj(),
                    jni_str!("request"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                    &[JValue::Object(input.as_ref())],
                )?
                .l()?;
            let response = env.cast_local::<JString>(response)?.try_to_string(env)?;
            Ok(serde_json::from_str(&response)?)
        })?;
        clash_mobile_runtime::native_response(result)
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_clashofrust_android_MainActivity_nativeEvent(
    mut unowned_env: EnvUnowned<'_>,
    _: JClass<'_>,
    text: JString<'_>,
) {
    unowned_env
        .with_env(|env| -> jni::errors::Result<()> {
            let value = text.try_to_string(env)?;
            if let Ok(sender) = EVENTS.lock()
                && let Some(sender) = sender.as_ref()
            {
                let _ = sender.try_send(value);
            }
            Ok(())
        })
        .resolve::<jni::errors::LogErrorAndDefault>();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_clashofrust_android_MainActivity_nativeDiagnostics(
    mut unowned_env: EnvUnowned<'_>,
    _: JClass<'_>,
) -> jni::sys::jstring {
    unowned_env
        .with_env(|env| {
            let (sender, receive) = std::sync::mpsc::sync_channel(1);
            let queued = slint::invoke_from_event_loop(move || {
                let snapshot = DIAGNOSTIC_WINDOW.with(|window| {
                    window
                        .borrow()
                        .as_ref()
                        .and_then(slint::Weak::upgrade)
                        .map(|ui| {
                            let geometry = ui.global::<UiGeometry>();
                            geometry.set_enabled(true);
                            geometry.set_revision(geometry.get_revision().wrapping_add(1));
                            ui.window().request_redraw();
                            presentation_snapshot(&ui)
                        })
                });
                let _ = sender.send(snapshot);
            })
            .is_ok();
            let live = if queued {
                receive.recv_timeout(Duration::from_secs(1)).ok().flatten()
            } else {
                None
            };
            let (snapshot, generation) = PRESENTATION
                .lock()
                .map(|s| {
                    (
                        s.clone(),
                        UI_GENERATION.load(std::sync::atomic::Ordering::Acquire),
                    )
                })
                .unwrap_or_default();
            let fresh = live.is_some();
            let mut snapshot = live.unwrap_or_else(|| {
                json!({"cached":serde_json::from_str::<Value>(&snapshot).unwrap_or_else(|_| json!({}))})
            });
            snapshot["live"] = json!(fresh);
            snapshot["ui_generation"] = json!(generation);
            if let Ok(first) = FIRST_APPEARANCE.lock()
                && let Some((dark, accent)) = *first
            {
                snapshot["startup_dark"] = json!(dark);
                snapshot["startup_accent"] = json!(accent);
            }
            env.new_string(snapshot.to_string()).map(JString::into_raw)
        })
        .resolve::<jni::errors::LogErrorAndDefault>()
}

#[derive(Clone)]
struct RuleForm {
    kind: String,
    payload: String,
    policy: String,
    no_resolve: bool,
}
#[derive(Clone)]
struct RuleEditor {
    form: RuleForm,
    raw: String,
    position: usize,
    text_mode: bool,
    enabled: bool,
}
struct Request {
    intent: MobileIntent,
    a: String,
    b: String,
    selected: String,
    profile_ticket: Option<u64>,
}
struct ProtectionChange {
    before: config::Settings,
    before_vpn: bool,
    enabled: bool,
    generation: Option<u64>,
    deadline: std::time::Instant,
    verified: bool,
    authorization: Option<(u64, core::claude_check::protection::Protection)>,
    auto_check: bool,
}
struct State {
    session: Session,
    application_policies: application_policies::Editor,
    overview: clash_mobile_runtime::overview::Monitor,
    overview_epoch: u64,
    page: i32,
    query: String,
    selected: String,
    notice: String,
    notice_error: bool,
    notice_critical: bool,
    notice_network_recovery: bool,
    notice_followup: bool,
    issue_events: VecDeque<Value>,
    copied: (i32, Arc<str>),
    logs: VecDeque<(String, String)>,
    log_level: String,
    connection_sort: String,
    connection_address: (String, String),
    ip_results: Vec<core::ip_report::ResultSlot>,
    claude: Option<core::claude_check::Report>,
    claude_status: String,
    protection_change: Option<ProtectionChange>,
    protection_status: String,
    guard_snapshot: core::claude_check::guard::Snapshot,
    protection_observed: bool,
    category: String,
    site_query: String,
    site_region: String,
    site_status: String,
    site_selected: String,
    rule_source: String,
    claude_busy: bool,
    claude_generation: u64,
    detecting: bool,
    generation: u64,
    visible: bool,
    completed: Option<MobileIntent>,
    probe: (String, String),
    dns: String,
    geo: String,
    geo_updating: bool,
    node_pages: BTreeMap<String, usize>,
    node_delays: BTreeMap<String, u32>,
    delay_generation: u64,
    delay_scope: Option<DelayScope>,
    connection_page: usize,
    rule_page: usize,
    rule_rows: Option<Vec<rule_manager::Row>>,
    rule_warnings: Vec<String>,
    rule_form: Option<RuleForm>,
    rule_editor: Option<RuleEditor>,
    update_package: Option<core::update::Package>,
    update_status: String,
    update_busy: bool,
    update_downloading: bool,
    update_progress: f32,
    update_downloaded: bool,
    update_path: PathBuf,
    update_generation: u64,
    update_confirmation: Option<core::update::channel::Consent>,
}

#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    await_previous_ui();
    if let Ok(mut snapshot) = PRESENTATION.lock() {
        snapshot.clear();
        UI_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
    }
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("ClashOfRust"),
    );
    if let Err(error) = run(app) {
        log::error!("Android startup: {error:#}");
    }
}
fn accent_index(color: config::AccentColor) -> i32 {
    config::AccentColor::ALL
        .iter()
        .position(|candidate| *candidate == color)
        .unwrap_or(0) as i32
}

fn run(app: slint::android::AndroidApp) -> Result<()> {
    let native = Arc::new(AndroidHost::new(&app)?);
    let root = native.directory(jni_str!("getFilesDir"))?.join("client");
    let update_path = native
        .directory(jni_str!("getCacheDir"))?
        .join("updates/update.apk");
    slint::android::init(app)?;
    fonts::register_emoji()?;
    let window = AndroidWindow::new()?;
    DIAGNOSTIC_GEOMETRY.with(|geometry| geometry.borrow_mut().clear());
    window.global::<UiGeometry>().on_measured(|key, bounds| {
        if !(key.starts_with("preview-")
            || key.starts_with("application-polic")
            || matches!(
                key.as_str(),
                "nodes-view" | "profiles-view" | "rule-shortcuts-open"
            ))
        {
            return;
        }
        let bounds: Vec<f32> = bounds.iter().collect();
        if bounds.len() == 4 && bounds.iter().all(|value| value.is_finite()) {
            DIAGNOSTIC_GEOMETRY.with(|geometry| {
                let mut geometry = geometry.borrow_mut();
                if geometry.len() < 64 || geometry.contains_key(key.as_str()) {
                    geometry.insert(key.to_string(), bounds);
                }
            });
        }
    });
    DIAGNOSTIC_WINDOW.with(|current| *current.borrow_mut() = Some(window.as_weak()));
    // No engine/Geo/network work precedes the first correctly themed frame.
    let appearance = native.request(json!({"op":"appearance"}))?;
    let theme = window.global::<Theme>();
    theme.set_appearance(appearance["mode"].as_i64().unwrap_or(2) as i32);
    theme.set_system_dark(appearance["dark"].as_bool().unwrap_or(false));
    theme.set_accent_index(accent_index(
        serde_json::from_value(appearance["accent_color"].clone()).unwrap_or_default(),
    ));
    if let Ok(mut first) = FIRST_APPEARANCE.lock() {
        *first = Some((theme.get_dark(), theme.get_accent_index()));
    }
    if let Some(size) = appearance["font_sizes"].get(6).and_then(Value::as_f64) {
        theme.set_text_scale(size as f32 / 16.);
    }
    window
        .global::<MobileData>()
        .set_version(core::VERSION.into());
    window.global::<MobileData>().set_protection_supported(true);
    let (commands, receive) = mpsc::channel::<Request>(32);
    let profile_gate = Arc::new(RequestGate::default());
    let (events, native_events) = mpsc::channel::<String>(16);
    if let Ok(mut sender) = EVENTS.lock() {
        *sender = Some(events);
    }
    let weak = window.as_weak();
    let gate = profile_gate.clone();
    window
        .global::<MobileActions>()
        .on_request(move |intent, a, b| {
            let Some(ui) = weak.upgrade() else { return };
            let profile_ticket = if matches!(
                intent,
                MobileIntent::SaveProfile
                    | MobileIntent::UpdateProfile
                    | MobileIntent::ActivateProfile
                    | MobileIntent::DeleteProfile
            ) {
                let Some(ticket) = gate.begin() else {
                    return;
                };
                Some(ticket)
            } else {
                None
            };
            ui.global::<MobileData>().set_profile_busy(gate.busy());
            ui.global::<MobileData>().set_working(
                gate.busy()
                    || !matches!(
                        intent,
                        MobileIntent::Navigate | MobileIntent::Query | MobileIntent::RuleTextMode
                    ),
            );
            if commands
                .try_send(Request {
                    intent,
                    a: a.into(),
                    b: b.into(),
                    selected: ui.get_selected_id().into(),
                    profile_ticket,
                })
                .is_err()
            {
                if let Some(ticket) = profile_ticket {
                    gate.finish(ticket);
                }
                ui.global::<MobileData>().set_profile_busy(gate.busy());
                ui.global::<MobileData>().set_working(gate.busy());
                ui.invoke_notify("操作过于频繁，请稍后再试".into());
            }
        });
    let host = native.clone();
    let routing_window = window.as_weak();
    let application_editor = Arc::new(Mutex::new(application_routing::Editor::default()));
    window
        .global::<AndroidActions>()
        .on_request(move |intent, value| {
            if application_routing::handles(intent) {
                if let Some(ui) = routing_window.upgrade()
                    && let Err(error) = application_routing::request(
                        &application_editor,
                        host.clone(),
                        &ui,
                        intent,
                        value.as_str(),
                    )
                {
                    ui.global::<AndroidData>()
                        .set_applications_error(format!("{error:#}").into());
                }
                return;
            }
            let op = match intent {
                AndroidIntent::VpnSettings => "vpn-settings",
                AndroidIntent::NotificationSpeeds => "notification-speeds",
                AndroidIntent::BatterySettings => "battery-settings",
                AndroidIntent::CheckUpdate => "open-url",
                AndroidIntent::Background => "background",
                _ => return,
            };
            let request = if op == "open-url" {
                json!({"op":op,"url":"https://github.com/Fu021/Clash-of-Rust/releases"})
            } else {
                json!({"op":op,"value":value.as_str() == "true"})
            };
            if let Err(error) = host.request(request) {
                log::error!("Android request: {error:#}");
            }
        });
    let weak = window.as_weak();
    let worker = crate::session_worker::SessionWorker::spawn(move |stopped| {
        let result = (|| -> Result<()> {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?;
            runtime.block_on(async {
                tokio::select! {
                    biased;
                    _ = stopped => Ok(()),
                    result = run_loop(
                        root,
                        update_path,
                        native,
                        receive,
                        native_events,
                        weak,
                        profile_gate,
                    ) => result,
                }
            })
        })();
        if let Err(error) = result {
            log::error!("Mobile runtime stopped: {error:#}");
        }
    })?;
    let result = window.run();
    if let Ok(mut sender) = EVENTS.lock() {
        *sender = None;
    }
    drop(worker);
    result?;
    Ok(())
}

#[derive(Clone)]
enum DelayScope {
    All,
    Group(String),
    Node(String),
}
enum Update {
    Subscription(
        u64,
        core::subscription::schedule::PendingUpdate,
        Result<(String, Option<String>)>,
    ),
    Delays(u64, std::result::Result<BTreeMap<String, u32>, String>),
    Unreachable(
        u64,
        u64,
        serde_json::Value,
        std::result::Result<bool, String>,
    ),
    Snapshot(u64, std::result::Result<Snapshot, String>),
    Ip(u64, usize, core::ip_report::ResultSlot),
    IpDone(u64),
    Claude(u64, std::result::Result<core::claude_check::Report, String>),
    Protection(std::result::Result<core::claude_check::protection::Protection, String>),
    ProtectionVerified(std::time::Instant, std::result::Result<(), String>),
    Address(u64, String, String),
    Log(String, String),
    Geo(std::result::Result<tempfile::TempDir, String>),
    Checked(
        u64,
        std::result::Result<Option<core::update::Package>, String>,
    ),
    DownloadProgress(u64, core::update::Progress),
    Downloaded(u64, std::result::Result<(), String>),
}
async fn run_loop(
    root: PathBuf,
    update_path: PathBuf,
    native: Arc<AndroidHost>,
    mut receive: mpsc::Receiver<Request>,
    mut native_events: mpsc::Receiver<String>,
    weak: slint::Weak<AndroidWindow>,
    profile_gate: Arc<RequestGate>,
) -> Result<()> {
    let mut state = State {
        session: Session::open(root, native)?,
        overview: Default::default(),
        overview_epoch: 0,
        application_policies: Default::default(),
        page: 0,
        query: String::new(),
        selected: String::new(),
        notice: String::new(),
        notice_error: false,
        notice_critical: false,
        notice_network_recovery: false,
        notice_followup: false,
        issue_events: VecDeque::new(),
        copied: Default::default(),
        logs: VecDeque::new(),
        log_level: "全部等级".into(),
        connection_sort: "下载速度 ↓".into(),
        connection_address: Default::default(),
        ip_results: vec![None; core::ip_check::services().len()],
        claude: None,
        claude_status: "尚未检测".into(),
        protection_change: None,
        protection_status: String::new(),
        guard_snapshot: Default::default(),
        protection_observed: false,
        category: "AI".into(),
        site_query: String::new(),
        site_region: "全部地区".into(),
        site_status: "全部结果".into(),
        site_selected: String::new(),
        rule_source: "全部来源".into(),
        claude_busy: false,
        claude_generation: 0,
        detecting: false,
        generation: 0,
        visible: true,
        completed: None,
        probe: ("尚未测试".into(), String::new()),
        dns: "尚未查询".into(),
        geo: "Geo 数据状态不可用".into(),
        geo_updating: false,
        node_pages: BTreeMap::new(),
        node_delays: BTreeMap::new(),
        delay_generation: 0,
        delay_scope: None,
        connection_page: 0,
        rule_page: 0,
        rule_rows: None,
        rule_warnings: Vec::new(),
        rule_form: None,
        rule_editor: None,
        update_package: None,
        update_status: "尚未检查更新".into(),
        update_busy: false,
        update_downloading: false,
        update_progress: 0.0,
        update_downloaded: false,
        update_path,
        update_generation: 0,
        update_confirmation: None,
    };
    if let Err(error) = state.session.attach_or_initialize() {
        state.notice = format!("内核启动失败：{error:#}");
        state.notice_error = true;
        state.notice_critical = true;
        log::error!("{}", state.notice);
        state.logs.push_back(("ERROR".into(), state.notice.clone()));
    }
    if let Ok(manifest) = core::geodata::read_manifest(&state.session.store.runtime()) {
        state.geo = manifest.display_status();
    }
    state.session.observe_native()?;
    if state.session.settings.claude_protection.is_some()
        && (!state.session.vpn
            || !state.session.platform["protection"]["enabled"]
                .as_bool()
                .unwrap_or(false))
    {
        let mut before = state.session.settings.clone();
        let saved = before.claude_protection.take().expect("saved protection");
        before.run_mode = saved.previous.mode.clone();
        state.protection_change = Some(ProtectionChange {
            before,
            before_vpn: saved.previous.proxy_mode == core::model::ProxyMode::Tun,
            enabled: true,
            generation: Some(
                state.session.platform["vpn_generation"]
                    .as_u64()
                    .unwrap_or(0),
            ),
            deadline: std::time::Instant::now() + Duration::from_secs(120),
            verified: false,
            authorization: None,
            auto_check: false,
        });
        state.protection_status = "正在恢复保护 VPN…".into();
        if let Err(error) = state.session.native.request(json!({"op":"reconnect"})) {
            rollback_protection(&mut state, &format!("保护恢复失败：{error:#}"))?;
        }
    }
    let (updates, mut update_rx) = mpsc::channel::<Update>(32);
    let (overview_send, mut overview_rx) = mpsc::channel(16);
    let mut checks: Option<tokio::task::JoinHandle<()>> = None;
    let mut claude_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut protection_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut protection_dns_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut address_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut address_generation = 0u64;
    let mut log_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut geo_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut log_port = 0;
    let mut last_delay = std::time::Instant::now();
    let mut subscription_schedule = core::subscription::schedule::Schedule::default();
    let mut subscription_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut subscription_generation = 0u64;
    let mut last_subscription_check = std::time::Instant::now() - Duration::from_secs(30);
    let mut delay_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut recovery_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut poll = tokio::time::interval(Duration::from_secs(1));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut refresh_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut refresh_generation = 0u64;
    let mut last_frame = std::time::Instant::now() - Duration::from_secs(1);
    let mut system_bars_state = None;
    let mut update_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut update_schedule = core::update::schedule::Schedule::default();
    loop {
        let mut immediate = false;
        let mut refresh_requested = false;
        let mut profile_completed = None;
        let mut overview_refresh = false;
        tokio::select! {
            value = receive.recv() => {
                let Some(request) = value else { break; };
                profile_completed = request.profile_ticket;
                if profile_completed.is_some() {
                    subscription_generation = subscription_generation.wrapping_add(1);
                    if let Some(job) = subscription_job.take() { job.abort(); let _ = job.await; }
                }
                state.selected = request.selected.clone();
                immediate = true;
                let intent = request.intent;
                if matches!(intent, MobileIntent::Connect | MobileIntent::Disconnect
                    | MobileIntent::RestartCore | MobileIntent::Mode | MobileIntent::SelectNode
                    | MobileIntent::ActivateProfile | MobileIntent::UpdateProfile | MobileIntent::SaveProfile
                    | MobileIntent::SavePorts | MobileIntent::ClaudeProtection | MobileIntent::DnsOverride
                    | MobileIntent::FakeIp | MobileIntent::FakeIpAdd | MobileIntent::FakeIpRemove
                    | MobileIntent::FakeIpRestoreDefaults | MobileIntent::Ipv6 | MobileIntent::Udp
                    | MobileIntent::RuleApply) {
                    state.overview_epoch = state.overview_epoch.wrapping_add(1);
                }
                if matches!(intent, MobileIntent::Connect | MobileIntent::Disconnect
                    | MobileIntent::RestartCore | MobileIntent::Mode | MobileIntent::SelectNode
                    | MobileIntent::ActivateProfile | MobileIntent::ClaudeProtection) {
                    if let Some(job) = recovery_job.take() { job.abort(); }
                    cancel_delay(&mut state, &mut delay_job);
                }
                let edited_profile = match intent {
                    MobileIntent::UpdateProfile => Some(request.a.as_str()),
                    MobileIntent::SaveProfile => Some(request.selected.as_str()),
                    _ => None,
                };
                let subscription_affects_active = edited_profile.is_some_and(|id| {
                    state.session.settings.active_profile.as_deref() == Some(id)
                });
                if matches!(intent, MobileIntent::Navigate | MobileIntent::ConnectionDetail) {
                    if let Some(job) = address_job.take() { job.abort(); }
                    address_generation = address_generation.wrapping_add(1);
                    state.connection_address = Default::default();
                    if intent == MobileIntent::ConnectionDetail
                        && let Some(connection) = state.session.connections.connections.iter()
                            .find(|c|c.id.as_ref() == request.a)
                        && connection.metadata.destination_ip.is_empty()
                    {
                        let host = core::connection::hostname(connection).trim_end_matches('.').to_owned();
                        if !host.is_empty() {
                            let api = state.session.api.clone();
                            let port = state.session.settings.mixed_port;
                            let id = request.a.clone();
                            let generation = address_generation;
                            let send = updates.clone();
                            address_job = Some(tokio::spawn(async move {
                                let value = core::connection::resolve_target(&api, &host, port).await.unwrap_or_default();
                                let _ = send.send(Update::Address(generation, id, value)).await;
                            }));
                        }
                    }
                }
                if matches!(intent, MobileIntent::RuleSelect | MobileIntent::RuleSave | MobileIntent::RuleMove
                    | MobileIntent::RuleDelete | MobileIntent::RuleDrop | MobileIntent::RuleEnable | MobileIntent::RuleRestore | MobileIntent::RuleApply | MobileIntent::RuleDiscard
                    | MobileIntent::RuleShortcut | MobileIntent::ActivateProfile | MobileIntent::UpdateProfile
                    | MobileIntent::SaveProfile | MobileIntent::DeleteProfile) {
                    state.rule_rows = None;
                }
                if intent == MobileIntent::OverviewRefresh {
                    overview_refresh = true;
                } else if intent == MobileIntent::OverviewPrivacy {
                    state.overview.toggle_privacy();
                } else if intent == MobileIntent::Navigate {
                    let page: i32 = request.a.parse()?;
                    if state.page == 8 && page != 8 {
                        if let Some(job) = claude_job.take() { job.abort(); }
                        state.claude_generation = state.claude_generation.wrapping_add(1);
                        state.claude_busy = false;
                    }
                    state.page = page;
                    state.query.clear();
                    state.connection_page = 0;
                    state.rule_page = 0;
                    if let Some(job) = refresh_job.take() { job.abort(); }
                    refresh_generation += 1;
                    refresh_requested = true;
                } else if intent == MobileIntent::CancelUpdate {
                    if state.update_downloading {
                        state.update_generation = state.update_generation.wrapping_add(1);
                        if let Some(job) = update_job.take() { job.abort(); let _ = job.await; }
                        state.update_busy = false; state.update_downloading = false;
                        state.update_downloaded = false; state.update_progress = 0.0;
                        state.update_status = "下载已取消，可重新下载".into();
                        for path in [&state.update_path, &state.update_path.with_extension("partial")] {
                            if let Err(error) = tokio::fs::remove_file(path).await
                                && error.kind() != std::io::ErrorKind::NotFound {
                                state.notice = format!("下载已取消，缓存清理失败：{error}");
                                state.notice_error = true;
                            }
                        }
                    }
                } else if intent == MobileIntent::ClaudeCancel {
                    if let Some(job) = claude_job.take() { job.abort(); }
                    state.claude_generation = state.claude_generation.wrapping_add(1);
                    state.claude_busy = false;
                    state.claude_status = "已停止检测，保留上次完成的报告".into();
                } else if intent==MobileIntent::CheckUpdate {
                    if !blocked_during_protection(&mut state) && !state.update_busy {
                        state.update_busy=true;state.update_downloading=false;state.update_status="正在检查更新…".into();state.update_progress=0.0;
                        update_schedule.checked(std::time::Instant::now());
                        state.update_generation = state.update_generation.wrapping_add(1);
                        state.update_confirmation = None;
                        update_job=Some(start_update_check(state.session.settings.download_ports(),state.session.settings.update_prereleases,state.update_generation,updates.clone()));
                    }
                } else if intent==MobileIntent::InstallUpdate {
                    if request.b == "cancel" {
                        state.update_confirmation = None;
                    } else if blocked_during_protection(&mut state) {
                    } else if !state.update_busy && let Some(package)=state.update_package.clone() {
                        if request.b == "prepare" {
                            if package.preview && state.session.settings.update_prereleases {
                                state.update_confirmation = Some(core::update::channel::Consent::new(package.version.clone()));
                            } else {
                                state.notice = "更新版本已变化，请重新检查更新".into();
                                state.notice_error = true;
                            }
                        } else if (package.preview && (!state.session.settings.update_prereleases
                            || request.b != "confirmed"
                            || !state.update_confirmation.as_ref().is_some_and(|consent| consent.accepts(&package.version))))
                            || (!package.preview && !request.b.is_empty()) {
                            state.notice = "请先阅读预发布风险提示，并等待 3 秒后确认".into();
                            state.notice_error = true;
                        } else {
                        state.update_confirmation = None;
                        if state.update_downloaded && state.update_path.is_file() {
                            if let Err(error)=state.session.native.request(json!({"op":"install-update"})) {state.notice=format!("{error:#}");state.notice_error=true;}
                        } else {
                            state.update_busy=true;state.update_downloading=true;state.update_status="正在下载更新…".into();state.update_progress=0.0;
                            let path=state.update_path.clone();
                            let port=state.session.settings.download_ports();let send=updates.clone();
                            let download_api=state.session.api.clone();
                            state.update_generation = state.update_generation.wrapping_add(1);
                            let generation = state.update_generation;
                            update_job=Some(tokio::spawn(async move {
                                let report=send.clone();
                                let result=core::update::download(&package,Some(port),&path,Some(&download_api),move|value|{let _=report.try_send(Update::DownloadProgress(generation,value));}).await;
                                let _=send.send(Update::Downloaded(generation,result.map_err(|e|format!("{e:#}")))).await;
                            }));
                        }
                        }
                    }
                } else if intent == MobileIntent::UpdateGeo {
                    if !blocked_during_protection(&mut state) && !state.geo_updating {
                        state.geo_updating=true;
                        let runtime=state.session.store.runtime();
                        let port=state.session.running.then_some(state.session.settings.download_ports());
                        let send=updates.clone();
                        geo_job=Some(tokio::spawn(async move {
                            let result=async {
                                let staged=tempfile::Builder::new().prefix(".geo-download-").tempdir_in(&runtime)?;
                                core::geodata::download_geo(staged.path(),port).await?;
                                Ok::<_,anyhow::Error>(staged)
                            }.await.map_err(|e|format!("{e:#}"));
                            let _=send.send(Update::Geo(result)).await;
                        }));
                    }
                } else if matches!(intent, MobileIntent::SiteStart | MobileIntent::SiteRetest | MobileIntent::SiteProbe) {
                    if let Some(job) = checks.take() { job.abort(); }
                    state.generation += 1; state.detecting = true;
                    if intent == MobileIntent::SiteStart { state.ip_results.fill(None); }
                    let generation = state.generation;
                    let port = state.session.settings.mixed_port;
                    let send = updates.clone();
                    let targets=core::ip_check::services().iter().enumerate()
                        .filter(|(_,service)| intent == MobileIntent::SiteStart
                            || (intent == MobileIntent::SiteProbe && service.id == request.a)
                            || (intent == MobileIntent::SiteRetest && (request.a == "all" || service.group == request.a)))
                        .map(|(index,service)|(index,service.id.clone())).collect::<Vec<_>>();
                    checks = Some(tokio::spawn(async move {
                        let mut jobs = stream::iter(targets)
                            .map(move |(index,id)|async move {(index,core::ip_check::check(&id,port).await.map_err(|e|e.to_string()))})
                            .buffer_unordered(4);
                        while let Some((index,result)) = jobs.next().await {
                            if send.send(Update::Ip(generation,index,Some(result))).await.is_err() { return; }
                        }
                        let _ = send.send(Update::IpDone(generation)).await;
                    }));
                } else if intent == MobileIntent::SiteCancel {
                    if let Some(job) = checks.take() { job.abort(); }
                    state.generation += 1; state.detecting = false;
                } else if matches!(intent,MobileIntent::NodeDelay|MobileIntent::GroupDelay|MobileIntent::DelayAll) {
                    if state.delay_scope.is_none() && state.protection_change.is_none() {
                        if let Some(job) = recovery_job.take() { job.abort(); }
                        last_delay=std::time::Instant::now();
                        let scope=match intent { MobileIntent::GroupDelay=>DelayScope::Group(request.a),
                            MobileIntent::DelayAll=>DelayScope::All,_=>DelayScope::Node(request.a) };
                        delay_job=Some(start_delay(&mut state,scope,updates.clone()));
                    }
                } else if intent == MobileIntent::ClaudeProtection && request.a == "true" {
                    if state.protection_change.is_none() && state.session.settings.claude_protection.is_none() {
                        cancel_delay(&mut state,&mut delay_job);
                        state.update_generation = state.update_generation.wrapping_add(1);
                        if let Some(job) = update_job.take() { job.abort(); let _ = job.await; }
                        state.update_busy = false; state.update_downloading = false;
                        state.update_downloaded = false; state.update_progress = 0.0;
                        state.update_status = "Claude 保护期间暂停客户端更新".into();
                        if let Some(job) = geo_job.take() { job.abort(); let _ = job.await; }
                        state.geo_updating = false;
                        subscription_generation = subscription_generation.wrapping_add(1);
                        if let Some(job) = subscription_job.take() { job.abort(); let _ = job.await; }
                        if let Some(job) = protection_dns_job.take() { job.abort(); }
                        state.protection_change = Some(ProtectionChange {
                            before: state.session.settings.clone(), before_vpn: state.session.vpn, enabled: true,
                            generation: None, deadline: std::time::Instant::now()+Duration::from_secs(120),
                            verified: false,
                            authorization: None,
                            auto_check: true,
                        });
                        state.protection_status="正在确认当前代理节点…".into();
                        let api=state.session.api.clone();let settings=state.session.settings.clone();
                        let raw=state.session.raw();let vpn=state.session.vpn;let send=updates.clone();
                        protection_job=Some(tokio::spawn(async move {
                            let result=async {
                                let raw=raw?;
                                tokio::time::timeout(Duration::from_secs(20),
                                    clash_mobile_runtime::prepare_protection(&api,&settings,&raw,vpn))
                                    .await.context("确认保护节点超时")?
                            }.await.map_err(|error|format!("{error:#}"));
                            let _=send.send(Update::Protection(result)).await;
                        }));
                    }
                } else if intent == MobileIntent::ClaudeStart {
                    start_claude_check(&mut state, &mut claude_job, updates.clone());
                } else {
                    state.notice_error = false;
                    state.notice_critical = false;
                    state.notice_followup = matches!(intent, MobileIntent::ExportDiagnostics | MobileIntent::CopyDiagnostics | MobileIntent::ReportIssue);
                    match handle(&mut state, request).await {
                        Ok(()) => {
                            state.completed = Some(intent);
                            if subscription_affects_active {
                                reset_subscription_results(&mut state, &mut checks, &mut claude_job);
                            }
                            if matches!(intent,MobileIntent::SelectNode|MobileIntent::ActivateProfile|MobileIntent::UpdateProfile
                                |MobileIntent::SaveProfile|MobileIntent::DeleteProfile|MobileIntent::RestartCore|MobileIntent::Mode
                                |MobileIntent::DnsOverride|MobileIntent::FakeIp|MobileIntent::FakeIpAdd|MobileIntent::FakeIpRemove|MobileIntent::FakeIpRestoreDefaults|MobileIntent::Ipv6|MobileIntent::Udp|MobileIntent::SavePorts
                                |MobileIntent::ClaudeProtection) {cancel_delay(&mut state,&mut delay_job);}
                            if !matches!(intent, MobileIntent::Query | MobileIntent::Copy | MobileIntent::ConnectionSort
                                | MobileIntent::ConnectionDetail | MobileIntent::ConnectionPage | MobileIntent::RulePage
                                | MobileIntent::LogLevel | MobileIntent::ClearLogs | MobileIntent::ExportLogs | MobileIntent::ExportDiagnostics | MobileIntent::CopyDiagnostics | MobileIntent::ReportIssue | MobileIntent::RuleTextMode
                                | MobileIntent::RuleSelect | MobileIntent::RuleEdit | MobileIntent::RuleSave
                                | MobileIntent::RuleMove | MobileIntent::RuleDelete | MobileIntent::RuleShortcut
                                | MobileIntent::RuleDiscard | MobileIntent::NodePage | MobileIntent::NodeSort
                                | MobileIntent::Appearance | MobileIntent::Accent | MobileIntent::TextScale) {
                                if let Some(job) = refresh_job.take() { job.abort(); }
                                refresh_generation += 1;
                                refresh_requested = true;
                            }
                        },
                        Err(error) => {
                            state.notice = format!("{error:#}");
                            state.notice_error = true;
                            state.notice_critical = intent == MobileIntent::RestartCore;
                            log::error!("{}",state.notice);
                            state.logs.push_back(("ERROR".into(),state.notice.clone()));
                        }
                    }
                }
            }
            value = native_events.recv() => {
                if let Some(value) = value {
                    immediate = true;
                    refresh_requested = true;
                    let event = serde_json::from_str::<Value>(&value).ok();
                    if event.as_ref().is_some_and(|event|event["event"]=="back") {
                        let weak=weak.clone();slint::invoke_from_event_loop(move||{if let Some(ui)=weak.upgrade(){ui.invoke_native_back();}})?;
                    } else if let Err(error) = native_event(&mut state,&value).await {
                        state.notice=format!("{error:#}");state.notice_error=true;
                        state.notice_critical=event.as_ref().is_some_and(|event|
                            matches!(event["event"].as_str(),Some("vpn-authorized"|"vpn-cancelled")));
                    }
                }
            }
            Some(event) = overview_rx.recv() => {
                state.overview.apply(event);
            }
            value = update_rx.recv() => {
                match value {
                    Some(Update::ProtectionVerified(token, result)) => {
                        if let Some(change) = state.protection_change.as_mut()
                            && change.deadline == token {
                            protection_dns_job = None;
                            match result {
                                Ok(()) => change.verified = true,
                                Err(error) => {
                                    if let Err(restore) = rollback_protection(&mut state, &format!("保护验证失败：{error}")) {
                                        state.notice = format!("保护验证失败：{error}；恢复失败：{restore:#}");
                                        state.notice_error = true;
                                        state.notice_critical = true;
                                    }
                                }
                            }
                            refresh_requested = true;
                            immediate = true;
                        }
                    }
                    Some(Update::Address(generation, id, value)) if generation == address_generation => {
                        address_job = None;
                        state.connection_address = (id, value);
                        immediate = true;
                    }
                    Some(Update::Delays(generation,result)) if generation==state.delay_generation => {
                        let all=matches!(state.delay_scope,Some(DelayScope::All));
                        delay_job=None;state.delay_scope=None;
                        match result {
                            Ok(values)=> {
                                let failed=values.values().filter(|&&delay|delay==0).count();
                                let unavailable = !values.is_empty() && failed == values.len();
                                state.notice=if unavailable {
                                    let detail = if state.session.settings.claude_protection.is_some() {
                                        "请检查 Claude 保护线路。"
                                    } else {
                                        "请更换节点；更新失败后会自动尝试直连。"
                                    };
                                    format!("本次测速：{} 个节点不可达。{detail}", values.len())
                                } else {
                                    format!("测速完成 · {} 项，{} 项超时",values.len(),failed)
                                };
                                state.notice_network_recovery = unavailable;
                                state.notice_error=failed > 0;
                                if unavailable && state.session.vpn && recovery_job.is_none()
                                    && state.protection_change.is_none()
                                    && state.session.settings.claude_protection.is_none() {
                                    let api = state.session.api.clone();
                                    let settings = serde_json::to_value(&state.session.settings)?;
                                    let vpn_generation = state.session.platform["vpn_generation"].as_u64().unwrap_or(0);
                                    let sender = updates.clone();
                                    recovery_job = Some(tokio::spawn(async move {
                                        let result = tokio::time::timeout(Duration::from_secs(120),
                                            api.confirm_all_nodes_unreachable()).await
                                            .context("全部节点复测超时")
                                            .and_then(|result| result).map_err(|error| format!("{error:#}"));
                                        let _ = sender.send(Update::Unreachable(generation, vpn_generation, settings, result)).await;
                                    }));
                                }
                                if all {state.node_delays=values;} else {state.node_delays.extend(values);}
                            }
                            Err(error)=>{state.notice=format!("测速失败：{error}");state.notice_error=true;},
                        }
                        immediate=true;refresh_requested=true;
                    }
                    Some(Update::Unreachable(generation, vpn_generation, settings, result)) if generation == state.delay_generation => {
                        recovery_job = None;
                        if state.session.vpn && state.protection_change.is_none()
                            && state.session.settings.claude_protection.is_none()
                            && serde_json::to_value(&state.session.settings)? == settings
                            && state.session.platform["vpn_generation"].as_u64().unwrap_or(0) == vpn_generation {
                            match result {
                                Ok(true) => {
                                    state.logs.push_back(("WARN".into(), "全部节点经两个独立目标复测仍不可达，VPN 保持连接".into()));
                                    state.notice = "全部节点不可达，VPN 仍连接。请更换节点；更新失败后会自动尝试直连。".into();
                                    state.notice_network_recovery = true;
                                },
                                Ok(false) => state.notice = "未确认全部节点失联，可重试或更换节点。".into(),
                                Err(error) => state.notice = format!("节点复测失败：{error}"),
                            }
                            state.notice_error = true;
                            immediate = true;
                            refresh_requested = true;
                        }
                    }
                    Some(Update::Snapshot(generation, result)) if generation == refresh_generation => {
                        refresh_job = None;
                        immediate = true;
                        match result {
                            Ok(snapshot) => {
                                state.session.apply_snapshot(snapshot);
                                let members=state.session.proxies.proxies.values().flat_map(|p|p.all.iter().map(|n|n.as_str()))
                                    .collect::<std::collections::BTreeSet<_>>();
                                state.node_delays.retain(|name,_|state.session.proxies.proxies.contains_key(name.as_str())||members.contains(name.as_str()));
                            }
                            Err(error) => {
                                let message = format!("状态刷新失败：{error}");
                                if state.logs.back().is_none_or(|(_,old)| old != &message) {
                                    log::warn!("{message}");
                                    state.logs.push_back(("WARN".into(), message));
                                }
                            }
                        }
                    }
                    Some(Update::Ip(generation,index,result)) if generation == state.generation => state.ip_results[index]=result,
                    Some(Update::IpDone(generation)) if generation == state.generation => { state.detecting=false; checks=None; }
                    Some(Update::Claude(generation,result)) if generation == state.claude_generation => {
                        claude_job = None; state.claude_busy = false; immediate = true;
                        match result {
                            Ok(report) => {state.claude=Some(report); state.claude_status="检测完成 · 浏览器及网络采样已结束".into(); state.notice="检测完成".into();}
                            Err(error) => {state.claude_status=format!("检测失败：{error}"); state.notice=error;state.notice_error=true;}
                        }
                    }
                    Some(Update::Protection(result)) => {
                        protection_job=None;
                        immediate=true;
                        let applied=(|| -> Result<()> {
                            let protection=result.map_err(anyhow::Error::msg)?;
                            ensure!(state.protection_change.is_some(), "保护操作已取消");
                            ensure!(state.session.settings.active_profile.as_deref()==Some(protection.profile_id.as_str()),
                                "订阅已变化，请重新开启保护");
                            let request=AUTHORIZATION_REQUEST.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
                            state.protection_change.as_mut().expect("pending protection")
                                .authorization=Some((request,protection));
                            state.protection_status="正在等待 VPN 授权，当前网络设置保持不变…".into();
                            state.session.native.request(json!({"op":"authorize-vpn","request":request}))?;
                            Ok(())
                        })();
                        if let Err(error)=applied
                            && let Err(restore)=rollback_protection(&mut state,&format!("保护开启失败：{error:#}")) {
                            state.notice=format!("保护开启失败：{error:#}；恢复失败：{restore:#}");
                            state.notice_error=true;
                            state.notice_critical=true;
                        }
                        refresh_requested=true;
                    }
                    Some(Update::Log(level,message)) => state.logs.push_back((level,message)),
                    Some(Update::Geo(result))=> {
                        if let Some(job) = refresh_job.take() { job.abort(); }
                        refresh_generation += 1;
                        refresh_requested = true;
                        state.geo_updating=false;geo_job=None;
                        match result.and_then(|staged|state.session.install_geo(staged.path()).map_err(|e|format!("{e:#}"))) {
                            Ok(manifest)=> {state.geo=manifest.display_status();state.notice="Geo 数据已更新".into();},
                            Err(error)=>{state.notice=format!("Geo 更新失败：{error}");state.notice_error=true;},
                        }
                    }
                    Some(Update::Subscription(generation, pending, result)) if generation == subscription_generation => {
                        subscription_job = None;
                        if !profile_gate.busy() && state.protection_change.is_none() {
                            match result {
                                Ok((raw, usage)) => match state.session.update_subscription(&pending, raw, usage) {
                                    Ok(true) => {
                                        cancel_delay(&mut state, &mut delay_job);
                                        reset_subscription_results(&mut state, &mut checks, &mut claude_job);
                                        state.rule_rows = None;
                                        if let Some(job) = refresh_job.take() { job.abort(); }
                                        refresh_generation = refresh_generation.wrapping_add(1);
                                        refresh_requested = true;
                                        immediate = true;
                                    }
                                    Ok(false) => {}
                                    Err(_) => state.logs.push_back(("WARN".into(), "订阅定时更新失败，已保留原配置与运行状态".into())),
                                },
                                Err(error) => {
                                    if pending.is_current(&state.session.settings, &state.session.profiles, &state.session.store).unwrap_or(false)
                                        && let Some(message) = core::subscription::format_notice(&error) {
                                        state.notice = format!("订阅定时更新失败，已保留原配置与运行状态：{message}");
                                        state.notice_error = true;
                                    }
                                    state.logs.push_back(("WARN".into(), format!("订阅定时下载失败，已保留原配置与运行状态：{error}")));
                                },
                            }
                        }
                    }
                    Some(Update::Checked(generation,result)) if generation == state.update_generation =>{
                        state.update_busy=false;state.update_downloading=false;
                        match result {
                            Ok(package)=>{
                                state.update_status=package.as_ref().map(|p|format!("{} · {:.1} MiB",core::update::channel::description(&p.version,p.preview,core::VERSION),p.size as f64/1024.0/1024.0))
                                    .unwrap_or_else(||"当前更新通道没有可用的 Android 更新".into());
                                state.update_downloaded=state.update_downloaded && state.update_package.as_ref().map(|p|&p.version)==package.as_ref().map(|p|&p.version);
                                state.update_package=package;
                            }
                            Err(error)=>{state.update_status=error.clone();state.notice=format!("检查更新失败：{error}");state.notice_error=true;},
                        }
                    }
                    Some(Update::DownloadProgress(generation,value)) if generation == state.update_generation =>{
                        state.update_progress=value.received as f32/value.total.max(1) as f32;
                        state.update_status=if value.received==0 {
                            format!("正在连接下载 · {}",value.routing())
                        } else {
                            format!("下载 {}/{} · {}/s · {}",bytes(value.received),bytes(value.total),bytes(value.bytes_per_second),value.routing())
                        };
                    }
                    Some(Update::Downloaded(generation,result)) if generation == state.update_generation =>{
                        state.update_busy=false;state.update_downloading=false;
                        match result {
                            Ok(())=>{
                                state.update_downloaded=true;state.update_status="下载完成，请在系统界面确认安装".into();
                                if let Err(error)=state.session.native.request(json!({"op":"install-update"})) {state.notice=format!("{error:#}");state.notice_error=true;}
                            }
                            Err(error)=>{state.update_status=error.clone();state.notice=format!("更新失败：{error}");state.notice_error=true;},
                        }
                    }
                    _ => {}
                }
            }
            _ = poll.tick() => {
                if state.session.settings.subscription_update_interval_minutes == 0
                    && let Some(job) = subscription_job.take() {
                    subscription_generation = subscription_generation.wrapping_add(1);
                    job.abort();
                    let _ = job.await;
                }
                if subscription_job.is_none() && !profile_gate.busy() && state.protection_change.is_none()
                    && last_subscription_check.elapsed() >= Duration::from_secs(30) {
                    last_subscription_check = std::time::Instant::now();
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                    if let Some(profile) = subscription_schedule.next(&state.session.settings, &state.session.profiles, now) {
                        match core::subscription::schedule::PendingUpdate::capture(profile, &state.session.store) {
                            Ok(pending) => {
                                let port = state.session.running.then_some(state.session.settings.download_ports());
                                let sender = updates.clone();
                                subscription_generation = subscription_generation.wrapping_add(1);
                                let generation = subscription_generation;
                                subscription_job = Some(tokio::spawn(async move {
                                    let result = async {
                                        let url = core::probe::validate_url(&pending.profile.source)?;
                                        core::subscription::download_routes(&url, &core::network::download_routes(port, None)).await
                                    }.await;
                                    let _ = sender.send(Update::Subscription(generation, pending, result)).await;
                                }));
                            }
                            Err(_) => state.logs.push_back(("WARN".into(), "订阅定时更新失败，已保留原配置".into())),
                        }
                    }
                }
                if update_schedule.due(state.session.settings.update_check_interval_minutes, std::time::Instant::now())
                    && state.session.running && !state.update_busy
                    && state.session.settings.claude_protection.is_none() && state.protection_change.is_none() {
                    update_schedule.checked(std::time::Instant::now());state.update_busy=true;state.update_downloading=false;state.update_status="正在检查更新…".into();
                    state.update_generation = state.update_generation.wrapping_add(1);
                    state.update_confirmation = None;
                    update_job=Some(start_update_check(state.session.settings.download_ports(),state.session.settings.update_prereleases,state.update_generation,updates.clone()));
                }
                let minutes=state.session.settings.delay_interval_minutes;
                if state.session.running && state.protection_change.is_none() && minutes>0
                    && last_delay.elapsed()>=Duration::from_secs(minutes as u64*60) && state.delay_scope.is_none()
                    && recovery_job.is_none() {
                    last_delay=std::time::Instant::now();
                    delay_job=Some(start_delay(&mut state,DelayScope::All,updates.clone()));
                }
                refresh_requested = true;
            }
        }
        if refresh_requested {
            let was_running = state.session.running;
            state.protection_observed = match state.session.observe_native() {
                Ok(()) => true,
                Err(error) => {
                    state
                        .logs
                        .push_back(("WARN".into(), format!("状态刷新失败：{error:#}")));
                    false
                }
            };
            if was_running && !state.session.running {
                state.notice = "内核已意外停止，请导出诊断后重试启动。".into();
                state.notice_error = true;
                state.notice_critical = true;
            }
            match observe_protection(&mut state) {
                Ok(true) => start_claude_check(&mut state, &mut claude_job, updates.clone()),
                Ok(false) => {}
                Err(error) => {
                    state.protection_observed = false;
                    state.notice = format!("保护状态异常：{error:#}");
                    state.notice_error = true;
                    state.notice_critical = true;
                    state.logs.push_back(("ERROR".into(), state.notice.clone()));
                }
            }
            if state.protection_change.is_none()
                && let Some(job) = protection_dns_job.take()
            {
                job.abort();
            }
            if let Some(change) = &state.protection_change
                && change.enabled
                && !change.verified
                && protection_dns_job.is_none()
                && change.generation.is_some_and(|old| {
                    state.session.platform["vpn_generation"]
                        .as_u64()
                        .unwrap_or(0)
                        > old
                })
                && state.session.vpn
                && state.guard_snapshot.connected
            {
                let api = state.session.api.clone();
                let token = change.deadline;
                let send = updates.clone();
                state.protection_status = "正在关闭旧连接并验证保护节点与加密 DNS…".into();
                protection_dns_job = Some(tokio::spawn(async move {
                    let result = api
                        .finish_protection_enable()
                        .await
                        .map_err(|error| format!("{error:#}"));
                    let _ = send.send(Update::ProtectionVerified(token, result)).await;
                }));
            }
            if !state.session.running || (!state.visible && !state.session.vpn) {
                if let Some(job) = refresh_job.take() {
                    job.abort();
                }
                refresh_generation += 1;
                state.session.clear_snapshot();
            } else if state.session.running && refresh_job.is_none() {
                let api = state.session.api.clone();
                let page = if state.visible { state.page } else { -1 };
                let refresh_route = state.visible && state.session.proxies.proxies.is_empty();
                let generation = refresh_generation;
                let send = updates.clone();
                refresh_job = Some(tokio::spawn(async move {
                    let result = tokio::time::timeout(
                        Duration::from_secs(3),
                        Snapshot::fetch(&api, page, refresh_route),
                    )
                    .await
                    .context("控制接口刷新超时")
                    .and_then(|result| result)
                    .map_err(|error| format!("{error:#}"));
                    let _ = send.send(Update::Snapshot(generation, result)).await;
                }));
            }
        }
        state.overview.tick(
            clash_mobile_runtime::overview::Input {
                context: clash_mobile_runtime::overview::Context {
                    running: state.session.running,
                    port: state.session.settings.mixed_port,
                    route: format!(
                        "{:?}",
                        (
                            &state.session.node,
                            &state.session.settings.active_profile,
                            &state.session.settings.run_mode,
                            state.session.vpn,
                            &state.session.platform["vpn_generation"],
                            &state.session.platform["network_revision"],
                            state.overview_epoch,
                        )
                    ),
                },
                visible: state.visible && state.page == 0,
                idle: !profile_gate.busy()
                    && !state.detecting
                    && !state.claude_busy
                    && !state.update_busy
                    && !state.geo_updating
                    && state.protection_change.is_none()
                    && state.delay_scope.is_none(),
            },
            state.session.native.clone(),
            overview_send.clone(),
            overview_refresh,
        );
        if log_port != state.session.settings.controller_port {
            if let Some(job) = log_job.take() {
                job.abort();
            }
            log_port = state.session.settings.controller_port;
            let api = state.session.api.clone();
            let send = updates.clone();
            log_job = Some(tokio::spawn(async move {
                loop {
                    if let Ok(mut response) = api.logs().await {
                        let mut frames = core::runtime::logging::LogFrames::default();
                        while let Ok(Some(chunk)) = response.chunk().await {
                            frames.push(&chunk, |line| {
                                if let Ok(value) = serde_json::from_slice::<Value>(line) {
                                    let level = value["type"]
                                        .as_str()
                                        .unwrap_or("info")
                                        .to_uppercase()
                                        .replace("WARNING", "WARN");
                                    let text: String = value["payload"]
                                        .as_str()
                                        .unwrap_or("")
                                        .chars()
                                        .take(8192)
                                        .collect();
                                    let time = core::support::timestamp();
                                    let _ = send.try_send(Update::Log(
                                        level,
                                        format!("[observed_at={time}] {text}"),
                                    ));
                                }
                            });
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }));
        }
        while state.logs.len() > 300 {
            state.logs.pop_front();
        }
        if (!state.visible && profile_completed.is_none())
            || (!immediate && last_frame.elapsed() < Duration::from_millis(100))
        {
            continue;
        }
        last_frame = std::time::Instant::now();
        let dark = match state.session.settings.theme_mode {
            Some(config::ThemeMode::Dark) => true,
            Some(config::ThemeMode::Light) => false,
            _ => state.session.platform["dark"].as_bool().unwrap_or(false),
        };
        let appearance = match state.session.settings.theme_mode {
            Some(config::ThemeMode::Dark) => 0,
            Some(config::ThemeMode::Light) => 1,
            _ => 2,
        };
        if system_bars_state != Some((appearance, dark))
            && state
                .session
                .native
                .request(json!({"op":"system-bars","dark":dark,"mode":appearance}))
                .is_ok()
        {
            system_bars_state = Some((appearance, dark));
        }
        if state.page == 4 {
            if state.rule_rows.is_none() {
                let (rows, warnings) = state.session.document().rows(&state.session.draft);
                state.rule_rows = Some(rows);
                state.rule_warnings = warnings;
            }
        } else {
            state.rule_rows = None;
            state.rule_warnings.clear();
        }
        let view = project(&state);
        state.completed = None;
        state.rule_form = None;
        state.rule_editor = None;
        let notice = std::mem::take(&mut state.notice);
        let notice_error = std::mem::take(&mut state.notice_error);
        let notice_critical = std::mem::take(&mut state.notice_critical);
        let notice_network_recovery = std::mem::take(&mut state.notice_network_recovery);
        let notice_followup = std::mem::take(&mut state.notice_followup);
        if notice_error && !notice.is_empty() {
            state
                .issue_events
                .push_back(core::support::incident(&notice));
            while state.issue_events.len() > 20 {
                state.issue_events.pop_front();
            }
        }
        let weak = weak.clone();
        let gate = profile_gate.clone();
        slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                if let Some(ticket) = profile_completed {
                    gate.finish(ticket);
                }
                ui.global::<MobileData>().set_profile_busy(gate.busy());
                apply(&ui, view);
                if !notice.is_empty() {
                    let current = ui.global::<MobileView>();
                    if !current.get_notice().is_empty()
                        && current.get_notice_error()
                        && notice_followup
                    {
                        ui.invoke_notify(format!("{}\n\n{notice}", current.get_notice()).into());
                    } else if notice_critical
                        || (!current.get_notice_modal()
                            && (!current.get_notice_error()
                                || current.get_notice().is_empty()
                                || notice_error))
                    {
                        current.set_notice_error(notice_error);
                        current.set_notice_critical(notice_critical);
                        current.set_notice_recover_network(notice_network_recovery);
                        ui.invoke_notify(notice.into());
                    }
                }
            }
        })?;
    }
    if let Some(job) = refresh_job {
        job.abort();
    }
    if let Some(job) = address_job {
        job.abort();
    }
    if let Some(job) = geo_job {
        job.abort();
    }
    if let Some(job) = checks {
        job.abort();
    }
    if let Some(job) = claude_job {
        job.abort();
    }
    if let Some(job) = protection_job {
        job.abort();
    }
    if let Some(job) = protection_dns_job {
        job.abort();
    }
    if let Some(job) = log_job {
        job.abort();
    }
    if let Some(job) = delay_job {
        job.abort();
    }
    if let Some(job) = recovery_job {
        job.abort();
    }
    if let Some(job) = update_job {
        job.abort();
    }
    if let Some(job) = subscription_job {
        job.abort();
    }
    Ok(())
}

fn start_update_check(
    port: core::network::DownloadPorts,
    include_prereleases: bool,
    generation: u64,
    send: mpsc::Sender<Update>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let suffix = if cfg!(target_arch = "aarch64") {
            "android-arm64.apk"
        } else if cfg!(target_arch = "arm") {
            "android-armv7.apk"
        } else {
            "android-x64.apk"
        };
        let result = core::update::check_with_prereleases(
            core::VERSION,
            suffix,
            Some(port),
            include_prereleases,
        )
        .await;
        let _ = send
            .send(Update::Checked(
                generation,
                result.map_err(|e| format!("{e:#}")),
            ))
            .await;
    })
}

fn rollback_protection(state: &mut State, reason: &str) -> Result<()> {
    if let Some(change) = state.protection_change.take() {
        if let Some((request, _)) = change.authorization {
            state
                .session
                .native
                .request(json!({"op":"cancel-vpn-authorization","request":request}))?;
        }
        if change.generation.is_some() {
            let mut previous = state.session.settings.clone();
            previous.claude_protection = change.before.claude_protection;
            previous.run_mode = change.before.run_mode;
            state
                .session
                .native
                .request(json!({"op":"vpn-plan","enabled":change.before_vpn}))?;
            state
                .session
                .commit_protection(previous.claude_protection, previous.run_mode)?;
            state
                .session
                .native
                .request(json!({"op":if change.before_vpn {"reconnect"} else {"disconnect"}}))?;
        }
    }
    state.protection_status = reason.to_owned();
    state.notice = reason.to_owned();
    state.notice_error = true;
    state.logs.push_back(("WARN".into(), reason.to_owned()));
    Ok(())
}

/// Returns true only after a user-requested enable has completed successfully.
fn observe_protection(state: &mut State) -> Result<bool> {
    let native = &state.session.platform["protection"];
    let active = native["enabled"].as_bool().unwrap_or(false);
    if let Some(snapshot) = state.session.protection_snapshot()? {
        state.guard_snapshot = snapshot;
    }
    if let Some(change) = &state.protection_change {
        if std::time::Instant::now() >= change.deadline {
            return rollback_protection(state, "保护切换超时，已恢复原状态").map(|_| false);
        }
        if change.generation.is_none() {
            return Ok(false);
        }
        let error = state.session.platform["vpn_error"].as_str().unwrap_or("");
        if change.enabled && !error.is_empty() {
            let message = format!("保护 VPN 连接失败：{error}");
            return rollback_protection(state, &message).map(|_| false);
        }
        let desired = change.enabled
            || change
                .before
                .claude_protection
                .as_ref()
                .is_some_and(|saved| saved.previous.proxy_mode == core::model::ProxyMode::Tun);
        let generation = state.session.platform["vpn_generation"]
            .as_u64()
            .unwrap_or(0);
        let ready = if desired {
            state.session.vpn
                && generation > change.generation.unwrap_or(0)
                && (!change.enabled
                    || (active && state.guard_snapshot.connected && change.verified))
        } else {
            !state.session.vpn && !active
        };
        if ready {
            let enabled = change.enabled;
            let auto_check = enabled && change.auto_check;
            state.protection_change = None;
            state.notice = if enabled {
                "Claude 保护已开启"
            } else {
                "Claude 保护已关闭，原网络状态已恢复"
            }
            .into();
            state.protection_status = state.notice.clone();
            return Ok(auto_check);
        }
    } else if let Some(protection) = &state.session.settings.claude_protection {
        state.protection_status = if !state.session.vpn || !active {
            "保护已配置，实时拦截未运行；请重新连接 VPN".into()
        } else if !state.guard_snapshot.error.is_empty() {
            state.guard_snapshot.error.clone()
        } else {
            format!(
                "保护节点：{} · 已拦截 {} 条",
                protection.node, state.guard_snapshot.blocked
            )
        };
    }
    Ok(false)
}

fn start_claude_check(
    state: &mut State,
    job: &mut Option<tokio::task::JoinHandle<()>>,
    send: mpsc::Sender<Update>,
) {
    if let Some(previous) = job.take() {
        previous.abort();
    }
    state.claude_generation = state.claude_generation.wrapping_add(1);
    state.claude_busy = true;
    let generation = state.claude_generation;
    state.claude_status = "正在检测，浏览器将自动采样…".into();
    let port = state.session.settings.mixed_port;
    let host = state.session.native.clone();
    *job = Some(tokio::spawn(async move {
        let result = async {
            let client = core::claude_check::client(Some(port))?;
            let browser = core::claude_check::browser::Session::start(client.clone()).await?;
            host.request(json!({"op":"open-url","url":browser.url}))?;
            let mut report = core::claude_check::Report::new(
                core::claude_check::Environment::Core,
                "Android 网络环境",
            );
            core::claude_check::network(&client, &mut report, "内核代理").await;
            core::claude_check::enhanced::extend(&client, &mut report).await;
            report
                .signals
                .extend(core::claude_check::system_dns(&client, "Android 系统").await);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
            loop {
                if let Some(sample) = browser.next_report() {
                    report.signals.extend(sample.signals);
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Ok::<_, anyhow::Error>(report)
        }
        .await;
        let _ = send
            .send(Update::Claude(
                generation,
                result.map_err(|error| error.to_string()),
            ))
            .await;
    }));
}

async fn native_event(state: &mut State, raw: &str) -> Result<()> {
    let value: Value = serde_json::from_str(raw)?;
    match value["event"].as_str().unwrap_or("") {
        "vpn-authorized" => {
            let result = &value["value"];
            let request = result["request"].as_u64();
            let matches = state
                .protection_change
                .as_ref()
                .and_then(|change| change.authorization.as_ref())
                .is_some_and(|(pending, _)| Some(*pending) == request);
            if !matches {
                return Ok(());
            }
            if state
                .protection_change
                .as_ref()
                .is_some_and(|change| std::time::Instant::now() >= change.deadline)
            {
                return rollback_protection(state, "VPN 授权等待超时，原网络状态未改变");
            }
            if !result["granted"].as_bool().unwrap_or(false) {
                let reason = result["error"]
                    .as_str()
                    .filter(|text| !text.is_empty())
                    .unwrap_or("VPN 授权已取消");
                return rollback_protection(state, reason);
            }
            let applied = (|| -> Result<()> {
                let change = state
                    .protection_change
                    .as_mut()
                    .expect("pending authorization");
                let (_, protection) = change.authorization.take().expect("matching authorization");
                ensure!(
                    state.session.settings.active_profile.as_deref()
                        == Some(protection.profile_id.as_str()),
                    "订阅已变化，请重新开启保护"
                );
                let before = state.session.settings.clone();
                change.before = before.clone();
                change.generation = Some(
                    state.session.platform["vpn_generation"]
                        .as_u64()
                        .unwrap_or(0),
                );
                change.deadline = std::time::Instant::now() + Duration::from_secs(120);
                let mut next = before;
                next.claude_protection = Some(Arc::new(protection));
                state
                    .session
                    .commit_protection(next.claude_protection, next.run_mode)?;
                state
                    .session
                    .native
                    .request(json!({"op":"vpn-plan","enabled":true}))?;
                state.guard_snapshot = Default::default();
                state.session.native.request(json!({"op":"reconnect"}))?;
                state.protection_status = "正在连接保护 VPN…".into();
                Ok(())
            })();
            if let Err(error) = applied {
                rollback_protection(state, &format!("保护开启失败：{error:#}"))?;
            }
        }
        "vpn-cancelled" => rollback_protection(state, "VPN 授权已取消，已恢复原状态")?,
        "visible" => {
            state.visible = value["value"].as_bool().unwrap_or(true);
        }
        "vpn" => {
            state.session.vpn = value["value"].as_bool().unwrap_or(false);
        }
        "error" | "critical" | "notice" | "export-error" | "export-notice" | "link-error" => {
            state.notice = value["value"].as_str().unwrap_or("").into();
            state.notice_error = matches!(
                value["event"].as_str(),
                Some("error" | "critical" | "export-error" | "link-error")
            );
            state.notice_critical = value["event"] == "critical";
            state.notice_followup = matches!(
                value["event"].as_str(),
                Some("export-error" | "export-notice" | "link-error")
            );
        }
        "file" => {
            let base = value["value"]["name"].as_str().unwrap_or("导入配置");
            let mut name = base.to_owned();
            let mut suffix = 2;
            while state.session.profiles.iter().any(|p| p.name == name) {
                name = format!("{base} ({suffix})");
                suffix += 1;
            }
            state
                .session
                .save_profile(
                    None,
                    &name,
                    "",
                    Some(value["value"]["content"].as_str().context("导入内容为空")?),
                )
                .await?;
            state.completed = Some(MobileIntent::SaveProfile);
            state.notice = "配置已导入，请选择启用".into();
        }
        _ => {}
    }
    Ok(())
}

async fn handle(state: &mut State, request: Request) -> Result<()> {
    let Request {
        intent,
        a,
        b,
        selected,
        profile_ticket: _,
    } = request;
    if intent == MobileIntent::Navigate {
        state.application_policies.clear();
    }
    if state.session.settings.claude_protection.is_some() || state.protection_change.is_some() {
        ensure!(
            mobile_allowed_during_protection(intent, &a),
            core::protection::READ_ONLY_NOTICE
        );
    }
    if (state.session.settings.claude_protection.is_some() || state.protection_change.is_some())
        && (matches!(
            intent,
            MobileIntent::RuleTextMode
                | MobileIntent::RuleEdit
                | MobileIntent::RuleAdd
                | MobileIntent::RuleSave
                | MobileIntent::RuleMove
                | MobileIntent::RuleDrop
                | MobileIntent::RuleEnable
                | MobileIntent::RuleRestore
                | MobileIntent::RuleDelete
                | MobileIntent::RuleApply
                | MobileIntent::RuleDiscard
                | MobileIntent::RuleShortcut
        ))
    {
        return Ok(());
    }
    ensure!(
        state.protection_change.is_none()
            || !matches!(
                intent,
                MobileIntent::Connect
                    | MobileIntent::Disconnect
                    | MobileIntent::RestartCore
                    | MobileIntent::Mode
                    | MobileIntent::SelectNode
                    | MobileIntent::ActivateProfile
                    | MobileIntent::UpdateProfile
                    | MobileIntent::SaveProfile
                    | MobileIntent::DeleteProfile
                    | MobileIntent::SavePorts
                    | MobileIntent::RuleApply
                    | MobileIntent::Ipv6
                    | MobileIntent::DnsOverride
                    | MobileIntent::FakeIp
                    | MobileIntent::Udp
                    | MobileIntent::ClaudeProtection
            ),
        "请等待 Claude 保护切换完成"
    );
    let session = &mut state.session;
    match intent {
        MobileIntent::ApplicationPolicy => {
            let notice = state.application_policies.request(session, &a, &b).await?;
            if !notice.is_empty() {
                state.notice = notice;
                state.completed = Some(MobileIntent::ApplicationPolicy);
            }
        }
        MobileIntent::Connect | MobileIntent::Disconnect => {
            ensure!(
                intent != MobileIntent::Disconnect || session.settings.claude_protection.is_none(),
                "请先关闭 Claude 保护，再断开 VPN"
            );
            session.native.request(
                json!({"op":if intent == MobileIntent::Connect {"connect"} else {"disconnect"}}),
            )?;
        }
        MobileIntent::RestartCore => session.initialize()?,
        MobileIntent::Mode => {
            ensure!(
                session.settings.claude_protection.is_none(),
                "请先关闭 Claude 保护，再切换运行模式"
            );
            let mut next = session.settings.clone();
            next.run_mode = ["rule", "global", "direct"]
                .get(a.parse::<usize>()?)
                .context("运行模式无效")?
                .to_string();
            session.commit_settings(next)?;
        }
        MobileIntent::SelectNode => {
            ensure!(
                session.settings.claude_protection.is_none(),
                "请先关闭 Claude 保护，再切换节点"
            );
            session.api.select(&a, &b).await?;
        }
        MobileIntent::SavePorts => {
            ensure!(!session.vpn, "请先断开 VPN，再修改端口");
            let mut next = session.settings.clone();
            next.mixed_port = a.parse()?;
            session.commit_settings(next)?;
            state.notice = "端口已保存".into();
        }
        MobileIntent::NodeSort => {
            let mut next = session.settings.clone();
            next.node_sort = *proxy_order::NodeSort::ALL
                .get(a.parse::<usize>()?)
                .context("排序无效")?;
            session.commit_settings(next)?;
        }
        MobileIntent::Query => {
            state.query = b;
            state.node_pages.clear();
            state.connection_page = 0;
            state.rule_page = 0;
        }
        MobileIntent::NodePage => {
            let value = state.node_pages.entry(a).or_default();
            *value = value.saturating_add_signed(b.parse()?);
        }
        MobileIntent::ConnectionPage => {
            state.connection_page = state.connection_page.saturating_add_signed(a.parse()?)
        }
        MobileIntent::RulePage => {
            state.rule_page = state.rule_page.saturating_add_signed(a.parse()?)
        }
        MobileIntent::RuleTextMode => {
            state.rule_form = if a.trim().is_empty() {
                None
            } else {
                let parts = rule_manager::draft_parts(&a)?;
                ensure!(
                    rule_manager::FORM_TYPES.contains(&parts.kind),
                    "此规则请继续使用文本编辑，以保留所有参数"
                );
                Some(RuleForm {
                    kind: parts.kind.into(),
                    payload: parts.payload.into(),
                    policy: parts.policy.into(),
                    no_resolve: parts.no_resolve,
                })
            };
        }
        MobileIntent::PickFile => {
            session.native.request(json!({"op":"pick-file"}))?;
        }
        MobileIntent::SaveProfile => {
            let id = session
                .profiles
                .iter()
                .find(|p| p.id == selected)
                .map(|p| p.id.clone());
            session.save_profile(id.as_deref(), &a, &b, None).await?;
            state.notice = "订阅已保存".into();
        }
        MobileIntent::ActivateProfile => {
            session.activate(&a)?;
        }
        MobileIntent::DeleteProfile => {
            let active = session.settings.active_profile.as_deref() == Some(&a);
            session.delete(&a)?;
            state.notice = if active {
                "订阅已删除，已切换到默认直连"
            } else {
                "订阅已删除"
            }
            .into();
        }
        MobileIntent::UpdateProfile => {
            session.update_profile(&a).await?;
            state.notice = "订阅已更新".into();
        }
        MobileIntent::CloseConnection => session.api.close_connection(&a).await?,
        MobileIntent::CloseAll => session.api.close_connections().await?,
        MobileIntent::ConnectionSort => state.connection_sort = a,
        MobileIntent::ConnectionDetail => state.selected = a,
        MobileIntent::Copy => {
            let key = a.clone();
            let text = if let Some(id) = a.strip_prefix("connection:") {
                let c = session
                    .connections
                    .connections
                    .iter()
                    .find(|c| c.id.as_ref() == id)
                    .context("该连接已结束")?;
                let resolved = if state.connection_address.0 == id {
                    state.connection_address.1.as_str()
                } else {
                    ""
                };
                core::connection::detail_values(c, resolved)
                    .into_iter()
                    .map(|(label, value)| {
                        format!("{label}: {}", if value.is_empty() { "—" } else { &value })
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                a
            };
            session.native.request(json!({"op":"copy","text":text}))?;
            state.copied = (state.copied.0.wrapping_add(1), key.into());
            state.notice = "已复制".into();
        }
        MobileIntent::LogLevel => state.log_level = a,
        MobileIntent::ClearLogs => state.logs.clear(),
        MobileIntent::ExportLogs => {
            session.native.request(json!({"op":"export","name":"clash-of-rust-logs.txt",
                "content":format!("Android UI: {}\n{}",PRESENTATION.lock().map(|s|s.clone()).unwrap_or_default(),
                    state.logs.iter().map(|(level,line)|format!("[{level}] {line}")).collect::<Vec<_>>().join("\n"))}))?;
        }
        MobileIntent::ExportDiagnostics
        | MobileIntent::CopyDiagnostics
        | MobileIntent::ReportIssue => {
            let root = session.store.root.clone();
            let context = json!({
                "error":a,"events":state.issue_events,"page":state.page,
                "core_running":session.running,"vpn_connected":session.vpn,
                "measurements":session.api.measurement_diagnostics(),
                "site_results":core::support::ip_results(&state.ip_results),"claude_status":state.claude_status,
                "presentation":PRESENTATION.lock().map(|value|value.clone()).unwrap_or_default(),
                "platform":session.native.request(json!({"op":"diagnostics"})).map_err(|error|format!("{error:#}")),
            });
            let lines = state
                .logs
                .iter()
                .map(|(level, line)| format!("[{level}] {line}"))
                .collect();
            let report = tokio::task::spawn_blocking(move || {
                core::support::issue_report(&root, context, lines)
            })
            .await?;
            if intent == MobileIntent::ReportIssue {
                session.native.request(json!({
                    "op":"open-url","url":core::support::issue_draft_url(&report)
                }))?;
            } else if intent == MobileIntent::CopyDiagnostics {
                let content = serde_json::to_string_pretty(&report)?;
                ensure!(content.len() <= 256 * 1024, "诊断报告较大，请使用导出诊断");
                session
                    .native
                    .request(json!({"op":"copy","text":content}))?;
                state.notice = "诊断已复制".into();
            } else {
                let content = serde_json::to_string_pretty(&report)?;
                session
                    .native
                    .request(json!({"op":"export","name":"cor-diagnose.json","content":content}))?;
            }
        }
        MobileIntent::RuleEdit => {
            let (rows, _) = session.document().rows(&session.draft);
            let (index, row) = rows
                .iter()
                .enumerate()
                .find(|(_, row)| row.id == a)
                .context("规则已不存在，请重新选择")?;
            let parts = rule_manager::parts(&row.raw)?;
            let text_mode = !rule_manager::FORM_TYPES.contains(&parts.kind);
            state.rule_editor = Some(RuleEditor {
                form: RuleForm {
                    kind: parts.kind.into(),
                    payload: parts.payload.into(),
                    policy: parts.policy.into(),
                    no_resolve: parts.no_resolve,
                },
                raw: row.raw.to_string(),
                position: index + 1,
                text_mode,
                enabled: row.enabled,
            });
        }
        MobileIntent::RuleSave => {
            let (position, enabled) = b.split_once('|').context("规则保存参数无效")?;
            session.save_rule_state(
                &selected,
                &a,
                position.trim().parse().context("插入位置应为正整数")?,
                enabled.parse()?,
            )?;
            state.notice = "已保存到草稿，请应用修改".into();
        }
        MobileIntent::RuleShortcut => {
            let index: usize = a.parse()?;
            ensure!(!session.document().builtin, "默认直连配置不支持修改规则");
            ensure!(
                !*session
                    .document()
                    .included
                    .get(index)
                    .context("快捷规则不存在")?,
                "订阅已包含此规则"
            );
            *session
                .draft
                .enabled
                .get_mut(index)
                .context("快捷规则不存在")? = b == "true";
        }
        MobileIntent::RuleDelete => {
            ensure!(
                a.starts_with("c:"),
                "订阅原规则可调整顺序；只有自定义规则可以删除"
            );
            session
                .draft
                .custom
                .retain(|r| r.id != a.trim_start_matches("c:"));
            session.draft.order.retain(|id| id != &a);
        }
        MobileIntent::RuleMove => {
            session.move_rule_to(&a, b.trim().parse().context("移动位置应为正整数")?)?
        }
        MobileIntent::RuleDrop => session.move_rule(&a, &b)?,
        MobileIntent::RuleEnable => session.toggle_rule(&a)?,
        MobileIntent::RuleRestore => session.draft.order.clear(),
        MobileIntent::RuleFilter => {
            state.rule_source = a;
            state.rule_page = 0;
        }
        MobileIntent::RuleApply => {
            session.apply_rules()?;
            state.notice = "规则已应用".into();
        }
        MobileIntent::RuleDiscard => {
            session.draft = rule_manager::ProfileRules::for_profile(
                &session.settings,
                &session.rule_profile,
                false,
            )
            .as_ref()
            .clone();
        }
        MobileIntent::RuleSelect if b == "profile" => {
            let id = session
                .profiles
                .iter()
                .find(|p| p.name == a)
                .context("订阅不存在")?
                .id
                .clone();
            session.select_rules(&id)?;
        }
        MobileIntent::FakeIp => session.set_fake_ip(a == "true")?,
        MobileIntent::FakeIpAdd
        | MobileIntent::FakeIpRemove
        | MobileIntent::FakeIpRestoreDefaults => {
            ensure!(
                session.settings.claude_protection.is_none() && state.protection_change.is_none(),
                "请先关闭 Claude 保护，再编辑排除项"
            );
            session.edit_fake_ip_exclusions(
                &a,
                match intent {
                    MobileIntent::FakeIpAdd => "add",
                    MobileIntent::FakeIpRemove => "remove",
                    _ => "restore",
                },
            )?;
            state.notice = "Fake-IP 排除项已保存".into();
        }
        MobileIntent::OpenRelease => {
            session.native.request(
                json!({"op":"open-url","url":"https://github.com/Fu021/Clash-of-Rust/releases"}),
            )?;
        }
        MobileIntent::DnsOverride | MobileIntent::Ipv6 | MobileIntent::Udp => {
            ensure!(
                !session.vpn || intent != MobileIntent::Ipv6,
                "请先断开 VPN，再修改 IPv6"
            );
            let mut next = session.settings.clone();
            match intent {
                MobileIntent::DnsOverride => next.dns_override = a == "true",
                MobileIntent::Ipv6 => next.ipv6 = a == "true",
                _ => next.udp = a == "true",
            }
            session.commit_settings(next)?;
        }
        MobileIntent::UpdatePrereleases => {
            ensure!(!state.update_busy, "请等待当前更新操作完成");
            let mut next = session.settings.clone();
            next.update_prereleases = a == "true";
            // This is an updater preference; it must not reload the core or VPN.
            session.store.save_settings(&next)?;
            session.settings = next;
            state.update_generation = state.update_generation.wrapping_add(1);
            state.update_confirmation = None;
            state.update_package = None;
            state.update_downloaded = false;
            state.update_status = "更新通道已更改，请检查更新".into();
            if let Err(error) = tokio::fs::remove_file(&state.update_path).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("Could not clear the previous update download: {error}");
            }
        }
        MobileIntent::TextScale => {} // Android font sizes belong to the OS.
        MobileIntent::Appearance | MobileIntent::Accent => {
            let mut next = session.settings.clone();
            match intent {
                MobileIntent::Appearance => {
                    next.theme_mode = Some(match a.as_str() {
                        "0" => config::ThemeMode::Dark,
                        "1" => config::ThemeMode::Light,
                        _ => config::ThemeMode::System,
                    })
                }
                MobileIntent::Accent => {
                    next.accent_color = *config::AccentColor::ALL
                        .get(a.parse::<usize>()?)
                        .context("强调色无效")?
                }
                _ => unreachable!(),
            }
            // Appearance changes do not reload the network configuration.
            session.commit_settings(next)?;
        }
        MobileIntent::Interval => {
            let mut next = session.settings.clone();
            next.delay_interval_minutes = a.parse()?;
            session.store.save_settings(&next)?;
            session.settings = next;
        }
        MobileIntent::SubscriptionInterval => {
            let mut next = session.settings.clone();
            next.subscription_update_interval_minutes = a.trim().parse()?;
            session.store.save_settings(&next)?;
            session.settings = next;
            state.notice = "订阅自动更新间隔已保存".into();
        }
        MobileIntent::UpdateInterval => {
            let mut next = session.settings.clone();
            next.update_check_interval_minutes = a.trim().parse()?;
            session.store.save_settings(&next)?;
            session.settings = next;
            state.notice = "自动检查更新间隔已保存".into();
        }
        MobileIntent::Probe => {
            let result = core::probe::website(a, session.settings.mixed_port, false).await?;
            state.probe = (result.detail, format!("{} ms", result.millis));
        }
        MobileIntent::Dns => {
            let ips = session.api.resolve_domain(&a).await?;
            state.dns = ips
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" · ");
        }
        MobileIntent::SystemNotification => {
            session.native.notify("Clash of Rust", "喵!", true)?;
        }
        MobileIntent::SiteCategory => {
            state.category = a;
            state.site_query.clear();
            state.site_region = "全部地区".into();
            state.site_status = "全部结果".into();
        }
        MobileIntent::SiteFilter => match b.as_str() {
            "query" => state.site_query = a,
            "region" => state.site_region = a,
            "status" => state.site_status = a,
            _ => {
                state.site_query.clear();
                state.site_region = "全部地区".into();
                state.site_status = "全部结果".into();
            }
        },
        MobileIntent::SiteDetail => state.site_selected = a,
        MobileIntent::ExportReport => {
            session
                .native
                .request(json!({"op":"export","name":"claude-report.json",
                "content":serde_json::to_string_pretty(&json!({"detection":state.claude,"enforcement":state.guard_snapshot,"protection_enabled":session.settings.claude_protection.is_some()}))?}))?;
        }
        MobileIntent::ClaudeProtection => {
            let Some(saved) = session.settings.claude_protection.clone() else {
                return Ok(());
            };
            ensure!(a == "false", "保护操作无效");
            let desired = saved.previous.proxy_mode == core::model::ProxyMode::Tun;
            let before = session.settings.clone();
            state.protection_change = Some(ProtectionChange {
                before: before.clone(),
                before_vpn: session.vpn,
                enabled: false,
                generation: Some(session.platform["vpn_generation"].as_u64().unwrap_or(0)),
                deadline: std::time::Instant::now() + Duration::from_secs(30),
                verified: false,
                authorization: None,
                auto_check: false,
            });
            state.protection_status = "正在恢复原网络状态…".into();
            let result = async {
                session
                    .native
                    .request(json!({"op":"vpn-plan","enabled":desired}))?;
                let mut next = before;
                next.claude_protection = None;
                next.run_mode = saved.previous.mode.clone();
                session.commit_protection(next.claude_protection, next.run_mode)?;
                session
                    .api
                    .restore_automatic_groups(&saved.locked_groups, &saved.fixed_groups)
                    .await?;
                let current: api::Proxies = session.api.get("proxies").await?;
                for (group, node) in &saved.previous.selectors {
                    if current
                        .proxies
                        .get(group.as_str())
                        .is_some_and(|proxy| proxy.now.as_str() != node)
                    {
                        session.api.select(group, node).await?;
                    }
                }
                session.api.close_connections().await?;
                session
                    .native
                    .request(json!({"op":if desired{"reconnect"}else{"disconnect"}}))?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(error) = result {
                rollback_protection(state, &format!("关闭保护失败：{error:#}"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn blocked_during_protection(state: &mut State) -> bool {
    if state.session.settings.claude_protection.is_none() && state.protection_change.is_none() {
        return false;
    }
    state.notice = core::protection::READ_ONLY_NOTICE.into();
    state.notice_error = true;
    true
}

fn mobile_allowed_during_protection(intent: MobileIntent, value: &str) -> bool {
    matches!(
        intent,
        MobileIntent::Navigate
            | MobileIntent::Appearance
            | MobileIntent::Accent
            | MobileIntent::TextScale
            | MobileIntent::NodeSort
            | MobileIntent::ClearLogs
            | MobileIntent::OverviewRefresh
            | MobileIntent::OverviewPrivacy
            | MobileIntent::Query
            | MobileIntent::NodePage
            | MobileIntent::ConnectionPage
            | MobileIntent::RulePage
            | MobileIntent::RuleFilter
            | MobileIntent::RuleSelect
            | MobileIntent::ConnectionSort
            | MobileIntent::ConnectionDetail
            | MobileIntent::Copy
            | MobileIntent::LogLevel
            | MobileIntent::ExportLogs
            | MobileIntent::ExportDiagnostics
            | MobileIntent::CopyDiagnostics
            | MobileIntent::ReportIssue
            | MobileIntent::Probe
            | MobileIntent::Dns
            | MobileIntent::SiteStart
            | MobileIntent::SiteCancel
            | MobileIntent::SiteCategory
            | MobileIntent::SiteFilter
            | MobileIntent::SiteRetest
            | MobileIntent::SiteProbe
            | MobileIntent::SiteDetail
            | MobileIntent::ClaudeStart
            | MobileIntent::ClaudeCancel
            | MobileIntent::ExportReport
            | MobileIntent::NodeDelay
            | MobileIntent::GroupDelay
            | MobileIntent::DelayAll
            | MobileIntent::SystemNotification
            | MobileIntent::CancelUpdate
            | MobileIntent::OpenRelease
    ) || (intent == MobileIntent::ClaudeProtection && value == "false")
        || (intent == MobileIntent::ApplicationPolicy
            && matches!(value, "open" | "refresh" | "query" | "edit" | "close"))
}

fn rows<T: Clone + 'static>(values: Vec<T>) -> ModelRc<T> {
    Rc::new(VecModel::from(values)).into()
}
fn stable<T: Clone + PartialEq + 'static>(old: ModelRc<T>, next: ModelRc<T>) -> ModelRc<T> {
    if let Some(model) = old.as_any().downcast_ref::<VecModel<T>>() {
        for (index, row) in next.iter().enumerate() {
            if index >= model.row_count() {
                model.push(row);
            } else if model.row_data(index).as_ref() != Some(&row) {
                model.set_row_data(index, row);
            }
        }
        while model.row_count() > next.row_count() {
            model.remove(model.row_count() - 1);
        }
        old
    } else {
        next
    }
}
fn bytes(value: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB"];
    let mut size = value as f64;
    let mut index = 0;
    while size >= 1024. && index < units.len() - 1 {
        size /= 1024.;
        index += 1;
    }
    if index == 0 {
        format!("{value} B")
    } else {
        format!("{size:.1} {}", units[index])
    }
}
fn cancel_delay(state: &mut State, job: &mut Option<tokio::task::JoinHandle<()>>) {
    if let Some(job) = job.take() {
        job.abort();
    }
    state.delay_generation = state.delay_generation.wrapping_add(1);
    state.delay_scope = None;
    state.node_delays.clear();
}
fn reset_subscription_results(
    state: &mut State,
    checks: &mut Option<tokio::task::JoinHandle<()>>,
    claude_job: &mut Option<tokio::task::JoinHandle<()>>,
) {
    if let Some(job) = checks.take() {
        job.abort();
    }
    if let Some(job) = claude_job.take() {
        job.abort();
    }
    state.generation = state.generation.wrapping_add(1);
    state.claude_generation = state.claude_generation.wrapping_add(1);
    state.detecting = false;
    state.claude_busy = false;
    state.ip_results.fill(None);
    state.claude = None;
    state.claude_status = "订阅已更新，请重新检测地区线索。".into();
    state.probe = Default::default();
    state.dns.clear();
}
fn start_delay(
    state: &mut State,
    scope: DelayScope,
    send: mpsc::Sender<Update>,
) -> tokio::task::JoinHandle<()> {
    state.delay_generation = state.delay_generation.wrapping_add(1);
    let generation = state.delay_generation;
    state.delay_scope = Some(scope.clone());
    let api = state.session.api.clone();
    tokio::spawn(async move {
        let result = match scope {
            DelayScope::All => api.delay_all_nodes().await,
            DelayScope::Group(name) => api.delay_group(&name).await,
            DelayScope::Node(name) => api.delay_node(&name).await,
        }
        .map_err(|e| format!("{e:#}"));
        let _ = send.send(Update::Delays(generation, result)).await;
    })
}
struct GroupView {
    name: String,
    group: api::Proxy,
    expanded: bool,
    nodes: Vec<(String, api::Proxy)>,
    count: usize,
    page: usize,
    testing: bool,
}
struct Projection {
    application_policies: application_policies::View,
    overview: core::overview::Presentation,
    copied: (i32, Arc<str>),
    page: i32,
    settings: config::Settings,
    running: bool,
    vpn: bool,
    profile: String,
    node: String,
    total: usize,
    upload: String,
    download: String,
    groups: Vec<GroupView>,
    profiles: Vec<config::Profile>,
    connections: Vec<api::Connection>,
    rules: Vec<(usize, rule_manager::Row)>,
    policies: Vec<String>,
    shortcuts: [bool; 5],
    shortcut_available: [bool; 5],
    dirty: bool,
    rule_warning: String,
    logs: Vec<(String, String)>,
    details: Vec<(String, String)>,
    rates: (String, String),
    platform: Value,
    rule_profile: String,
    completed: Option<MobileIntent>,
    categories: Vec<(String, String, String, i32)>,
    sites: Vec<ReportRow>,
    site_regions: Vec<String>,
    site_details: Vec<(String, String)>,
    claude_busy: bool,
    claude: Vec<ReportRow>,
    claude_counts: (usize, usize, usize),
    progress: (usize, usize),
    detecting: bool,
    exit: (String, String),
    probe: (String, String),
    dns: String,
    geo: String,
    connection_page: (usize, usize),
    rule_page: (usize, usize),
    rule_form: Option<RuleForm>,
    rule_editor: Option<RuleEditor>,
    update_status: String,
    update_busy: bool,
    update_downloading: bool,
    update_progress: f32,
    update_available: bool,
    update_preview: bool,
    update_version: String,
    update_confirm_wait: i32,
    geo_updating: bool,
    delay_busy: bool,
    site_summary: String,
    claude_status: String,
    protection_pending: bool,
    protection_status: String,
    protection_health: core::protection::status::Status,
}
struct ReportRow {
    id: String,
    title: String,
    value: String,
    detail: String,
    source: String,
    tone: i32,
    country: String,
    country_label: String,
    millis: Option<u128>,
}
fn project(state: &State) -> Projection {
    let s = &state.session;
    let query = state.query.to_lowercase();
    let mut groups = vec![];
    for (name, group) in s.proxies.proxies.iter().filter(|_| state.page == 1) {
        if group.all.is_empty()
            || s.settings
                .claude_protection
                .as_ref()
                .is_some_and(|p| p.route == name.as_str())
        {
            continue;
        }
        let indices = proxy_order::sorted_indices(
            &group.all,
            s.settings.node_sort,
            |node| {
                state
                    .node_delays
                    .get(node)
                    .copied()
                    .or_else(|| s.proxies.proxies.get(node).and_then(|n| n.delay))
            },
            |node| {
                query.is_empty()
                    || node.to_lowercase().contains(&query)
                    || name.as_str().to_lowercase().contains(&query)
            },
        );
        let count = indices.len();
        let page = state
            .node_pages
            .get(name.as_str())
            .copied()
            .unwrap_or(0)
            .min(count.saturating_sub(1) / 48);
        let nodes = indices
            .into_iter()
            .skip(page * 48)
            .take(48)
            .map(|i| {
                let name = group.all[i].to_string();
                let mut proxy = s
                    .proxies
                    .proxies
                    .get(name.as_str())
                    .cloned()
                    .unwrap_or_default();
                if let Some(&delay) = state.node_delays.get(&name) {
                    proxy.delay = Some(delay);
                }
                (name, proxy)
            })
            .collect();
        groups.push(GroupView {
            name: name.to_string(),
            group: group.clone(),
            expanded: s.active_group() == Some(name.as_str()),
            nodes,
            count,
            page,
            testing: matches!(&state.delay_scope,Some(DelayScope::All))
                || matches!(&state.delay_scope,Some(DelayScope::Group(group)) if group==name.as_str()),
        });
    }
    // Show the home page's actual outbound group first, keeping every other
    // group's relative order. Direct mode has no active proxy group to expand.
    if let Some(index) = groups.iter().position(|group| group.expanded) {
        groups[..=index].rotate_right(1);
    }
    let node = s.node.clone();
    let document = if state.page == 4 {
        Some(s.document())
    } else {
        None
    };
    let rules = state.rule_rows.as_deref().unwrap_or_default();
    let saved = rule_manager::ProfileRules::for_profile(&s.settings, &s.rule_profile, false);
    let mut connections = s
        .connections
        .connections
        .iter()
        .filter(|_| matches!(state.page, 0 | 3))
        .filter(|c| {
            query.is_empty()
                || format!(
                    "{} {} {} {:?}",
                    core::connection::name(c),
                    c.metadata.destination_ip,
                    c.rule,
                    c.chains
                )
                .to_lowercase()
                .contains(&query)
        })
        .collect::<Vec<_>>();
    let field = if state.connection_sort.starts_with("名称") {
        core::connection::Field::Name
    } else if state.connection_sort.starts_with("上传速度") {
        core::connection::Field::UploadRate
    } else if state.connection_sort.starts_with("下载量") {
        core::connection::Field::DownloadTotal
    } else {
        core::connection::Field::DownloadRate
    };
    let sort = core::connection::Sort {
        field: Some(field),
        descending: state.connection_sort.ends_with('↓') || state.connection_sort.ends_with("Z–A"),
    };
    connections.sort_by(|a, b| sort.compare(a, b));
    let connection_count = connections.len();
    let connection_page = state
        .connection_page
        .min(connection_count.saturating_sub(1) / 50);
    let connections = connections
        .into_iter()
        .skip(connection_page * 50)
        .take(50)
        .cloned()
        .collect();
    let rules = rules
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            (query.is_empty() || r.raw.to_lowercase().contains(&query))
                && (state.rule_source == "全部来源" || r.source.to_string() == state.rule_source)
        })
        .collect::<Vec<_>>();
    let rule_count = rules.len();
    let rule_page = state.rule_page.min(rule_count.saturating_sub(1) / 100);
    let rules = rules
        .into_iter()
        .skip(rule_page * 100)
        .take(100)
        .map(|(index, row)| (index, row.clone()))
        .collect();
    let mut details = vec![];
    if let Some(c) = s
        .connections
        .connections
        .iter()
        .find(|c| c.id.as_ref() == state.selected)
    {
        let resolved = if state.connection_address.0 == c.id.as_ref() {
            state.connection_address.1.as_str()
        } else {
            ""
        };
        details = core::connection::detail_values(c, resolved)
            .into_iter()
            .map(|(label, value)| (label.into(), value))
            .collect();
    }
    let report = core::ip_report::Report::from_results(&state.ip_results);
    let categories = report
        .categories
        .iter()
        .map(|c| {
            (
                c.name.to_owned(),
                format!("{} / {}", c.counts.non_red(), c.counts.total()),
                format!(
                    "可用与部分可达 {} · 其他 {}",
                    c.counts.non_red(),
                    c.counts.total() - c.counts.non_red()
                ),
                if c.counts.non_red() == c.counts.total() {
                    1
                } else {
                    2
                },
            )
        })
        .collect();
    let mut sites = vec![];
    let mut site_regions = std::collections::BTreeSet::new();
    let mut site_details = vec![];
    let mut exit = ("—".to_string(), "未检测".to_string());
    for (index, service) in core::ip_check::services().iter().enumerate() {
        let slot = state.ip_results.get(index).and_then(Option::as_ref);
        if service.id == "exit-ip"
            && let Some(Ok(value)) = slot
        {
            exit = (value.summary.clone(), value.country.clone());
        }
        let (value, detail, country, tone) = match slot {
            Some(Ok(value)) => (
                value.summary.clone(),
                value.detail.clone(),
                value.country.clone(),
                match value.state {
                    core::ip_check::State::Confirmed | core::ip_check::State::Identified => 1,
                    core::ip_check::State::Partial
                    | core::ip_check::State::Reachable
                    | core::ip_check::State::Unknown => 2,
                    _ => 3,
                },
            ),
            Some(Err(error)) => ("检测失败".into(), error.clone(), String::new(), 3),
            None => ("尚未检测".into(), String::new(), String::new(), 4),
        };
        if !country.is_empty() {
            site_regions.insert(country.clone());
        }
        if service.id == state.site_selected {
            site_details = vec![
                ("平台".into(), service.name.clone()),
                ("结果".into(), value.clone()),
                ("详情".into(), detail.clone()),
                ("分类".into(), service.group.clone()),
                ("地区".into(), country.clone()),
            ];
        }
        if state.category != "all" && service.group != state.category {
            continue;
        }
        let status_matches = match state.site_status.as_str() {
            "确认可用" => tone == 1,
            "仅网页可达" => tone == 2,
            "不可用或受限" => tone == 3,
            _ => true,
        };
        if !status_matches
            || (state.site_region != "全部地区" && country != state.site_region)
            || (!state.site_query.trim().is_empty()
                && !format!("{} {} {} {}", service.name, value, detail, country)
                    .to_lowercase()
                    .contains(&state.site_query.trim().to_lowercase()))
        {
            continue;
        }
        sites.push(ReportRow {
            id: service.id.clone(),
            title: service.name.clone(),
            value,
            detail,
            source: service.group.clone(),
            tone,
            country_label: core::probe::country_name(&country).unwrap_or_else(|| country.clone()),
            country,
            millis: slot
                .and_then(|result| result.as_ref().ok())
                .map(|result| result.millis),
        });
    }
    let mut claude = vec![];
    let mut claude_counts = (0, 0, 0);
    if let Some(report) = &state.claude {
        for (index, signal) in report.signals.iter().enumerate() {
            let country_label =
                if signal.group == core::claude_check::Group::Dns && !signal.ip.is_empty() {
                    core::probe::country_name(&signal.country).unwrap_or_default()
                } else {
                    String::new()
                };
            let tone = if report.differs(signal) {
                claude_counts.1 += 1;
                3
            } else if !signal.context_only() {
                claude_counts.0 += 1;
                1
            } else {
                claude_counts.2 += 1;
                2
            };
            claude.push(ReportRow {
                id: index.to_string(),
                title: signal.display_title().into(),
                value: if country_label.is_empty() {
                    signal.display_value().into()
                } else {
                    signal.ip.clone()
                },
                detail: signal.explanation(),
                source: signal.source.clone(),
                tone,
                country: signal.country.clone(),
                country_label,
                millis: None,
            });
        }
    }
    for (index, route) in state.guard_snapshot.routes.iter().enumerate() {
        claude_counts.1 += 1;
        claude.push(ReportRow {
            id: format!("guard-{index}"),
            title: route.display_title().into(),
            value: if route.outcome.is_empty() {
                "仅记录选路".into()
            } else {
                route.outcome.clone()
            },
            detail: route.explanation(),
            source: "实时拦截 · 内核连接记录".into(),
            tone: 3,
            country: String::new(),
            country_label: String::new(),
            millis: None,
        });
    }
    if !state.guard_snapshot.error.is_empty() {
        claude_counts.1 += 1;
        claude.push(ReportRow {
            id: "guard-status".into(),
            title: "实时保护状态".into(),
            value: state.guard_snapshot.error.clone(),
            detail: state.guard_snapshot.error.clone(),
            source: "Android 后台保护服务".into(),
            tone: 3,
            country: String::new(),
            country_label: String::new(),
            millis: None,
        });
    }
    Projection {
        application_policies: state.application_policies.view(s),
        overview: state.overview.present(s.running),
        copied: state.copied.clone(),
        page: state.page,
        settings: s.settings.clone(),
        running: s.running,
        vpn: s.vpn,
        profile: s.profile().name.clone(),
        node,
        total: s.count,
        upload: bytes(s.connections.upload_total),
        download: bytes(s.connections.download_total),
        groups,
        profiles: s.profiles.clone(),
        connections,
        rates: (
            format!("{}/s", bytes(s.rates.0)),
            format!("{}/s", bytes(s.rates.1)),
        ),
        platform: s.platform.clone(),
        rule_profile: s
            .profiles
            .iter()
            .find(|p| p.id == s.rule_profile)
            .map(|p| p.name.clone())
            .unwrap_or_default(),
        completed: state.completed,
        categories,
        sites,
        site_regions: std::iter::once("全部地区".to_owned())
            .chain(site_regions)
            .collect(),
        site_details,
        claude_busy: state.claude_busy,
        claude,
        claude_counts,
        progress: (report.totals.completed(), report.totals.total()),
        detecting: state.detecting,
        exit,
        probe: state.probe.clone(),
        dns: state.dns.clone(),
        geo: state.geo.clone(),
        rules,
        connection_page: (connection_page, connection_count),
        rule_page: (rule_page, rule_count),
        rule_form: state.rule_form.clone(),
        rule_editor: state.rule_editor.clone(),
        update_status: state.update_status.clone(),
        update_busy: state.update_busy,
        update_downloading: state.update_downloading,
        update_progress: state.update_progress,
        update_available: state.update_package.is_some(),
        update_preview: state
            .update_package
            .as_ref()
            .is_some_and(|package| package.preview),
        update_version: state
            .update_package
            .as_ref()
            .map_or_else(String::new, |package| package.version.clone()),
        update_confirm_wait: state
            .update_confirmation
            .as_ref()
            .map_or(3, |consent| consent.remaining() as i32),
        geo_updating: state.geo_updating,
        delay_busy: state.delay_scope.is_some(),
        site_summary: format!(
            "确认可用 {} · 网页可达 {} · 受限 {} · 失败 {}",
            report.totals.count(core::ip_report::Status::Available),
            report.totals.count(core::ip_report::Status::Partial)
                + report.totals.count(core::ip_report::Status::Reachable),
            report.totals.count(core::ip_report::Status::Restricted),
            report.totals.count(core::ip_report::Status::Failed)
        ),
        claude_status: state.claude_status.clone(),
        protection_pending: state.protection_change.is_some(),
        protection_status: state.protection_status.clone(),
        protection_health: core::protection::status::Status::observe(
            s.settings.claude_protection.is_some(),
            state.protection_change.is_some(),
            s.running,
            s.vpn,
            state.protection_observed.then_some((
                state.guard_snapshot.connected,
                if !state.guard_snapshot.error.is_empty() {
                    &state.guard_snapshot.error
                } else if !s.platform["protection"]["enabled"]
                    .as_bool()
                    .unwrap_or(false)
                {
                    "实时拦截未运行，请重新连接 VPN"
                } else {
                    ""
                },
            )),
        ),
        policies: document.map(|d| d.policies.clone()).unwrap_or_default(),
        shortcuts: std::array::from_fn(|index| {
            document.is_some_and(|doc| doc.included[index] || s.draft.enabled[index])
        }),
        shortcut_available: std::array::from_fn(|index| {
            document.is_some_and(|doc| !doc.builtin && !doc.included[index])
        }),
        dirty: s.draft != *saved,
        rule_warning: state.rule_warnings.join("\n"),
        logs: state
            .logs
            .iter()
            .filter(|(level, line)| {
                state.page == 5
                    && (state.log_level == "全部等级" || level == &state.log_level)
                    && (query.is_empty() || line.to_lowercase().contains(&query))
            })
            .cloned()
            .collect(),
        details,
    }
}

fn apply(ui: &AndroidWindow, p: Projection) {
    if ui.get_current_page() != p.page {
        return;
    }
    let data = ui.global::<MobileData>();
    let view = ui.global::<MobileView>();
    let policies = ui.global::<AndroidData>();
    policies.set_policies_supported(p.application_policies.supported);
    policies.set_policies_profile(p.application_policies.profile.into());
    policies.set_policies_status(p.application_policies.status.into());
    policies.set_policies_applications(stable(
        policies.get_policies_applications(),
        rows(
            p.application_policies
                .applications
                .into_iter()
                .map(Into::into)
                .collect(),
        ),
    ));
    policies.set_policies_targets(stable(
        policies.get_policies_targets(),
        rows(
            p.application_policies
                .targets
                .into_iter()
                .map(Into::into)
                .collect(),
        ),
    ));
    policies.set_policies(stable(
        policies.get_policies(),
        rows(
            p.application_policies
                .rules
                .into_iter()
                .map(|(package, name, target, status)| ApplicationPolicyItem {
                    package: package.into(),
                    name: name.into(),
                    target: target.into(),
                    status: status.into(),
                })
                .collect(),
        ),
    ));
    if policies.get_policies_revision() != p.application_policies.revision {
        policies.set_policies_revision(p.application_policies.revision);
        policies.set_policies_application(p.application_policies.form.0.into());
        policies.set_policies_target(p.application_policies.form.1.into());
        policies.set_policies_reject_missing(p.application_policies.form.2);
        policies.set_policies_query("".into());
        if view.get_detail() == 19 {
            view.set_draft_dirty(false);
        }
    }
    let overview_row = |entry: core::overview::Item| OverviewItem {
        id: entry.id.into(),
        title: entry.title.into(),
        value: entry.value.into(),
        detail: entry.detail.into(),
        tone: entry.tone,
        delay: entry.delay.into(),
        delay_tone: entry.delay_tone,
        flagged: !entry.flag.is_empty(),
        flag: entry.flag.into(),
        fogged: entry.fogged,
    };
    data.set_overview_network(stable(
        data.get_overview_network(),
        rows(p.overview.network.into_iter().map(overview_row).collect()),
    ));
    data.set_overview_services(stable(
        data.get_overview_services(),
        rows(p.overview.services.into_iter().map(overview_row).collect()),
    ));
    data.set_overview_status(p.overview.status.into());
    data.set_overview_busy(p.overview.busy);
    data.set_overview_hidden(p.overview.hidden);
    if view.get_copy_revision() != p.copied.0 {
        view.set_copy_value(p.copied.1.as_ref().into());
        view.set_copy_revision(p.copied.0);
    }
    data.set_working(data.get_profile_busy());
    data.set_vpn_pending(p.platform["vpn_pending"].as_bool().unwrap_or(false));
    data.set_geo_updating(p.geo_updating);
    data.set_delay_busy(p.delay_busy);
    data.set_probe_status(p.probe.0.into());
    data.set_probe_time(p.probe.1.into());
    data.set_dns_result(p.dns.into());
    data.set_geo_status(p.geo.into());
    data.set_site_summary(p.site_summary.into());
    data.set_claude_status(p.claude_status.into());
    data.set_claude_busy(p.claude_busy);
    data.set_site_regions(stable(
        data.get_site_regions(),
        rows(p.site_regions.into_iter().map(Into::into).collect()),
    ));
    data.set_site_details(stable(
        data.get_site_details(),
        rows(
            p.site_details
                .into_iter()
                .map(|(label, value)| DetailItem {
                    label: label.into(),
                    value: value.into(),
                })
                .collect(),
        ),
    ));
    data.set_rule_types(stable(
        data.get_rule_types(),
        rows(
            rule_manager::FORM_TYPES
                .iter()
                .map(|s| (*s).into())
                .collect(),
        ),
    ));
    data.set_fake_ip_removed_defaults(p.settings.fake_ip_disabled_defaults.len() as i32);
    data.set_fake_ip_exclusions(stable(
        data.get_fake_ip_exclusions(),
        rows(
            p.settings
                .fake_ip_exclusion_entries()
                .into_iter()
                .map(|entry| FakeIpExclusionItem {
                    domain: entry.domain.into(),
                    source: match (entry.custom, entry.dns_override) {
                        (true, true) => "自定义、DNS 覆盖",
                        (true, false) => "自定义",
                        _ => "DNS 覆盖",
                    }
                    .into(),
                })
                .collect(),
        ),
    ));
    if p.completed == Some(MobileIntent::SiteCategory) {
        view.set_site_query("".into());
        view.set_site_region("全部地区".into());
        view.set_site_status("全部结果".into());
    }
    if p.completed == Some(MobileIntent::FakeIpAdd) {
        view.set_fake_ip_input("".into());
    }
    if matches!(
        p.completed,
        Some(
            MobileIntent::RuleEnable
                | MobileIntent::RuleDelete
                | MobileIntent::RuleMove
                | MobileIntent::RuleRestore
        )
    ) && ui.get_current_detail() == 15
    {
        ui.set_current_detail(0);
    }
    for (page, count, size, rules) in [
        (p.connection_page.0, p.connection_page.1, 50, false),
        (p.rule_page.0, p.rule_page.1, 100, true),
    ] {
        let label = format!(
            "{} / {} · {count} 条",
            page + 1,
            count.div_ceil(size).max(1)
        );
        if rules {
            data.set_rule_page(label.into());
            data.set_rule_page_index(page as i32);
            data.set_rule_previous(page > 0);
            data.set_rule_next((page + 1) * size < count);
        } else {
            data.set_connection_page(label.into());
            data.set_connection_page_index(page as i32);
            data.set_connection_previous(page > 0);
            data.set_connection_next((page + 1) * size < count);
        }
    }
    if p.completed == Some(MobileIntent::RuleTextMode) {
        let view = ui.global::<MobileView>();
        if let Some(parts) = p.rule_form {
            view.set_rule_kind(parts.kind.into());
            view.set_rule_payload(parts.payload.into());
            view.set_rule_policy(parts.policy.into());
            view.set_rule_no_resolve(parts.no_resolve);
        }
        view.set_rule_text_mode(false);
    }
    if let Some(editor) = p.rule_editor {
        let view = ui.global::<MobileView>();
        view.set_rule_kind(editor.form.kind.into());
        view.set_rule_payload(editor.form.payload.into());
        view.set_rule_policy(editor.form.policy.into());
        view.set_rule_raw(editor.raw.into());
        view.set_rule_position(editor.position.to_string().into());
        view.set_rule_text_mode(editor.text_mode);
        view.set_rule_no_resolve(editor.form.no_resolve);
        view.set_rule_enabled(editor.enabled);
        view.set_draft_dirty(false);
        ui.set_current_detail(2);
    }
    if p.completed == Some(MobileIntent::RuleSelect) {
        ui.global::<MobileView>().set_rule_selected(false);
    }
    data.set_protection_supported(true);
    data.set_protected(p.settings.claude_protection.is_some());
    data.set_protection_pending(p.protection_pending);
    if (data.get_protected() || p.protection_pending) && ui.global::<MobileView>().get_detail() == 2
    {
        ui.global::<MobileView>().set_detail(0);
    }
    data.set_protection_status(p.protection_status.into());
    data.set_protection_health(p.protection_health.state as i32);
    data.set_protection_message(p.protection_health.message.into());
    data.set_upload_rate(p.rates.0.into());
    data.set_download_rate(p.rates.1.into());
    data.set_rule_profile(p.rule_profile.into());
    let android = ui.global::<AndroidData>();
    android.set_update_status(p.update_status.into());
    android.set_update_busy(p.update_busy);
    android.set_update_downloading(p.update_downloading);
    android.set_update_progress(p.update_progress);
    android.set_update_available(p.update_available);
    data.set_update_prereleases(p.settings.update_prereleases);
    data.set_update_preview(p.update_preview);
    data.set_update_version(p.update_version.into());
    data.set_update_confirm_wait(p.update_confirm_wait);
    android.set_authorization_status(if p.vpn {
        "已连接".into()
    } else {
        "查看授权".into()
    });
    android.set_notification_speeds(p.platform["notification_speeds"].as_bool().unwrap_or(true));
    android.set_application_scope(
        p.platform["application_scope"]
            .as_str()
            .unwrap_or("全部应用")
            .into(),
    );
    if p.completed == Some(MobileIntent::SavePorts) {
        android.set_ports_dirty(false);
    }
    if matches!(
        p.completed,
        Some(MobileIntent::SaveProfile | MobileIntent::RuleSave | MobileIntent::DeleteProfile)
    ) {
        ui.global::<MobileView>().set_draft_dirty(false);
        ui.set_current_detail(0);
    }
    data.set_detecting(p.detecting);
    data.set_progress(if p.progress.1 == 0 {
        0.0
    } else {
        p.progress.0 as f32 / p.progress.1 as f32
    });
    data.set_site_progress(format!("完成 {} / {}", p.progress.0, p.progress.1).into());
    data.set_exit_ip(p.exit.0.into());
    data.set_exit_country(core::countries::country_text(&p.exit.1).into());
    data.set_exit_flag(
        core::countries::flag_emoji(&p.exit.1)
            .unwrap_or_default()
            .into(),
    );
    data.set_categories(stable(
        data.get_categories(),
        rows(
            p.categories
                .chunks(2)
                .map(|pair| {
                    let item = |c: &(String, String, String, i32)| CategoryItem {
                        id: c.0.clone().into(),
                        title: c.0.clone().into(),
                        count: c.1.clone().into(),
                        detail: c.2.clone().into(),
                        tone: c.3,
                    };
                    CategoryPair {
                        left: item(&pair[0]),
                        right: pair.get(1).map(item).unwrap_or_default(),
                    }
                })
                .collect(),
        ),
    ));
    let result = |r: ReportRow| {
        let flag = core::countries::flag_emoji(&r.country);
        ReportItem {
            id: r.id.into(),
            title: r.title.into(),
            value: r.value.into(),
            country: r.country_label.into(),
            detail: r.detail.into(),
            source: r.source.into(),
            tone: r.tone,
            flagged: flag.is_some(),
            flag: flag.unwrap_or_default().into(),
            delay: r
                .millis
                .map_or(String::new(), |millis| format!("{millis} ms"))
                .into(),
            delay_tone: r.millis.map_or(4, proxy_order::response_tone),
        }
    };
    data.set_site_results(stable(
        data.get_site_results(),
        rows(p.sites.into_iter().map(result).collect()),
    ));
    data.set_claude_results(stable(
        data.get_claude_results(),
        rows(p.claude.into_iter().map(result).collect()),
    ));
    data.set_claude_normal(p.claude_counts.0.to_string().into());
    data.set_claude_risk(p.claude_counts.1.to_string().into());
    data.set_claude_clues(p.claude_counts.2.to_string().into());

    data.set_core_running(p.running);
    if p.completed == Some(MobileIntent::RestartCore) && p.running && view.get_notice_modal() {
        view.set_notice_error(false);
        view.set_notice_critical(false);
        view.set_notice_recover_network(false);
        ui.invoke_notify("内核已启动".into());
    }
    data.set_vpn_connected(p.vpn);
    data.set_profile(p.profile.into());
    data.set_node(p.node.into());
    data.set_connection_count(p.total.to_string().into());
    data.set_upload_total(p.upload.into());
    data.set_download_total(p.download.into());
    data.set_dns_override(p.settings.dns_override);
    data.set_fake_ip(p.settings.fake_ip_enabled());
    data.set_ipv6(p.settings.ipv6);
    data.set_udp(p.settings.udp);
    if !android.get_ports_dirty() {
        data.set_mixed_port(p.settings.mixed_port.to_string().into());
    }
    if !ui.global::<MobileView>().get_interval_dirty() {
        data.set_interval(p.settings.delay_interval_minutes.to_string().into());
    }
    if p.completed == Some(MobileIntent::Interval) {
        ui.global::<MobileView>().set_interval_dirty(false);
    }
    if !ui.global::<MobileView>().get_subscription_interval_dirty()
        || p.completed == Some(MobileIntent::SubscriptionInterval)
    {
        data.set_subscription_interval(
            p.settings
                .subscription_update_interval_minutes
                .to_string()
                .into(),
        );
    }
    if p.completed == Some(MobileIntent::SubscriptionInterval) {
        ui.global::<MobileView>()
            .set_subscription_interval_dirty(false);
    }
    let update_interval_saved = p.completed == Some(MobileIntent::UpdateInterval)
        && data.get_update_interval().trim().parse::<u32>()
            == Ok(p.settings.update_check_interval_minutes);
    if !ui.global::<MobileView>().get_update_interval_dirty() || update_interval_saved {
        data.set_update_interval(p.settings.update_check_interval_minutes.to_string().into());
    }
    if update_interval_saved {
        ui.global::<MobileView>().set_update_interval_dirty(false);
    }
    data.set_mode(
        match p
            .settings
            .claude_protection
            .as_ref()
            .map_or(p.settings.run_mode.as_str(), |protection| {
                protection.run_mode()
            }) {
            "global" => 1,
            "direct" => 2,
            _ => 0,
        },
    );
    data.set_rule_dirty(p.dirty);
    data.set_rule_warning(p.rule_warning.into());
    data.set_shortcuts(stable(
        data.get_shortcuts(),
        rows(
            [
                "拦截广告",
                "私有域名直连",
                "私有 IP 直连",
                "国内域名直连",
                "国内 IP 直连",
            ]
            .into_iter()
            .map(Into::into)
            .collect(),
        ),
    ));
    data.set_shortcut_values(stable(data.get_shortcut_values(), rows(p.shortcuts.into())));
    data.set_shortcut_available(stable(
        data.get_shortcut_available(),
        rows(p.shortcut_available.into()),
    ));
    data.set_rule_profiles(stable(
        data.get_rule_profiles(),
        rows(p.profiles.iter().map(|p| p.name.clone().into()).collect()),
    ));
    data.set_rule_policies(stable(
        data.get_rule_policies(),
        rows(p.policies.into_iter().map(Into::into).collect()),
    ));
    data.set_connection_details(stable(
        data.get_connection_details(),
        rows(
            p.details
                .into_iter()
                .map(|(label, value)| DetailItem {
                    label: label.into(),
                    value: if value.is_empty() {
                        "—".into()
                    } else {
                        value.into()
                    },
                })
                .collect(),
        ),
    ));
    data.set_profiles(stable(
        data.get_profiles(),
        rows(
            p.profiles
                .into_iter()
                .map(|profile| {
                    let builtin = profile.is_default();
                    let local = profile.is_local();
                    ProfileItem {
                        id: profile.id.clone().into(),
                        name: profile.name.into(),
                        source: profile.source.clone().into(),
                        detail: if local {
                            "本地配置".into()
                        } else {
                            core::subscription::summary(profile.usage.as_deref(), profile.updated)
                                .into()
                        },
                        active: p.settings.active_profile.as_deref() == Some(&profile.id),
                        local,
                        builtin,
                    }
                })
                .collect(),
        ),
    ));
    let previous_groups = data.get_groups();
    data.set_groups(stable(
        previous_groups.clone(),
        rows(
            p.groups
                .into_iter()
                .map(|view| {
                    let GroupView {
                        name,
                        group,
                        expanded,
                        nodes,
                        count,
                        page,
                        testing,
                    } = view;
                    let group_id = name.clone();
                    let node_items = nodes
                        .into_iter()
                        .map(|(name, node)| NodeItem {
                            group: group_id.clone().into(),
                            selected: name == group.now.as_str(),
                            name: name.into(),
                            kind: node.kind.to_string().into(),
                            delay: proxy_order::delay_label(node.delay).into(),
                            tone: proxy_order::delay_tone(node.delay),
                            valid: true,
                        })
                        .collect::<Vec<_>>();
                    GroupItem {
                        id: name.clone().into(),
                        title: name.into(),
                        detail: format!("{} · {count} 个节点", group.kind).into(),
                        current: group.now.as_str().into(),
                        expanded,
                        testing,
                        nodes: stable(
                            previous_groups
                                .iter()
                                .find(|old| old.id.as_str() == group_id)
                                .map(|old| old.nodes)
                                .unwrap_or_default(),
                            rows(
                                node_items
                                    .chunks(2)
                                    .map(|pair| NodePair {
                                        left: pair[0].clone(),
                                        right: pair.get(1).cloned().unwrap_or_default(),
                                    })
                                    .collect(),
                            ),
                        ),
                        page: format!("{} / {} · {count} 个", page + 1, count.div_ceil(48).max(1))
                            .into(),
                        previous: page > 0,
                        next: (page + 1) * 48 < count,
                    }
                })
                .collect(),
        ),
    ));
    data.set_connections(stable(
        data.get_connections(),
        rows(
            p.connections
                .into_iter()
                .map(|c| ConnectionItem {
                    id: c.id.to_string().into(),
                    host: format!(
                        "{}:{}",
                        core::connection::name(&c),
                        c.metadata.destination_port
                    )
                    .into(),
                    detail: c.start.to_string().into(),
                    rule: format!("{} {}", c.rule, c.rule_payload).into(),
                    route: c
                        .chains
                        .iter()
                        .rev()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(" → ")
                        .into(),
                    up: format!("{}/s", bytes(c.rates.map(|r| r.upload).unwrap_or(0))).into(),
                    down: format!("{}/s", bytes(c.rates.map(|r| r.download).unwrap_or(0))).into(),
                    upload: bytes(c.upload).into(),
                    download: bytes(c.download).into(),
                })
                .collect(),
        ),
    ));
    data.set_rules(stable(
        data.get_rules(),
        rows(
            p.rules
                .into_iter()
                .map(|(index, row)| {
                    let parts = rule_manager::parts(&row.raw).ok();
                    RuleItem {
                        id: row.id.into(),
                        position: (index + 1).to_string().into(),
                        kind: parts.as_ref().map_or("复杂规则", |parts| parts.kind).into(),
                        value: parts
                            .as_ref()
                            .map_or(row.raw.as_ref(), |parts| parts.payload)
                            .into(),
                        policy: parts.as_ref().map_or("—", |parts| parts.policy).into(),
                        source: row.source.to_string().into(),
                        selected: false,
                        enabled: row.enabled,
                        raw: row.raw.as_ref().into(),
                    }
                })
                .collect(),
        ),
    ));
    if ui.get_current_page() == 4 && ui.global::<MobileView>().get_rule_selected() {
        let view = ui.global::<MobileView>();
        let selected = view.get_selected_id();
        if let Some(row) = data.get_rules().iter().find(|r| r.id == selected) {
            view.set_selected_rule(row);
        } else {
            view.set_rule_selected(false);
            view.set_selected_id("".into());
        }
    }
    data.set_logs(stable(
        data.get_logs(),
        rows(
            p.logs
                .into_iter()
                .map(|(level, message)| LogItem {
                    tone: if level == "ERROR" {
                        3
                    } else if level == "WARN" {
                        2
                    } else {
                        4
                    },
                    level: level.into(),
                    message: message.into(),
                    time: "".into(),
                })
                .collect(),
        ),
    ));
    let theme = ui.global::<Theme>();
    theme.set_system_dark(p.platform["dark"].as_bool().unwrap_or(false));
    ui.global::<MobileView>()
        .set_node_sort(match p.settings.node_sort {
            proxy_order::NodeSort::LatencyAscending => 0,
            proxy_order::NodeSort::LatencyDescending => 1,
            _ => 2,
        });
    theme.set_appearance(match p.settings.theme_mode {
        Some(config::ThemeMode::Dark) => 0,
        Some(config::ThemeMode::Light) => 1,
        _ => 2,
    });
    let typography = ui.global::<MobileTypography>();
    let sizes = p.platform["font_sizes"]
        .as_array()
        .filter(|values| values.len() == 31)
        .map(|values| {
            values
                .iter()
                .map(|v| v.as_f64().unwrap_or(16.) as f32)
                .collect::<Vec<_>>()
        });
    if let Some(sizes) = sizes {
        theme.set_text_scale(sizes[6] / 16.);
        typography.set_sizes(stable(typography.get_sizes(), rows(sizes)));
        typography.set_system_fonts(true);
    } else {
        theme.set_text_scale(1.);
        typography.set_system_fonts(false);
    }
    theme.set_accent_index(accent_index(p.settings.accent_color));
    // Read back the actual Slint globals; diagnostics must reveal stale presentation too.
    if let Ok(mut snapshot) = PRESENTATION.lock() {
        *snapshot = presentation_snapshot(ui).to_string();
    }
}

fn presentation_snapshot(ui: &AndroidWindow) -> Value {
    let data = ui.global::<MobileData>();
    let android = ui.global::<AndroidData>();
    let view = ui.global::<MobileView>();
    let theme = ui.global::<Theme>();
    let scale = ui.window().scale_factor();
    let size = ui.window().size();
    let mut snapshot = json!({"core_running":data.get_core_running(),"vpn":data.get_vpn_connected(),
            "page":ui.get_current_page(),"detail":ui.get_current_detail(),
            "groups":data.get_groups().row_count(),"profiles":data.get_profiles().row_count(),
            "vpn_pending":data.get_vpn_pending(),
            "delay_busy":data.get_delay_busy(),
            "logs":data.get_logs().row_count(),"dark":theme.get_dark(),"accent":theme.get_accent_index(),"text_scale":theme.get_text_scale(),
            "body_size":ui.global::<MobileTypography>().invoke_font_size(16),
            "width":size.width as f32/scale,"height":size.height as f32/scale,"scale":scale,
            "safe_top":view.get_safe_top(),"safe_bottom":view.get_safe_bottom(),
            "keyboard_inset":view.get_keyboard_inset(),
            "notice_active":!view.get_notice().is_empty(),"notice_error":view.get_notice_error(),
            "notice_modal":view.get_notice_modal()});
    let targets = json!({
            "notice_close_target":ui.get_notice_close_target().iter().collect::<Vec<_>>(),
            "profile_add_target":view.get_profile_add_target().iter().collect::<Vec<_>>(),
            "profile_name_target":view.get_profile_name_target().iter().collect::<Vec<_>>(),
            "profile_source_target":view.get_profile_source_target().iter().collect::<Vec<_>>(),
            "profile_editor_viewport":view.get_profile_editor_viewport().iter().collect::<Vec<_>>(),
            "profile_name_focused":view.get_profile_name_focused(),
            "profile_source_focused":view.get_profile_source_focused(),
            "profile_name_length":view.get_profile_name().chars().count(),
            "profile_source_length":view.get_profile_source().chars().count(),
            "settings_scroll_y":view.get_settings_scroll_y(),"settings_content_height":view.get_settings_content_height(),
            "settings_viewport_height":view.get_settings_viewport_height()});
    let applications = json!({
            "targets": DIAGNOSTIC_GEOMETRY.with(|geometry| geometry.borrow().clone()),
            "shortcuts_open":view.get_shortcuts_open(),
            "nodes_list":ui.global::<PageViews>().get_nodes_list(),
            "profiles_list":ui.global::<PageViews>().get_profiles_list(),
            "applications_system":android.get_applications_system(),
            "applications_mode":android.get_applications_mode(),
            "applications_filter":android.get_applications_filter(),
            "applications_selected":android.get_applications_selected(),
            "applications_dirty":view.get_draft_dirty(),
            "applications_saved_summary":android.get_applications_saved_summary().as_str(),
            "applications_visibility_limited":android.get_applications_visibility_limited(),
            "applications_query_permission":android.get_applications_query_permission(),
            "applications_icon_failures":android.get_applications_icon_failures(),
            "applications_icons":android.get_applications().iter().filter(|item| item.has_icon).count(),
            "applications_loading":android.get_applications_loading(),
            "applications_loaded":android.get_applications_loaded(),
            "applications_total":android.get_applications_total(),
            "applications_visible":android.get_applications().row_count(),
            "applications_scroll_y":android.get_applications_scroll_y(),
            "applications_error":android.get_applications_error().as_str()});
    let policies = json!({
        "application_policies_loaded":!android.get_policies_profile().is_empty(),
        "application_policies_supported":android.get_policies_supported(),
        "application_policies_count":android.get_policies().row_count()
    });
    for section in [targets, applications, policies] {
        if let Value::Object(fields) = section {
            snapshot.as_object_mut().unwrap().extend(fields);
        }
    }
    snapshot
}
