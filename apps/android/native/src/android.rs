use anyhow::{Context, Result, bail, ensure};
use clash_mobile_runtime::{NativeHost, Session, core};
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
// Also included in exported diagnostics. Contains view counts/status, never URLs or secrets.
static PRESENTATION: Mutex<String> = Mutex::new(String::new());

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
    env.new_string(if snapshot.is_empty() { "{}" } else { &snapshot })
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
struct State {
    session: Session,
    page: i32,
    query: String,
    selected: String,
    notice: String,
    logs: VecDeque<(String, String)>,
    log_level: String,
    connection_sort: String,
    ip_results: Vec<core::ip_report::ResultSlot>,
    claude: Option<core::claude_check::Report>,
    claude_status: String,
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
    connection_page: usize,
    rule_page: usize,
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
fn run(app: slint::android::AndroidApp) -> Result<()> {
    let native = Arc::new(AndroidHost::new(&app)?);
    let root = native.directory("getFilesDir")?.join("client");
    let update_path = native.directory("getCacheDir")?.join("updates/update.apk");
    slint::android::init(app)?;
    let window = AndroidWindow::new()?;
    window
        .global::<MobileData>()
        .set_version(core::VERSION.into());
    window
        .global::<MobileData>()
        .set_protection_supported(false);
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
                MobileIntent::Query | MobileIntent::RuleTextMode
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
    let current_page = Arc::new(std::sync::atomic::AtomicI32::new(0));
    let page = current_page.clone();
    let observed = window.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(250),
        move || {
            if let Some(ui) = observed.upgrade() {
                page.store(ui.get_current_page(), std::sync::atomic::Ordering::Relaxed);
            }
        },
    );
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
                    current_page,
                ))
            })();
            if let Err(error) = result {
                log::error!("Mobile runtime stopped: {error:#}");
            }
        })?;
    window.run()?;
    Ok(())
}

