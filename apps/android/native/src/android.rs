use crate::{
    claude_results::{self, BrowserStatus},
    connection_detail::{
        self, CONNECTIONS_PAGE, Detail, Selection as ConnectionSelection, Status as DetailStatus,
    },
    list_presentation::{
        self, CONNECTION_PAGE_SIZE, ConnectionPageCursor, ConnectionPager, LogMessage,
    },
    network_probe::NetworkProbe,
    profile_picker::{Outcome, ProfilePicker, Settlement},
    protection_presentation::{self, PendingVpnRestoration, VpnRestoration},
    request_gate::RequestGate,
    settings_input, site_report,
    startup_events::{
        NativeEventReceiver, NativeEvents, StartupEvents, StartupRecovery, next_vpn_authorization,
        vpn_authorization_result,
    },
    startup_protection::{
        ProtectionDeadline, ProtectionJob, RetryOutcome, RollbackProgress, SavedProtectionRecovery,
        VpnBaseline, VpnStartTimeout, VpnTransition,
    },
};
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
    sync::{Arc, Condvar, LazyLock, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, watch};

#[path = "application_policies.rs"]
mod application_policies;
#[path = "application_routing.rs"]
mod application_routing;
#[path = "../../../../crates/client-ui/src/fonts.rs"]
mod fonts;
#[path = "language_ui.rs"]
mod language_ui;
#[path = "live_language.rs"]
mod live_language;

