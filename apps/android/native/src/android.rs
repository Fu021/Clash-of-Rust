use anyhow::{Context, Result, bail, ensure};
use clash_mobile_runtime::{NativeHost, Session, Snapshot, core};
use core::{api, config, proxy_order, rule_manager};
use futures_util::{StreamExt, stream};
use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JClass, JObject, JString, JValue},
};
use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

slint::include_modules!();
static EVENTS: Mutex<Option<mpsc::Sender<String>>> = Mutex::new(None);
static AUTHORIZATION_REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
// Also included in exported diagnostics. Contains view counts/status, never URLs or secrets.
static PRESENTATION: Mutex<String> = Mutex::new(String::new());
static FIRST_APPEARANCE: Mutex<Option<(bool, i32)>> = Mutex::new(None);

struct AndroidHost {
    vm: JavaVM,
    activity: GlobalRef,
}
impl AndroidHost {
    fn new(app: &slint::android::AndroidApp) -> Result<Self> {
        // android-activity owns the VM and Activity for this process.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
        let activity = {
            let env = vm.attach_current_thread()?;
            let object = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
            env.new_global_ref(object)?
        };
        Ok(Self { vm, activity })
    }
    fn directory(&self, method: &str) -> Result<PathBuf> {
        let mut env = self.vm.attach_current_thread()?;
        let directory = env
            .call_method(self.activity.as_obj(), method, "()Ljava/io/File;", &[])?
            .l()?;
        let path = env
            .call_method(directory, "getAbsolutePath", "()Ljava/lang/String;", &[])?
            .l()?;
        let text: String = env.get_string(&JString::from(path))?.into();
        Ok(PathBuf::from(text))
    }
}
impl NativeHost for AndroidHost {
    fn request(&self, value: Value) -> Result<Value> {
        let mut env = self.vm.attach_current_thread()?;
        let result: Result<Value> = env.with_local_frame(16, |env| {
            let input = env.new_string(value.to_string())?;
            let response = env
                .call_method(
                    self.activity.as_obj(),
                    "request",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    &[JValue::Object(input.as_ref())],
                )?
                .l()?;
            let response: String = env.get_string(&JString::from(response))?.into();
            Ok(serde_json::from_str(&response)?)
        });
        if env.exception_check()? {
            env.exception_clear()?;
            bail!("Android 原生操作失败");
        }
        clash_mobile_runtime::native_response(result?)
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_clashofrust_android_MainActivity_nativeEvent(
    mut env: JNIEnv<'_>,
    _: JClass<'_>,
    text: JString<'_>,
) {
    let value = env.get_string(&text).map(String::from);
    if let Ok(value) = value
        && let Ok(sender) = EVENTS.lock()
        && let Some(sender) = sender.as_ref()
    {
        let _ = sender.try_send(value);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_clashofrust_android_MainActivity_nativeDiagnostics(
    env: JNIEnv<'_>,
    _: JClass<'_>,
) -> jni::sys::jstring {
    let snapshot = PRESENTATION.lock().map(|s| s.clone()).unwrap_or_default();
    let mut snapshot: Value = serde_json::from_str(&snapshot).unwrap_or_else(|_| json!({}));
    if let Ok(first) = FIRST_APPEARANCE.lock()
        && let Some((dark, accent)) = *first
    {
        snapshot["startup_dark"] = json!(dark);
        snapshot["startup_accent"] = json!(accent);
    }
    env.new_string(snapshot.to_string())
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[derive(Clone)]
struct RuleForm {
    kind: String,
    payload: String,
    policy: String,
}
#[derive(Clone)]
struct RuleEditor {
    form: RuleForm,
    raw: String,
    position: usize,
    text_mode: bool,
}
struct Request {
    intent: MobileIntent,
    a: String,
    b: String,
    selected: String,
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
    page: i32,
    query: String,
    selected: String,
    notice: String,
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
    category: String,
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
    rule_form: Option<RuleForm>,
    rule_editor: Option<RuleEditor>,
    update_package: Option<core::update::Package>,
    update_status: String,
    update_busy: bool,
    update_progress: f32,
    update_downloaded: bool,
    update_path: PathBuf,
}

#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
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
    let root = native.directory("getFilesDir")?.join("client");
    let update_path = native.directory("getCacheDir")?.join("updates/update.apk");
    slint::android::init(app)?;
    let window = AndroidWindow::new()?;
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
    let (events, native_events) = mpsc::channel::<String>(16);
    if let Ok(mut sender) = EVENTS.lock() {
        *sender = Some(events);
    }
    let weak = window.as_weak();
    window
        .global::<MobileActions>()
        .on_request(move |intent, a, b| {
            let Some(ui) = weak.upgrade() else { return };
            ui.global::<MobileData>().set_working(!matches!(
                intent,
                MobileIntent::Navigate | MobileIntent::Query | MobileIntent::RuleTextMode
            ));
            if commands
                .try_send(Request {
                    intent,
                    a: a.into(),
                    b: b.into(),
                    selected: ui.get_selected_id().into(),
                })
                .is_err()
            {
                ui.global::<MobileData>().set_working(false);
                ui.invoke_notify("操作过于频繁，请稍后再试".into());
            }
        });
    let host = native.clone();
    window
        .global::<AndroidActions>()
        .on_request(move |intent, value| {
            let op = match intent {
                AndroidIntent::VpnSettings => "vpn-settings",
                AndroidIntent::ApplicationRouting => "application-routing",
                AndroidIntent::NotificationSpeeds => "notification-speeds",
                AndroidIntent::BatterySettings => "battery-settings",
                AndroidIntent::CheckUpdate => "open-url",
                AndroidIntent::Background => "background",
                AndroidIntent::SavePorts => return, // Routed through MobileActions below.
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
    std::thread::Builder::new()
        .name("mobile-runtime".into())
        .spawn(move || {
            let result = (|| -> Result<()> {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()?;
                runtime.block_on(run_loop(
                    root,
                    update_path,
                    native,
                    receive,
                    native_events,
                    weak,
                ))
            })();
            if let Err(error) = result {
                log::error!("Mobile runtime stopped: {error:#}");
            }
        })?;
    window.run()?;
    Ok(())
}

#[derive(Clone)]
enum DelayScope {
    All,
    Group(String),
    Node(String),
}
enum Update {
    Delays(u64, std::result::Result<BTreeMap<String, u32>, String>),
    Snapshot(u64, std::result::Result<Snapshot, String>),
    Ip(u64, usize, core::ip_report::ResultSlot),
    IpDone(u64),
    Claude(std::result::Result<core::claude_check::Report, String>),
    Protection(std::result::Result<core::claude_check::protection::Protection, String>),
    ProtectionDns(std::time::Instant, std::result::Result<(), String>),
    Address(u64, String, String),
    Log(String, String),
    Geo(std::result::Result<core::geodata::GeoManifest, String>),
    Checked(std::result::Result<Option<core::update::Package>, String>),
    DownloadProgress(u64, u64),
    Downloaded(std::result::Result<(), String>),
}
async fn run_loop(
    root: PathBuf,
    update_path: PathBuf,
    native: Arc<AndroidHost>,
    mut receive: mpsc::Receiver<Request>,
    mut native_events: mpsc::Receiver<String>,
    weak: slint::Weak<AndroidWindow>,
) -> Result<()> {
    let mut state = State {
        session: Session::open(root, native)?,
        page: 0,
        query: String::new(),
        selected: String::new(),
        notice: String::new(),
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
        category: "AI".into(),
        detecting: false,
        generation: 0,
        visible: true,
        completed: None,
        probe: ("尚未测试".into(), String::new()),
        dns: "尚未查询".into(),
        geo: "已内置离线数据".into(),
        geo_updating: false,
        node_pages: BTreeMap::new(),
        node_delays: BTreeMap::new(),
        delay_generation: 0,
        delay_scope: None,
        connection_page: 0,
        rule_page: 0,
        rule_rows: None,
        rule_form: None,
        rule_editor: None,
        update_package: None,
        update_status: "尚未检查更新".into(),
        update_busy: false,
        update_progress: 0.0,
        update_downloaded: false,
        update_path,
    };
    if let Err(error) = state.session.initialize() {
        state.notice = format!("内核启动失败：{error:#}");
        log::error!("{}", state.notice);
        state.logs.push_back(("ERROR".into(), state.notice.clone()));
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
    let mut delay_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut poll = tokio::time::interval(Duration::from_secs(1));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut refresh_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut refresh_generation = 0u64;
    let mut last_frame = std::time::Instant::now() - Duration::from_secs(1);
    let mut system_bars_state = None;
    let mut update_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut last_update_check = std::time::Instant::now() - Duration::from_secs(6 * 60 * 60);
    loop {
        let mut immediate = false;
        let mut refresh_requested = false;
        tokio::select! {
            value = receive.recv() => {
                let Some(request) = value else { break; };
                state.selected = request.selected.clone();
                immediate = true;
                let intent = request.intent;
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
                    | MobileIntent::RuleDelete | MobileIntent::RuleApply | MobileIntent::RuleDiscard
                    | MobileIntent::RuleShortcut | MobileIntent::ActivateProfile | MobileIntent::UpdateProfile
                    | MobileIntent::SaveProfile | MobileIntent::DeleteProfile) {
                    state.rule_rows = None;
                }
                if intent == MobileIntent::Navigate {
                    let page: i32 = request.a.parse()?;
                    state.page = page;
                    state.query.clear();
                    state.connection_page = 0;
                    state.rule_page = 0;
                    if let Some(job) = refresh_job.take() { job.abort(); }
                    refresh_generation += 1;
                    refresh_requested = true;
                } else if intent==MobileIntent::CheckUpdate {
                    if !state.update_busy {
                        state.update_busy=true;state.update_status="正在检查更新…".into();state.update_progress=0.0;
                        last_update_check=std::time::Instant::now();
                        update_job=Some(start_update_check(state.session.settings.mixed_port,updates.clone()));
                    }
                } else if intent==MobileIntent::InstallUpdate {
                    if !state.update_busy && let Some(package)=state.update_package.clone() {
                        if state.update_downloaded && state.update_path.is_file() {
                            if let Err(error)=state.session.native.request(json!({"op":"install-update"})) {state.notice=format!("{error:#}");}
                        } else {
                            state.update_busy=true;state.update_status="正在下载更新…".into();state.update_progress=0.0;
                            let path=state.update_path.clone();
                            let port=state.session.settings.mixed_port;let send=updates.clone();
                            update_job=Some(tokio::spawn(async move {
                                let report=send.clone();
                                let result=core::update::download(&package,Some(port),&path,move|n,total|{let _=report.try_send(Update::DownloadProgress(n,total));}).await;
                                let _=send.send(Update::Downloaded(result.map_err(|e|format!("{e:#}")))).await;
                            }));
                        }
                    }
                } else if intent == MobileIntent::UpdateGeo {
                    if !state.geo_updating {
                        state.geo_updating=true;
                        let runtime=state.session.store.runtime();
                        let port=state.session.running.then_some(state.session.settings.mixed_port);
                        let send=updates.clone();
                        geo_job=Some(tokio::spawn(async move {
                            let result=core::geodata::download_geo(&runtime,port).await.map_err(|e|format!("{e:#}"));
                            let _=send.send(Update::Geo(result)).await;
                        }));
                    }
                } else if matches!(intent, MobileIntent::SiteStart | MobileIntent::SiteRetest) {
                    if let Some(job) = checks.take() { job.abort(); }
                    state.generation += 1; state.detecting = true;
                    if intent == MobileIntent::SiteStart { state.ip_results.fill(None); }
                    let generation = state.generation;
                    let port = state.session.settings.mixed_port;
                    let send = updates.clone();
                    let targets=core::ip_check::services().iter().enumerate()
                        .filter(|(_,service)| intent == MobileIntent::SiteStart || service.group == request.a)
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
                        last_delay=std::time::Instant::now();
                        let scope=match intent { MobileIntent::GroupDelay=>DelayScope::Group(request.a),
                            MobileIntent::DelayAll=>DelayScope::All,_=>DelayScope::Node(request.a) };
                        delay_job=Some(start_delay(&mut state,scope,updates.clone()));
                    }
                } else if intent == MobileIntent::ClaudeProtection && request.a == "true" {
                    if state.protection_change.is_none() && state.session.settings.claude_protection.is_none() {
                        cancel_delay(&mut state,&mut delay_job);
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
                    match handle(&mut state, request).await {
                        Ok(()) => {
                            state.completed = Some(intent);
                            if matches!(intent,MobileIntent::SelectNode|MobileIntent::ActivateProfile|MobileIntent::UpdateProfile
                                |MobileIntent::SaveProfile|MobileIntent::DeleteProfile|MobileIntent::RestartCore|MobileIntent::Mode
                                |MobileIntent::DnsOverride|MobileIntent::Ipv6|MobileIntent::Udp|MobileIntent::SavePorts
                                |MobileIntent::ClaudeProtection) {cancel_delay(&mut state,&mut delay_job);}
                            if !matches!(intent, MobileIntent::Query | MobileIntent::Copy | MobileIntent::ConnectionSort
                                | MobileIntent::ConnectionDetail | MobileIntent::ConnectionPage | MobileIntent::RulePage
                                | MobileIntent::LogLevel | MobileIntent::ClearLogs | MobileIntent::RuleTextMode
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
                    if serde_json::from_str::<Value>(&value).ok().is_some_and(|v|v["event"]=="back") {
                        let weak=weak.clone();slint::invoke_from_event_loop(move||{if let Some(ui)=weak.upgrade(){ui.invoke_native_back();}})?;
                    } else if let Err(error) = native_event(&mut state,&value).await { state.notice=format!("{error:#}"); }
                }
            }
            value = update_rx.recv() => {
                match value {
                    Some(Update::ProtectionDns(token, result)) => {
                        if let Some(change) = state.protection_change.as_mut()
                            && change.deadline == token {
                            protection_dns_job = None;
                            match result {
                                Ok(()) => change.verified = true,
                                Err(error) => {
                                    if let Err(restore) = rollback_protection(&mut state, &format!("保护 DNS 验证失败：{error}")) {
                                        state.notice = format!("保护 DNS 验证失败：{error}；恢复失败：{restore:#}");
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
                                state.notice=format!("测速完成 · {} 项，{} 项超时",values.len(),failed);
                                if all {state.node_delays=values;} else {state.node_delays.extend(values);}
                            }
                            Err(error)=>state.notice=format!("测速失败：{error}"),
                        }
                        immediate=true;refresh_requested=true;
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
                    Some(Update::Claude(Ok(report))) => { state.claude=Some(report); state.claude_status="检测完成 · 浏览器及网络采样已结束".into(); state.notice="检测完成".into(); },
                    Some(Update::Claude(Err(error))) => {state.claude_status=format!("检测失败：{error}");state.notice=error;},
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
                        }
                        refresh_requested=true;
                    }
                    Some(Update::Log(level,message)) => state.logs.push_back((level,message)),
                    Some(Update::Geo(result))=> {
                        if let Some(job) = refresh_job.take() { job.abort(); }
                        refresh_generation += 1;
                        refresh_requested = true;
                        state.geo_updating=false;geo_job=None;
                        match result.and_then(|manifest|state.session.initialize().map(|()|manifest).map_err(|e|format!("{e:#}"))) {
                            Ok(manifest)=> {state.geo=manifest.version;state.notice="Geo 数据已更新".into();},
                            Err(error)=>state.notice=format!("Geo 更新失败：{error}"),
                        }
                    }
                    Some(Update::Checked(result))=>{
                        state.update_busy=false;
                        match result {
                            Ok(package)=>{
                                state.update_status=package.as_ref().map(|p|format!("发现新版本 {} · {:.1} MiB",p.version,p.size as f64/1024.0/1024.0))
                                    .unwrap_or_else(||"当前没有更新的 Android 正式版".into());
                                state.update_downloaded=state.update_downloaded && state.update_package.as_ref().map(|p|&p.version)==package.as_ref().map(|p|&p.version);
                                state.update_package=package;
                            }
                            Err(error)=>state.update_status=error,
                        }
                    }
                    Some(Update::DownloadProgress(n,total))=>{
                        state.update_progress=n as f32/total.max(1) as f32;
                        state.update_status=format!("正在下载更新… {:.0}%",state.update_progress*100.0);
                    }
                    Some(Update::Downloaded(result))=>{
                        state.update_busy=false;
                        match result {
                            Ok(())=>{
                                state.update_downloaded=true;state.update_status="下载完成，请在系统界面确认安装".into();
                                if let Err(error)=state.session.native.request(json!({"op":"install-update"})) {state.notice=format!("{error:#}");}
                            }
                            Err(error)=>state.update_status=error,
                        }
                    }
                    _ => {}
                }
            }
            _ = poll.tick() => {
                if state.session.running && !state.update_busy && last_update_check.elapsed()>=Duration::from_secs(6*60*60) {
                    last_update_check=std::time::Instant::now();state.update_busy=true;state.update_status="正在检查更新…".into();
                    update_job=Some(start_update_check(state.session.settings.mixed_port,updates.clone()));
                }
                let minutes=state.session.settings.delay_interval_minutes;
                if state.session.running && state.protection_change.is_none() && minutes>0
                    && last_delay.elapsed()>=Duration::from_secs(minutes as u64*60) && state.delay_scope.is_none() {
                    last_delay=std::time::Instant::now();
                    delay_job=Some(start_delay(&mut state,DelayScope::All,updates.clone()));
                }
                refresh_requested = true;
            }
        }
        if refresh_requested {
            if let Err(error) = state.session.observe_native() {
                state
                    .logs
                    .push_back(("WARN".into(), format!("状态刷新失败：{error:#}")));
            }
            match observe_protection(&mut state) {
                Ok(true) => start_claude_check(&mut state, &mut claude_job, updates.clone()),
                Ok(false) => {}
                Err(error) => {
                    state.notice = format!("保护状态异常：{error:#}");
                    state.logs.push_back(("ERROR".into(), state.notice.clone()));
                }
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
                state.protection_status = "正在验证保护节点与加密 DNS…".into();
                protection_dns_job = Some(tokio::spawn(async move {
                    let result = api
                        .wait_for_dns("www.cloudflare.com", Duration::from_secs(30))
                        .await
                        .map(|_| ())
                        .map_err(|error| format!("{error:#}"));
                    let _ = send.send(Update::ProtectionDns(token, result)).await;
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
            if state.session.vpn {
                let _ = state.session.native.request(json!({"op":"traffic", "down":format!("{}/s",bytes(state.session.rates.1)), "up":format!("{}/s",bytes(state.session.rates.0))}));
            }
        }
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
                                    let text = value["payload"]
                                        .as_str()
                                        .unwrap_or("")
                                        .chars()
                                        .take(8192)
                                        .collect();
                                    let _ = send.try_send(Update::Log(level, text));
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
        if !state.visible || (!immediate && last_frame.elapsed() < Duration::from_millis(100)) {
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
                state.rule_rows = Some(state.session.document().rows(&state.session.draft).0);
            }
        } else {
            state.rule_rows = None;
        }
        let view = project(&state);
        state.completed = None;
        state.rule_form = None;
        state.rule_editor = None;
        let notice = std::mem::take(&mut state.notice);
        let weak = weak.clone();
        slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                apply(&ui, view);
                if !notice.is_empty() {
                    ui.invoke_notify(notice.into());
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
    if let Some(job) = update_job {
        job.abort();
    }
    Ok(())
}

fn start_update_check(port: u16, send: mpsc::Sender<Update>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let suffix = if cfg!(target_arch = "aarch64") {
            "android-arm64.apk"
        } else if cfg!(target_arch = "arm") {
            "android-armv7.apk"
        } else {
            "android-x64.apk"
        };
        let result = core::update::check(core::VERSION, suffix, Some(port)).await;
        let _ = send
            .send(Update::Checked(result.map_err(|e| format!("{e:#}"))))
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
            state.session.commit_settings(previous)?;
            state
                .session
                .native
                .request(json!({"op":if change.before_vpn {"reconnect"} else {"disconnect"}}))?;
        }
    }
    state.protection_status = reason.to_owned();
    state.notice = reason.to_owned();
    state.logs.push_back(("WARN".into(), reason.to_owned()));
    Ok(())
}

/// Returns true only after a user-requested enable has completed successfully.
fn observe_protection(state: &mut State) -> Result<bool> {
    let native = &state.session.platform["protection"];
    let active = native["enabled"].as_bool().unwrap_or(false);
    if active {
        state.guard_snapshot = serde_json::from_value(native["snapshot"].clone())?;
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
    state.claude = None;
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
            core::claude_check::enhanced::extend(&client, &mut report, None).await;
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
            .send(Update::Claude(result.map_err(|error| error.to_string())))
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
                state.session.commit_settings(next)?;
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
        "error" | "notice" => {
            state.notice = value["value"].as_str().unwrap_or("").into();
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
    } = request;
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
                    | MobileIntent::SavePorts
                    | MobileIntent::RuleApply
                    | MobileIntent::Ipv6
                    | MobileIntent::DnsOverride
                    | MobileIntent::Udp
                    | MobileIntent::ClaudeProtection
            ),
        "请等待 Claude 保护切换完成"
    );
    let session = &mut state.session;
    match intent {
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
            let (mixed, controller) = a.split_once(',').context("端口格式无效")?;
            let mut next = session.settings.clone();
            next.mixed_port = mixed.parse()?;
            next.controller_port = controller.parse()?;
            session.commit_settings(next)?;
            state.notice = "端口已保存".into();
        }
        MobileIntent::NodeSort => {
            let mut next = session.settings.clone();
            next.node_sort = *proxy_order::NodeSort::ALL
                .get(a.parse::<usize>()?)
                .context("排序无效")?;
            session.store.save_settings(&next)?;
            session.settings = next;
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
                    !parts.no_resolve
                        && [
                            "DOMAIN-SUFFIX",
                            "DOMAIN",
                            "IP-CIDR",
                            "IP-CIDR6",
                            "GEOIP",
                            "GEOSITE",
                            "RULE-SET"
                        ]
                        .contains(&parts.kind),
                    "此规则请继续使用文本编辑，以保留所有参数"
                );
                Some(RuleForm {
                    kind: parts.kind.into(),
                    payload: parts.payload.into(),
                    policy: parts.policy.into(),
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
            session.delete(&a)?;
            state.notice = "订阅已删除".into();
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
        MobileIntent::RuleEdit => {
            let (rows, _) = session.document().rows(&session.draft);
            let (index, row) = rows
                .iter()
                .enumerate()
                .find(|(_, row)| row.id == a)
                .context("规则已不存在，请重新选择")?;
            let parts = rule_manager::parts(&row.raw)?;
            let text_mode = parts.no_resolve
                || ![
                    "DOMAIN-SUFFIX",
                    "DOMAIN",
                    "IP-CIDR",
                    "IP-CIDR6",
                    "GEOIP",
                    "GEOSITE",
                    "RULE-SET",
                ]
                .contains(&parts.kind);
            state.rule_editor = Some(RuleEditor {
                form: RuleForm {
                    kind: parts.kind.into(),
                    payload: parts.payload.into(),
                    policy: parts.policy.into(),
                },
                raw: row.raw.to_string(),
                position: index + 1,
                text_mode,
            });
        }
        MobileIntent::RuleSave => {
            session.save_rule(&selected, &a, b.parse()?)?;
            state.notice = "已保存到草稿，请应用修改".into();
        }
        MobileIntent::RuleShortcut => {
            let index: usize = a.parse()?;
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
            let (rows, _) = session.document().rows(&session.draft);
            let mut ids: Vec<_> = rows.into_iter().map(|r| r.id).collect();
            let index = ids
                .iter()
                .position(|id| id == &selected)
                .context("请先选择规则")?;
            let next = (index as i32 + a.parse::<i32>()?)
                .clamp(0, ids.len().saturating_sub(1) as i32) as usize;
            ids.swap(index, next);
            session.draft.order = ids;
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
            session.store.save_settings(&next)?;
            session.settings = next;
        }
        MobileIntent::Interval => {
            let mut next = session.settings.clone();
            next.delay_interval_minutes = a.parse()?;
            session.store.save_settings(&next)?;
            session.settings = next;
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
        MobileIntent::SiteCategory => state.category = a,
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
                session.commit_settings(next)?;
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
    nodes: Vec<(String, api::Proxy)>,
    count: usize,
    page: usize,
    testing: bool,
}
struct Projection {
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
    dirty: bool,
    logs: Vec<(String, String)>,
    details: Vec<(String, String)>,
    rates: (String, String),
    platform: Value,
    rule_profile: String,
    completed: Option<MobileIntent>,
    categories: Vec<(String, String, String, i32)>,
    sites: Vec<ReportRow>,
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
    update_progress: f32,
    update_available: bool,
    geo_updating: bool,
    delay_busy: bool,
    site_summary: String,
    claude_status: String,
    protection_pending: bool,
    protection_status: String,
}
struct ReportRow {
    id: String,
    title: String,
    value: String,
    detail: String,
    source: String,
    tone: i32,
    country: String,
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
            nodes,
            count,
            page,
            testing: matches!(&state.delay_scope,Some(DelayScope::All))
                || matches!(&state.delay_scope,Some(DelayScope::Group(group)) if group==name.as_str()),
        });
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
        .filter(|_| state.page == 3)
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
        .filter(|(_, r)| query.is_empty() || r.raw.to_lowercase().contains(&query))
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
    let mut exit = ("—".to_string(), "未检测".to_string());
    for (index, service) in core::ip_check::services().iter().enumerate() {
        let slot = state.ip_results.get(index).and_then(Option::as_ref);
        if service.id == "exit-ip"
            && let Some(Ok(value)) = slot
        {
            exit = (value.summary.clone(), value.country.clone());
        }
        if state.category != "all" && service.group != state.category {
            continue;
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
        sites.push(ReportRow {
            id: service.id.clone(),
            title: service.name.clone(),
            value,
            detail,
            source: service.group.clone(),
            tone,
            country,
        });
    }
    let mut claude = vec![];
    let mut claude_counts = (0, 0, 0);
    if let Some(report) = &state.claude {
        for (index, signal) in report.signals.iter().enumerate() {
            let tone = if report.differs(signal) {
                claude_counts.1 += 1;
                3
            } else if signal.evidence == core::claude_check::Evidence::Measured {
                claude_counts.0 += 1;
                1
            } else {
                claude_counts.2 += 1;
                2
            };
            claude.push(ReportRow {
                id: index.to_string(),
                title: signal.title.clone(),
                value: signal.value.clone(),
                detail: signal.detail.clone(),
                source: signal.source.clone(),
                tone,
                country: signal.country.clone(),
            });
        }
    }
    for (index, route) in state.guard_snapshot.routes.iter().enumerate() {
        claude_counts.1 += 1;
        claude.push(ReportRow {
            id: format!("guard-{index}"),
            title: route.host.clone(),
            value: route.outcome.clone(),
            detail: format!(
                "规则：{}\n线路：{}\n时间：{}",
                route.rule, route.chain, route.time
            ),
            source: "实时拦截 · 内核连接记录".into(),
            tone: 3,
            country: String::new(),
        });
    }
    if !state.guard_snapshot.error.is_empty() {
        claude_counts.2 += 1;
        claude.push(ReportRow {
            id: "guard-status".into(),
            title: "实时保护状态".into(),
            value: state.guard_snapshot.error.clone(),
            detail: state.guard_snapshot.error.clone(),
            source: "Android 后台保护服务".into(),
            tone: 2,
            country: String::new(),
        });
    }
    Projection {
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
        update_progress: state.update_progress,
        update_available: state.update_package.is_some(),
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
        policies: document.map(|d| d.policies.clone()).unwrap_or_default(),
        shortcuts: s.draft.enabled,
        dirty: s.draft != *saved,
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
    if view.get_copy_revision() != p.copied.0 {
        view.set_copy_value(p.copied.1.as_ref().into());
        view.set_copy_revision(p.copied.0);
    }
    data.set_working(false);
    data.set_geo_updating(p.geo_updating);
    data.set_delay_busy(p.delay_busy);
    data.set_probe_status(p.probe.0.into());
    data.set_probe_time(p.probe.1.into());
    data.set_dns_result(p.dns.into());
    data.set_geo_status(p.geo.into());
    data.set_site_summary(p.site_summary.into());
    data.set_claude_status(p.claude_status.into());
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
            data.set_rule_previous(page > 0);
            data.set_rule_next((page + 1) * size < count);
        } else {
            data.set_connection_page(label.into());
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
        view.set_draft_dirty(false);
        ui.set_current_detail(2);
    }
    if p.completed == Some(MobileIntent::RuleSelect) {
        ui.global::<MobileView>().set_rule_selected(false);
    }
    data.set_protection_supported(true);
    data.set_protected(p.settings.claude_protection.is_some());
    data.set_protection_pending(p.protection_pending);
    data.set_protection_status(p.protection_status.into());
    data.set_upload_rate(p.rates.0.into());
    data.set_download_rate(p.rates.1.into());
    data.set_rule_profile(p.rule_profile.into());
    let android = ui.global::<AndroidData>();
    android.set_update_status(p.update_status.into());
    android.set_update_busy(p.update_busy);
    android.set_update_progress(p.update_progress);
    android.set_update_available(p.update_available);
    android.set_authorization_status(if p.vpn {
        "已连接".into()
    } else {
        "查看授权".into()
    });
    android.set_notification_speeds(p.platform["notification_speeds"].as_bool().unwrap_or(true));
    android.set_application_scope(match p.platform["excluded_apps"].as_u64().unwrap_or(0) {
        0 => "所有应用".into(),
        n => format!("已排除 {n} 个应用").into(),
    });
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
    data.set_exit_flag(flag(&p.exit.1));
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
        let flag = flag(&r.country);
        ReportItem {
            id: r.id.into(),
            title: r.title.into(),
            value: r.value.into(),
            detail: r.detail.into(),
            source: r.source.into(),
            tone: r.tone,
            flagged: flag.size().width > 0,
            flag,
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
    data.set_vpn_connected(p.vpn);
    data.set_profile(p.profile.into());
    data.set_node(p.node.into());
    data.set_connection_count(p.total.to_string().into());
    data.set_upload_total(p.upload.into());
    data.set_download_total(p.download.into());
    data.set_dns_override(p.settings.dns_override);
    data.set_ipv6(p.settings.ipv6);
    data.set_udp(p.settings.udp);
    if !android.get_ports_dirty() {
        data.set_mixed_port(p.settings.mixed_port.to_string().into());
        data.set_controller_port(p.settings.controller_port.to_string().into());
    }
    if !ui.global::<MobileView>().get_interval_dirty() {
        data.set_interval(p.settings.delay_interval_minutes.to_string().into());
    }
    if p.completed == Some(MobileIntent::Interval) {
        ui.global::<MobileView>().set_interval_dirty(false);
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
                .map(|profile| ProfileItem {
                    id: profile.id.clone().into(),
                    name: profile.name.into(),
                    source: profile.source.clone().into(),
                    detail: profile
                        .usage
                        .unwrap_or_else(|| format!("更新：{}", profile.updated))
                        .into(),
                    active: p.settings.active_profile.as_deref() == Some(&profile.id),
                    local: profile.source.is_empty(),
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
                .enumerate()
                .map(|(group_index, view)| {
                    let GroupView {
                        name,
                        group,
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
                        current: group.now.to_string().into(),
                        expanded: group_index == 0,
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
                .filter_map(|(index, row)| {
                    let parts = rule_manager::parts(&row.raw).ok()?;
                    Some(RuleItem {
                        id: row.id.into(),
                        position: (index + 1).to_string().into(),
                        kind: parts.kind.into(),
                        value: parts.payload.into(),
                        policy: parts.policy.into(),
                        source: row.source.to_string().into(),
                        selected: false,
                        enabled: row.enabled,
                    })
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
    let dark = theme.get_dark();
    // Read back the actual Slint globals; diagnostics must reveal stale presentation too.
    if let Ok(mut snapshot) = PRESENTATION.lock() {
        let view = ui.global::<MobileView>();
        let scale = ui.window().scale_factor();
        let size = ui.window().size();
        *snapshot = json!({"core_running":data.get_core_running(),"vpn":data.get_vpn_connected(),
            "page":ui.get_current_page(),"detail":ui.get_current_detail(),
            "groups":data.get_groups().row_count(),"profiles":data.get_profiles().row_count(),
            "logs":data.get_logs().row_count(),"dark":dark,"accent":theme.get_accent_index(),"text_scale":theme.get_text_scale(),
            "body_size":typography.invoke_font_size(16),
            "width":size.width as f32/scale,"height":size.height as f32/scale,"scale":scale,
            "safe_top":view.get_safe_top(),"safe_bottom":view.get_safe_bottom(),
            "settings_scroll_y":view.get_settings_scroll_y(),"settings_content_height":view.get_settings_content_height(),
            "settings_viewport_height":view.get_settings_viewport_height()})
        .to_string();
    }
}

fn flag(country: &str) -> slint::Image {
    const PIXELS: &[u8] = include_bytes!("../../../../resources/flags/flags.rgba");
    static CODES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let codes = CODES.get_or_init(|| {
        serde_json::from_str(include_str!("../../../../resources/flags/codes.json"))
            .expect("flag codes")
    });
    let text = core::countries::country_text(country);
    let code = core::countries::country_code(text).unwrap_or(text);
    if matches!(code, "HK" | "MO" | "TW") {
        return slint::Image::default();
    }
    let Some(index) = codes.iter().position(|c| c == code) else {
        return slint::Image::default();
    };
    const BYTES: usize = 72 * 72 * 4;
    thread_local! { static FLAGS:std::cell::RefCell<std::collections::BTreeMap<usize,slint::Image>>=std::cell::RefCell::default(); }
    FLAGS.with(|flags| {
        flags
            .borrow_mut()
            .entry(index)
            .or_insert_with(|| {
                slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                    &PIXELS[index * BYTES..(index + 1) * BYTES],
                    72,
                    72,
                ))
            })
            .clone()
    })
}