enum Update {
    Ip(u64, usize, core::ip_report::ResultSlot),
    IpDone(u64),
    Claude(std::result::Result<core::claude_check::Report, String>),
    Log(String, String),
    Notice(String),
    Geo(std::result::Result<(), String>),
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
    current_page: Arc<std::sync::atomic::AtomicI32>,
) -> Result<()> {
    let mut state = State {
        session: Session::open(root, native)?,
        page: 0,
        query: String::new(),
        selected: String::new(),
        notice: String::new(),
        logs: VecDeque::new(),
        log_level: "全部等级".into(),
        connection_sort: "下载速度 ↓".into(),
        ip_results: vec![None; core::ip_check::services().len()],
        claude: None,
        claude_status: "尚未检测".into(),
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
        connection_page: 0,
        rule_page: 0,
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
    let (updates, mut update_rx) = mpsc::channel::<Update>(32);
    let mut checks: Option<tokio::task::JoinHandle<()>> = None;
    let mut claude_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut log_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut geo_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut log_port = 0;
    let mut last_delay = std::time::Instant::now();
    let mut delay_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut poll = tokio::time::interval(Duration::from_secs(1));
    let mut last_frame = std::time::Instant::now() - Duration::from_secs(1);
    let mut system_bars_dark = None;
    let mut update_job: Option<tokio::task::JoinHandle<()>> = None;
    let mut last_update_check = std::time::Instant::now() - Duration::from_secs(6 * 60 * 60);
    loop {
        let page = current_page.load(std::sync::atomic::Ordering::Relaxed);
        if page != state.page {
            state.query.clear();
            state.page = page;
        }
        let mut immediate = false;
        tokio::select! {
            value = receive.recv() => {
                let Some(request) = value else { break; };
                state.selected = request.selected.clone();
                immediate = true;
                let intent = request.intent;
                if intent==MobileIntent::CheckUpdate {
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
                } else if intent == MobileIntent::NodeDelay {
                    if let Some(job)=delay_job.take(){job.abort();}
                    last_delay=std::time::Instant::now();
                    let api=state.session.api.clone();let send=updates.clone();
                    state.notice="正在测速".into();
                    delay_job=Some(tokio::spawn(async move{
                        let result=if request.a=="all"{api.delay_all_nodes().await.map(|_|())}
                            else{api.delay(&request.a).await.map(|_|())};
                        let text=match result {Ok(())=>"测速完成".into(),Err(error)=>format!("测速失败：{error}")};
                        let _=send.send(Update::Notice(text)).await;
                    }));
                } else if intent == MobileIntent::ClaudeStart {
                    if let Some(job) = claude_job.take() { job.abort(); }
                    state.claude=None;state.claude_status="正在检测，浏览器将自动采样…".into();
                    let port = state.session.settings.mixed_port;
                    let host = state.session.native.clone();
                    let send = updates.clone();
                    claude_job = Some(tokio::spawn(async move {
                        let result = async {
                            let client = core::claude_check::client(Some(port))?;
                            let browser = core::claude_check::browser::Session::start(client.clone()).await?;
                            host.request(json!({"op":"open-url","url":browser.url}))?;
                            let mut report = core::claude_check::Report::new(core::claude_check::Environment::Core,"Android 网络环境");
                            core::claude_check::network(&client,&mut report,"内核代理").await;
                            core::claude_check::enhanced::extend(&client,&mut report,None).await;
                            report.signals.extend(core::claude_check::system_dns(&client,"Android 系统").await);
                            let deadline = tokio::time::Instant::now()+Duration::from_secs(120);
                            loop {
                                if let Some(sample)=browser.next_report() { report.signals.extend(sample.signals); break; }
                                if tokio::time::Instant::now()>=deadline { break; }
                                tokio::time::sleep(Duration::from_millis(500)).await;
                            }
                            Ok::<_,anyhow::Error>(report)
                        }.await;
                        let _ = send.send(Update::Claude(result.map_err(|e|e.to_string()))).await;
                    }));
                } else {
                    match handle(&mut state, request).await {
                        Ok(()) => state.completed = Some(intent),
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
                    if serde_json::from_str::<Value>(&value).ok().is_some_and(|v|v["event"]=="back") {
                        let weak=weak.clone();slint::invoke_from_event_loop(move||{if let Some(ui)=weak.upgrade(){ui.invoke_native_back();}})?;
                    } else if let Err(error) = native_event(&mut state,&value).await { state.notice=format!("{error:#}"); }
                }
            }
            value = update_rx.recv() => {
                match value {
                    Some(Update::Ip(generation,index,result)) if generation == state.generation => state.ip_results[index]=result,
                    Some(Update::IpDone(generation)) if generation == state.generation => { state.detecting=false; checks=None; }
                    Some(Update::Claude(Ok(report))) => { state.claude=Some(report); state.claude_status="检测完成 · 浏览器及网络采样已结束".into(); state.notice="检测完成".into(); },
                    Some(Update::Claude(Err(error))) => {state.claude_status=format!("检测失败：{error}");state.notice=error;},
                    Some(Update::Log(level,message)) => state.logs.push_back((level,message)),
                    Some(Update::Notice(message))=>state.notice=message,
                    Some(Update::Geo(result))=> {
                        state.geo_updating=false;geo_job=None;
                        match result.and_then(|()|state.session.initialize().map_err(|e|format!("{e:#}"))) {
                            Ok(())=> {state.geo="已更新".into();state.notice="Geo 数据已更新".into();},
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
                if minutes>0 && last_delay.elapsed()>=Duration::from_secs(minutes as u64*60) && delay_job.as_ref().is_none_or(|job|job.is_finished()) {
                    last_delay=std::time::Instant::now();
                    let api=state.session.api.clone();let send=updates.clone();
                    delay_job=Some(tokio::spawn(async move { if let Err(error)=api.delay_all_nodes().await{let _=send.send(Update::Notice(format!("定时测速失败：{error}"))).await;} }));
                }
                if state.visible || state.session.vpn {
                    let page = if state.visible { state.page } else { -1 };
                    if let Err(error) = state.session.refresh(page).await {
                        let message=format!("状态刷新失败：{error:#}");
                        if state.logs.back().is_none_or(|(_,old)| old != &message) {
                            log::warn!("{message}");
                            state.logs.push_back(("WARN".into(),message));
                        }
                    }
                    if state.session.vpn {
                        let _ = state.session.native.request(json!({"op":"traffic", "down":format!("{}/s",bytes(state.session.rates.1)), "up":format!("{}/s",bytes(state.session.rates.0))}));
                    }
                } else {
                    state.session.proxies=api::Proxies::default();
                    state.session.connections.connections.clear();
                    state.session.connections.connections.shrink_to_fit();
                }
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
        if system_bars_dark != Some(dark)
            && state
                .session
                .native
                .request(json!({"op":"system-bars","dark":dark}))
                .is_ok()
        {
            system_bars_dark = Some(dark);
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
    if let Some(job) = geo_job {
        job.abort();
    }
    if let Some(job) = checks {
        job.abort();
    }
    if let Some(job) = claude_job {
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

async fn native_event(state: &mut State, raw: &str) -> Result<()> {
    let value: Value = serde_json::from_str(raw)?;
    match value["event"].as_str().unwrap_or("") {
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
    let session = &mut state.session;
    match intent {
        MobileIntent::Connect | MobileIntent::Disconnect => {
            session.native.request(
                json!({"op":if intent == MobileIntent::Connect {"connect"} else {"disconnect"}}),
            )?;
        }
        MobileIntent::RestartCore => session.initialize()?,
        MobileIntent::Mode => {
            let mut next = session.settings.clone();
            next.run_mode = ["rule", "global", "direct"]
                .get(a.parse::<usize>()?)
                .context("运行模式无效")?
                .to_string();
            session.commit_settings(next)?;
        }
        MobileIntent::SelectNode => {
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
            let text = if let Some(id) = a.strip_prefix("connection:") {
                let c = session
                    .connections
                    .connections
                    .iter()
                    .find(|c| c.id.as_ref() == id)
                    .context("该连接已结束")?;
                format!(
                    "{}:{}\n域名: {}\n嗅探域名: {}\n目标 IP: {}\n协议: {}\n匹配规则: {} {}\n代理链: {}\n上传: {}  下载: {}\n开始时间: {}\n连接 ID: {}",
                    core::connection::name(c),
                    c.metadata.destination_port,
                    c.metadata.host,
                    c.metadata.sniff_host,
                    c.metadata.destination_ip,
                    c.metadata.network,
                    c.rule,
                    c.rule_payload,
                    c.chains
                        .iter()
                        .rev()
                        .map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(" → "),
                    bytes(c.upload),
                    bytes(c.download),
                    c.start,
                    c.id
                )
            } else {
                a
            };
            session.native.request(json!({"op":"copy","text":text}))?;
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
            let (rows, _) = session.document()?.rows(&session.draft);
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
            let (rows, _) = session.document()?.rows(&session.draft);
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
        MobileIntent::Appearance | MobileIntent::Accent | MobileIntent::TextScale => {
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
                _ => next.text_scale = a.trim_end_matches('%').parse()?,
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
            session.native.request(json!({"op":"notify"}))?;
        }
        MobileIntent::SiteCategory => state.category = a,
        MobileIntent::ExportReport => {
            session
                .native
                .request(json!({"op":"export","name":"claude-report.json",
                "content":serde_json::to_string_pretty(&state.claude)?}))?;
        }
        MobileIntent::ClaudeProtection => {
            bail!("Android 保护链路尚未通过真实设备验证，暂未开放此开关");
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
struct GroupView {
    name: String,
    group: api::Proxy,
    nodes: Vec<(String, api::Proxy)>,
    count: usize,
    page: usize,
}
struct Projection {
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
    site_summary: String,
    claude_status: String,
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
    for (name, group) in &s.proxies.proxies {
        if group.all.is_empty() {
            continue;
        }
        let indices = proxy_order::sorted_indices(
            &group.all,
            s.settings.node_sort,
            |node| s.proxies.proxies.get(node).and_then(|n| n.delay),
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
                let proxy = s
                    .proxies
                    .proxies
                    .get(name.as_str())
                    .cloned()
                    .unwrap_or_default();
                (name, proxy)
            })
            .collect();
        groups.push(GroupView {
            name: name.to_string(),
            group: group.clone(),
            nodes,
            count,
            page,
        });
    }
    let node = s.node.clone();
    let document = if state.page == 4 {
        s.document().ok()
    } else {
        None
    };
    let (rules, _) = document
        .as_ref()
        .map(|d| d.rows(&s.draft))
        .unwrap_or_default();
    let saved = rule_manager::ProfileRules::for_profile(&s.settings, &s.rule_profile, false);
    let mut connections = s
        .connections
        .connections
        .iter()
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
        .into_iter()
        .enumerate()
        .filter(|(_, r)| query.is_empty() || r.raw.to_lowercase().contains(&query))
        .collect::<Vec<_>>();
    let rule_count = rules.len();
    let rule_page = state.rule_page.min(rule_count.saturating_sub(1) / 100);
    let rules = rules.into_iter().skip(rule_page * 100).take(100).collect();
    let mut details = vec![];
    if let Some(c) = s
        .connections
        .connections
        .iter()
        .find(|c| c.id.as_ref() == state.selected)
    {
        details = vec![
            ("连接名称".into(), core::connection::name(c).into()),
            ("域名".into(), c.metadata.host.to_string()),
            (
                "嗅探域名".into(),
                if c.metadata.sniff_host.as_str().is_empty() {
                    c.metadata.host.to_string()
                } else {
                    c.metadata.sniff_host.to_string()
                },
            ),
            ("目标 IP".into(), c.metadata.destination_ip.to_string()),
            ("目标端口".into(), c.metadata.destination_port.to_string()),
            ("协议".into(), c.metadata.network.to_string()),
            ("匹配规则".into(), format!("{} {}", c.rule, c.rule_payload)),
            (
                "代理链".into(),
                c.chains
                    .iter()
                    .rev()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" → "),
            ),
            (
                "上传速度".into(),
                format!("{}/s", bytes(c.rates.map(|r| r.upload).unwrap_or(0))),
            ),
            (
                "下载速度".into(),
                format!("{}/s", bytes(c.rates.map(|r| r.download).unwrap_or(0))),
            ),
            ("累计上传".into(), bytes(c.upload)),
            ("累计下载".into(), bytes(c.download)),
            ("开始时间".into(), c.start.to_string()),
            ("连接 ID".into(), c.id.to_string()),
        ];
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
    Projection {
        settings: s.settings.clone(),
        running: s.running,
        vpn: s.vpn,
        profile: s.profile().name.clone(),
        node,
        total: s.count,
        upload: bytes(s.connections.upload_total),
        download: bytes(s.connections.download_total),
        groups,
        profiles: s.profiles.iter().cloned().collect(),
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
        site_summary: format!(
            "确认可用 {} · 网页可达 {} · 受限 {} · 失败 {}",
            report.totals.count(core::ip_report::Status::Available),
            report.totals.count(core::ip_report::Status::Partial)
                + report.totals.count(core::ip_report::Status::Reachable),
            report.totals.count(core::ip_report::Status::Restricted),
            report.totals.count(core::ip_report::Status::Failed)
        ),
        claude_status: state.claude_status.clone(),
        policies: document.map(|d| d.policies).unwrap_or_default(),
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
    let data = ui.global::<MobileData>();
    data.set_working(false);
    data.set_geo_updating(p.geo_updating);
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
    data.set_protection_supported(false);
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
    data.set_mode(match p.settings.run_mode.as_str() {
        "global" => 1,
        "direct" => 2,
        _ => 0,
    });
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
                        ..Default::default()
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
    theme.set_text_scale(p.settings.text_scale as f32 / 100.);
    theme.set_accent_index(
        config::AccentColor::ALL
            .iter()
            .position(|c| c == &p.settings.accent_color)
            .unwrap_or(0) as i32,
    );
    let dark = theme.get_dark();
    // Read back the actual Slint globals; diagnostics must reveal stale presentation too.
    if let Ok(mut snapshot) = PRESENTATION.lock() {
        let view = ui.global::<MobileView>();
        let scale = ui.window().scale_factor();
        let size = ui.window().size();
        *snapshot = json!({"core_running":data.get_core_running(),"vpn":data.get_vpn_connected(),
            "page":ui.get_current_page(),"detail":ui.get_current_detail(),
            "groups":data.get_groups().row_count(),"profiles":data.get_profiles().row_count(),
            "logs":data.get_logs().row_count(),"dark":dark,
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