slint::include_modules!();
static EVENTS: LazyLock<NativeEvents> = LazyLock::new(NativeEvents::new);
// Also included in exported diagnostics. Contains view counts/status, never URLs or secrets.
static PRESENTATION: Mutex<String> = Mutex::new(String::new());
static FIRST_APPEARANCE: Mutex<Option<(bool, i32)>> = Mutex::new(None);
static UI_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SESSION_ATTEMPTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CLAUDE_BROWSER_LAUNCHES: claude_results::BrowserLaunches =
    claude_results::BrowserLaunches::new();
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
    static DISPLAYED_CONNECTION_PAGE: std::cell::Cell<Option<ConnectionPageCursor>> = const { std::cell::Cell::new(None) };
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
pub extern "system" fn Java_org_clashofrust_android_CoreBridge_nativeColdRuntime(
    mut unowned_env: EnvUnowned<'_>,
    _: JClass<'_>,
    root: JString<'_>,
    application_rules_supported: jni::sys::jboolean,
) -> jni::sys::jstring {
    unowned_env
        .with_env(|env| {
            let root = root.try_to_string(env)?;
            let response = match Session::cold_android_runtime(
                std::path::Path::new(&root),
                application_rules_supported,
            ) {
                Ok((raw, settings)) => json!({
                    "config":raw,
                    "ipv6":clash_mobile_runtime::effective_ipv6(&settings),
                    "protected":settings.claude_protection.is_some(),
                }),
                Err(error) => json!({"error":format!("{error:#}")}),
            };
            env.new_string(response.to_string()).map(JString::into_raw)
        })
        .resolve::<jni::errors::LogErrorAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_clashofrust_android_MainActivity_nativeEvent(
    mut unowned_env: EnvUnowned<'_>,
    _: JClass<'_>,
    text: JString<'_>,
) {
    unowned_env
        .with_env(|env| -> jni::errors::Result<()> {
            match serde_json::from_str::<Value>(&text.try_to_string(env)?) {
                Ok(ref event) if event["event"] == "claude-browser-open-failed" => {
                    if let Some(ticket) = event["value"].as_i64() {
                        CLAUDE_BROWSER_LAUNCHES.open_failed(ticket);
                    }
                }
                Ok(event) => EVENTS.dispatch(event),
                Err(error) => log::warn!("Invalid Android event: {error}"),
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
    connection_page: Option<ConnectionPageCursor>,
    profile_ticket: Option<u64>,
}
struct ProtectionChange {
    kind: ProtectionChangeKind,
    transition: Option<VpnTransition>,
    deadline: ProtectionDeadline,
    verified: bool,
    authorization: Option<(i64, ProtectionAuthorization)>,
    job: Option<ProtectionJob>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProtectionChangeKind {
    Enable,
    Restore,
    Disable,
}
enum ProtectionAuthorization {
    Enable(Box<core::claude_check::protection::Protection>),
    Restore,
}
struct State {
    session: Session,
    presentation_language: config::Language,
    application_policies: application_policies::Editor,
    overview: clash_mobile_runtime::overview::Monitor,
    overview_epoch: u64,
    page: i32,
    query: String,
    notice: String,
    notice_error: bool,
    notice_critical: bool,
    notice_network_recovery: bool,
    notice_followup: bool,
    issue_events: VecDeque<Value>,
    system_incidents: crate::incident_delivery::Delivery,
    copied: (i32, Arc<str>),
    logs: VecDeque<(String, LogMessage)>,
    log_level: String,
    connection_sort: String,
    connection_detail: Option<Detail>,
    ip_results: Vec<core::ip_report::ResultSlot>,
    claude: claude_results::Results,
    claude_status: ClaudeStatus,
    claude_browser_status: BrowserStatus,
    protection_change: Option<ProtectionChange>,
    saved_protection_recovery: SavedProtectionRecovery,
    protection_disable_confirmation: Option<PendingVpnRestoration>,
    protection_status: String,
    protection_rollback_error: Option<String>,
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
    probe: NetworkProbe,
    probe_busy: bool,
    network_tool: i32,
    geo: Option<String>,
    geo_updating: bool,
    node_pages: BTreeMap<String, usize>,
    node_delays: BTreeMap<String, u32>,
    delay_generation: u64,
    delay_scope: Option<DelayScope>,
    group_expansion: BTreeMap<String, bool>,
    group_page: usize,
    profile_page: usize,
    connection_page: ConnectionPager,
    rule_page: usize,
    rule_rows: Option<Vec<rule_manager::Row>>,
    rule_warnings: Vec<String>,
    rule_form: Option<RuleForm>,
    rule_editor: Option<RuleEditor>,
    update_package: Option<core::update::Package>,
    update_status: UpdateStatus,
    update_busy: bool,
    update_downloading: bool,
    update_progress: f32,
    update_downloaded: bool,
    update_path: PathBuf,
    update_generation: u64,
    update_confirmation: Option<core::update::channel::Consent>,
    profile_picker: ProfilePicker,
    picked_profile_name: Option<String>,
}

#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    await_previous_ui();
    SESSION_ATTEMPTS.store(0, std::sync::atomic::Ordering::Release);
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
    let preference = config::LanguagePreference::read(&root).unwrap_or_default();
    let language = live_language::resolve(preference, native.as_ref());
    slint::android::init(app)?;
    fonts::register_emoji()?;
    let window = AndroidWindow::new()?;
    language_ui::apply(&window, language)?;
    language_ui::install(&window);
    let views = window.global::<PageViews>();
    match native.request(json!({"op":"page-views"})) {
        Ok(saved) => {
            views.set_nodes_list(saved["nodes_list"].as_bool().unwrap_or(false));
            views.set_profiles_list(saved["profiles_list"].as_bool().unwrap_or(false));
        }
        Err(error) => log::warn!("Could not restore page views: {error:#}"),
    }
    let view_host = native.clone();
    window
        .global::<MobileView>()
        .on_page_views_changed(move |nodes_list, profiles_list| {
            if let Err(error) = view_host.request(json!({
                "op":"save-page-views",
                "nodes_list":nodes_list,
                "profiles_list":profiles_list,
            })) {
                log::warn!("Could not save page views: {error:#}");
            }
        });
    window
        .global::<MobileData>()
        .set_language(preference.tag().into());
    DIAGNOSTIC_GEOMETRY.with(|geometry| geometry.borrow_mut().clear());
    window.global::<UiGeometry>().on_measured(|key, bounds| {
        if !(key.starts_with("preview-")
            || key.starts_with("application-polic")
            || matches!(
                key.as_str(),
                "nodes-view"
                    | "profiles-view"
                    | "connections-pagination:next"
                    | "connections-pagination:previous"
                    | "connections-sort"
                    | "popup:connections-sort"
                    | "option:connections-sort/1"
                    | "rule-shortcuts-open"
                    | "notice-retry-core"
                    | "retry-core"
                    | "retry-protection"
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
    window
        .global::<MobileData>()
        .set_connection_detail_snapshots(true);
    window.global::<MobileData>().set_working(true);
    let (commands, receive) = mpsc::channel::<Request>(32);
    let (show_detail, visible_detail) = watch::channel(None);
    let dismiss_detail = show_detail.clone();
    window
        .global::<MobileActions>()
        .on_dismiss_connection_detail(move |id, request| {
            connection_detail::dismiss(&dismiss_detail, id.as_str(), request);
        });
    let profile_gate = Arc::new(RequestGate::default());
    let recovery = Arc::new(StartupRecovery::default());
    let (native_events, visibility) = EVENTS.connect();
    let weak = window.as_weak();
    let gate = profile_gate.clone();
    let startup_recovery = recovery.clone();
    window
        .global::<MobileActions>()
        .on_request(move |intent, a, b| {
            let Some(ui) = weak.upgrade() else { return };
            let mut startup_retry = false;
            if !startup_recovery.ready() {
                match intent {
                    MobileIntent::Navigate => {
                        startup_recovery.navigate(ui.get_current_page());
                        if startup_recovery.opening() {
                            return;
                        }
                    }
                    MobileIntent::RestartCore => {
                        if !startup_recovery.retry() {
                            return;
                        }
                        startup_retry = true;
                    }
                    MobileIntent::ExportDiagnostics | MobileIntent::ReportIssue
                        if !startup_recovery.opening() => {}
                    _ => return,
                }
            }
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
            let detail_request = if intent == MobileIntent::ConnectionDetail {
                b.as_str()
                    .parse::<i32>()
                    .ok()
                    .map(|request| (a.clone(), request))
            } else {
                None
            };
            if let Ok(command) = commands.try_reserve() {
                if let Some((id, request)) = detail_request {
                    show_detail.send_replace(Some(ConnectionSelection {
                        id: id.to_string(),
                        request,
                    }));
                    connection_detail::ui::begin_loading(&ui, id, request);
                }
                command.send(Request {
                    intent,
                    a: a.into(),
                    b: b.into(),
                    selected: ui.get_selected_id().into(),
                    connection_page: if intent == MobileIntent::ConnectionPage {
                        DISPLAYED_CONNECTION_PAGE.get()
                    } else {
                        None
                    },
                    profile_ticket,
                });
            } else {
                if detail_request.is_some() {
                    show_detail.send_replace(None);
                }
                if startup_retry {
                    startup_recovery.fail();
                }
                if let Some(ticket) = profile_ticket {
                    gate.finish(ticket);
                }
                ui.global::<MobileData>().set_profile_busy(gate.busy());
                ui.global::<MobileData>().set_working(gate.busy());
                ui.invoke_notify(
                    language_ui::current(&ui)
                        .text(
                            "操作过于频繁，请稍后再试",
                            "Too many requests; try again shortly",
                        )
                        .into(),
                );
            }
        });
    let host = native.clone();
    let startup_recovery = recovery.clone();
    let routing_window = window.as_weak();
    let application_editor = Arc::new(Mutex::new(application_routing::Editor::default()));
    window
        .global::<AndroidActions>()
        .on_request(move |intent, value| {
            if !startup_recovery.ready() && intent != AndroidIntent::Background {
                return;
            }
            if application_routing::handles(intent) {
                if let Some(ui) = routing_window.upgrade()
                    && let Err(error) = application_routing::request(
                        &application_editor,
                        host.clone(),
                        &ui,
                        intent,
                        value.as_str(),
                        language_ui::current(&ui),
                    )
                {
                    ui.global::<AndroidData>()
                        .set_applications_error(format!("{error:#}").into());
                    application_routing::report_error(host.as_ref(), &format!("{error:#}"));
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
                let mut receive = receive;
                let mut native_events = native_events;
                tokio::select! {
                    biased;
                    _ = stopped => Ok(()),
                    result = async {
                        let mut startup = Startup { root, update_path, language };
                        let Some(session) = open_session(
                            &startup,
                            &native,
                            &mut receive,
                            &mut native_events,
                            &weak,
                            &profile_gate,
                            &recovery,
                        ).await? else {
                            return Ok(());
                        };
                        startup.language = live_language::resolve(
                            session.session.settings.language,
                            native.as_ref(),
                        );
                        run_loop(
                            startup,
                            session,
                            SessionInputs { receive, native_events, visibility, visible_detail },
                            weak,
                            profile_gate,
                            recovery,
                        ).await
                    } => result,
                }
            })
        })();
        if let Err(error) = result {
            log::error!("Mobile runtime stopped: {error:#}");
        }
    })?;
    let result = window.run();
    EVENTS.disconnect();
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
    Snapshot(
        u64,
        Option<ConnectionSelection>,
        std::result::Result<Snapshot, String>,
    ),
    Ip(u64, usize, core::ip_report::ResultSlot),
    IpDone(u64),
    ClaudeNetwork(u64, std::result::Result<core::claude_check::Report, String>),
    ClaudeBrowser(
        i64,
        std::result::Result<core::claude_check::Report, BrowserStatus>,
    ),
    ProtectionPrepared(
        String,
        std::result::Result<core::claude_check::protection::Protection, String>,
    ),
    ProtectionVerified(String, String, std::result::Result<(), String>),
    ProtectionRollbackVerified(String, String, Result<()>),
    Address(
        ConnectionSelection,
        connection_detail::AddressRequest,
        String,
    ),
    NetworkProbe(
        u64,
        std::result::Result<core::network_tools::Report, String>,
    ),
    Log(String, String),
    Geo(u64, std::result::Result<tempfile::TempDir, String>),
    Checked(
        u64,
        std::result::Result<Option<core::update::Package>, String>,
    ),
    DownloadProgress(u64, core::update::Progress),
    Downloaded(u64, std::result::Result<(), String>),
    IncidentPresented(u64),
}

#[derive(Clone)]
enum ClaudeStatus {
    NotChecked,
    Checking,
    ProtectionPaused,
    Stopped,
    Complete,
    Failed(String),
    SubscriptionChanged,
}

impl ClaudeStatus {
    fn render(&self, language: config::Language) -> String {
        match self {
            Self::NotChecked => language.text("尚未检测", "Not checked").into(),
            Self::Checking => language.text("正在检测网络…", "Checking network…").into(),
            Self::ProtectionPaused => language
                .text(
                    "保护切换或恢复待处理，网络检测已暂停",
                    "Network checks are paused while a protection change or recovery is pending",
                )
                .into(),
            Self::Stopped => language
                .text("网络检测已停止", "Network check stopped")
                .into(),
            Self::Complete => language
                .text("网络检测已完成", "Network check complete")
                .into(),
            Self::Failed(error) => format!(
                "{}{}",
                language.text("检测失败：", "Check failed: "),
                core::error_message::localize(error, language)
            ),
            Self::SubscriptionChanged => language
                .text(
                    "订阅已更新，请重新检测地区线索。",
                    "Subscription updated; run the check again.",
                )
                .into(),
        }
    }
}

#[derive(Clone)]
enum UpdateStatus {
    NotChecked,
    Checking,
    Available {
        version: String,
        preview: bool,
        size: u64,
    },
    NoUpdate,
    CheckFailed(String),
    ChannelChanged,
    ProtectionPaused,
    Cancelled,
    PreparingDownload,
    Connecting(core::update::Progress),
    Downloading(core::update::Progress),
    Downloaded,
    Failed(String),
}

impl UpdateStatus {
    fn render(&self, language: config::Language) -> String {
        match self {
            Self::NotChecked => language.text("尚未检查更新", "Updates not checked").into(),
            Self::Checking => language
                .text("正在检查更新…", "Checking for updates…")
                .into(),
            Self::Available {
                version,
                preview,
                size,
            } => format!(
                "{} · {:.1} MiB",
                core::update::channel::description_in(version, *preview, core::VERSION, language),
                *size as f64 / 1024.0 / 1024.0
            ),
            Self::NoUpdate => language
                .text(
                    "当前更新通道没有可用的 Android 更新",
                    "No Android update is available in the current channel",
                )
                .into(),
            Self::CheckFailed(error) => {
                crate::update_presentation::check_failure(error, language).into_owned()
            }
            Self::ChannelChanged => language
                .text(
                    "更新通道已更改，请检查更新",
                    "Update channel changed; check for updates",
                )
                .into(),
            Self::ProtectionPaused => language
                .text(
                    "Claude 保护期间暂停客户端更新",
                    "Updates are paused while Claude protection is active",
                )
                .into(),
            Self::Cancelled => language
                .text(
                    "下载已取消，可重新下载",
                    "Download cancelled; you can retry",
                )
                .into(),
            Self::PreparingDownload => language.text("正在下载更新…", "Downloading update…").into(),
            Self::Connecting(progress) => format!(
                "{} · {}",
                language.text("正在连接下载", "Connecting to download"),
                progress.routing_in(language)
            ),
            Self::Downloading(progress) => format!(
                "{} {}/{} · {}/s · {}",
                language.text("下载", "Download"),
                bytes(progress.received),
                bytes(progress.total),
                bytes(progress.bytes_per_second),
                progress.routing_in(language)
            ),
            Self::Downloaded => language
                .text(
                    "下载完成，请在系统界面确认安装",
                    "Download complete; confirm installation in the system dialog",
                )
                .into(),
            Self::Failed(error) => crate::update_presentation::download_failure(error, language),
        }
    }
}

struct Startup {
    root: PathBuf,
    update_path: PathBuf,
    language: config::Language,
}

struct OpenedSession {
    session: Session,
    incidents: VecDeque<Value>,
    pending_events: StartupEvents,
    recovered: bool,
}

fn show_startup_failure(
    weak: &slint::Weak<AndroidWindow>,
    profile_gate: &Arc<RequestGate>,
    recovery: &Arc<StartupRecovery>,
    error: String,
    detail: Option<String>,
    attempt_failed: bool,
) -> Result<()> {
    let weak = weak.clone();
    let gate = profile_gate.clone();
    let recovery = recovery.clone();
    slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            if attempt_failed {
                recovery.fail();
            }
            let language = language_ui::current(&ui);
            let message = crate::startup_presentation::failure(&error, detail.as_deref(), language);
            let data = ui.global::<MobileData>();
            data.set_profile_busy(gate.busy());
            data.set_working(recovery.opening() || gate.busy());
            let view = ui.global::<MobileView>();
            view.set_notice_error(true);
            view.set_notice_critical(true);
            view.set_notice_recover_network(false);
            ui.invoke_notify(message.into());
        }
    })?;
    Ok(())
}

async fn open_session(
    startup: &Startup,
    native: &Arc<AndroidHost>,
    receive: &mut mpsc::Receiver<Request>,
    native_events: &mut NativeEventReceiver,
    weak: &slint::Weak<AndroidWindow>,
    profile_gate: &Arc<RequestGate>,
    recovery: &Arc<StartupRecovery>,
) -> Result<Option<OpenedSession>> {
    let mut failed = false;
    let mut incidents = VecDeque::new();
    let mut pending_events = StartupEvents::default();
    loop {
        SESSION_ATTEMPTS.fetch_add(1, std::sync::atomic::Ordering::Release);
        let error = match Session::open(startup.root.clone(), native.clone()) {
            Ok(session) => {
                return Ok(Some(OpenedSession {
                    session,
                    incidents,
                    pending_events,
                    recovered: failed,
                }));
            }
            Err(error) => format!("{error:#}"),
        };
        failed = true;
        log::error!("Mobile session initialization failed: {error}");
        incidents.push_back(core::support::incident(&error, true));
        if incidents.len() > 20 {
            incidents.pop_front();
        }
        if let Err(notification_error) =
            native.request(json!({"op":"notify-incident","critical":true}))
        {
            log::warn!("Incident notification unavailable: {notification_error:#}");
        }
        show_startup_failure(weak, profile_gate, recovery, error.clone(), None, true)?;
        loop {
            let request = tokio::select! {
                request = receive.recv() => {
                    let Some(request) = request else {
                        return Ok(None);
                    };
                    request
                }
                Some(event) = native_events.recv() => {
                    match event["event"].as_str() {
                        Some("back") => {
                            let weak = weak.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(ui) = weak.upgrade() {
                                    ui.invoke_native_back();
                                }
                            })?;
                        }
                        Some("export-error" | "export-notice" | "link-error") => {
                            let detail = event["value"].as_str().map(str::to_owned);
                            show_startup_failure(weak, profile_gate, recovery, error.clone(), detail, false)?;
                        }
                        _ => pending_events.record(event),
                    }
                    continue;
                }
            };
            if let Some(ticket) = request.profile_ticket {
                profile_gate.finish(ticket);
            }
            if request.intent == MobileIntent::RestartCore {
                break;
            }
            let detail = if matches!(
                request.intent,
                MobileIntent::ExportDiagnostics | MobileIntent::ReportIssue
            ) {
                let result = async {
                    let root = startup.root.clone();
                    let context = json!({
                        "error": error.clone(),
                        "events": incidents.clone(),
                        "platform": native.request(json!({"op":"diagnostics"}))
                            .map_err(|error| format!("{error:#}")),
                    });
                    let report = tokio::task::spawn_blocking(move || {
                        core::support::issue_report(&root, context, vec![])
                    })
                    .await?;
                    if request.intent == MobileIntent::ReportIssue {
                        native.request(json!({
                            "op":"open-url","url":core::support::issue_draft_url(&report)
                        }))?;
                    } else {
                        native.request(json!({
                            "op":"export","name":"cor-diagnose.json",
                            "content":serde_json::to_string_pretty(&report)?
                        }))?;
                    }
                    Ok::<(), anyhow::Error>(())
                }
                .await;
                match result {
                    Ok(()) => {
                        let weak = weak.clone();
                        let recovery = recovery.clone();
                        let gate = profile_gate.clone();
                        slint::invoke_from_event_loop(move || {
                            if let Some(ui) = weak.upgrade() {
                                ui.global::<MobileData>()
                                    .set_working(recovery.opening() || gate.busy());
                            }
                        })?;
                        continue;
                    }
                    Err(error) => Some(format!("{error:#}")),
                }
            } else {
                None
            };
            show_startup_failure(weak, profile_gate, recovery, error.clone(), detail, false)?;
        }
    }
}

struct SessionInputs {
    receive: mpsc::Receiver<Request>,
    native_events: NativeEventReceiver,
    visibility: watch::Receiver<bool>,
    visible_detail: watch::Receiver<Option<ConnectionSelection>>,
}

async fn run_loop(
    startup: Startup,
    opened: OpenedSession,
    inputs: SessionInputs,
    weak: slint::Weak<AndroidWindow>,
    profile_gate: Arc<RequestGate>,
    recovery: Arc<StartupRecovery>,
) -> Result<()> {
    let SessionInputs {
        mut receive,
        mut native_events,
        mut visibility,
        mut visible_detail,
    } = inputs;
    let OpenedSession {
        session,
        incidents,
        mut pending_events,
        mut recovered,
    } = opened;
    let saved_protection_recovery =
        SavedProtectionRecovery::for_process(&session.store.root, startup.language);
    let mut state = State {
        session,
        presentation_language: startup.language,
        overview: Default::default(),
        overview_epoch: 0,
        application_policies: Default::default(),
        page: recovery.page(),
        query: String::new(),
        notice: String::new(),
        notice_error: false,
        notice_critical: false,
        notice_network_recovery: false,
        notice_followup: false,
        issue_events: incidents,
        system_incidents: crate::incident_delivery::Delivery::default(),
        copied: Default::default(),
        logs: VecDeque::new(),
        log_level: "全部等级".into(),
        connection_sort: "下载速度 ↓".into(),
        connection_detail: None,
        ip_results: vec![None; core::ip_check::services().len()],
        claude: Default::default(),
        claude_status: ClaudeStatus::NotChecked,
        claude_browser_status: BrowserStatus::Idle,
        protection_change: None,
        saved_protection_recovery,
        protection_disable_confirmation: None,
        protection_status: String::new(),
        protection_rollback_error: None,
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
        visible: *visibility.borrow_and_update(),
        completed: None,
        probe: Default::default(),
        probe_busy: false,
        network_tool: 0,
        geo: None,
        geo_updating: false,
        node_pages: BTreeMap::new(),
        node_delays: BTreeMap::new(),
        delay_generation: 0,
        delay_scope: None,
        group_expansion: BTreeMap::new(),
        group_page: 0,
        profile_page: 0,
        connection_page: ConnectionPager::default(),
        rule_page: 0,
        rule_rows: None,
        rule_warnings: Vec::new(),
        rule_form: None,
        rule_editor: None,
        update_package: None,
        update_status: UpdateStatus::NotChecked,
        update_busy: false,
        update_downloading: false,
        update_progress: 0.0,
        update_downloaded: false,
        update_path: startup.update_path,
        update_generation: 0,
        update_confirmation: None,
        profile_picker: ProfilePicker::default(),
        picked_profile_name: None,
    };
    if let Some(error) = state.saved_protection_recovery.last_error() {
        present_rollback(&mut state, RollbackProgress::Failed(error));
    }
    let attached = if state.saved_protection_recovery.is_active() {
        state.session.observe_native()
    } else {
        state.session.attach_or_initialize()
    };
    if let Err(error) = attached {
        state.notice = format!(
            "{}{error:#}",
            state
                .presentation_language
                .text("内核启动失败：", "Core startup failed: ")
        );
        state.notice_error = true;
        state.notice_critical = true;
        log::error!("{}", state.notice);
        state
            .logs
            .push_back(("ERROR".into(), state.notice.clone().into()));
    }
    if let Ok(manifest) = core::geodata::read_manifest(&state.session.store.runtime()) {
        state.geo = Some(manifest.display_status());
    }
    let mut restore_protection = state.session.settings.claude_protection.is_some();
    let (updates, mut update_rx) = mpsc::channel::<Update>(32);
    let (overview_send, mut overview_rx) = mpsc::channel(16);
    let mut checks: Option<tokio::task::JoinHandle<()>> = None;
    let mut claude_jobs = ClaudeJobs::default();
    let mut rollback_dns_job: Option<ProtectionJob> = None;
    let mut address_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut log_job: Option<tokio::task::JoinHandle<()>> = None;
    let geo_gate = RequestGate::default();
    let mut geo_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut log_port = 0;
    let mut last_delay = std::time::Instant::now();
    let mut subscription_schedule = core::subscription::schedule::Schedule::default();
    let mut subscription_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut subscription_generation = 0u64;
    let mut last_subscription_check = std::time::Instant::now() - Duration::from_secs(30);
    let mut delay_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut network_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut network_generation = 0u64;
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
        let mut protection_dns_result = None;
        let mut protection_reservation_released = false;
        let mut profile_completed = None;
        let mut overview_refresh = false;
        tokio::select! {
            value = receive.recv() => {
                let Some(request) = value else { break; };
                let intent = request.intent;
                if intent == MobileIntent::ConnectionDetail
                    && !visible_detail.borrow().as_ref().is_some_and(|visible| {
                        visible.id == request.a && Some(visible.request) == request.b.parse().ok()
                    })
                {
                    continue;
                }
                profile_completed = request.profile_ticket;
                if profile_completed.is_some() {
                    subscription_generation = subscription_generation.wrapping_add(1);
                    if let Some(job) = subscription_job.take() { job.abort(); let _ = job.await; }
                }
                immediate = true;
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
                if matches!(intent, MobileIntent::Connect | MobileIntent::Disconnect
                    | MobileIntent::RestartCore | MobileIntent::Mode | MobileIntent::SelectNode
                    | MobileIntent::ActivateProfile | MobileIntent::UpdateProfile | MobileIntent::SaveProfile
                    | MobileIntent::ClaudeProtection | MobileIntent::DnsOverride | MobileIntent::FakeIp
                    | MobileIntent::FakeIpAdd | MobileIntent::FakeIpRemove | MobileIntent::FakeIpRestoreDefaults
                    | MobileIntent::Ipv6 | MobileIntent::Udp | MobileIntent::SavePorts | MobileIntent::RuleApply) {
                    network_generation = network_generation.wrapping_add(1);
                    if let Some(job) = network_job.take() { job.abort(); }
                    state.probe = Default::default();
                    state.probe_busy = false;
                }
                if intent == MobileIntent::ClaudeProtection
                    || (intent == MobileIntent::RestartCore && state.saved_protection_recovery.is_active())
                {
                    if let Some(job) = checks.take() { job.abort(); let _ = job.await; }
                    state.generation = state.generation.wrapping_add(1);
                    state.detecting = false;
                    for job in claude_jobs.stop(&mut state).into_iter().flatten() {
                        let _ = job.await;
                    }
                    if let Some(job) = address_job.take() { job.abort(); let _ = job.await; }
                    if let Some(job) = recovery_job.take() { job.abort(); let _ = job.await; }
                }
                let edited_profile = match intent {
                    MobileIntent::UpdateProfile => Some(request.a.as_str()),
                    _ => None,
                };
                let subscription_affects_active = edited_profile.is_some_and(|id| {
                    state.session.settings.active_profile.as_deref() == Some(id)
                });
                if intent == MobileIntent::ConnectionDetail
                    && let Some(job) = address_job.take()
                {
                    job.abort();
                }
                if matches!(intent, MobileIntent::RuleSelect | MobileIntent::RuleSave | MobileIntent::RuleMove
                    | MobileIntent::RuleDelete | MobileIntent::RuleDrop | MobileIntent::RuleEnable | MobileIntent::RuleRestore | MobileIntent::RuleApply | MobileIntent::RuleDiscard
                    | MobileIntent::RuleShortcut | MobileIntent::ActivateProfile | MobileIntent::UpdateProfile
                    | MobileIntent::SaveProfile | MobileIntent::DeleteProfile) {
                    state.rule_rows = None;
                }
                if matches!(intent, MobileIntent::Probe | MobileIntent::SiteStart | MobileIntent::SiteRetest
                    | MobileIntent::SiteProbe | MobileIntent::NodeDelay | MobileIntent::GroupDelay
                    | MobileIntent::DelayAll | MobileIntent::ClaudeStart | MobileIntent::ClaudeBrowserRetry)
                    && !can_run_active_probe(&state)
                {
                    let message = state.presentation_language.text(
                        "保护切换或恢复待处理，网络检测已暂停",
                        "Network checks are paused while a protection change or recovery is pending",
                    );
                    if intent == MobileIntent::Probe {
                        state.probe = NetworkProbe::Paused;
                    } else if intent == MobileIntent::ClaudeStart {
                        state.claude_status = ClaudeStatus::ProtectionPaused;
                    } else {
                        state.notice = message.into();
                        state.notice_error = true;
                    }
                } else if intent == MobileIntent::OverviewRefresh {
                    overview_refresh = true;
                } else if intent == MobileIntent::OverviewPrivacy {
                    state.overview.toggle_privacy();
                } else if intent == MobileIntent::Navigate {
                    let page: i32 = request.a.parse()?;
                    if page != CONNECTIONS_PAGE {
                        if let Some(job) = address_job.take() { job.abort(); }
                        state.connection_detail = None;
                    }
                    if state.page == 6 && page != 6 && state.probe_busy {
                        network_generation = network_generation.wrapping_add(1);
                        if let Some(job) = network_job.take() { job.abort(); }
                        state.probe = Default::default();
                        state.probe_busy = false;
                    }
                    if state.page == 8 && page != 8 {
                        claude_jobs.stop(&mut state);
                    }
                    state.page = page;
                    state.query.clear();
                    state.group_expansion.clear();
                    state.group_page = 0;
                    state.profile_page = 0;
                    state.connection_page.reset();
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
                        state.update_status = UpdateStatus::Cancelled;
                        for path in [&state.update_path, &state.update_path.with_extension("partial")] {
                            if let Err(error) = tokio::fs::remove_file(path).await
                                && error.kind() != std::io::ErrorKind::NotFound {
                                state.notice = format!("{}{error}", state.presentation_language.text("下载已取消，缓存清理失败：", "Download cancelled; cache cleanup failed: "));
                                state.notice_error = true;
                            }
                        }
                    }
                } else if matches!(intent, MobileIntent::NetworkTool | MobileIntent::ProbeCancel) {
                    network_generation = network_generation.wrapping_add(1);
                    if let Some(job) = network_job.take() { job.abort(); }
                    state.probe = Default::default();
                    state.probe_busy = false;
                    if intent == MobileIntent::NetworkTool {
                        if let Some(tool) = request.a.parse::<i32>().ok().and_then(|index| core::network_tools::Tool::at(index).ok()) {
                            state.network_tool = tool.index();
                        }
                    } else {
                        state.probe = NetworkProbe::Cancelled;
                    }
                } else if intent == MobileIntent::Probe {
                    if network_job.is_none() && state.page == 6 {
                        state.probe = Default::default();
                        state.probe_busy = true;
                        network_generation = network_generation.wrapping_add(1);
                        let generation = network_generation;
                        let target = request.a;
                        let port = state.session.running.then_some(state.session.settings.mixed_port);
                        let api = port.map(|_| state.session.api.clone());
                        let protected = state.session.settings.claude_protection.is_some();
                        let tool = core::network_tools::Tool::at(state.network_tool).unwrap_or(core::network_tools::Tool::Website);
                        let environment = core::network_tools::Environment {
                            port, api, protected, udp_enabled: state.session.settings.udp,
                        };
                        let options = core::network_tools::Options::from_fields(&request.b);
                        let send = updates.clone();
                        network_job = Some(tokio::spawn(async move {
                            let result = core::network_tools::run(tool, &target, environment, options)
                                .await.map_err(|error| format!("{error:#}"));
                            let _ = send.send(Update::NetworkProbe(generation, result)).await;
                        }));
                    }
                } else if intent == MobileIntent::ClaudeCancel {
                    claude_jobs.stop(&mut state);
                } else if intent==MobileIntent::CheckUpdate {
                    if !blocked_during_protection(&mut state) && !state.update_busy {
                        state.update_busy=true;state.update_downloading=false;state.update_status=UpdateStatus::Checking;state.update_progress=0.0;
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
                                state.notice = state.presentation_language.text("更新版本已变化，请重新检查更新", "The update changed; check for updates again").into();
                                state.notice_error = true;
                            }
                        } else if (package.preview && (!state.session.settings.update_prereleases
                            || request.b != "confirmed"
                            || !state.update_confirmation.as_ref().is_some_and(|consent| consent.accepts(&package.version))))
                            || (!package.preview && !request.b.is_empty()) {
                            state.notice = state.presentation_language.text("请先阅读预发布风险提示，并等待 3 秒后确认", "Read the pre-release warning and wait 3 seconds before confirming").into();
                            state.notice_error = true;
                        } else {
                        state.update_confirmation = None;
                        if state.update_downloaded && state.update_path.is_file() {
                            if let Err(error)=state.session.native.request(json!({"op":"install-update"})) {state.notice=format!("{error:#}");state.notice_error=true;}
                        } else {
                            state.update_busy=true;state.update_downloading=true;state.update_status=UpdateStatus::PreparingDownload;state.update_progress=0.0;
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
                    if !blocked_during_protection(&mut state) && let Some(ticket) = geo_gate.begin() {
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
                            let _=send.send(Update::Geo(ticket, result)).await;
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
                    for (index, id) in &targets {
                        if id != "exit-ip" {
                            state.ip_results[*index] = None;
                        }
                    }
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
                    if state.protection_change.is_none() && state.session.settings.claude_protection.is_none()
                        && !state.saved_protection_recovery.is_active() {
                        geo_gate.cancel();
                        cancel_delay(&mut state,&mut delay_job);
                        state.update_generation = state.update_generation.wrapping_add(1);
                        if let Some(job) = update_job.take() { job.abort(); let _ = job.await; }
                        state.update_busy = false; state.update_downloading = false;
                        state.update_downloaded = false; state.update_progress = 0.0;
                        state.update_status = UpdateStatus::ProtectionPaused;
                        if let Some(job) = geo_job.take() { job.abort(); let _ = job.await; }
                        state.geo_updating = false;
                        subscription_generation = subscription_generation.wrapping_add(1);
                        if let Some(job) = subscription_job.take() { job.abort(); let _ = job.await; }
                        state.protection_disable_confirmation = None;
                        let deadline = ProtectionDeadline::after(Duration::from_secs(120));
                        state.protection_status="正在确认当前代理节点…".into();
                        let api=state.session.api.clone();let settings=state.session.settings.clone();
                        let language=state.presentation_language;
                        let raw=state.session.raw();let send=updates.clone();
                        let job = ProtectionJob::spawn(None, |ticket| async move {
                            let result=async {
                                let raw=raw?;
                                tokio::time::timeout(Duration::from_secs(20),
                                    clash_mobile_runtime::prepare_protection(&api,&settings,&raw))
                                    .await.context(language.text("确认保护节点超时", "Timed out checking the protected node"))?
                            }.await.map_err(|error|format!("{error:#}"));
                            let _=send.send(Update::ProtectionPrepared(ticket, result)).await;
                        });
                        state.protection_change = Some(ProtectionChange {
                            kind: ProtectionChangeKind::Enable,
                            transition: None,
                            deadline,
                            verified: false,
                            authorization: None,
                            job: Some(job),
                        });
                    }
                } else if intent == MobileIntent::ClaudeStart {
                    claude_jobs.start(&mut state, updates.clone());
                } else if intent == MobileIntent::ClaudeBrowserRetry {
                    if state.page == 8 && state.claude_browser_status.retryable() {
                        claude_jobs.start_browser(&mut state, updates.clone());
                    }
                } else {
                    state.notice_error = false;
                    state.notice_critical = false;
                    state.notice_followup = matches!(intent, MobileIntent::ExportDiagnostics | MobileIntent::CopyDiagnostics | MobileIntent::ReportIssue);
                    let recovering_protection = intent == MobileIntent::RestartCore
                        && state.saved_protection_recovery.is_active();
                    match handle(&mut state, request).await {
                        Ok(()) => {
                            if intent != MobileIntent::PickFile
                                && !recovering_protection
                                && (intent != MobileIntent::RestartCore
                                    || state.protection_rollback_error.is_none())
                            {
                                state.completed = Some(intent);
                            }
                            if matches!(intent, MobileIntent::CloseConnection | MobileIntent::CloseAll)
                                && state.connection_detail.as_ref().is_some_and(|detail| detail.status() == DetailStatus::Loading)
                                && let Some(job) = address_job.take()
                            {
                                job.abort();
                            }
                            if intent == MobileIntent::RestartCore {
                                restore_protection = !recovering_protection
                                    && state.session.settings.claude_protection.is_some();
                                protection_reservation_released = recovering_protection
                                    && !state.saved_protection_recovery.is_active();
                            }
                            if subscription_affects_active {
                                reset_subscription_results(&mut state, &mut checks, &mut claude_jobs);
                            }
                            if matches!(intent,MobileIntent::SelectNode|MobileIntent::ActivateProfile|MobileIntent::UpdateProfile
                                |MobileIntent::SaveProfile|MobileIntent::DeleteProfile|MobileIntent::RestartCore|MobileIntent::Mode
                                |MobileIntent::DnsOverride|MobileIntent::FakeIp|MobileIntent::FakeIpAdd|MobileIntent::FakeIpRemove|MobileIntent::FakeIpRestoreDefaults|MobileIntent::Ipv6|MobileIntent::Udp|MobileIntent::SavePorts
                                |MobileIntent::ClaudeProtection) {cancel_delay(&mut state,&mut delay_job);}
                            if !matches!(intent, MobileIntent::Query | MobileIntent::Copy | MobileIntent::ConnectionSort
                                | MobileIntent::ConnectionPage | MobileIntent::RulePage
                                | MobileIntent::LogLevel | MobileIntent::ClearLogs | MobileIntent::ExportLogs | MobileIntent::ExportDiagnostics | MobileIntent::CopyDiagnostics | MobileIntent::ReportIssue | MobileIntent::RuleTextMode
                                | MobileIntent::RuleSelect | MobileIntent::RuleEdit | MobileIntent::RuleSave
                                | MobileIntent::RuleMove | MobileIntent::RuleDelete | MobileIntent::RuleShortcut
                                | MobileIntent::RuleDiscard | MobileIntent::NodePage | MobileIntent::NodeSort
                                | MobileIntent::GroupExpansion | MobileIntent::GroupPage | MobileIntent::GroupPagination
                                | MobileIntent::ProfilePage | MobileIntent::ProfilePagination
                                | MobileIntent::Appearance | MobileIntent::Accent | MobileIntent::TextScale
                                | MobileIntent::Language) {
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
                            state.logs.push_back(("ERROR".into(), LogMessage::ActionFailed(state.notice.clone())));
                        }
                    }
                }
            }
            Ok(()) = visibility.changed() => {
                immediate = true;
                refresh_requested = true;
            }
            Ok(()) = visible_detail.changed() => {
                connection_detail::discard_hidden(
                    &mut state.connection_detail,
                    &mut address_job,
                    visible_detail.borrow_and_update().as_ref(),
                );
                immediate = true;
            }
            Some(event) = pending_events.next(&mut native_events) => {
                immediate = true;
                refresh_requested = true;
                if event["event"] == "back" {
                    let weak=weak.clone();slint::invoke_from_event_loop(move||{if let Some(ui)=weak.upgrade(){ui.invoke_native_back();}})?;
                } else if let Err(error) = native_event(&mut state,&event).await {
                    state.notice=format!("{error:#}");state.notice_error=true;
                    state.notice_critical=event["event"]=="vpn-authorized";
                }
            }
            Some(event) = overview_rx.recv() => {
                state.overview.apply(event);
            }
            value = update_rx.recv() => {
                match value {
                    Some(Update::IncidentPresented(sequence)) => {
                        if state.system_incidents.can_confirm(sequence, state.visible) {
                            match state.session.native.request(json!({
                                "op":"acknowledge-incident","sequence":sequence
                            })) {
                                Ok(_) => {
                                    state.system_incidents.confirm(sequence, state.visible);
                                }
                                Err(error) => {
                                    log::warn!("Incident acknowledgement unavailable: {error:#}");
                                }
                            }
                        }
                    }
                    Some(Update::ProtectionVerified(ticket, request, result)) => {
                        if let Some(change) = state.protection_change.as_mut()
                            && ProtectionJob::finish(&mut change.job, &ticket, Some(&request)) {
                            protection_dns_result = Some((request, result));
                            refresh_requested = true;
                            immediate = true;
                        }
                    }
                    Some(Update::ProtectionRollbackVerified(ticket, request, result))
                        if ProtectionJob::finish(&mut rollback_dns_job, &ticket, Some(&request)) => {
                        if state.session.observe_native().is_ok() {
                            let progress = state.saved_protection_recovery.verified(
                                &state.session, &request, result,
                            );
                            present_rollback(&mut state, progress);
                        }
                        immediate = true;
                        refresh_requested = true;
                    }
                    Some(Update::Address(selection, request, value)) => {
                        if connection_detail::accept_address(
                            &mut state.connection_detail,
                            &mut address_job,
                            visible_detail.borrow().as_ref(),
                            &selection,
                            &request,
                            value,
                        ) {
                            immediate = true;
                        }
                    }
                    Some(Update::NetworkProbe(generation, result))
                        if generation == network_generation && state.page == 6 => {
                        network_job = None;
                        state.probe_busy = false;
                        state.probe = match result {
                            Ok(report) => NetworkProbe::Complete(report),
                            Err(error) => NetworkProbe::Failed(error),
                        };
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
                                        state.presentation_language.text("请检查 Claude 保护线路。", "Check the Claude protection route.")
                                    } else {
                                        state.presentation_language.text("请更换节点；更新失败后会自动尝试直连。", "Choose another node; failed updates will retry directly.")
                                    };
                                    match state.presentation_language {
                                        config::Language::SimplifiedChinese => format!("本次测速：{} 个节点不可达。{detail}", values.len()),
                                        config::Language::English => format!("{} nodes unreachable. {detail}", values.len()),
                                    }
                                } else {
                                    match state.presentation_language {
                                        config::Language::SimplifiedChinese => format!("测速完成 · {} 项，{} 项超时", values.len(), failed),
                                        config::Language::English => format!("Speed test complete · {} checked, {} timed out", values.len(), failed),
                                    }
                                };
                                state.notice_network_recovery = unavailable;
                                state.notice_error=failed > 0;
                                if unavailable && state.session.vpn && recovery_job.is_none()
                                    && can_run_active_probe(&state)
                                    && state.session.settings.claude_protection.is_none() {
                                    let api = state.session.api.clone();
                                    let settings = serde_json::to_value(&state.session.settings)?;
                                    let vpn_generation = state.session.platform["vpn_generation"].as_u64().unwrap_or(0);
                                    let sender = updates.clone();
                                    let language = state.presentation_language;
                                    recovery_job = Some(tokio::spawn(async move {
                                        let result = tokio::time::timeout(Duration::from_secs(120),
                                            api.confirm_all_nodes_unreachable()).await
                                            .context(language.text("全部节点复测超时", "Timed out rechecking all nodes"))
                                            .and_then(|result| result).map_err(|error| format!("{error:#}"));
                                        let _ = sender.send(Update::Unreachable(generation, vpn_generation, settings, result)).await;
                                    }));
                                }
                                if all {state.node_delays=values;} else {state.node_delays.extend(values);}
                            }
                            Err(error)=>{state.notice=format!("{}{error}", state.presentation_language.text("测速失败：", "Speed test failed: "));state.notice_error=true;},
                        }
                        immediate=true;refresh_requested=true;
                    }
                    Some(Update::Unreachable(generation, vpn_generation, settings, result)) if generation == state.delay_generation => {
                        recovery_job = None;
                        if state.session.vpn && can_run_active_probe(&state)
                            && state.session.settings.claude_protection.is_none()
                            && serde_json::to_value(&state.session.settings)? == settings
                            && state.session.platform["vpn_generation"].as_u64().unwrap_or(0) == vpn_generation {
                            match result {
                                Ok(true) => {
                                    state.logs.push_back(("WARN".into(), LogMessage::NodesUnreachable));
                                    state.notice = state.presentation_language.text("全部节点不可达，VPN 仍连接。请更换节点；更新失败后会自动尝试直连。", "All nodes are unreachable; the VPN remains connected. Choose another node; failed updates will retry directly.").into();
                                    state.notice_network_recovery = true;
                                },
                                Ok(false) => state.notice = state.presentation_language.text("未确认全部节点失联，可重试或更换节点。", "Not all nodes were confirmed unreachable. Retry or choose another node.").into(),
                                Err(error) => state.notice = format!("{}{error}", state.presentation_language.text("节点复测失败：", "Node recheck failed: ")),
                            }
                            state.notice_error = true;
                            immediate = true;
                            refresh_requested = true;
                        }
                    }
                    Some(Update::Snapshot(generation, selection, result)) if generation == refresh_generation => {
                        refresh_job = None;
                        immediate = true;
                        match result {
                            Ok(snapshot) => {
                                state.session.apply_snapshot(snapshot);
                                if let Some(selection) = selection
                                    && Some(&selection) == visible_detail.borrow().as_ref()
                                    && let Some(detail) = state.connection_detail.as_mut()
                                    && detail.confirm(&selection, &state.session.connections.connections)
                                {
                                    if detail.status() == DetailStatus::Ended || !detail.needs_address() {
                                        if let Some(job) = address_job.take() { job.abort(); }
                                    } else if crate::startup_protection::active_probe_allowed(
                                        state.protection_change.is_some() || state.session.platform["vpn_held"] == true,
                                        &state.saved_protection_recovery)
                                        && let Some(request) = detail.begin_address()
                                    {
                                        if let Some(job) = address_job.take() { job.abort(); }
                                        let api = state.session.api.clone();
                                        let port = state.session.settings.mixed_port;
                                        let send = updates.clone();
                                        address_job = Some(tokio::spawn(async move {
                                            let value = core::connection::resolve_target(&api, &request.host, port).await.unwrap_or_default();
                                            let _ = send.send(Update::Address(selection, request, value)).await;
                                        }));
                                    }
                                }
                                let members=state.session.proxies.proxies.values().flat_map(|p|p.all.iter().map(|n|n.as_str()))
                                    .collect::<std::collections::BTreeSet<_>>();
                                state.node_delays.retain(|name,_|state.session.proxies.proxies.contains_key(name.as_str())||members.contains(name.as_str()));
                            }
                            Err(error) => {
                                if let Some(selection) = selection.as_ref()
                                    && Some(selection) == visible_detail.borrow().as_ref()
                                    && let Some(detail) = state.connection_detail.as_mut()
                                {
                                    detail.refresh_failed(selection);
                                }
                                let message = LogMessage::StatusRefreshFailed(error);
                                if state.logs.back().is_none_or(|(_,old)| old != &message) {
                                    log::warn!("{}", message.display(state.presentation_language));
                                    state.logs.push_back(("WARN".into(), message));
                                }
                            }
                        }
                    }
                    Some(Update::Ip(generation,index,result)) if generation == state.generation => state.ip_results[index]=result,
                    Some(Update::IpDone(generation)) if generation == state.generation => { state.detecting=false; checks=None; }
                    Some(Update::ClaudeNetwork(generation,result)) if generation == state.claude_generation => {
                        claude_jobs.network = None; state.claude_busy = false; immediate = true;
                        match result {
                            Ok(report) => {state.claude.network(report); state.claude_status=ClaudeStatus::Complete;}
                            Err(error) => {state.claude_status=ClaudeStatus::Failed(error.clone()); state.notice=error;state.notice_error=true;}
                        }
                    }
                    Some(Update::ClaudeBrowser(ticket, result)) => {
                        immediate |= claude_jobs.finish_browser(ticket, &mut state, result).is_some();
                    }
                    Some(Update::ProtectionPrepared(ticket, result)) => {
                        if let Some(change) = state.protection_change.as_mut()
                            && ProtectionJob::finish(&mut change.job, &ticket, None) {
                            immediate = true;
                            refresh_requested = true;
                            if !change.deadline.reached() {
                                let applied = (|| -> Result<()> {
                                    let protection = result.map_err(anyhow::Error::msg)?;
                                    ensure!(state.session.settings.active_profile.as_deref() == Some(protection.profile_id.as_str()),
                                        "{}", state.presentation_language.text("订阅已变化，请重新开启保护", "The subscription changed; enable protection again"));
                                    let request = next_vpn_authorization(&state.session.store.root)?;
                                    change.authorization = Some((request, ProtectionAuthorization::Enable(Box::new(protection))));
                                    state.protection_status = "正在等待 VPN 授权，当前网络设置保持不变…".into();
                                    request_vpn_authorization(&state.session, state.presentation_language, request)?;
                                    Ok(())
                                })();
                                if let Err(error) = applied {
                                    let reason = format!("{}{error:#}", state.presentation_language.text("保护开启失败：", "Failed to enable protection: "));
                                    rollback_protection(&mut state, &reason);
                                }
                            }
                        }
                    }
                    Some(Update::Log(level,message)) => state.logs.push_back((level,message.into())),
                    Some(Update::Geo(ticket, result)) if geo_gate.finish(ticket) => {
                        state.geo_updating = false;
                        geo_job = None;
                        if state.protection_change.is_some() || state.session.settings.claude_protection.is_some()
                            || state.saved_protection_recovery.is_active() {
                            continue;
                        }
                        network_generation = network_generation.wrapping_add(1);
                        if let Some(job) = network_job.take() { job.abort(); }
                        state.probe = Default::default();
                        state.probe_busy = false;
                        if let Some(job) = refresh_job.take() { job.abort(); }
                        refresh_generation += 1;
                        refresh_requested = true;
                        match result.and_then(|staged|state.session.install_geo(staged.path()).map_err(|e|format!("{e:#}"))) {
                            Ok(manifest)=> {state.geo=Some(manifest.display_status());state.notice=state.presentation_language.text("Geo 数据已更新", "Geo data updated").into();},
                            Err(error)=>{state.notice=format!("Geo 更新失败：{error}");state.notice_error=true;},
                        }
                    }
                    Some(Update::Subscription(generation, pending, result)) if generation == subscription_generation => {
                        subscription_job = None;
                        if !profile_gate.busy() && state.protection_change.is_none() {
                            match result {
                                Ok((raw, usage)) => match state.session.update_subscription(&pending, raw, usage) {
                                    Ok(true) => {
                                        network_generation = network_generation.wrapping_add(1);
                                        if let Some(job) = network_job.take() { job.abort(); }
                                        cancel_delay(&mut state, &mut delay_job);
                                        reset_subscription_results(&mut state, &mut checks, &mut claude_jobs);
                                        state.rule_rows = None;
                                        if let Some(job) = refresh_job.take() { job.abort(); }
                                        refresh_generation = refresh_generation.wrapping_add(1);
                                        refresh_requested = true;
                                        immediate = true;
                                    }
                                    Ok(false) => {}
                                    Err(_) => state.logs.push_back(("WARN".into(), LogMessage::SubscriptionUpdateFailed)),
                                },
                                Err(error) => {
                                    if pending.is_current(&state.session.settings, &state.session.profiles, &state.session.store).unwrap_or(false)
                                        && let Some(message) = core::subscription::format_notice(&error) {
                                        state.notice = LogMessage::SubscriptionDownloadFailed(message)
                                            .display(state.presentation_language);
                                        state.notice_error = true;
                                    }
                                    state.logs.push_back(("WARN".into(), LogMessage::SubscriptionDownloadFailed(error.to_string())));
                                },
                            }
                        }
                    }
                    Some(Update::Checked(generation,result)) if generation == state.update_generation =>{
                        state.update_busy=false;state.update_downloading=false;
                        match result {
                            Ok(package)=>{
                                state.update_status=package.as_ref().map_or(UpdateStatus::NoUpdate,|p|UpdateStatus::Available{version:p.version.clone(),preview:p.preview,size:p.size});
                                state.update_downloaded=state.update_downloaded && state.update_package.as_ref().map(|p|&p.version)==package.as_ref().map(|p|&p.version);
                                state.update_package=package;
                            }
                            Err(error)=>{
                                let message = crate::update_presentation::check_failure(
                                    &error,
                                    state.presentation_language,
                                )
                                .into_owned();
                                state.update_status=UpdateStatus::CheckFailed(error);
                                state.notice=message;
                                state.notice_error=true;
                            },
                        }
                    }
                    Some(Update::DownloadProgress(generation,value)) if generation == state.update_generation =>{
                        state.update_progress=value.received as f32/value.total.max(1) as f32;
                        state.update_status=if value.received==0 {
                            UpdateStatus::Connecting(value)
                        } else {
                            UpdateStatus::Downloading(value)
                        };
                    }
                    Some(Update::Downloaded(generation,result)) if generation == state.update_generation =>{
                        state.update_busy=false;state.update_downloading=false;
                        match result {
                            Ok(())=>{
                                state.update_downloaded=true;state.update_status=UpdateStatus::Downloaded;
                                if let Err(error)=state.session.native.request(json!({"op":"install-update"})) {state.notice=format!("{error:#}");state.notice_error=true;}
                            }
                            Err(error)=>{
                                state.update_status=UpdateStatus::Failed(error);
                                state.notice=state.update_status.render(state.presentation_language);
                                state.notice_error=true;
                            },
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
                    && !state.saved_protection_recovery.is_active()
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
                            Err(_) => state.logs.push_back(("WARN".into(), LogMessage::SubscriptionPrepareFailed)),
                        }
                    }
                }
                if update_schedule.due(state.session.settings.update_check_interval_minutes, std::time::Instant::now())
                    && state.session.running && !state.update_busy
                    && state.session.settings.claude_protection.is_none() && state.protection_change.is_none()
                    && !state.saved_protection_recovery.is_active() {
                    update_schedule.checked(std::time::Instant::now());state.update_busy=true;state.update_downloading=false;state.update_status=UpdateStatus::Checking;
                    state.update_generation = state.update_generation.wrapping_add(1);
                    state.update_confirmation = None;
                    update_job=Some(start_update_check(state.session.settings.download_ports(),state.session.settings.update_prereleases,state.update_generation,updates.clone()));
                }
                let minutes=state.session.settings.delay_interval_minutes;
                if state.session.running && minutes>0
                    && can_run_active_probe(&state)
                    && last_delay.elapsed()>=Duration::from_secs(minutes as u64*60) && state.delay_scope.is_none()
                    && recovery_job.is_none() {
                    last_delay=std::time::Instant::now();
                    delay_job=Some(start_delay(&mut state,DelayScope::All,updates.clone()));
                }
                refresh_requested = true;
            }
        }
        if !recovery.ready() {
            state.page = recovery.page();
        }
        let visible = *visibility.borrow_and_update();
        if state.visible != visible {
            state.visible = visible;
            immediate = true;
            refresh_requested = true;
        }
        if refresh_requested {
            let was_running = state.session.running;
            state.protection_observed = match state.session.observe_native() {
                Ok(()) => true,
                Err(error) => {
                    state.logs.push_back((
                        "WARN".into(),
                        LogMessage::StatusRefreshFailed(format!("{error:#}")),
                    ));
                    false
                }
            };
            if state.saved_protection_recovery.is_active()
                && !state.saved_protection_recovery.is_applying()
            {
                restore_protection = false;
                let progress = state
                    .saved_protection_recovery
                    .poll_observed(Some(&state.session), state.protection_observed);
                match progress {
                    RollbackProgress::VerifyProtection {
                        request,
                        check_dns: true,
                    } => {
                        if !rollback_dns_job
                            .as_ref()
                            .is_some_and(|job| job.verifies(&request))
                        {
                            drop(rollback_dns_job.take());
                            let api = state.session.api.clone();
                            let language = state.presentation_language;
                            let sender = updates.clone();
                            rollback_dns_job = Some(ProtectionJob::spawn(
                                Some(request.clone()),
                                |ticket| async move {
                                    let result =
                                        tokio::time::timeout(
                                            Duration::from_secs(40),
                                            api.finish_protection_enable(),
                                        )
                                        .await
                                        .context(language.text(
                                            "保护验证超时",
                                            "Protection verification timed out",
                                        ))
                                        .and_then(|result| result);
                                    let _ = sender
                                        .send(Update::ProtectionRollbackVerified(
                                            ticket, request, result,
                                        ))
                                        .await;
                                },
                            ));
                        }
                    }
                    RollbackProgress::VerifyProtection {
                        request,
                        check_dns: false,
                    } => {
                        rollback_dns_job = None;
                        let progress = state.saved_protection_recovery.verified(
                            &state.session,
                            &request,
                            Ok(()),
                        );
                        present_rollback(&mut state, progress);
                    }
                    other => {
                        rollback_dns_job = None;
                        present_rollback(&mut state, other);
                    }
                }
            } else {
                rollback_dns_job = None;
                if restore_protection && state.protection_observed && state.session.running {
                    restore_protection = !restore_saved_protection(&mut state);
                }
            }
            if was_running && !state.session.running {
                state.notice = state.presentation_language.text("内核已意外停止，请导出诊断后重试启动。", "The core stopped unexpectedly. Export diagnostics, then try starting it again.").into();
                state.notice_error = true;
                state.notice_critical = true;
            }
            match observe_protection(&mut state, protection_dns_result) {
                Ok(true) => claude_jobs.start(&mut state, updates.clone()),
                Ok(false) => {}
                Err(error) => {
                    state.protection_observed = false;
                    state.notice = format!(
                        "{}{error:#}",
                        state
                            .presentation_language
                            .text("保护状态异常：", "Protection status error: ")
                    );
                    state.notice_error = true;
                    state.notice_critical = true;
                    state
                        .logs
                        .push_back(("ERROR".into(), state.notice.clone().into()));
                }
            }
            if let Some(change) = state.protection_change.as_mut()
                && (change.deadline.reached()
                    || change
                        .transition
                        .as_ref()
                        .is_some_and(VpnTransition::needs_settlement))
            {
                change.job = None;
            }
            if let Some(change) = state.protection_change.as_mut()
                && !change.deadline.reached()
                && !change.verified
                && change.job.is_none()
                && state.protection_observed
                && let Some(transition) = &change.transition
                && !transition.needs_settlement()
                && let RollbackProgress::VerifyProtection {
                    request,
                    check_dns: true,
                } = transition.poll(
                    &state.session,
                    state.session.settings.claude_protection.is_some(),
                )
            {
                let api = state.session.api.clone();
                let send = updates.clone();
                state.protection_status = "正在关闭旧连接并验证保护节点与加密 DNS…".into();
                change.job = Some(ProtectionJob::spawn(
                    Some(request.clone()),
                    |ticket| async move {
                        let result = api
                            .finish_protection_enable()
                            .await
                            .map_err(|error| format!("{error:#}"));
                        let _ = send
                            .send(Update::ProtectionVerified(ticket, request, result))
                            .await;
                    },
                ));
            }
            if !state.session.running || (!state.visible && !state.session.vpn) {
                if let Some(job) = refresh_job.take() {
                    job.abort();
                }
                refresh_generation += 1;
                state.session.clear_snapshot();
                if !state.session.running {
                    if let Some(job) = address_job.take() {
                        job.abort();
                    }
                    if let Some(detail) = state.connection_detail.as_mut() {
                        detail.core_stopped();
                    }
                }
            } else if state.session.running && refresh_job.is_none() {
                let api = state.session.api.clone();
                let page = if state.visible { state.page } else { -1 };
                let selection = state
                    .connection_detail
                    .as_ref()
                    .and_then(|detail| detail.selection_for_refresh(page))
                    .filter(|selection| Some(selection) == visible_detail.borrow().as_ref());
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
                    let _ = send
                        .send(Update::Snapshot(generation, selection, result))
                        .await;
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
                    && !claude_jobs.busy()
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
        let incident = &state.session.platform["incident_notifications"];
        let sequence = incident["sequence"].as_u64().unwrap_or(0);
        let mut system_notice = false;
        let mut record_system_notice = false;
        if state.system_incidents.pending(sequence) && state.notice.is_empty() {
            record_system_notice = state.system_incidents.observe(sequence);
            state.notice = incident["message"].as_str().unwrap_or("").into();
            state.notice_error = true;
            state.notice_critical = incident["critical"].as_bool().unwrap_or(false);
            system_notice = true;
        }
        let view = project(&mut state);
        state.completed = None;
        state.rule_form = None;
        state.rule_editor = None;
        let notice = std::mem::take(&mut state.notice);
        let notice_error = std::mem::take(&mut state.notice_error);
        let notice_critical = std::mem::take(&mut state.notice_critical);
        let notice_network_recovery = std::mem::take(&mut state.notice_network_recovery);
        let notice_followup = std::mem::take(&mut state.notice_followup);
        if notice_error && !notice.is_empty() {
            if !system_notice
                && let Err(error) = state.session.native.request(json!({
                    "op":"notify-incident","critical":notice_critical
                }))
            {
                log::warn!("Incident notification unavailable: {error:#}");
            }
            if !system_notice || record_system_notice {
                state
                    .issue_events
                    .push_back(core::support::incident(&notice, notice_critical));
                while state.issue_events.len() > 20 {
                    state.issue_events.pop_front();
                }
            }
        }
        let notice = if notice_error {
            core::error_message::localize(&notice, state.presentation_language).into_owned()
        } else {
            notice
        };
        let weak = weak.clone();
        let gate = profile_gate.clone();
        let incident_sequence = system_notice.then_some(sequence);
        let incident_acknowledgement = updates.clone();
        let recovery = recovery.clone();
        let clear_startup_notice = std::mem::take(&mut recovered);
        slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                if let Some(ticket) = profile_completed {
                    gate.finish(ticket);
                }
                ui.global::<MobileData>().set_profile_busy(gate.busy());
                if clear_startup_notice || protection_reservation_released {
                    let view = ui.global::<MobileView>();
                    view.set_notice_error(false);
                    view.set_notice_critical(false);
                    view.set_notice_recover_network(false);
                    view.set_notice("".into());
                }
                let page = view.page;
                apply(&ui, view);
                if ui.get_current_page() == page {
                    recovery.publish(page);
                }
                if !notice.is_empty() {
                    let current = ui.global::<MobileView>();
                    let presented = if !current.get_notice().is_empty()
                        && current.get_notice_error()
                        && notice_followup
                    {
                        ui.invoke_notify(format!("{}\n\n{notice}", current.get_notice()).into());
                        true
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
                        true
                    } else {
                        false
                    };
                    if presented && let Some(sequence) = incident_sequence {
                        let _ =
                            incident_acknowledgement.try_send(Update::IncidentPresented(sequence));
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
    claude_jobs.stop(&mut state);
    drop(state.protection_change.take());
    drop(rollback_dns_job);
    if let Some(job) = log_job {
        job.abort();
    }
    if let Some(job) = delay_job {
        job.abort();
    }
    if let Some(job) = network_job {
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

fn protected_vpn_attached(session: &Session) -> bool {
    session.vpn
        && session.platform["vpn_held"] == false
        && session.platform["vpn_transition_pending"] == false
        && session.platform["protection"]["enabled"] == true
        && session.platform["protection"]["snapshot"]["connected"] == true
        && session.platform["protection"]["snapshot"]["error"].as_str() == Some("")
}

fn can_run_active_probe(state: &State) -> bool {
    crate::startup_protection::active_probe_allowed(
        state.protection_change.is_some() || state.session.platform["vpn_held"] == true,
        &state.saved_protection_recovery,
    )
}

fn restore_saved_protection(state: &mut State) -> bool {
    if state.protection_change.is_some() || state.session.settings.claude_protection.is_none() {
        return true;
    }
    if VpnBaseline::capture(&state.session).is_none() {
        return false;
    }
    if state.protection_rollback_error.is_none() && protected_vpn_attached(&state.session) {
        return true;
    }
    let language = state.presentation_language;
    state.protection_status = "正在恢复保护 VPN…".into();
    let result = (|| -> Result<()> {
        let request = next_vpn_authorization(&state.session.store.root)?;
        state.protection_disable_confirmation = None;
        state.protection_change = Some(ProtectionChange {
            kind: ProtectionChangeKind::Restore,
            transition: None,
            deadline: ProtectionDeadline::after(Duration::from_secs(120)),
            verified: false,
            authorization: Some((request, ProtectionAuthorization::Restore)),
            job: None,
        });
        request_vpn_authorization(&state.session, language, request)
    })();
    if let Err(error) = result {
        let reason = format!(
            "{}{error:#}",
            language.text("保护恢复失败：", "Failed to restore protection: ")
        );
        rollback_protection(state, &reason);
    }
    true
}

fn request_vpn_authorization(
    session: &Session,
    language: config::Language,
    request: i64,
) -> Result<()> {
    ensure!(
        EVENTS.expect_authorization(request),
        "{}",
        language.text("VPN 授权入口不可用", "VPN authorization is unavailable")
    );
    session
        .native
        .request(json!({"op":"authorize-vpn","request":request}))?;
    Ok(())
}

fn rollback_protection(state: &mut State, reason: &str) {
    if state.saved_protection_recovery.is_finalizing() {
        return;
    }
    let mut failures = Vec::new();
    if let Some(mut change) = state.protection_change.take() {
        drop(change.job.take());
        if let Some((request, _)) = change.authorization {
            EVENTS.cancel_authorization(request);
            if let Err(error) = state
                .session
                .native
                .request(json!({"op":"cancel-vpn-authorization","request":request}))
            {
                failures.push(format!("cancel-vpn-authorization: {error:#}"));
            }
        }
        if state.saved_protection_recovery.is_applying() {
            let result = state.saved_protection_recovery.rollback(&mut state.session);
            if let Err(error) = result {
                failures.push(format!("{error:#}"));
            }
        }
    }
    let failed = !failures.is_empty();
    let message = if failed {
        format!(
            "{reason}{}{}",
            state
                .presentation_language
                .text("；恢复失败：", "; recovery failed: "),
            failures.join(" · ")
        )
    } else {
        reason.to_owned()
    };
    if failed {
        state.protection_rollback_error = Some(message.clone());
    }
    state.protection_status = message.clone();
    state.notice = message.clone();
    state.notice_error = true;
    state.notice_critical = failed;
    state
        .logs
        .push_back((if failed { "ERROR" } else { "WARN" }.into(), message.into()));
}

fn present_rollback(state: &mut State, progress: RollbackProgress) {
    let language = state.presentation_language;
    match progress {
        RollbackProgress::Restored => {
            state.protection_rollback_error = None;
            state.protection_status.clear();
        }
        RollbackProgress::Failed(error) => {
            let message = format!("恢复原网络状态失败：{error}");
            if state.protection_rollback_error.as_deref() != Some(message.as_str()) {
                state.notice = message.clone();
                state.notice_error = true;
                state.notice_critical = true;
                state.protection_rollback_error = Some(message.clone());
            }
            state.protection_status = message;
        }
        RollbackProgress::Pending => {
            state.protection_status = "正在恢复原网络状态…".into();
        }
        RollbackProgress::WaitingForVpnSettlement(detail) => {
            let waiting = language.text(
                "正在等待 Android VPN 服务结束原请求。",
                "Waiting for the Android VPN service to finish the original request.",
            );
            let message = format!("{detail} · {waiting}");
            state.protection_rollback_error = None;
            if state.protection_status != message {
                state.protection_status = message.clone();
                state.notice = message;
                state.notice_error = true;
            }
        }
        RollbackProgress::Idle | RollbackProgress::VerifyProtection { .. } => {}
    }
}

fn disabled_and_restored(language: config::Language) -> &'static str {
    language.text(
        "Claude 保护已关闭，原网络状态已恢复",
        "Claude protection disabled; previous network state restored",
    )
}

fn settled_protection_disable(state: &mut State) -> Option<&'static str> {
    let language = state.presentation_language;
    let confirmation = protection_presentation::observe_vpn_restoration(
        &mut state.protection_disable_confirmation,
        state.protection_observed.then_some(&state.session.platform),
    )?;
    Some(match confirmation {
        VpnRestoration::Restored => disabled_and_restored(language),
        VpnRestoration::Superseded => {
            language.text("Claude 保护已关闭", "Claude protection disabled")
        }
    })
}

fn complete_protection_change(state: &mut State) -> bool {
    let change = state
        .protection_change
        .take()
        .expect("committed protection change");
    let transition = change.transition.expect("committed VPN transition");
    let language = state.presentation_language;
    state.protection_rollback_error = None;
    state.protection_observed = state.session.observe_native().is_ok();
    let live = state.protection_observed && protected_vpn_attached(&state.session);
    let status = if change.kind != ProtectionChangeKind::Disable && live {
        language.text("Claude 保护已开启", "Claude protection enabled")
    } else if change.kind != ProtectionChangeKind::Disable {
        language.text(
            "保护已保存，实时状态待确认",
            "Protection saved; live status unconfirmed",
        )
    } else {
        state.protection_disable_confirmation = Some(PendingVpnRestoration {
            connected: transition.target_connected(),
            request: transition.request,
        });
        settled_protection_disable(state).unwrap_or_else(|| {
            language.text(
                "Claude 保护已关闭，VPN 状态待确认",
                "Claude protection disabled; VPN state unconfirmed",
            )
        })
    };
    state.protection_status = status.into();
    if state.protection_disable_confirmation.is_none() {
        state.notice = status.into();
    }
    change.kind == ProtectionChangeKind::Enable && live
}

/// Returns true only after a user-requested enable has completed successfully.
fn observe_protection(
    state: &mut State,
    dns_result: Option<(String, std::result::Result<(), String>)>,
) -> Result<bool> {
    if let Some(message) = settled_protection_disable(state) {
        state.protection_status = message.into();
        if state.notice.is_empty() {
            state.notice = message.into();
        }
    }
    if state.saved_protection_recovery.is_active() && !state.saved_protection_recovery.is_applying()
    {
        return Ok(false);
    }
    let language = state.presentation_language;
    if state.protection_change.is_some() && state.saved_protection_recovery.is_finalizing() {
        match state
            .saved_protection_recovery
            .poll_observed(Some(&state.session), state.protection_observed)
        {
            RollbackProgress::Restored => return Ok(complete_protection_change(state)),
            RollbackProgress::Failed(error) => rollback_protection(state, &error),
            _ => {}
        }
        return Ok(false);
    }
    if let Some(change) = &state.protection_change
        && (change.deadline.reached()
            || change
                .transition
                .as_ref()
                .is_some_and(VpnTransition::needs_settlement))
    {
        let timeout = language.text("保护切换超时", "Protection change timed out");
        let Some(transition) = &change.transition else {
            rollback_protection(state, timeout);
            return Ok(false);
        };
        let reason = transition
            .expiry_reason()
            .map_or_else(|| timeout.to_owned(), |error| format!("{error:#}"));
        let detail = match transition.resolve_start_timeout(&state.session) {
            VpnStartTimeout::ReadyForRecovery => {
                rollback_protection(state, &reason);
                return Ok(false);
            }
            VpnStartTimeout::Waiting => reason,
            VpnStartTimeout::Unconfirmed(error) => format!(
                "{reason}{}{error:#}",
                language.text(
                    "。无法确认 VPN 启动状态：",
                    ". Could not confirm VPN start status: "
                ),
            ),
        };
        present_rollback(state, RollbackProgress::WaitingForVpnSettlement(detail));
        return Ok(false);
    }
    match state.session.protection_snapshot() {
        Ok(Some(snapshot)) => state.guard_snapshot = snapshot,
        Ok(None) => {}
        Err(error) => {
            state.protection_observed = false;
            let message = format!("暂时无法读取保护状态：{error:#}");
            let diagnostic = if state.protection_change.is_some()
                || state.session.settings.claude_protection.is_some()
            {
                &mut state.protection_status
            } else {
                &mut state.guard_snapshot.error
            };
            if *diagnostic != message {
                state
                    .logs
                    .push_back(("WARN".into(), LogMessage::ActionFailed(message.clone())));
                *diagnostic = message;
            }
            return Ok(false);
        }
    }
    let native = &state.session.platform["protection"];
    let active = native["enabled"].as_bool().unwrap_or(false);
    if let Some(change) = &mut state.protection_change {
        let Some(transition) = &change.transition else {
            return Ok(false);
        };
        if !state.protection_observed {
            return Ok(false);
        }
        if let Some((request, result)) = dns_result
            && transition.accepts_dns_result(&state.session, &request)
        {
            match result {
                Ok(()) => change.verified = true,
                Err(error) => {
                    let prefix =
                        language.text("保护验证失败：", "Protection verification failed: ");
                    rollback_protection(state, &format!("{prefix}{error}"));
                    return Ok(false);
                }
            }
        }
        let enabled = change.kind != ProtectionChangeKind::Disable;
        let ready = match transition.poll(&state.session, enabled) {
            RollbackProgress::Failed(error) => {
                rollback_protection(state, &error);
                return Ok(false);
            }
            RollbackProgress::VerifyProtection { check_dns, .. } => !check_dns || change.verified,
            _ => false,
        };
        if ready {
            match state.saved_protection_recovery.finish(
                &state.session,
                transition,
                change.verified,
            ) {
                Ok(true) => return Ok(complete_protection_change(state)),
                Ok(false) => {
                    state.protection_status = "正在确认 VPN 提交…".into();
                }
                Err(error) if !transition.needs_settlement() => {
                    rollback_protection(state, &format!("{error:#}"));
                }
                Err(_) => {}
            }
        }
    } else if let Some(protection) = &state.session.settings.claude_protection {
        state.protection_status =
            if !state.session.vpn || state.session.platform["vpn_held"] != false || !active {
                language
                    .text(
                        "保护已配置，实时拦截未运行；请重新连接 VPN",
                        "Protection configured, but live blocking is stopped; reconnect the VPN",
                    )
                    .into()
            } else if !state.guard_snapshot.error.is_empty() {
                state.guard_snapshot.error.clone()
            } else {
                match language {
                    config::Language::SimplifiedChinese => format!(
                        "保护节点：{} · 已拦截 {} 条",
                        protection.node, state.guard_snapshot.blocked
                    ),
                    config::Language::English => format!(
                        "Protected node: {} · Blocked {}",
                        protection.node, state.guard_snapshot.blocked
                    ),
                }
            };
    }
    Ok(false)
}

#[derive(Default)]
struct ClaudeJobs {
    network: Option<tokio::task::JoinHandle<()>>,
    browser: Option<(i64, tokio::task::JoinHandle<()>)>,
}

impl ClaudeJobs {
    fn busy(&self) -> bool {
        self.network.is_some() || self.browser.is_some()
    }

    fn stop(&mut self, state: &mut State) -> [Option<tokio::task::JoinHandle<()>>; 2] {
        let network = self.network.take();
        let browser = self.browser.take().map(|(_, job)| job);
        if let Some(job) = &network {
            job.abort();
            state.claude_generation = state.claude_generation.wrapping_add(1);
            state.claude_busy = false;
            state.claude_status = ClaudeStatus::Stopped;
        }
        if let Some(job) = &browser {
            job.abort();
            state.claude_browser_status = BrowserStatus::Stopped;
        }
        [network, browser]
    }

    fn start(&mut self, state: &mut State, send: mpsc::Sender<Update>) {
        self.stop(state);
        state.claude.begin();
        state.claude_generation = state.claude_generation.wrapping_add(1);
        state.claude_busy = true;
        let generation = state.claude_generation;
        state.claude_status = ClaudeStatus::Checking;
        let port = state.session.settings.mixed_port;
        let api = state.session.api.clone();
        let network_send = send.clone();
        self.network = Some(tokio::spawn(async move {
            let result = async {
                let client = core::claude_check::client(Some(port))?;
                let mut report = core::claude_check::Report::new(
                    core::claude_check::Environment::Core,
                    "Android 网络环境",
                );
                core::claude_check::network(&client, &mut report, "内核代理").await;
                if let Ok(routes) = api
                    .get::<core::claude_check::ObservedConnections>("connections")
                    .await
                {
                    report.routes = routes.connections;
                }
                core::claude_check::enhanced::extend(&client, &mut report).await;
                report
                    .signals
                    .extend(core::claude_check::system_dns(&client, "Android 系统").await);
                Ok::<_, anyhow::Error>(report)
            }
            .await;
            let _ = network_send
                .send(Update::ClaudeNetwork(
                    generation,
                    result.map_err(|error| error.to_string()),
                ))
                .await;
        }));
        self.start_browser(state, send);
    }

    fn start_browser(&mut self, state: &mut State, send: mpsc::Sender<Update>) {
        if self.browser.is_some() {
            return;
        }
        let mut launch = CLAUDE_BROWSER_LAUNCHES.register();
        let ticket = launch.ticket;
        let port = state.session.settings.mixed_port;
        let host = state.session.native.clone();
        state.claude_browser_status = BrowserStatus::Sampling;
        let job = tokio::spawn(async move {
            let result = async {
                let client = core::claude_check::client(Some(port))
                    .map_err(|_| BrowserStatus::Unavailable)?;
                let browser = core::claude_check::browser::Session::start(client)
                    .await
                    .map_err(|_| BrowserStatus::Unavailable)?;
                host.request(json!({
                    "op":"open-url","url":browser.url,"claude_browser_sample":ticket
                }))
                .map_err(|_| BrowserStatus::OpenFailed)?;
                let read = async {
                    loop {
                        if let Some(sample) = browser.next_report() {
                            return Ok(sample);
                        }
                        if browser.finished() {
                            return Err(BrowserStatus::Unavailable);
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                };
                tokio::select! {
                    _ = &mut launch.failure => Err(BrowserStatus::OpenFailed),
                    result = tokio::time::timeout(Duration::from_secs(120), read) =>
                        result.map_err(|_| BrowserStatus::TimedOut)?,
                }
            }
            .await;
            let _ = send.send(Update::ClaudeBrowser(ticket, result)).await;
        });
        self.browser = Some((ticket, job));
    }

    fn finish_browser(
        &mut self,
        ticket: i64,
        state: &mut State,
        result: std::result::Result<core::claude_check::Report, BrowserStatus>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let (active_ticket, _) = self.browser.as_ref()?;
        if *active_ticket != ticket {
            return None;
        }
        state.claude_browser_status = match result {
            Ok(report) => {
                state.claude.browser(report);
                BrowserStatus::Complete
            }
            Err(status) => status,
        };
        self.browser.take().map(|(_, job)| job)
    }
}

fn picker_request(value: &Value, language: config::Language) -> Result<i32> {
    value["value"]["request"]
        .as_i64()
        .and_then(|request| i32::try_from(request).ok())
        .context(language.text("本地文件请求无效", "Invalid local file request"))
}

fn unique_profile_name(state: &State, base: &str) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    while state
        .session
        .profiles
        .iter()
        .any(|profile| profile.name == name)
    {
        name = format!("{base} ({suffix})");
        suffix += 1;
    }
    name
}

fn settle_picker(
    state: &mut State,
    request: i32,
    selected_name: Option<String>,
    language: config::Language,
) {
    match state.profile_picker.settle(request, selected_name) {
        Some(Settlement::Selected(name)) => {
            state.picked_profile_name = Some(name);
            state.notice = language
                .text(
                    "已选择本地配置，保存后生效",
                    "Local configuration selected; save to apply",
                )
                .into();
            state.notice_error = false;
        }
        Some(Settlement::Cancelled) => {
            state.notice = language
                .text(
                    "已取消选择本地配置",
                    "Local configuration selection cancelled",
                )
                .into();
            state.notice_error = false;
        }
        Some(Settlement::Failed(error)) => {
            state.notice = error;
            state.notice_error = true;
        }
        Some(Settlement::Blocked) => {
            state.notice = language
                .text("本地配置尚未就绪", "The local configuration is not ready")
                .into();
            state.notice_error = true;
        }
        None => {}
    }
}

async fn native_event(state: &mut State, value: &Value) -> Result<()> {
    let language = state.presentation_language;
    match value["event"].as_str().unwrap_or("") {
        "vpn-authorized" => {
            let result = state
                .protection_change
                .as_ref()
                .and_then(|change| change.authorization.as_ref())
                .and_then(|(request, _)| vpn_authorization_result(value, *request));
            let Some(result) = result else {
                return Ok(());
            };
            if state
                .protection_change
                .as_ref()
                .is_some_and(|change| change.deadline.reached())
            {
                rollback_protection(
                    state,
                    language.text("VPN 授权等待超时", "VPN authorization timed out"),
                );
                return Ok(());
            }
            if !result["granted"].as_bool().unwrap_or(false) {
                let reason = protection_presentation::authorization_failure(result);
                rollback_protection(state, reason);
                return Ok(());
            }
            let applied = (|| -> Result<()> {
                let retry = state.protection_rollback_error.is_some();
                let change = state
                    .protection_change
                    .as_mut()
                    .expect("pending authorization");
                let (_, authorization) =
                    change.authorization.take().expect("matching authorization");
                match &authorization {
                    ProtectionAuthorization::Enable(protection) => {
                        ensure!(
                            state.session.settings.active_profile.as_deref()
                                == Some(protection.profile_id.as_str()),
                            "{}",
                            language.text(
                                "订阅已变化，请重新开启保护",
                                "The subscription changed; enable protection again",
                            )
                        );
                    }
                    ProtectionAuthorization::Restore => {}
                }
                state.session.observe_native()?;
                let before_vpn = VpnBaseline::capture(&state.session).context(language.text(
                    "VPN 状态正在切换，请重试",
                    "The VPN state is changing. Try again.",
                ))?;
                state
                    .saved_protection_recovery
                    .begin(&state.session, before_vpn)?;
                change.deadline = ProtectionDeadline::after(Duration::from_secs(120));
                if let ProtectionAuthorization::Enable(mut protection) = authorization {
                    before_vpn.save_for_disable(&mut protection);
                    state.session.commit_protection(
                        Some(Arc::new(*protection)),
                        state.session.settings.run_mode.clone(),
                    )?;
                } else if retry {
                    state.session.restore_protection(
                        state.session.settings.claude_protection.clone(),
                        state.session.settings.run_mode.clone(),
                    )?;
                }
                state.guard_snapshot = Default::default();
                change.transition = Some(state.saved_protection_recovery.apply_vpn(
                    &state.session,
                    VpnBaseline::target(true),
                    change.deadline,
                )?);
                state.protection_status = "正在连接保护 VPN…".into();
                Ok(())
            })();
            if let Err(error) = applied {
                rollback_protection(
                    state,
                    &format!(
                        "{}{error:#}",
                        language.text("保护开启失败：", "Failed to enable protection: ")
                    ),
                );
            }
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
            let request = picker_request(value, language)?;
            let base = value["value"]["name"]
                .as_str()
                .unwrap_or(language.text("导入配置", "Imported configuration"));
            let outcome = if state.saved_protection_recovery.is_active() {
                Outcome::Failed(
                    language
                        .text(
                            "请等待原网络状态恢复",
                            "Wait for the previous network state to be restored",
                        )
                        .into(),
                )
            } else if let Some(content) = value["value"]["content"].as_str() {
                Outcome::Selected {
                    content: Arc::from(content),
                }
            } else {
                Outcome::Failed(
                    language
                        .text("导入内容为空", "The imported content is empty")
                        .into(),
                )
            };
            if !state.profile_picker.complete(request, outcome) {
                EVENTS.finish_file(request);
                return Ok(());
            }
            let name = unique_profile_name(state, base);
            settle_picker(state, request, Some(name), language);
            EVENTS.finish_file(request);
        }
        "file-cancelled" => {
            let request = picker_request(value, language)?;
            if state.profile_picker.complete(request, Outcome::Cancelled) {
                settle_picker(state, request, None, language);
            }
            EVENTS.finish_file(request);
        }
        "file-error" => {
            let request = picker_request(value, language)?;
            let error = value["value"]["message"]
                .as_str()
                .unwrap_or(language.text("导入配置失败", "Failed to import configuration"))
                .to_owned();
            if state
                .profile_picker
                .complete(request, Outcome::Failed(error))
            {
                settle_picker(state, request, None, language);
            }
            EVENTS.finish_file(request);
        }
        _ => {}
    }
    Ok(())
}

async fn handle(state: &mut State, request: Request) -> Result<()> {
    let language = state.presentation_language;
    let Request {
        intent,
        a,
        b,
        selected,
        connection_page,
        profile_ticket: _,
    } = request;
    if intent == MobileIntent::Navigate {
        state.application_policies.clear();
    }
    crate::protection_intent::check(
        intent,
        &a,
        state.session.settings.claude_protection.is_some(),
        state.protection_change.is_some() || state.saved_protection_recovery.is_active(),
        language,
    )?;
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
        (state.protection_change.is_none()
            && (!state.saved_protection_recovery.is_active()
                || intent != MobileIntent::ClaudeProtection))
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
        "{}",
        language.text(
            "请等待 Claude 保护切换完成",
            "Wait for the Claude protection change to finish",
        )
    );
    let session = &mut state.session;
    match intent {
        MobileIntent::ApplicationPolicy => {
            let notice = state
                .application_policies
                .request(session, &a, &b, language)
                .await?;
            if !notice.is_empty() {
                state.notice = notice;
                state.completed = Some(MobileIntent::ApplicationPolicy);
            }
        }
        MobileIntent::Connect | MobileIntent::Disconnect => {
            ensure!(
                intent != MobileIntent::Disconnect || session.settings.claude_protection.is_none(),
                "{}",
                language.text(
                    "请先关闭 Claude 保护，再断开 VPN",
                    "Disable Claude protection before disconnecting the VPN",
                )
            );
            session.native.request(
                json!({"op":if intent == MobileIntent::Connect {"connect"} else {"disconnect"}}),
            )?;
        }
        MobileIntent::RestartCore => {
            if state.saved_protection_recovery.is_active() {
                match state.saved_protection_recovery.retry(session)? {
                    RetryOutcome::ReservationReleased | RetryOutcome::FinalizationConfirmed => {
                        state.protection_rollback_error = None;
                        state.protection_status.clear();
                        state.notice.clear();
                    }
                    RetryOutcome::RollbackPending => {}
                }
            } else if session.settings.claude_protection.is_none() {
                session.initialize()?;
            } else if state.protection_rollback_error.is_none() {
                session.attach_or_initialize()?;
            }
        }
        MobileIntent::Mode => {
            ensure!(
                session.settings.claude_protection.is_none(),
                "{}",
                language.text(
                    "请先关闭 Claude 保护，再切换运行模式",
                    "Disable Claude protection before changing the routing mode",
                )
            );
            let mut next = session.settings.clone();
            next.run_mode = ["rule", "global", "direct"]
                .get(a.parse::<usize>()?)
                .context(language.text("运行模式无效", "Invalid routing mode"))?
                .to_string();
            session.commit_settings(next)?;
        }
        MobileIntent::SelectNode => {
            ensure!(
                session.settings.claude_protection.is_none(),
                "{}",
                language.text(
                    "请先关闭 Claude 保护，再切换节点",
                    "Disable Claude protection before changing nodes",
                )
            );
            session.api.select(&a, &b).await?;
        }
        MobileIntent::SavePorts => {
            ensure!(
                !session.vpn,
                "{}",
                language.text(
                    "请先断开 VPN，再修改端口",
                    "Disconnect the VPN before changing the port"
                )
            );
            let mut next = session.settings.clone();
            next.mixed_port = settings_input::parse_number(&a)?;
            session.commit_settings(next)?;
            state.notice = language.text("端口已保存", "Port saved").into();
        }
        MobileIntent::NodeSort => {
            let mut next = session.settings.clone();
            next.node_sort = *proxy_order::NodeSort::ALL
                .get(a.parse::<usize>()?)
                .context(language.text("排序无效", "Invalid sort order"))?;
            session.commit_settings(next)?;
        }
        MobileIntent::Query => {
            state.query = b;
            state.node_pages.clear();
            state.group_page = 0;
            state.profile_page = 0;
            state.connection_page.reset();
            state.rule_page = 0;
        }
        MobileIntent::NodePage => {
            let value = state.node_pages.entry(a).or_default();
            *value = value.saturating_add_signed(b.parse()?);
        }
        MobileIntent::GroupExpansion => {
            if state.page == 1 && session.proxies.proxies.contains_key(a.as_str()) {
                state.group_expansion.insert(a, b.parse()?);
            }
        }
        MobileIntent::GroupPage => state.group_page = a.parse()?,
        MobileIntent::ProfilePage => state.profile_page = a.parse()?,
        MobileIntent::GroupPagination => {
            let mut next = session.settings.clone();
            next.proxy_groups_paginated = a.parse()?;
            session.commit_settings(next)?;
            state.group_page = 0;
        }
        MobileIntent::ProfilePagination => {
            let mut next = session.settings.clone();
            next.profiles_paginated = a.parse()?;
            session.commit_settings(next)?;
            state.profile_page = 0;
        }
        MobileIntent::ConnectionPage => {
            if state.page == 3
                && let Some(displayed) = connection_page
            {
                state.connection_page.turn(displayed, a.parse()?);
            }
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
                    "{}",
                    language.text(
                        "此规则请继续使用文本编辑，以保留所有参数",
                        "Continue editing this rule as text to retain all parameters",
                    )
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
            let request = state.profile_picker.begin().context(language.text(
                "请先打开添加订阅表单",
                "Open the add subscription form first",
            ))?;
            EVENTS.expect_file(request);
            if let Err(error) = session
                .native
                .request(json!({"op":"pick-file", "request":request}))
            {
                state.profile_picker.complete(request, Outcome::Cancelled);
                state.profile_picker.settle(request, None);
                EVENTS.finish_file(request);
                return Err(error);
            }
        }
        MobileIntent::ProfileNew => {
            if let Some(request) = state.profile_picker.pending() {
                EVENTS.finish_file(request);
            }
            state.profile_picker.open();
            state.picked_profile_name = None;
        }
        MobileIntent::ProfileClose => {
            if let Some(request) = state.profile_picker.pending() {
                EVENTS.finish_file(request);
            }
            state.profile_picker.discard();
            state.picked_profile_name = None;
        }
        MobileIntent::ProfileClearFile => state.profile_picker.clear_file(),
        MobileIntent::SaveProfile => {
            if selected.is_empty() {
                let content = state.profile_picker.save_content(&b, language)?;
                session.save_profile(None, &a, b.trim(), content).await?;
            } else {
                session.edit_profile(&selected, &a, &b)?;
            }
            if let Some(request) = state.profile_picker.pending() {
                EVENTS.finish_file(request);
            }
            state.profile_picker.discard();
            state.picked_profile_name = None;
            state.notice = language.text("订阅已保存", "Subscription saved").into();
        }
        MobileIntent::ActivateProfile => {
            session.activate(&a)?;
        }
        MobileIntent::DeleteProfile => {
            let active = session.settings.active_profile.as_deref() == Some(&a);
            session.delete(&a)?;
            state.notice = if active {
                language.text(
                    "订阅已删除，已切换到默认直连",
                    "Subscription removed; switched to the default direct connection",
                )
            } else {
                language.text("订阅已删除", "Subscription removed")
            }
            .into();
        }
        MobileIntent::UpdateProfile => {
            session.update_profile(&a).await?;
            state.notice = language.text("订阅已更新", "Subscription updated").into();
        }
        MobileIntent::CloseConnection => {
            session.api.close_connection(&a).await?;
            if let Some(detail) = state.connection_detail.as_mut() {
                detail.wait_for_close_confirmation(&a);
            }
        }
        MobileIntent::CloseAll => {
            ensure!(
                session.running,
                "{}",
                language.text("请先启动内核", "Start the core first")
            );
            session.api.close_connections().await?;
            if let Some(detail) = state.connection_detail.as_mut() {
                detail.wait_for_all_close_confirmation();
            }
            state.notice = language
                .text("连接已全部关闭", "All connections closed")
                .into();
        }
        MobileIntent::ConnectionSort => {
            if a != state.connection_sort {
                state.connection_sort = a;
                state.connection_page.reset();
            }
        }
        MobileIntent::ConnectionDetail => {
            state.connection_detail = Some(Detail::new(ConnectionSelection {
                id: a,
                request: b.parse()?,
            }));
        }
        MobileIntent::Copy => {
            let key = a.clone();
            let text = if let Some(id) = a.strip_prefix("connection:") {
                state
                    .connection_detail
                    .as_ref()
                    .filter(|detail| detail.selection.id == id)
                    .and_then(|detail| detail.values(language))
                    .context(
                        language.text("连接详情暂不可用。", "Connection details are unavailable."),
                    )?
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
            state.notice = language.text("已复制", "Copied").into();
        }
        MobileIntent::LogLevel => state.log_level = a,
        MobileIntent::ClearLogs => state.logs.clear(),
        MobileIntent::ExportLogs => {
            session.native.request(json!({"op":"export","name":"clash-of-rust-logs.txt",
                "content":format!("Android UI: {}\n{}",PRESENTATION.lock().map(|s|s.clone()).unwrap_or_default(),
                    state.logs.iter().map(|(level,line)|format!("[{level}] {}", line.display(config::Language::SimplifiedChinese))).collect::<Vec<_>>().join("\n"))}))?;
        }
        MobileIntent::ExportDiagnostics
        | MobileIntent::CopyDiagnostics
        | MobileIntent::ReportIssue => {
            let root = session.store.root.clone();
            let context = json!({
                "error":a,"events":state.issue_events,"page":state.page,
                "core_running":session.running,"vpn_connected":session.vpn,
                "measurements":session.api.measurement_diagnostics(),
                "network_tool":core::network_tools::Tool::at(state.network_tool).ok().map(|tool| tool.id()),
                "network_tests":state.probe.lines(language),"network_test_codes":state.probe.codes(),
                "site_results":core::support::ip_results(&state.ip_results),"claude_status":state.claude_status.render(language),
                "presentation":PRESENTATION.lock().map(|value|value.clone()).unwrap_or_default(),
                "platform":session.native.request(json!({"op":"diagnostics"})).map_err(|error|format!("{error:#}")),
            });
            let lines = state
                .logs
                .iter()
                .map(|(level, line)| {
                    format!(
                        "[{level}] {}",
                        line.display(config::Language::SimplifiedChinese)
                    )
                })
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
                ensure!(
                    content.len() <= 256 * 1024,
                    "{}",
                    language.text(
                        "诊断报告较大，请使用导出诊断",
                        "The diagnostic report is large; export it instead",
                    )
                );
                session
                    .native
                    .request(json!({"op":"copy","text":content}))?;
                state.notice = language.text("诊断已复制", "Diagnostics copied").into();
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
                .context(language.text(
                    "规则已不存在，请重新选择",
                    "This rule no longer exists; select a rule again",
                ))?;
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
            let (position, enabled) = b
                .split_once('|')
                .context(language.text("规则保存参数无效", "Invalid rule save parameters"))?;
            session.save_rule_state(
                &selected,
                &a,
                position.trim().parse().context(language.text(
                    "插入位置应为正整数",
                    "The insertion position must be a positive integer",
                ))?,
                enabled.parse()?,
            )?;
            state.notice = language
                .text(
                    "已保存到草稿，请应用修改",
                    "Saved to draft; apply the changes",
                )
                .into();
        }
        MobileIntent::RuleShortcut => {
            let index: usize = a.parse()?;
            ensure!(
                !session.document().builtin,
                "{}",
                language.text(
                    "默认直连配置不支持修改规则",
                    "Rules cannot be modified in the default direct configuration",
                )
            );
            ensure!(
                !*session
                    .document()
                    .included
                    .get(index)
                    .context(language.text("快捷规则不存在", "Rule shortcut not found"))?,
                "{}",
                language.text(
                    "订阅已包含此规则",
                    "The subscription already includes this rule"
                )
            );
            *session
                .draft
                .enabled
                .get_mut(index)
                .context(language.text("快捷规则不存在", "Rule shortcut not found"))? = b == "true";
        }
        MobileIntent::RuleDelete => {
            ensure!(
                a.starts_with("c:"),
                "{}",
                language.text(
                    "订阅原规则可调整顺序；只有自定义规则可以删除",
                    "Subscription rules can be reordered; only custom rules can be deleted",
                )
            );
            session
                .draft
                .custom
                .retain(|r| r.id != a.trim_start_matches("c:"));
            session.draft.order.retain(|id| id != &a);
        }
        MobileIntent::RuleMove => session.move_rule_to(
            &a,
            b.trim().parse().context(language.text(
                "移动位置应为正整数",
                "The destination must be a positive integer",
            ))?,
        )?,
        MobileIntent::RuleDrop => session.move_rule(&a, &b)?,
        MobileIntent::RuleEnable => session.toggle_rule(&a)?,
        MobileIntent::RuleRestore => session.draft.order.clear(),
        MobileIntent::RuleFilter => {
            state.rule_source = a;
            state.rule_page = 0;
        }
        MobileIntent::RuleApply => {
            session.apply_rules()?;
            state.notice = language.text("规则已应用", "Rules applied").into();
        }
        MobileIntent::RuleDiscard => {
            session.draft = rule_manager::ProfileRules::for_profile(
                &session.settings,
                &session.rule_profile,
                session.profile_builtin(&session.rule_profile),
            )
            .as_ref()
            .clone();
        }
        MobileIntent::RuleSelect if b == "profile" => {
            if session.rule_profile != a {
                session.select_rules(&a)?;
                state.rule_page = 0;
            }
        }
        MobileIntent::FakeIp => session.set_fake_ip(a == "true")?,
        MobileIntent::FakeIpAdd
        | MobileIntent::FakeIpRemove
        | MobileIntent::FakeIpRestoreDefaults => {
            ensure!(
                session.settings.claude_protection.is_none() && state.protection_change.is_none(),
                "{}",
                language.text(
                    "请先关闭 Claude 保护，再编辑排除项",
                    "Disable Claude protection before editing exclusions",
                )
            );
            session.edit_fake_ip_exclusions(
                &a,
                match intent {
                    MobileIntent::FakeIpAdd => "add",
                    MobileIntent::FakeIpRemove => "remove",
                    _ => "restore",
                },
            )?;
            state.notice = language
                .text("Fake-IP 排除项已保存", "Fake-IP exclusions saved")
                .into();
        }
        MobileIntent::OpenRelease => {
            session.native.request(
                json!({"op":"open-url","url":"https://github.com/Fu021/Clash-of-Rust/releases"}),
            )?;
        }
        MobileIntent::Ipv6 => settings_input::save_ipv6(session, a.parse()?, language)?,
        MobileIntent::DnsOverride | MobileIntent::Udp => {
            let mut next = session.settings.clone();
            match intent {
                MobileIntent::DnsOverride => next.dns_override = a == "true",
                _ => next.udp = a == "true",
            }
            session.commit_settings(next)?;
        }
        MobileIntent::UpdatePrereleases => {
            ensure!(
                !state.update_busy,
                "{}",
                language.text(
                    "请等待当前更新操作完成",
                    "Wait for the current update to finish"
                )
            );
            let mut next = session.settings.clone();
            next.update_prereleases = a == "true";
            // This is an updater preference; it must not reload the core or VPN.
            session.save_preferences(next)?;
            state.update_generation = state.update_generation.wrapping_add(1);
            state.update_confirmation = None;
            state.update_package = None;
            state.update_downloaded = false;
            state.update_status = UpdateStatus::ChannelChanged;
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
                        .context(language.text("强调色无效", "Invalid accent color"))?
                }
                _ => unreachable!(),
            }
            // Appearance changes do not reload the network configuration.
            session.commit_settings(next)?;
        }
        MobileIntent::Language => {
            let preference = config::LanguagePreference::from_tag(&a)
                .context(language.text("语言无效", "Invalid language"))?;
            state.presentation_language = live_language::save(session, preference)?;
        }
        MobileIntent::Interval => {
            let mut next = session.settings.clone();
            next.delay_interval_minutes = settings_input::parse_number(&a)?;
            session.save_preferences(next)?;
        }
        MobileIntent::SubscriptionInterval => {
            let mut next = session.settings.clone();
            next.subscription_update_interval_minutes = a.trim().parse()?;
            session.save_preferences(next)?;
            state.notice = language
                .text(
                    "订阅自动更新间隔已保存",
                    "Subscription update interval saved",
                )
                .into();
        }
        MobileIntent::UpdateInterval => {
            let mut next = session.settings.clone();
            next.update_check_interval_minutes = a.trim().parse()?;
            session.save_preferences(next)?;
            state.notice = language
                .text("自动检查更新间隔已保存", "Update check interval saved")
                .into();
        }
        MobileIntent::SystemNotification => {
            session
                .native
                .notify("Clash of Rust", language.text("喵!", "Meow!"), true)?;
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
                "content":serde_json::to_string_pretty(&json!({"detection":state.claude.report(),"enforcement":state.guard_snapshot,"protection_enabled":session.settings.claude_protection.is_some()}))?}))?;
        }
        MobileIntent::ClaudeProtection => {
            let Some(saved) = session.settings.claude_protection.clone() else {
                return Ok(());
            };
            ensure!(
                a == "false",
                "{}",
                language.text("保护操作无效", "Invalid protection action")
            );
            let target = VpnBaseline::target_for_disable(&saved);
            session.observe_native()?;
            let before_vpn = VpnBaseline::capture(session).context(language.text(
                "VPN 状态正在切换，请重试",
                "The VPN state is changing. Try again.",
            ))?;
            let deadline = ProtectionDeadline::after(Duration::from_secs(30));
            state.saved_protection_recovery.begin(session, before_vpn)?;
            state.protection_disable_confirmation = None;
            state.protection_change = Some(ProtectionChange {
                kind: ProtectionChangeKind::Disable,
                transition: None,
                deadline,
                verified: false,
                authorization: None,
                job: None,
            });
            state.protection_status = "正在恢复原网络状态…".into();
            let result = async {
                session.commit_protection(None, saved.previous.mode.clone())?;
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
                state
                    .saved_protection_recovery
                    .apply_vpn(session, target, deadline)
            }
            .await;
            match result {
                Ok(transition) => {
                    state
                        .protection_change
                        .as_mut()
                        .expect("pending protection disable")
                        .transition = Some(transition);
                }
                Err(error) => {
                    rollback_protection(
                        state,
                        &format!(
                            "{}{error:#}",
                            language.text("关闭保护失败：", "Failed to disable protection: ")
                        ),
                    );
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn blocked_during_protection(state: &mut State) -> bool {
    if state.session.settings.claude_protection.is_none()
        && state.protection_change.is_none()
        && !state.saved_protection_recovery.is_active()
    {
        return false;
    }
    state.notice = state
        .presentation_language
        .text(
            core::protection::READ_ONLY_NOTICE,
            "Disable Claude protection first; network changes and updates are locked while protection is active",
        )
        .into();
    state.notice_error = true;
    true
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
    claude_jobs: &mut ClaudeJobs,
) {
    if let Some(job) = checks.take() {
        job.abort();
    }
    claude_jobs.stop(state);
    state.generation = state.generation.wrapping_add(1);
    state.detecting = false;
    state.ip_results.fill(None);
    state.claude = Default::default();
    state.claude_status = ClaudeStatus::SubscriptionChanged;
    state.claude_browser_status = BrowserStatus::Idle;
    state.probe = Default::default();
    state.probe_busy = false;
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
    nodes: Vec<(String, api::Proxy, String)>,
    count: usize,
    page: usize,
    testing: bool,
}
struct ConnectionDetailProjection {
    selection: ConnectionSelection,
    status: DetailStatus,
    rows: Vec<(&'static str, String)>,
}
struct Projection {
    language: config::Language,
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
    rule_profiles: Vec<(String, String)>,
    connections: Vec<api::Connection>,
    rules: Vec<(usize, rule_manager::Row)>,
    policies: Vec<String>,
    shortcuts: [bool; 5],
    shortcut_available: [bool; 5],
    dirty: bool,
    rule_warning: String,
    logs: Vec<(String, String)>,
    connection_detail: Option<ConnectionDetailProjection>,
    rates: (String, String),
    platform: Value,
    rule_profile: String,
    rule_profile_label: String,
    completed: Option<MobileIntent>,
    categories: Vec<(String, String, String, i32)>,
    sites: Vec<ReportRow>,
    site_regions: Vec<String>,
    site_details: Vec<(String, String)>,
    claude_busy: bool,
    claude_browser_status: &'static str,
    claude_browser_retry: bool,
    claude: Vec<ReportRow>,
    claude_counts: (usize, usize, usize),
    progress: (usize, usize),
    detecting: bool,
    exit: (String, String),
    probe: Vec<String>,
    probe_busy: bool,
    network_tool: i32,
    geo: String,
    group_page: (usize, usize),
    profile_page: (usize, usize),
    connection_page: (usize, usize),
    connection_cursor: ConnectionPageCursor,
    rule_page: (usize, usize),
    rule_form: Option<RuleForm>,
    rule_editor: Option<RuleEditor>,
    picked_profile_name: Option<String>,
    profile_file_selected: bool,
    profile_file_pending: bool,
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
    protection_retry: bool,
    protection_status: String,
    protection_health: core::protection::status::Status,
}
struct ReportRow {
    id: String,
    title: String,
    value: String,
    detail: String,
    source: String,
    status_label: String,
    tone: i32,
    country: String,
    country_label: String,
    millis: Option<u128>,
    block_tone: i32,
    block_items: i32,
}
fn project(state: &mut State) -> Projection {
    let s = &state.session;
    let language = state.presentation_language;
    let query = state.query.to_lowercase();
    let mut matching_groups = s
        .proxies
        .proxies
        .iter()
        .filter(|_| state.page == 1)
        .filter(|(name, group)| {
            !group.all.is_empty()
                && !s
                    .settings
                    .claude_protection
                    .as_ref()
                    .is_some_and(|protection| protection.route == name.as_str())
                && list_presentation::group_matches(
                    name.as_str(),
                    group.all.iter().map(|node| node.as_str()),
                    &query,
                )
        })
        .collect::<Vec<_>>();
    if let Some(index) = matching_groups
        .iter()
        .position(|(name, _)| s.active_group() == Some(name.as_str()))
    {
        matching_groups[..=index].rotate_right(1);
    }
    let matching_groups = list_presentation::page(
        matching_groups,
        state.group_page,
        8,
        s.settings.proxy_groups_paginated,
    );
    let group_page = (matching_groups.index, matching_groups.total);
    let mut groups = vec![];
    for (index, (name, group)) in matching_groups.items.into_iter().enumerate() {
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
                list_presentation::contains_query(node, &query)
                    || list_presentation::contains_query(name.as_str(), &query)
            },
        );
        let count = indices.len();
        let page = state
            .node_pages
            .get(name.as_str())
            .copied()
            .unwrap_or(0)
            .min(count.saturating_sub(1) / 48);
        let parent_group = name.as_str();
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
                let details = proxy_order::node_details(&name, parent_group, &s.proxies);
                (name, proxy, details)
            })
            .collect();
        groups.push(GroupView {
            name: name.to_string(),
            group: group.clone(),
            expanded: list_presentation::group_expanded(
                name.as_str(),
                s.active_group(),
                &query,
                index,
                &state.group_expansion,
            ),
            nodes,
            count,
            page,
            testing: matches!(&state.delay_scope,Some(DelayScope::All))
                || matches!(&state.delay_scope,Some(DelayScope::Group(group)) if group==name.as_str()),
        });
    }
    let matching_profiles = list_presentation::profiles(
        if state.page == 2 { &s.profiles } else { &[] },
        &query,
        state.profile_page,
        s.settings.profiles_paginated,
    );
    let profile_page = (matching_profiles.index, matching_profiles.total);
    let profiles = matching_profiles.items.into_iter().cloned().collect();
    let node = s.node.clone();
    let document = if state.page == 4 {
        Some(s.document())
    } else {
        None
    };
    let rules = state.rule_rows.as_deref().unwrap_or_default();
    let saved = rule_manager::ProfileRules::for_profile(
        &s.settings,
        &s.rule_profile,
        s.profile_builtin(&s.rule_profile),
    );
    let connections = state.connection_page.project(
        if matches!(state.page, 0 | 3) {
            &s.connections.connections
        } else {
            &[]
        },
        &query,
        list_presentation::connection_sort(&state.connection_sort),
    );
    let connection_page = (connections.index, connections.total);
    let connections = connections.items.into_iter().cloned().collect();
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
    state.rule_page = rule_page;
    let rules = rules
        .into_iter()
        .skip(rule_page * 100)
        .take(100)
        .map(|(index, row)| (index, row.clone()))
        .collect();
    let connection_detail =
        state
            .connection_detail
            .as_ref()
            .map(|detail| ConnectionDetailProjection {
                selection: detail.selection.clone(),
                status: detail.status(),
                rows: detail.values(language).unwrap_or_default(),
            });
    let report = core::ip_report::Report::from_results(&state.ip_results);
    let categories = report
        .categories
        .iter()
        .map(|category| site_report::category_row(category, language))
        .collect();
    let mut sites = vec![];
    let mut site_regions = std::collections::BTreeSet::new();
    let mut site_details = vec![];
    let site_query = state.site_query.trim().to_lowercase();
    let mut exit = (
        "—".to_string(),
        language.text("未检测", "Not checked").to_string(),
    );
    for (index, service) in core::ip_check::services().iter().enumerate() {
        let slot = state.ip_results.get(index).and_then(Option::as_ref);
        let status = core::ip_report::Status::of(slot);
        if service.id == "exit-ip"
            && let Some(Ok(value)) = slot
        {
            exit = (value.summary.clone(), value.country.clone());
        }
        let (value, country, tone) = match slot {
            Some(Ok(value)) => (
                if value.authentication_check_skipped {
                    status.label_in(language)
                } else {
                    status.summary_in(&value.summary, language)
                }
                .to_owned(),
                value.country.clone(),
                match value.state {
                    core::ip_check::State::Confirmed | core::ip_check::State::Identified => 1,
                    core::ip_check::State::Partial
                    | core::ip_check::State::Reachable
                    | core::ip_check::State::Unknown => 2,
                    _ => 3,
                },
            ),
            Some(Err(_)) => (
                language.text("检测失败", "Check failed").into(),
                String::new(),
                3,
            ),
            None => (
                language.text("尚未检测", "Not checked").into(),
                String::new(),
                4,
            ),
        };
        if !country.is_empty() {
            site_regions.insert(country.clone());
        }
        if service.id == state.site_selected {
            site_details = site_report::detail_rows(service, slot, &value, &country, language);
        }
        if state.category != "all" && service.group != state.category {
            continue;
        }
        let status = status.to_string();
        let status_matches = state.site_status == "全部结果" || state.site_status == status;
        if !status_matches
            || (state.site_region != "全部地区" && country != state.site_region)
            || !site_report::matches_query(service, slot, &value, language, &site_query)
        {
            continue;
        }
        sites.push(ReportRow {
            id: service.id.clone(),
            title: service.name.clone(),
            value,
            detail: core::ip_report::list_detail_in(service, slot, language),
            source: service.group.clone(),
            status_label: status,
            tone,
            country_label: site_report::country_label(slot, &country, language),
            country,
            millis: slot
                .and_then(|result| result.as_ref().ok())
                .and_then(core::ip_check::CheckResult::response_millis),
            block_tone: 0,
            block_items: 0,
        });
    }
    let mut claude = vec![];
    let mut claude_counts = (0, 0, 0);
    if let Some(report) = state.claude.report() {
        for signal in &report.signals {
            let country_label =
                if signal.group == core::claude_check::Group::Dns && !signal.ip.is_empty() {
                    match language {
                        config::Language::SimplifiedChinese => {
                            core::probe::country_name(&signal.country).unwrap_or_default()
                        }
                        config::Language::English => core::countries::english_name(&signal.country)
                            .unwrap_or_default()
                            .to_owned(),
                    }
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
            let order = core::claude_check::summary_order(Some(signal), tone == 3);
            let section = core::claude_check::report_section(signal.group);
            let row = ReportRow {
                id: section.into(),
                title: signal.display_title_in(language),
                value: if country_label.is_empty() {
                    signal.display_value_in(language)
                } else {
                    signal.ip.clone()
                },
                detail: signal.explanation_in(language),
                source: signal.source_in(language),
                status_label: String::new(),
                tone,
                country: signal.country.clone(),
                country_label,
                millis: None,
                block_tone: 0,
                block_items: 0,
            };
            claude.push(core::claude_check::BlockRow {
                section,
                title: row.title.clone(),
                value: row.value.clone(),
                priority: order,
                tone,
                summary_hint: false,
                item: row,
            });
        }
        for routes in core::claude_check::group_routes(&report.routes) {
            let route = routes[0];
            let grouped = routes.len() > 1;
            let tone = if route.risky() { 3 } else { 2 };
            if tone == 3 {
                claude_counts.1 += routes.len();
            } else {
                claude_counts.2 += routes.len();
            }
            let section = core::claude_check::REPORT_SECTIONS[1];
            let row = ReportRow {
                id: section.into(),
                title: if grouped {
                    language
                        .text("Claude 连接与选路", "Claude connections and routing")
                        .into()
                } else {
                    route.display_title_in(language).into()
                },
                value: if grouped {
                    core::claude_check::route_group_value_in(&routes, language)
                } else {
                    route.display_outcome_in(language)
                },
                detail: if grouped {
                    core::claude_check::route_group_detail_in(&routes, language)
                } else {
                    route.explanation_in(language)
                },
                source: language
                    .text("内核连接记录", "Core connection records")
                    .into(),
                status_label: String::new(),
                tone,
                country: String::new(),
                country_label: String::new(),
                millis: None,
                block_tone: 0,
                block_items: 0,
            };
            claude.push(core::claude_check::BlockRow {
                section,
                title: row.title.clone(),
                value: row.value.clone(),
                priority: core::claude_check::summary_order(None, tone == 3),
                tone,
                summary_hint: grouped,
                item: row,
            });
        }
    }
    let guard = &state.guard_snapshot;
    let guard_enabled = s.settings.claude_protection.is_some();
    if guard_enabled || guard.blocked > 0 || !guard.routes.is_empty() || !guard.error.is_empty() {
        let mut signal = core::claude_check::guard::report_signal(guard);
        if !guard_enabled
            || !state.protection_observed
            || !s.running
            || !s.vpn
            || !s.platform["protection"]["enabled"]
                .as_bool()
                .unwrap_or(false)
        {
            core::claude_check::guard::mark_report_history(&mut signal);
        }
        let tone = if signal.risk {
            claude_counts.1 += 1;
            3
        } else {
            claude_counts.2 += 1;
            2
        };
        let section = core::claude_check::guard::REPORT_SECTION;
        let row = ReportRow {
            id: section.into(),
            title: signal.display_title_in(language),
            value: signal.display_value_in(language),
            detail: signal.explanation_in(language),
            source: signal.source_in(language),
            status_label: String::new(),
            tone,
            country: String::new(),
            country_label: String::new(),
            millis: None,
            block_tone: 0,
            block_items: 0,
        };
        claude.push(core::claude_check::BlockRow {
            section,
            title: row.title.clone(),
            value: row.value.clone(),
            priority: core::claude_check::summary_order(Some(&signal), tone == 3),
            tone,
            summary_hint: false,
            item: row,
        });
    }
    let guard_order = core::claude_check::summary_order(None, true);
    for route in &state.guard_snapshot.routes {
        claude_counts.1 += 1;
        let section = core::claude_check::guard::REPORT_SECTION;
        let row = ReportRow {
            id: section.into(),
            title: route.display_title_in(language).into(),
            value: route.display_outcome_in(language),
            detail: route.explanation_in(language),
            source: language
                .text(
                    "实时拦截 · 内核连接记录",
                    "Live blocking · core connection records",
                )
                .into(),
            status_label: String::new(),
            tone: 3,
            country: String::new(),
            country_label: String::new(),
            millis: None,
            block_tone: 0,
            block_items: 0,
        };
        claude.push(core::claude_check::BlockRow {
            section,
            title: row.title.clone(),
            value: row.value.clone(),
            priority: guard_order,
            tone: 3,
            summary_hint: false,
            item: row,
        });
    }
    let mut claude_headers = Vec::new();
    let mut claude_rows = Vec::new();
    for block in core::claude_check::report_blocks_in(claude, language) {
        let block_items = block.rows.len() as i32;
        claude_headers.push(ReportRow {
            id: block.section.into(),
            title: block.section.into(),
            value: block.overview,
            detail: block.attention,
            source: String::new(),
            status_label: String::new(),
            tone: 0,
            country: String::new(),
            country_label: String::new(),
            millis: None,
            block_tone: block.tone,
            block_items,
        });
        claude_rows.extend(block.rows);
    }
    claude_headers.extend(claude_rows);
    let protection_error = state
        .protection_rollback_error
        .as_deref()
        .filter(|_| state.protection_change.is_none());
    let protection_pending = state.protection_change.is_some()
        || (state.saved_protection_recovery.is_active() && protection_error.is_none());
    let protection_health = if let Some(error) = protection_error {
        core::protection::status::Status {
            state: core::protection::status::State::Error,
            message: error.to_owned(),
        }
    } else {
        core::protection::status::Status::observe(
            guard_enabled,
            protection_pending,
            s.running,
            s.vpn
                && s.platform["vpn_held"] == false
                && s.platform["vpn_transition_pending"] == false,
            state.protection_observed.then_some((
                guard.connected,
                if !guard.error.is_empty() {
                    &guard.error
                } else if !s.platform["protection"]["enabled"]
                    .as_bool()
                    .unwrap_or(false)
                {
                    language.text(
                        "实时拦截未运行，请重新连接 VPN",
                        "Live blocking is not running; reconnect the VPN",
                    )
                } else {
                    ""
                },
            )),
        )
    };
    let (protection_status, protection_health) = protection_presentation::project(
        protection_error.unwrap_or(&state.protection_status),
        protection_health,
        language,
    );
    Projection {
        language,
        application_policies: state.application_policies.view(s, language),
        overview: state.overview.present_in(s.running, language),
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
        profiles,
        rule_profiles: s
            .profiles
            .iter()
            .map(|profile| (profile.id.clone(), profile.name.clone()))
            .collect(),
        connections,
        rates: (
            format!("{}/s", bytes(s.rates.0)),
            format!("{}/s", bytes(s.rates.1)),
        ),
        platform: s.platform.clone(),
        rule_profile: s.rule_profile.clone(),
        rule_profile_label: s
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
        claude_busy: state.claude_busy || state.claude_browser_status == BrowserStatus::Sampling,
        claude_browser_status: state.claude_browser_status.render(language),
        claude_browser_retry: state.claude_browser_status.retryable()
            && can_run_active_probe(state),
        claude: claude_headers,
        claude_counts,
        progress: (report.totals.completed(), report.totals.total()),
        detecting: state.detecting,
        exit,
        probe: state.probe.lines(language),
        probe_busy: state.probe_busy,
        network_tool: state.network_tool,
        geo: state
            .geo
            .as_deref()
            .map(|status| core::geodata::localize_status(status, language))
            .unwrap_or_else(|| {
                language
                    .text("Geo 数据状态不可用", "Geo data status unavailable")
                    .into()
            }),
        group_page,
        profile_page,
        rules,
        connection_page,
        connection_cursor: state.connection_page.cursor(),
        rule_page: (rule_page, rule_count),
        rule_form: state.rule_form.clone(),
        rule_editor: state.rule_editor.clone(),
        picked_profile_name: state.picked_profile_name.take(),
        profile_file_selected: state.profile_picker.file_selected(),
        profile_file_pending: state.profile_picker.pending().is_some(),
        update_status: state.update_status.render(language),
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
        site_summary: report.totals.summary_in(language),
        claude_status: state.claude_status.render(language),
        protection_pending,
        protection_retry: !protection_pending
            && (protection_error.is_some()
                || (guard_enabled && (!s.running || !protected_vpn_attached(s)))),
        protection_status,
        protection_health,
        policies: document.map(|d| d.policies.clone()).unwrap_or_default(),
        shortcuts: std::array::from_fn(|index| {
            document.is_some_and(|doc| doc.included[index] || s.draft.enabled[index])
        }),
        shortcut_available: std::array::from_fn(|index| {
            document.is_some_and(|doc| !doc.builtin && !doc.included[index])
        }),
        dirty: s.draft != *saved,
        rule_warning: list_presentation::rule_warning(&state.rule_warnings, language),
        logs: if state.page == 5 {
            list_presentation::logs(&state.logs, &state.log_level, &query, language)
        } else {
            Vec::new()
        },
        connection_detail,
    }
}

fn apply(ui: &AndroidWindow, p: Projection) {
    if ui.get_current_page() != p.page {
        return;
    }
    let language = p.language;
    if let Err(error) = language_ui::apply(ui, language) {
        log::error!("Could not select the UI language: {error}");
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
                .map(|application| {
                    let icon = if application.icon.is_empty() {
                        None
                    } else {
                        slint::Image::load_from_path(std::path::Path::new(&application.icon)).ok()
                    };
                    ApplicationPolicyApp {
                        name: application.name.into(),
                        package: application.package.into(),
                        selected: application.selected,
                        available: application.available,
                        assignment: application.assignment.into(),
                        has_icon: icon.is_some(),
                        icon: icon.unwrap_or_default(),
                    }
                })
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
                .map(
                    |(id, name, target, status, count, applications)| ApplicationPolicyItem {
                        id: id.into(),
                        name: name.into(),
                        target: target.into(),
                        status: status.into(),
                        count,
                        applications: applications.into(),
                    },
                )
                .collect(),
        ),
    ));
    if policies.get_policies_revision() != p.application_policies.revision {
        policies.set_policies_revision(p.application_policies.revision);
        policies.set_policies_name(p.application_policies.form.0.into());
        policies.set_policies_target(p.application_policies.form.1.into());
        policies.set_policies_reject_missing(p.application_policies.form.2);
        policies.set_policies_editing(p.application_policies.editing);
        policies.set_policies_choosing(false);
        policies.set_policies_query(p.application_policies.query.into());
        if view.get_detail() == 19 {
            view.set_draft_dirty(false);
        }
    }
    policies.set_policies_filter(p.application_policies.filter);
    policies.set_policies_system(p.application_policies.show_system);
    policies.set_policies_selected(p.application_policies.selected);
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
    data.set_probe_results(stable(
        data.get_probe_results(),
        rows(p.probe.into_iter().map(Into::into).collect()),
    ));
    data.set_probe_busy(p.probe_busy);
    data.set_network_tool(p.network_tool);
    data.set_geo_status(p.geo.into());
    data.set_site_summary(p.site_summary.into());
    data.set_claude_status(p.claude_status.into());
    data.set_claude_busy(p.claude_busy);
    data.set_claude_browser_status(p.claude_browser_status.into());
    data.set_claude_browser_retry(p.claude_browser_retry);
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
                        (true, true) => language.text("自定义、DNS 覆盖", "Custom, DNS override"),
                        (true, false) => language.text("自定义", "Custom"),
                        _ => language.text("DNS 覆盖", "DNS override"),
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
    data.set_groups_paginated(p.settings.proxy_groups_paginated);
    data.set_profiles_paginated(p.settings.profiles_paginated);
    for (page, count, size, paginated, profiles) in [
        (
            p.group_page.0,
            p.group_page.1,
            8,
            p.settings.proxy_groups_paginated,
            false,
        ),
        (
            p.profile_page.0,
            p.profile_page.1,
            60,
            p.settings.profiles_paginated,
            true,
        ),
    ] {
        let pages = if paginated {
            count.div_ceil(size).max(1)
        } else {
            1
        };
        let label = match (language, paginated, profiles) {
            (config::Language::SimplifiedChinese, true, _) => {
                format!("{} / {pages} · {count} 条", page + 1)
            }
            (config::Language::English, true, _) => {
                format!("{} / {pages} · {count} items", page + 1)
            }
            (config::Language::SimplifiedChinese, false, true) => format!("{count} 条订阅"),
            (config::Language::SimplifiedChinese, false, false) => format!("{count} 个代理组"),
            (config::Language::English, false, true) => format!("{count} subscriptions"),
            (config::Language::English, false, false) => format!("{count} groups"),
        };
        if profiles {
            data.set_profile_page(label.into());
            data.set_profile_page_index(page as i32);
            data.set_profile_previous(page > 0);
            data.set_profile_next(paginated && (page + 1) * size < count);
        } else {
            data.set_group_page(label.into());
            data.set_group_page_index(page as i32);
            data.set_group_previous(page > 0);
            data.set_group_next(paginated && (page + 1) * size < count);
        }
    }
    for (page, count, size, rules) in [
        (
            p.connection_page.0,
            p.connection_page.1,
            CONNECTION_PAGE_SIZE,
            false,
        ),
        (p.rule_page.0, p.rule_page.1, 100, true),
    ] {
        let label = match language {
            config::Language::SimplifiedChinese => format!(
                "{} / {} · {count} 条",
                page + 1,
                count.div_ceil(size).max(1)
            ),
            config::Language::English => format!(
                "{} / {} · {count} items",
                page + 1,
                count.div_ceil(size).max(1)
            ),
        };
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
            DISPLAYED_CONNECTION_PAGE.set(Some(p.connection_cursor));
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
    data.set_profile_file_selected(p.profile_file_selected);
    data.set_profile_file_pending(p.profile_file_pending);
    if let Some(name) = p.picked_profile_name
        && ui.get_current_detail() == 3
        && ui.global::<MobileView>().get_selected_id().is_empty()
    {
        let view = ui.global::<MobileView>();
        view.set_profile_name(name.into());
        view.set_profile_source("".into());
        view.set_draft_dirty(true);
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
    data.set_rule_profile_label(p.rule_profile_label.into());
    let android = ui.global::<AndroidData>();
    android.set_ipv6_locked(!settings_input::ipv6_change_available(&p.platform));
    android.set_protection_retry(p.protection_retry);
    android.set_update_status(p.update_status.into());
    android.set_update_busy(p.update_busy);
    android.set_update_downloading(p.update_downloading);
    android.set_update_progress(p.update_progress);
    android.set_update_available(p.update_available);
    data.set_update_prereleases(p.settings.update_prereleases);
    data.set_language(p.settings.language.tag().into());
    data.set_update_preview(p.update_preview);
    data.set_update_version(p.update_version.into());
    data.set_update_confirm_wait(p.update_confirm_wait);
    android.set_authorization_status(if p.vpn {
        language.text("已连接", "Connected").into()
    } else {
        language.text("查看授权", "View authorization").into()
    });
    android.set_notification_speeds(p.platform["notification_speeds"].as_bool().unwrap_or(true));
    android.set_application_scope(
        p.platform["application_scope"]
            .as_str()
            .unwrap_or(language.text("全部应用", "All apps"))
            .into(),
    );
    if p.completed == Some(MobileIntent::SavePorts)
        && data.get_mixed_port().trim().parse::<u16>() == Ok(p.settings.mixed_port)
    {
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
    data.set_site_progress(
        match language {
            config::Language::SimplifiedChinese => {
                format!("完成 {} / {}", p.progress.0, p.progress.1)
            }
            config::Language::English => format!("Completed {} / {}", p.progress.0, p.progress.1),
        }
        .into(),
    );
    data.set_exit_ip(p.exit.0.into());
    let exit_country = core::countries::country_text(&p.exit.1);
    data.set_exit_country(
        match language {
            config::Language::SimplifiedChinese => exit_country,
            config::Language::English => core::countries::country_code(exit_country)
                .and_then(core::countries::english_name)
                .unwrap_or(exit_country),
        }
        .into(),
    );
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
            status_label: r.status_label.into(),
            tone: r.tone,
            flagged: flag.is_some(),
            flag: flag.unwrap_or_default().into(),
            delay: r
                .millis
                .map_or(String::new(), |millis| format!("{millis} ms"))
                .into(),
            delay_tone: if r.tone == 0 {
                r.block_tone
            } else {
                r.millis.map_or(4, proxy_order::response_tone)
            },
            block_items: r.block_items,
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
        ui.invoke_notify(language.text("内核已启动", "Core started").into());
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
                language.text("拦截广告", "Block ads"),
                language.text("私有域名直连", "Direct private domains"),
                language.text("私有 IP 直连", "Direct private IPs"),
                language.text("国内域名直连", "Direct domestic domains"),
                language.text("国内 IP 直连", "Direct domestic IPs"),
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
        rows(
            p.rule_profiles
                .into_iter()
                .map(|(id, label)| SelectOption {
                    id: id.into(),
                    label: label.into(),
                })
                .collect(),
        ),
    ));
    data.set_rule_policies(stable(
        data.get_rule_policies(),
        rows(p.policies.into_iter().map(Into::into).collect()),
    ));
    if let Some(detail) = p.connection_detail
        && ui.get_current_detail() == 5
        && detail.selection.matches(
            ui.get_selected_id().as_str(),
            view.get_connection_detail_request(),
        )
    {
        let status = match detail.status {
            DetailStatus::Unavailable => ConnectionDetailStatus::Unavailable,
            DetailStatus::Loading => ConnectionDetailStatus::Loading,
            DetailStatus::Available => ConnectionDetailStatus::Available,
            DetailStatus::Ended => ConnectionDetailStatus::Ended,
        };
        let next = rows(
            detail
                .rows
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
        );
        let previous = data.get_connection_detail();
        let rows = if detail
            .selection
            .matches(previous.id.as_str(), previous.request)
            && previous.status == status
        {
            stable(previous.rows, next)
        } else {
            next
        };
        data.set_connection_detail(ConnectionDetailSnapshot {
            id: detail.selection.id.into(),
            request: detail.selection.request,
            status,
            rows,
        });
    }
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
                            language.text("本地配置", "Local configuration").into()
                        } else {
                            core::subscription::summary(
                                profile.usage.as_deref(),
                                profile.updated,
                                language,
                            )
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
                        .map(|(name, node, details)| NodeItem {
                            group: group_id.clone().into(),
                            selected: name == group.now.as_str(),
                            name: name.into(),
                            kind: node.kind.to_string().into(),
                            delay: proxy_order::delay_label_in(node.delay, language).into(),
                            tone: proxy_order::delay_tone(node.delay),
                            valid: true,
                            details: details.into(),
                        })
                        .collect::<Vec<_>>();
                    GroupItem {
                        id: name.clone().into(),
                        title: name.into(),
                        detail: match language {
                            config::Language::SimplifiedChinese => {
                                format!("{} · {count} 个节点", group.kind)
                            }
                            config::Language::English => format!("{} · {count} nodes", group.kind),
                        }
                        .into(),
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
                        page: match language {
                            config::Language::SimplifiedChinese => {
                                format!("{} / {} · {count} 个", page + 1, count.div_ceil(48).max(1))
                            }
                            config::Language::English => format!(
                                "{} / {} · {count} items",
                                page + 1,
                                count.div_ceil(48).max(1)
                            ),
                        }
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
                        kind: parts
                            .as_ref()
                            .map_or(language.text("复杂规则", "Complex rule"), |parts| {
                                parts.kind
                            })
                            .into(),
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
                    original: "".into(),
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
            "session_attempts":SESSION_ATTEMPTS.load(std::sync::atomic::Ordering::Acquire),
            "language":data.get_language().as_str(),"presentation_language":data.get_presentation_language().as_str(),
            "page":ui.get_current_page(),"detail":ui.get_current_detail(),
            "working":data.get_working(),"profile_busy":data.get_profile_busy(),
            "draft_dirty":view.get_draft_dirty(),"modal":view.get_modal(),
            "groups":data.get_groups().row_count(),"profiles":data.get_profiles().row_count(),
            "connections":data.get_connections().row_count(),
            "connection_page":data.get_connection_page_index(),"connection_next":data.get_connection_next(),
            "connection_previous":data.get_connection_previous(),
            "connection_sort":view.get_connection_sort().as_str(),
            "protected":data.get_protected(),"protection_pending":data.get_protection_pending(),
            "rule_profiles":data.get_rule_profiles().row_count(),
            "vpn_pending":data.get_vpn_pending(),
            "delay_busy":data.get_delay_busy(),
            "logs":data.get_logs().row_count(),"dark":theme.get_dark(),"accent":theme.get_accent_index(),"text_scale":theme.get_text_scale(),
            "body_size":ui.global::<MobileTypography>().invoke_font_size(16),
            "width":size.width as f32/scale,"height":size.height as f32/scale,"scale":scale,
            "safe_top":view.get_safe_top(),"safe_bottom":view.get_safe_bottom(),
            "keyboard_inset":view.get_keyboard_inset(),
            "notice_active":!view.get_notice().is_empty(),"notice_error":view.get_notice_error(),
            "notice_modal":view.get_notice_modal(),"notice_revision":view.get_notice_revision()});
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
            "applications_snapshot":android.get_applications_snapshot().as_str(),
            "applications_visible":android.get_applications().row_count(),
            "applications_scroll_y":android.get_applications_scroll_y(),
            "applications_error":android.get_applications_error().as_str()});
    let policies = json!({
        "application_policies_loaded":!android.get_policies_profile().is_empty(),
        "application_policies_supported":android.get_policies_supported(),
        "application_policies_count":android.get_policies().row_count(),
        "application_policies_editing":android.get_policies_editing(),
        "application_policies_choosing":android.get_policies_choosing(),
        "application_policies_selected":android.get_policies_selected(),
        "application_policies_target_selected":!android.get_policies_target().is_empty(),
        "application_policies_reject_missing":android.get_policies_reject_missing()
    });
    for section in [targets, applications, policies] {
        if let Value::Object(fields) = section {
            snapshot.as_object_mut().unwrap().extend(fields);
        }
    }
    snapshot
}
