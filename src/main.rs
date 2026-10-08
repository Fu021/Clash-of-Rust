#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod presentation;
mod typography;

use clash_of_rust::{
    config::{Profile, Settings, Store},
    engine::{Engine, ProxyMode, Scope, Snapshot},
    flags, icons, ip_check, platform, probe, tray, update,
};
use futures_util::{
    SinkExt,
    future::{AbortHandle, Abortable},
};
use iced::{
    Color, Element, Length, Subscription, Task, Theme,
    widget::{Space, button, column, container, image, progress_bar, scrollable, text, text_input},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;

const DEFAULT_WINDOW_SIZE: iced::Size = iced::Size::new(950.0, 700.0);
const MIN_WINDOW_SIZE: iced::Size = iced::Size::new(800.0, 450.0);

macro_rules! aligned_row {
    ($($child:expr),* $(,)?) => {
        iced::widget::row![$($child),*].align_y(iced::alignment::Vertical::Center)
    };
}

fn main() -> iced::Result {
    if update::helper_main() {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    // This is the first initialization, before threads or GUI/DBus libraries.
    unsafe {
        platform::initialize_desktop();
    }
    #[cfg(windows)]
    let elevated_handoff = elevation_path().is_some() || update_restart();
    #[cfg(windows)]
    let _installation_guard = {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            match platform::application_guard() {
                Ok(Some(guard)) => break guard,
                Ok(None) if elevated_handoff && std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(250))
                }
                Ok(None) => {
                    if !elevated_handoff && !background_start() {
                        platform::show_existing();
                    }
                    return Ok(());
                }
                Err(_) => {
                    if !elevated_handoff && !background_start() {
                        platform::show_existing();
                    }
                    return Ok(());
                }
            }
        }
    };
    #[cfg(not(windows))]
    let _installation_guard = {
        let Ok(store) = startup_store() else {
            return Ok(());
        };
        let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(store.root.join("desktop.lock"))
        else {
            return Ok(());
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while file.try_lock().is_err() {
            if !update_restart() || std::time::Instant::now() >= deadline {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        file
    };
    typography::initialize();
    iced::application(App::new, App::update, App::view)
        .title("Clash of Rust · 原生代理客户端")
        .theme(App::theme)
        .subscription(App::subscription)
        .settings(iced::Settings {
            default_text_size: iced::Pixels(16.2),
            ..Default::default()
        })
        .default_font(typography::ENGLISH_FONT)
        .window(iced::window::Settings {
            size: DEFAULT_WINDOW_SIZE,
            min_size: Some(MIN_WINDOW_SIZE),
            icon: Some(icons::window()),
            exit_on_close_request: false,
            visible: !background_start(),
            ..Default::default()
        })
        .run()
}

fn update_restart() -> bool {
    std::env::args_os().any(|arg| arg == "--update-result")
}

fn elevation_path() -> Option<std::path::PathBuf> {
    let mut args = std::env::args_os();
    while let Some(arg) = args.next() {
        if arg == "--elevated-tun" {
            return args.next().map(std::path::PathBuf::from);
        }
    }
    None
}
fn background_start() -> bool {
    std::env::args_os().any(|arg| arg == "--background")
}
fn startup_store() -> anyhow::Result<Store> {
    if let Some(path) = elevation_path() {
        Store::at(
            path.parent()
                .ok_or_else(|| anyhow::anyhow!("提权配置路径无效"))?
                .to_owned(),
        )
    } else {
        Store::discover()
    }
}

const ACCENT: Color = Color::from_rgb(0.35, 0.77, 0.70);
const PAGE_SIZE: usize = 60;

fn last_page_offset(total: usize) -> usize {
    total.saturating_sub(1) / PAGE_SIZE * PAGE_SIZE
}

// Most host names and rules are ASCII; avoid allocating lowercase copies for
// every entry on every redraw. Unicode names still use Unicode case conversion.
fn contains_query(value: &str, lower_query: &str) -> bool {
    if lower_query.is_empty() {
        return true;
    }
    if value.is_ascii() && lower_query.is_ascii() {
        value
            .as_bytes()
            .windows(lower_query.len())
            .any(|part| part.eq_ignore_ascii_case(lower_query.as_bytes()))
    } else {
        value.to_lowercase().contains(lower_query)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Proxies,
    Profiles,
    Connections,
    Rules,
    Logs,
    Tests,
    Websites,
    Settings,
}

impl Page {
    const ALL: [Self; 9] = [
        Self::Home,
        Self::Proxies,
        Self::Profiles,
        Self::Connections,
        Self::Rules,
        Self::Logs,
        Self::Tests,
        Self::Websites,
        Self::Settings,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::Home => "首页",
            Self::Proxies => "代理",
            Self::Profiles => "订阅",
            Self::Connections => "连接",
            Self::Rules => "规则",
            Self::Logs => "日志",
            Self::Tests => "网络诊断",
            Self::Websites => "IP检测",
            Self::Settings => "设置",
        }
    }
    fn scope(self) -> Scope {
        match self {
            Self::Home => Scope::Home,
            Self::Proxies => Scope::Proxies,
            Self::Connections => Scope::Connections,
            Self::Rules => Scope::Rules,
            Self::Logs => Scope::Logs,
            _ => Scope::Other,
        }
    }
}

#[derive(Debug, Clone)]
enum Action {
    Refresh,
    Start,
    Save(Settings),
    Theme(bool),
    Autostart(bool),
    DelayInterval(u32),
    Import(String, String),
    Activate(String),
    UpdateProfile(String, bool),
    DeleteProfile(String),
    OpenProfiles,
    Mode(String),
    ProxyMode(ProxyMode),
    Select(String, String),
    Delay(String),
    Close(String),
    ClearLogs,
    ExportLogs,
    UpdateGeo,
}

#[derive(Debug, Clone)]
struct Reply {
    scope: Scope,
    settings: Settings,
    profiles: Option<Vec<Profile>>,
    snapshot: Option<Snapshot>,
    notice: Result<String, String>,
    running: bool,
    geo_status: String,
    autostart: bool,
    exit_after_start: bool,
}

#[derive(Debug, Clone)]
enum Message {
    Navigate(Page),
    Query(String),
    ListPage(usize),
    ProfileName(String),
    ProfileSource(String),
    ControllerPort(String),
    MixedPort(String),
    WindowWidth(String),
    WindowHeight(String),
    ResetWindowSize,
    ApplyWindowSize(u64, iced::Size),
    ResizeWindow(u64, iced::Size, Option<iced::window::Id>),
    ReadWindowSize(Option<iced::window::Id>),
    WindowSize(iced::Size),
    TestUrl(String),
    DnsHost(String),
    Dark(bool),
    Elevated(Result<(), String>),
    Action(Action),
    Finished(Box<Reply>),
    CheckUpdate,
    UpdateChecked(Result<Option<update::Available>, String>),
    InstallUpdate(bool),
    UpdateProgress(update::Progress),
    UpdateInstalling,
    UpdateHandoff(Result<update::InstallSession, String>),
    OpenRelease,
    ReleaseOpened(Result<(), String>),
    Tick,
    ToggleGroup(String),
    GroupPage(String, bool),
    GroupDelay(String),
    GroupDelayDone(
        Option<String>,
        String,
        Result<BTreeMap<String, u32>, String>,
    ),
    Tray(tray::Command),
    ShowWindow(Option<iced::window::Id>),
    SiteAll,
    SiteCancel,
    IntervalInput(String),
    SaveInterval,
    PeriodicDelay,
    PeriodicDone(Option<String>, Result<BTreeMap<String, u32>, String>),
    SiteProbe(String),
    SiteDone(u64, String, Result<ip_check::CheckResult, String>),
    Probe(bool),
    ProbeDone(Result<probe::ProbeResult, String>),
    Dns,
    DnsDone(Result<Vec<String>, String>),
    Window((iced::window::Id, iced::window::Event)),
    ExitFinished(Result<(), String>),
}

struct App {
    engine: Option<Arc<Mutex<Engine>>>,
    page: Page,
    settings: Settings,
    profiles: Vec<Profile>,
    snapshot: Snapshot,
    busy: bool,
    working: bool,
    elevating: bool,
    pending_refresh: bool,
    queued_actions: VecDeque<Action>,
    probing: bool,
    visible: bool,
    hidden_ticks: u8,
    expanded: BTreeSet<String>,
    group_offsets: BTreeMap<String, usize>,
    testing_groups: BTreeSet<String>,
    node_delays: BTreeMap<String, u32>,
    tray: Option<tray::Guard>,
    site_queue: VecDeque<String>,
    interval_input: String,
    periodic_testing: bool,
    site_results: Vec<(String, Result<ip_check::CheckResult, String>)>,
    site_generation: u64,
    site_aborters: BTreeMap<String, AbortHandle>,
    site_busy: BTreeSet<String>,
    exiting: bool,
    notice: String,
    error: bool,
    query: String,
    list_offset: usize,
    profile_name: String,
    profile_source: String,
    controller_port: String,
    mixed_port: String,
    window_width: String,
    window_height: String,
    size_revision: u64,
    size_pending: bool,
    dark: bool,
    autostart: bool,
    test_url: String,
    dns_host: String,
    test_results: Vec<String>,
    geo_status: String,
    updates: update::State,
}

async fn execute(engine: Arc<Mutex<Engine>>, action: Action, scope: Scope) -> Reply {
    let include_profiles = !matches!(action, Action::Refresh);
    let mut engine = engine.lock().await;
    let mut exit_after_start = false;
    let result: anyhow::Result<String> = async {
        match action {
            Action::Refresh => Ok(String::new()),
            Action::Start => {
                if let Some(path) = elevation_path().filter(|path| path.exists()) {
                    engine.finish_elevation(&path).await?;
                } else {
                    engine.start().await?;
                    exit_after_start = engine.restore_proxy_mode().await?;
                }
                Ok("内核已启动".into())
            }
            Action::Save(settings) => {
                engine.save_settings(settings).await?;
                Ok("端口已保存，运行中的内核已自动重启".into())
            }
            Action::Theme(dark) => {
                let settings = Settings {
                    dark,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("外观已自动保存".into())
            }
            Action::Autostart(enabled) => {
                platform::set_autostart(enabled)?;
                Ok(if enabled {
                    "已开启开机自启，将在后台启动"
                } else {
                    "已关闭开机自启"
                }
                .into())
            }
            Action::DelayInterval(minutes) => {
                let settings = Settings {
                    delay_interval_minutes: minutes,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("定时测速间隔已保存".into())
            }
            Action::DeleteProfile(id) => {
                engine.delete_profile(&id).await?;
                Ok("订阅已删除；使用中的订阅会先切换到默认直连".into())
            }
            Action::OpenProfiles => {
                platform::open_directory(&engine.store.root.join("profiles"))?;
                Ok("已打开订阅配置目录".into())
            }
            Action::Import(name, source) => {
                engine.import(name, source, None).await?;
                Ok("配置已导入".into())
            }
            Action::Activate(id) => {
                engine.activate(id).await?;
                Ok("当前配置已切换".into())
            }
            Action::UpdateProfile(id, via_proxy) => {
                let profile = engine
                    .profiles
                    .iter()
                    .find(|p| p.id == id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("配置不存在"))?;
                if via_proxy {
                    engine
                        .import_via_proxy(profile.name, profile.source, Some(profile.id))
                        .await?;
                } else {
                    engine
                        .import(profile.name, profile.source, Some(profile.id))
                        .await?;
                }
                Ok("订阅已更新".into())
            }
            Action::Mode(mode) => {
                engine.select_mode(&mode).await?;
                Ok("运行模式已切换".into())
            }
            Action::ProxyMode(mode) => {
                engine.select_proxy_mode(mode).await?;
                Ok(match mode {
                    ProxyMode::System => "系统代理已开启",
                    ProxyMode::Tun => "TUN 模式已开启",
                    ProxyMode::Off => "代理模式已关闭",
                }
                .into())
            }
            Action::Select(group, node) => {
                engine.api.select(&group, &node).await?;
                Ok(format!("已选择 {node}"))
            }
            Action::Delay(node) => {
                let delay = engine.api.delay(&node).await?;
                Ok(format!("{node} · {delay} ms"))
            }
            Action::Close(id) => {
                engine.api.close_connection(&id).await?;
                Ok("连接已关闭".into())
            }
            Action::ClearLogs => {
                engine.clear_logs();
                Ok("日志已清空".into())
            }
            Action::ExportLogs => {
                let path = engine.export_logs()?;
                Ok(format!("日志已导出到 {}", path.display()))
            }
            Action::UpdateGeo => {
                engine.update_geo().await?;
                Ok("GeoIP、GeoSite、Country、ASN 数据已全部更新".into())
            }
        }
    }
    .await;
    let mut notice = result.map_err(|e| e.to_string());
    let snapshot = match engine.poll(scope).await {
        Ok(snapshot) => Some(snapshot),
        Err(e) => {
            if notice.as_ref().is_ok_and(|s| s.is_empty()) {
                notice = Err(e.to_string());
            }
            None
        }
    };
    Reply {
        scope,
        settings: engine.settings.clone(),
        profiles: include_profiles.then(|| engine.profiles.clone()),
        snapshot,
        notice,
        running: engine.running(),
        geo_status: engine.geo_manifest.version.clone(),
        autostart: platform::autostart_enabled().unwrap_or(false),
        exit_after_start,
    }
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let created = startup_store().and_then(Engine::new);
        let (engine, settings, profiles, geo_status, notice, error) = match created {
            Ok(engine) => {
                let settings = engine.settings.clone();
                let profiles = engine.profiles.clone();
                let geo_status = engine.geo_manifest.version.clone();
                (
                    Some(Arc::new(Mutex::new(engine))),
                    settings,
                    profiles,
                    geo_status,
                    "内核与 Geo 数据已就绪，正在自动启动当前配置…".into(),
                    false,
                )
            }
            Err(e) => (
                None,
                Settings::default(),
                vec![],
                String::new(),
                format!("初始化失败：{e:#}"),
                true,
            ),
        };
        let tray_result = tray::start();
        let (tray, notice, error) = match tray_result {
            Ok(tray) => (Some(tray), notice, error),
            Err(e) => (
                None,
                format!("{notice} 托盘不可用：{e}；关闭窗口将退出。"),
                true,
            ),
        };
        let mut app = Self {
            controller_port: settings.controller_port.to_string(),
            interval_input: settings.delay_interval_minutes.to_string(),
            periodic_testing: false,
            mixed_port: settings.mixed_port.to_string(),
            window_width: format!("{:.0}", DEFAULT_WINDOW_SIZE.width),
            window_height: format!("{:.0}", DEFAULT_WINDOW_SIZE.height),
            size_revision: 0,
            size_pending: false,
            dark: settings.dark,
            autostart: platform::autostart_enabled().unwrap_or(false),
            engine,
            settings,
            profiles,
            geo_status,
            notice,
            error,
            page: Page::Home,
            snapshot: Snapshot::default(),
            busy: false,
            working: false,
            elevating: false,
            pending_refresh: false,
            queued_actions: VecDeque::new(),
            probing: false,
            visible: !background_start() || tray.is_none(),
            hidden_ticks: 0,
            expanded: BTreeSet::new(),
            group_offsets: BTreeMap::new(),
            testing_groups: BTreeSet::new(),
            node_delays: BTreeMap::new(),
            tray,
            site_queue: VecDeque::new(),
            site_results: vec![],
            site_generation: 0,
            site_aborters: BTreeMap::new(),
            site_busy: BTreeSet::new(),
            exiting: false,
            query: String::new(),
            list_offset: 0,
            profile_name: String::new(),
            profile_source: String::new(),
            test_url: "https://www.gstatic.com/generate_204".into(),
            dns_host: "github.com".into(),
            test_results: vec![],
            updates: update::State::default(),
        };
        if let Some((message, error)) = update::startup_outcome() {
            if error {
                app.updates.failure = Some(message);
            } else {
                app.updates.outcome = Some(message);
            }
            app.error = error;
        }
        let update_task = if app.engine.is_none() {
            app.check_update()
        } else {
            Task::none()
        };
        let task = Task::batch([
            app.dispatch(Action::Start),
            iced::window::latest().map(Message::ReadWindowSize),
            if background_start() && app.tray.is_none() {
                iced::window::latest().then(|id| match id {
                    Some(id) => iced::window::set_mode(id, iced::window::Mode::Windowed),
                    None => Task::none(),
                })
            } else {
                Task::none()
            },
            update_task,
        ]);
        (app, task)
    }

    fn scaled(&self, size: u16) -> f32 {
        size as f32 * 1.35
    }

    fn label<'a>(&self, value: impl text::IntoFragment<'a>) -> iced::widget::Text<'a> {
        text(value)
            .font(typography::ENGLISH_FONT)
            .size(self.scaled(12))
            .color(self.foreground())
    }

    fn foreground(&self) -> Color {
        if self.dark {
            Color::WHITE
        } else {
            self.theme().palette().text
        }
    }

    fn theme(&self) -> Theme {
        if self.dark {
            Theme::TokyoNight
        } else {
            Theme::Light
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        // Hidden windows only check core health; they do not fetch page data.
        let polling = iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick);
        let tray_events = if self.tray.is_some() {
            Subscription::run(|| {
                iced::stream::channel::<Message>(16, async move |mut output| {
                    while let Some(command) = tray::next().await {
                        if output.send(Message::Tray(command)).await.is_err() {
                            break;
                        }
                    }
                })
            })
        } else {
            Subscription::none()
        };
        Subscription::batch([
            polling,
            iced::time::every(update::CHECK_INTERVAL).map(|_| Message::CheckUpdate),
            if self.settings.delay_interval_minutes == 0 {
                Subscription::none()
            } else {
                iced::time::every(Duration::from_secs(
                    u64::from(self.settings.delay_interval_minutes) * 60,
                ))
                .map(|_| Message::PeriodicDelay)
            },
            tray_events,
            iced::window::events().map(Message::Window),
        ])
    }

    fn check_update(&mut self) -> Task<Message> {
        if self.exiting || !self.updates.begin() {
            return Task::none();
        }
        let proxy_port = self.snapshot.running.then_some(self.settings.mixed_port);
        Task::perform(
            async move {
                update::check(proxy_port)
                    .await
                    .map_err(|error| error.to_string())
            },
            Message::UpdateChecked,
        )
    }

    fn dispatch(&mut self, action: Action) -> Task<Message> {
        let scope = if self.visible {
            self.page.scope()
        } else {
            Scope::Other
        };
        self.dispatch_scope(action, scope)
    }

    fn dispatch_scope(&mut self, action: Action, scope: Scope) -> Task<Message> {
        if (self.updates.install_pending || self.updates.installing.is_some())
            && !matches!(action, Action::Refresh)
        {
            return Task::none();
        }
        if self.busy || self.exiting {
            if matches!(action, Action::Refresh) {
                self.pending_refresh = true;
            } else if !self.exiting && self.queued_actions.len() < 16 {
                self.queued_actions.push_back(action);
            }
            return Task::none();
        }
        let Some(engine) = self.engine.clone() else {
            return Task::none();
        };
        if matches!(
            action,
            Action::Start
                | Action::Save(_)
                | Action::Activate(_)
                | Action::Mode(_)
                | Action::ProxyMode(_)
                | Action::Select(_, _)
        ) {
            self.cancel_site_checks();
            self.site_results.clear();
        }
        self.busy = true;
        self.working = !matches!(action, Action::Refresh);
        Task::perform(execute(engine, action, scope), |reply| {
            Message::Finished(Box::new(reply))
        })
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::CheckUpdate => return self.check_update(),
            Message::UpdateChecked(result) => self.updates.finish(result),
            Message::InstallUpdate(proxied) => {
                if self.exiting || self.updates.busy() {
                    return Task::none();
                }
                let Some(available) = self.updates.available.clone() else {
                    return Task::none();
                };
                if let Err(error) = update::ensure_installable() {
                    self.updates.fail(error.to_string());
                    return Task::none();
                }
                if proxied && !self.snapshot.running {
                    self.updates.fail("请先启动内核，再使用代理更新".into());
                    return Task::none();
                }
                self.updates.failure = None;
                self.updates.outcome = None;
                self.updates.progress = Some(update::Progress {
                    received: 0,
                    total: available
                        .package
                        .as_ref()
                        .map_or(1, |package| package.asset.size),
                });
                let port = proxied.then_some(self.settings.mixed_port);
                return Task::run(
                    iced::stream::channel(8, async move |mut output| {
                        let (sender, mut progress) = tokio::sync::mpsc::channel(4);
                        let download = update::download(available, port, move |value| {
                            let _ = sender.try_send(value);
                        });
                        tokio::pin!(download);
                        let downloaded = loop {
                            tokio::select! {
                                result = &mut download => break result,
                                Some(value) = progress.recv() => { let _ = output.send(Message::UpdateProgress(value)).await; }
                            }
                        };
                        let result = match downloaded {
                            Ok(downloaded) => {
                                let _ = output.send(Message::UpdateInstalling).await;
                                match tokio::task::spawn_blocking(move || {
                                    update::handoff(downloaded)
                                })
                                .await
                                {
                                    Ok(result) => result.map_err(|error| error.to_string()),
                                    Err(_) => Err("安装助手无法启动，请重试".into()),
                                }
                            }
                            Err(error) => Err(error.to_string()),
                        };
                        let _ = output.send(Message::UpdateHandoff(result)).await;
                    }),
                    |message| message,
                );
            }
            Message::UpdateProgress(progress) => {
                if self.updates.progress.is_some() {
                    self.updates.progress = Some(progress);
                }
            }
            Message::UpdateInstalling => {
                self.updates.progress = None;
                self.updates.install_pending = true;
            }
            Message::UpdateHandoff(result) => {
                self.updates.install_pending = false;
                match result {
                    Ok(directory) => {
                        self.updates.installing = Some(directory);
                    }
                    Err(reason) => self.updates.fail(reason),
                }
            }
            Message::OpenRelease => {
                let url = self.updates.available.as_ref().map_or_else(
                    || update::RELEASES_URL.to_owned(),
                    |update| update.url.clone(),
                );
                return Task::perform(
                    async move {
                        match tokio::task::spawn_blocking(move || platform::open_url(&url)).await {
                            Ok(result) => result.map_err(|error| error.to_string()),
                            Err(error) => Err(error.to_string()),
                        }
                    },
                    Message::ReleaseOpened,
                );
            }
            Message::ReleaseOpened(result) => {
                if let Err(error) = result {
                    self.notice = format!("无法打开 Release 页面：{error}");
                    self.error = true;
                }
            }
            Message::Navigate(page) => {
                self.page = page;
                self.query.clear();
                self.list_offset = 0;
                self.snapshot.retain_scope(page.scope());
                return self.dispatch(Action::Refresh);
            }
            Message::Query(value) => {
                self.query = value;
                self.group_offsets.clear();
                self.list_offset = 0;
            }
            Message::ListPage(offset) => self.list_offset = offset,
            Message::ProfileName(value) => self.profile_name = value,
            Message::ProfileSource(value) => self.profile_source = value,
            Message::ControllerPort(value) => self.controller_port = value,
            Message::MixedPort(value) => self.mixed_port = value,
            Message::WindowWidth(value) => {
                self.window_width = value;
                return self.schedule_window_size();
            }
            Message::WindowHeight(value) => {
                self.window_height = value;
                return self.schedule_window_size();
            }
            Message::ResetWindowSize => {
                self.window_width = format!("{:.0}", DEFAULT_WINDOW_SIZE.width);
                self.window_height = format!("{:.0}", DEFAULT_WINDOW_SIZE.height);
                self.size_revision = self.size_revision.wrapping_add(1);
                self.size_pending = true;
                return Task::done(Message::ApplyWindowSize(
                    self.size_revision,
                    DEFAULT_WINDOW_SIZE,
                ));
            }
            Message::ApplyWindowSize(revision, size) => {
                if revision == self.size_revision {
                    return iced::window::latest()
                        .map(move |id| Message::ResizeWindow(revision, size, id));
                }
            }
            Message::ResizeWindow(revision, size, Some(id)) => {
                if revision == self.size_revision {
                    self.size_pending = false;
                    self.set_window_size(size);
                    return iced::window::resize(id, size);
                }
            }
            Message::ResizeWindow(_, _, None) => self.size_pending = false,
            Message::ReadWindowSize(Some(id)) => {
                return iced::window::size(id).map(Message::WindowSize);
            }
            Message::ReadWindowSize(None) => {}
            Message::WindowSize(size) => self.set_window_size(size),
            Message::Window((_, iced::window::Event::Resized(size))) => {
                self.size_pending = false;
                self.size_revision = self.size_revision.wrapping_add(1);
                self.set_window_size(size);
            }
            Message::TestUrl(value) => self.test_url = value,
            Message::DnsHost(value) => self.dns_host = value,
            Message::Dark(value) => {
                self.dark = value;
                return self.dispatch(Action::Theme(value));
            }
            Message::Action(Action::ProxyMode(ProxyMode::Tun)) if !platform::is_elevated() => {
                if self.working || self.elevating || self.exiting {
                    return Task::none();
                }
                if let Some(engine) = self.engine.clone() {
                    self.elevating = true;
                    return Task::perform(
                        async move {
                            engine
                                .lock()
                                .await
                                .prepare_elevation()
                                .await
                                .map_err(|error| error.to_string())
                        },
                        Message::Elevated,
                    );
                }
            }
            Message::Elevated(result) => {
                self.elevating = false;
                match result {
                    Ok(()) => return self.exit(),
                    Err(error) => {
                        self.error = true;
                        self.notice = error;
                    }
                }
            }
            Message::Action(Action::Save(_)) => {
                let ports = self.controller_port.parse::<u16>().and_then(|controller| {
                    self.mixed_port
                        .parse::<u16>()
                        .map(|mixed| (controller, mixed))
                });
                match ports {
                    Ok((controller_port, mixed_port)) => {
                        let settings = Settings {
                            controller_port,
                            mixed_port,
                            dark: self.dark,
                            ..self.settings.clone()
                        };
                        return self.dispatch(Action::Save(settings));
                    }
                    Err(_) => {
                        self.error = true;
                        self.notice = "端口必须为 1–65535 的整数".into();
                    }
                }
            }
            Message::Action(action) => {
                if let Action::Delay(node) = &action {
                    self.node_delays.remove(node);
                }
                if matches!(action, Action::Activate(_) | Action::UpdateProfile(_, _)) {
                    self.node_delays.clear();
                }
                return self.dispatch(action);
            }
            Message::ToggleGroup(group) => {
                if !self.expanded.remove(&group) {
                    self.expanded.insert(group);
                }
            }
            Message::GroupDelay(group) => {
                if !self.snapshot.running
                    || self.working
                    || self.elevating
                    || self.testing_groups.len() >= 2
                    || !self.testing_groups.insert(group.clone())
                {
                    return Task::none();
                }
                let engine = self.engine.clone().unwrap();
                let label = group.clone();
                let profile = self.settings.active_profile.clone();
                return Task::perform(
                    async move {
                        let api = engine.lock().await.api.clone();
                        api.delay_group(&group).await.map_err(|e| e.to_string())
                    },
                    move |result| Message::GroupDelayDone(profile.clone(), label.clone(), result),
                );
            }
            Message::GroupDelayDone(profile, group, result) => {
                self.testing_groups.remove(&group);
                if profile != self.settings.active_profile {
                    return Task::none();
                }
                match result {
                    Ok(delays) => {
                        let good = delays.values().filter(|d| **d > 0).count();
                        self.notice = format!(
                            "{group} 测速完成：{good}/{} 个节点可达，其余超时或失败",
                            delays.len()
                        );
                        self.error = false;
                        self.node_delays.extend(delays);
                    }
                    Err(error) => {
                        self.notice = format!("{group} 测速失败：{error}");
                        self.error = true;
                    }
                }
                return self.dispatch(Action::Refresh);
            }
            Message::GroupPage(group, next) => {
                let offset = self.group_offsets.entry(group).or_default();
                *offset = if next {
                    offset.saturating_add(PAGE_SIZE)
                } else {
                    offset.saturating_sub(PAGE_SIZE)
                };
            }
            Message::IntervalInput(value) => self.interval_input = value,
            Message::SaveInterval => match self.interval_input.trim().parse::<u32>() {
                Ok(minutes) if minutes <= 1440 => {
                    return self.dispatch(Action::DelayInterval(minutes));
                }
                _ => {
                    self.error = true;
                    self.notice = "请输入 0–1440 分钟，0 表示关闭定时测速".into();
                }
            },
            Message::PeriodicDelay => {
                if !self.snapshot.running
                    || self.working
                    || self.elevating
                    || self.periodic_testing
                    || !self.testing_groups.is_empty()
                    || self.exiting
                {
                    return Task::none();
                }
                let Some(engine) = self.engine.clone() else {
                    return Task::none();
                };
                self.periodic_testing = true;
                let profile = self.settings.active_profile.clone();
                return Task::perform(
                    async move {
                        let api = engine.lock().await.api.clone();
                        api.delay_all_nodes()
                            .await
                            .map_err(|error| error.to_string())
                    },
                    move |result| Message::PeriodicDone(profile.clone(), result),
                );
            }
            Message::PeriodicDone(profile, result) => {
                self.periodic_testing = false;
                if profile == self.settings.active_profile {
                    if let Ok(delays) = result {
                        self.node_delays = delays;
                    }
                    if self.page == Page::Proxies {
                        return self.dispatch(Action::Refresh);
                    }
                }
            }
            Message::SiteAll => {
                if !self.snapshot.running || !self.site_busy.is_empty() {
                    return Task::none();
                }
                self.site_results.clear();
                self.site_generation = self.site_generation.wrapping_add(1);
                self.site_queue = ip_check::services()
                    .iter()
                    .map(|item| item.id.clone())
                    .collect();
                return self.start_site_queue();
            }
            Message::SiteCancel => {
                self.cancel_site_checks();
            }
            Message::SiteProbe(url) => {
                if !self.snapshot.running
                    || self.site_busy.len() >= 3
                    || !self.site_busy.insert(url.clone())
                {
                    return Task::none();
                }
                let port = self.settings.mixed_port;
                let label = url.clone();
                let generation = self.site_generation;
                let (handle, registration) = AbortHandle::new_pair();
                self.site_aborters.insert(url.clone(), handle);
                return Task::perform(
                    async move {
                        Abortable::new(ip_check::check(&url, port), registration)
                            .await
                            .map_err(|_| "检测已取消".into())
                            .and_then(|result| result.map_err(|e| e.to_string()))
                    },
                    move |result| Message::SiteDone(generation, label.clone(), result),
                );
            }
            Message::SiteDone(generation, url, result) => {
                if generation != self.site_generation {
                    return Task::none();
                }
                self.site_busy.remove(&url);
                self.site_aborters.remove(&url);
                self.site_results.retain(|(target, _)| target != &url);
                self.site_results.push((url, result));
                return self.start_site_queue();
            }
            Message::Tray(command) => {
                // Native check items toggle on click; keep marks tied to confirmed core state.
                if let Some(tray) = &self.tray {
                    tray.refresh(&self.snapshot);
                }
                match command {
                    tray::Command::Show => return iced::window::latest().map(Message::ShowWindow),
                    tray::Command::Exit => return self.exit(),
                    tray::Command::Rule => return self.dispatch(Action::Mode("rule".into())),
                    tray::Command::Global => return self.dispatch(Action::Mode("global".into())),
                    tray::Command::Direct => return self.dispatch(Action::Mode("direct".into())),
                    tray::Command::SystemProxy => {
                        return self.dispatch(Action::ProxyMode(ProxyMode::System));
                    }
                    tray::Command::Tun => {
                        if !self.snapshot.tun {
                            return self.update(Message::Action(Action::ProxyMode(ProxyMode::Tun)));
                        }
                    }
                    tray::Command::ProxyOff => {
                        return self.dispatch(Action::ProxyMode(ProxyMode::Off));
                    }
                }
            }
            Message::ShowWindow(Some(id)) => {
                self.visible = true;
                let refresh = self.dispatch(Action::Refresh);
                return Task::batch([
                    iced::window::set_mode(id, iced::window::Mode::Windowed)
                        .chain(iced::window::minimize(id, false))
                        .chain(iced::window::gain_focus(id)),
                    refresh,
                ]);
            }
            Message::ShowWindow(None) => {}
            Message::Tick => {
                if let Some(session) = self.updates.installing.clone()
                    && let Some((message, error)) = session.outcome()
                {
                    self.updates.installing = None;
                    self.updates.progress = None;
                    if error {
                        self.updates.failure = Some(message);
                    } else {
                        self.updates.outcome = Some(message);
                    }
                    self.error = error;
                    if !error {
                        return self.exit();
                    }
                    update::cleanup(&session.directory);
                }
                if platform::exit_requested() {
                    return self.exit();
                }
                if platform::show_requested() {
                    return iced::window::latest().map(Message::ShowWindow);
                }
                if !self.visible {
                    self.hidden_ticks = (self.hidden_ticks + 1) % 5;
                    if self.hidden_ticks != 0 {
                        return Task::none();
                    }
                }
                if (self.snapshot.running || self.page == Page::Logs)
                    && !self.busy
                    && !self.elevating
                {
                    let scope = if !self.visible || self.page == Page::Rules {
                        Scope::Other
                    } else {
                        self.page.scope()
                    };
                    return self.dispatch_scope(Action::Refresh, scope);
                }
            }
            Message::Finished(reply) => {
                let reply = *reply;
                self.busy = false;
                self.working = false;
                if reply.exit_after_start {
                    return self.exit();
                }
                self.autostart = reply.autostart;
                self.settings = reply.settings;
                if let Some(profiles) = reply.profiles {
                    self.profiles = profiles;
                }
                self.geo_status = reply.geo_status;
                self.snapshot.running = reply.running;
                if let Some(mut snapshot) = reply.snapshot {
                    if reply.scope == Scope::Proxies && snapshot.running {
                        let proxies = &snapshot.proxies.proxies;
                        self.expanded.retain(|name| proxies.contains_key(name));
                        self.group_offsets
                            .retain(|name, _| proxies.contains_key(name));
                        self.node_delays
                            .retain(|name, _| proxies.contains_key(name));
                    }
                    // Rules are static between explicit refreshes. Keep only the
                    // current page's data, including when an older poll finishes.
                    if snapshot.running
                        && self.visible
                        && self.page == Page::Proxies
                        && reply.scope != Scope::Proxies
                    {
                        snapshot.proxies = std::mem::take(&mut self.snapshot.proxies);
                    }
                    if snapshot.running
                        && self.visible
                        && self.page == Page::Rules
                        && reply.scope != Scope::Rules
                    {
                        snapshot.rules = std::mem::take(&mut self.snapshot.rules);
                    }
                    if snapshot.running && !matches!(reply.scope, Scope::Home | Scope::Connections)
                    {
                        snapshot.connections = std::mem::take(&mut self.snapshot.connections);
                        snapshot.connection_count = self.snapshot.connection_count;
                    }
                    self.snapshot = snapshot;
                    self.snapshot.retain_scope(if self.visible {
                        self.page.scope()
                    } else {
                        Scope::Other
                    });
                }
                if let Some(tray) = &self.tray {
                    tray.update(&self.snapshot);
                }
                match reply.notice {
                    Ok(notice) if !notice.is_empty() => {
                        self.notice = notice;
                        self.error = false;
                    }
                    Err(error) => {
                        self.notice = error;
                        self.error = true;
                    }
                    _ => {}
                }
                // Start checking after core startup, so its mixed-port proxy is ready.
                let update_task = if !self.updates.started {
                    self.check_update()
                } else {
                    Task::none()
                };
                let next_task = if let Some(action) = self.queued_actions.pop_front() {
                    self.dispatch(action)
                } else if self.pending_refresh {
                    self.pending_refresh = false;
                    self.dispatch(Action::Refresh)
                } else {
                    Task::none()
                };
                return Task::batch([update_task, next_task]);
            }
            Message::Probe(exit_ip) => {
                if self.probing {
                    return Task::none();
                }
                if !self.snapshot.running {
                    self.error = true;
                    self.notice = "请先启动内核".into();
                    return Task::none();
                }
                self.probing = true;
                let url = if exit_ip {
                    "https://api.ipify.org?format=json".into()
                } else {
                    self.test_url.clone()
                };
                let port = self.settings.mixed_port;
                return Task::perform(
                    async move {
                        probe::website(url, port, exit_ip)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    Message::ProbeDone,
                );
            }
            Message::ProbeDone(result) => {
                self.probing = false;
                let line = match result {
                    Ok(result) => {
                        format!("{} · {} ms · {}", result.url, result.millis, result.detail)
                    }
                    Err(error) => format!("测试失败：{error}"),
                };
                self.add_test(line);
            }
            Message::Dns => {
                if self.probing {
                    return Task::none();
                }
                self.probing = true;
                let host = self.dns_host.clone();
                return Task::perform(
                    async move { probe::dns(host).await.map_err(|e| e.to_string()) },
                    Message::DnsDone,
                );
            }
            Message::DnsDone(result) => {
                self.probing = false;
                self.add_test(match result {
                    Ok(addresses) => {
                        format!("系统 DNS · {} → {}", self.dns_host, addresses.join(", "))
                    }
                    Err(error) => format!("DNS 查询失败：{error}"),
                });
            }
            Message::Window((_, iced::window::Event::Focused)) => {
                self.visible = true;
                return self.dispatch(Action::Refresh);
            }
            Message::Window((id, iced::window::Event::CloseRequested)) => {
                if self.tray.is_some() {
                    self.visible = false;
                    self.snapshot.retain_scope(Scope::Other);
                    let stop_stream = self.dispatch_scope(Action::Refresh, Scope::Other);
                    return Task::batch([
                        iced::window::set_mode(id, iced::window::Mode::Hidden),
                        stop_stream,
                    ]);
                }
                return self.exit();
            }
            Message::ExitFinished(result) => match result {
                Ok(()) => return iced::exit(),
                Err(error) => {
                    self.exiting = false;
                    self.error = true;
                    self.notice = format!("退出失败：{error}");
                    if self.updates.busy() {
                        self.updates.progress = None;
                        self.updates.failure =
                            Some("更新失败：客户端无法正常退出，请检查代理恢复或内核状态".into());
                    }
                }
            },
            Message::Window(_) => {}
        }
        Task::none()
    }

    fn exit(&mut self) -> Task<Message> {
        if self.exiting {
            return Task::none();
        }
        self.cancel_site_checks();
        if let Some(engine) = self.engine.clone() {
            self.exiting = true;
            Task::perform(
                async move { engine.lock().await.stop().await.map_err(|e| e.to_string()) },
                Message::ExitFinished,
            )
        } else {
            iced::exit()
        }
    }

    fn start_site_queue(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        while self.site_busy.len() < 3 {
            let Some(url) = self.site_queue.pop_front() else {
                break;
            };
            if !self.site_busy.contains(&url) {
                tasks.push(self.update(Message::SiteProbe(url)));
            }
        }
        Task::batch(tasks)
    }

    fn cancel_site_checks(&mut self) {
        self.site_generation = self.site_generation.wrapping_add(1);
        self.site_queue.clear();
        for (_, handle) in std::mem::take(&mut self.site_aborters) {
            handle.abort();
        }
        self.site_busy.clear();
    }

    fn set_window_size(&mut self, size: iced::Size) {
        if !self.size_pending {
            self.window_width = format!("{:.0}", size.width);
            self.window_height = format!("{:.0}", size.height);
        }
    }

    fn schedule_window_size(&mut self) -> Task<Message> {
        self.size_revision = self.size_revision.wrapping_add(1);
        self.size_pending = true;
        let (Ok(width), Ok(height)) = (
            self.window_width.parse::<u16>(),
            self.window_height.parse::<u16>(),
        ) else {
            self.size_pending = false;
            return Task::none();
        };
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            self.size_pending = false;
            return Task::none();
        }
        let revision = self.size_revision;
        let size = iced::Size::new(
            f32::from(width).max(MIN_WINDOW_SIZE.width),
            f32::from(height).max(MIN_WINDOW_SIZE.height),
        );
        Task::perform(
            async move {
                tokio::time::sleep(Duration::from_millis(250)).await;
            },
            move |_| Message::ApplyWindowSize(revision, size),
        )
    }

    fn add_test(&mut self, line: String) {
        if self.test_results.len() == 20 {
            self.test_results.remove(0);
        }
        self.test_results.push(line);
    }

    fn action<'a>(&self, label: &'a str, action: Action, enabled: bool) -> Element<'a, Message> {
        button(self.label(label).size(self.scaled(12)))
            .padding([4, 10])
            .style(rounded_primary)
            .on_press_maybe(
                (enabled
                    && !self.working
                    && !self.elevating
                    && self.engine.is_some()
                    && !self.exiting)
                    .then_some(Message::Action(action)),
            )
            .into()
    }

    fn view(&self) -> Element<'_, Message> {
        let compact = self.window_height.parse::<u16>().unwrap_or(700) < 560;
        let mut nav = column![
            aligned_row![
                image(icons::sidebar()).width(24).height(24),
                self.label("Clash of Rust")
                    .size(typography::brand_size())
                    .shaping(text::Shaping::Advanced)
                    .wrapping(text::Wrapping::None)
                    .width(Length::Fill)
                    .color(ACCENT)
            ]
            .spacing(6)
            .width(Length::Fill),
            Space::new().height(8)
        ]
        .spacing(if compact { 6 } else { 10 });
        for page in Page::ALL {
            nav = nav.push(
                button(self.label(page.name()).size(self.scaled(12)))
                    .width(Length::Fill)
                    .padding([4, 10])
                    .style(if self.page == page {
                        rounded_primary
                    } else {
                        rounded_text
                    })
                    .on_press(Message::Navigate(page)),
            );
        }
        nav = nav.push(Space::new().height(Length::Fill)).push(
            self.label(clash_of_rust::VERSION)
                .size(self.scaled(10))
                .color(self.foreground()),
        );
        let sidebar = container(nav)
            .width(164)
            .height(Length::Fill)
            .padding(10)
            .style(sidebar_panel);
        let status = if self.exiting {
            "正在恢复代理并退出…"
        } else if self.working || self.elevating {
            "正在处理…"
        } else if self.snapshot.running {
            "内核运行中"
        } else {
            "内核未启动"
        };
        let active = self
            .profiles
            .iter()
            .find(|p| Some(&p.id) == self.settings.active_profile.as_ref())
            .map(|p| p.name.as_str())
            .unwrap_or("未选择配置");
        let header = aligned_row![
            self.label(self.page.name()).size(self.scaled(18)),
            Space::new().width(Length::Fill),
            column![
                self.label(status)
                    .size(self.scaled(12))
                    .color(if self.snapshot.running {
                        ACCENT
                    } else {
                        self.foreground()
                    }),
                self.label(active)
                    .size(self.scaled(10))
                    .color(self.foreground())
            ]
            .spacing(4)
            .width(170)
            .align_x(iced::alignment::Horizontal::Right)
        ];
        let update_status = if self.updates.failure.is_some() {
            self.updates.failure.as_deref()
        } else if self.updates.install_pending || self.updates.installing.is_some() {
            Some("正在安装更新，请完成系统授权…")
        } else {
            self.updates.outcome.as_deref()
        };
        let notice = container(
            self.label(update_status.unwrap_or(&self.notice))
                .size(self.scaled(11))
                .color(
                    if (self.error || self.updates.failure.is_some()) && !self.dark {
                        Color::from_rgb(0.95, 0.47, 0.48)
                    } else {
                        self.foreground()
                    },
                ),
        )
        .padding(8)
        .width(Length::Fill)
        .style(panel);
        let content = match self.page {
            Page::Home => self.home(),
            Page::Proxies => self.proxies(),
            Page::Profiles => self.profiles_view(),
            Page::Connections => self.connections(),
            Page::Rules => self.rules(),
            Page::Logs => self.logs(),
            Page::Tests => self.tests(),
            Page::Websites => self.websites(),
            Page::Settings => self.settings_view(),
        };
        let mut body = column![header, notice].spacing(8);
        if let Some(update) = &self.updates.available {
            body = body.push(
                container(
                    aligned_row![
                        self.label(format!("发现新版本 {}", update.version))
                            .color(ACCENT),
                        Space::new().width(Length::Fill),
                        button(self.label("更新"))
                            .style(rounded_primary)
                            .padding([4, 10])
                            .on_press_maybe(
                                (!self.updates.busy() && !self.exiting)
                                    .then_some(Message::InstallUpdate(false))
                            ),
                        button(self.label("更新（代理）"))
                            .style(rounded_secondary)
                            .padding([4, 10])
                            .on_press_maybe(
                                (!self.updates.busy() && !self.exiting)
                                    .then_some(Message::InstallUpdate(true))
                            )
                    ]
                    .spacing(8),
                )
                .padding(8)
                .width(Length::Fill)
                .style(panel),
            );
        }
        if let Some(progress) = self.updates.progress {
            let total = progress.total.max(1);
            let percent = (progress.received as f64 / total as f64 * 100.0).min(100.0);
            body = body.push(
                column![
                    self.label(format!(
                        "正在下载更新 · {percent:.0}%（{:.1} / {:.1} MiB）",
                        progress.received as f64 / 1048576.0,
                        progress.total as f64 / 1048576.0
                    ))
                    .size(self.scaled(11)),
                    progress_bar(0.0..=100.0, percent as f32)
                ]
                .spacing(4),
            );
        }
        let body = body.push(content);
        aligned_row![
            sidebar,
            container(body)
                .padding(8)
                .width(Length::Fill)
                .height(Length::Fill)
        ]
        .height(Length::Fill)
        .into()
    }

    fn card<'a>(&self, title: &'a str, value: String, detail: &'a str) -> Element<'a, Message> {
        container(
            column![
                aligned_row![
                    self.label(title)
                        .size(self.scaled(11))
                        .color(self.foreground()),
                    Space::new().width(Length::Fill),
                    self.label(detail)
                        .size(self.scaled(10))
                        .color(self.foreground())
                ]
                .spacing(4),
                self.label(value).size(self.scaled(24)).color(ACCENT),
            ]
            .spacing(3),
        )
        .padding(8)
        .width(Length::Fill)
        .style(panel)
        .into()
    }

    fn home(&self) -> Element<'_, Message> {
        let stats = aligned_row![
            self.card(
                "上传速度",
                format!("{}/s", bytes(self.snapshot.upload_rate)),
                "实时流量"
            ),
            self.card(
                "下载速度",
                format!("{}/s", bytes(self.snapshot.download_rate)),
                "实时流量"
            ),
            self.card(
                "活跃连接",
                self.snapshot.connection_count.to_string(),
                "当前连接"
            )
        ]
        .spacing(10);
        let mut modes = aligned_row![
            self.label("运行模式").size(self.scaled(12)),
            Space::new().width(Length::Fill)
        ]
        .spacing(8);
        for (name, mode) in [("规则", "rule"), ("全局", "global"), ("直连", "direct")] {
            modes = modes.push(
                button(self.label(name).size(self.scaled(12)))
                    .padding([4, 10])
                    .style(if self.snapshot.mode == mode {
                        rounded_primary
                    } else {
                        rounded_secondary
                    })
                    .on_press_maybe(
                        (self.snapshot.running
                            && !self.working
                            && !self.elevating
                            && !self.exiting
                            && self.snapshot.mode != mode)
                            .then_some(Message::Action(Action::Mode(mode.into()))),
                    ),
            );
        }
        let proxy_mode = if self.snapshot.tun {
            ProxyMode::Tun
        } else if self.snapshot.system_proxy {
            ProxyMode::System
        } else {
            ProxyMode::Off
        };
        let mut options = aligned_row![].spacing(8);
        for (label, mode) in [
            ("系统代理", ProxyMode::System),
            ("TUN模式", ProxyMode::Tun),
            ("关闭", ProxyMode::Off),
        ] {
            options = options.push(
                button(self.label(label).size(self.scaled(12)))
                    .padding([5, 10])
                    .style(if proxy_mode == mode {
                        rounded_primary
                    } else {
                        rounded_secondary
                    })
                    .on_press_maybe(
                        (!self.working
                            && !self.elevating
                            && !self.exiting
                            && self.engine.is_some()
                            && proxy_mode != mode
                            && (self.snapshot.running || mode == ProxyMode::Off))
                            .then_some(Message::Action(Action::ProxyMode(mode))),
                    ),
            );
        }
        let ingress = aligned_row![
            self.label("代理模式").size(self.scaled(12)),
            Space::new().width(Length::Fill),
            options
        ]
        .spacing(10);
        let totals = aligned_row![
            self.card(
                "累计上传",
                bytes(self.snapshot.connections.upload_total),
                "本次运行"
            ),
            self.card(
                "累计下载",
                bytes(self.snapshot.connections.download_total),
                "本次运行"
            )
        ]
        .spacing(10);
        let footer = aligned_row![
            Space::new().width(Length::Fill),
            self.label(format!(
                "mihomo {}",
                if self.snapshot.version.is_empty() {
                    "—"
                } else {
                    &self.snapshot.version
                }
            ))
            .size(self.scaled(11))
            .color(self.foreground())
        ]
        .spacing(10);
        column![
            stats,
            container(modes).padding(8).width(Length::Fill).style(panel),
            container(ingress)
                .padding(8)
                .width(Length::Fill)
                .style(panel),
            totals,
            Space::new().height(Length::Fill),
            footer
        ]
        .spacing(8)
        .height(Length::Fill)
        .into()
    }

    fn search(&self, placeholder: &'static str) -> Element<'_, Message> {
        text_input(placeholder, &self.query)
            .style(rounded_input)
            .size(self.scaled(12))
            .on_input(Message::Query)
            .padding(8)
            .into()
    }

    fn proxies(&self) -> Element<'_, Message> {
        let query = self.query.to_lowercase();
        let mut list = column![].spacing(8);
        for (group, proxy) in &self.snapshot.proxies.proxies {
            if proxy.all.is_empty() {
                continue;
            }
            let group_match = contains_query(group, &query);
            let nodes = proxy
                .all
                .iter()
                .filter(|node| group_match || contains_query(node, &query));
            let total = nodes.clone().count();
            if total == 0 {
                continue;
            }
            let expanded = self.expanded.contains(group) || !query.is_empty();
            let mut contents = column![
                aligned_row![
                    button(
                        aligned_row![
                            self.label(if expanded { "▼" } else { "▶" })
                                .size(self.scaled(12)),
                            column![
                                self.label(group).size(self.scaled(12)),
                                self.label(format!(
                                    "{} · 当前：{} · {} 个节点",
                                    proxy.kind,
                                    proxy.now,
                                    proxy.all.len()
                                ))
                                .size(self.scaled(10))
                                .color(self.foreground())
                            ]
                            .spacing(4)
                        ]
                        .spacing(8)
                    )
                    .padding(10)
                    .width(Length::Fill)
                    .style(rounded_text)
                    .on_press(Message::ToggleGroup(group.clone())),
                    button(
                        self.label(if self.testing_groups.contains(group) {
                            "测速中…"
                        } else {
                            "全部测速"
                        })
                        .size(self.scaled(11))
                    )
                    .padding([7, 12])
                    .style(rounded_secondary)
                    .on_press_maybe(
                        (self.snapshot.running
                            && !self.working
                            && self.testing_groups.len() < 2
                            && !self.testing_groups.contains(group))
                        .then(|| Message::GroupDelay(group.clone()))
                    )
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center)
            ]
            .spacing(4);
            if expanded {
                let offset = self
                    .group_offsets
                    .get(group)
                    .copied()
                    .unwrap_or(0)
                    .min(last_page_offset(total));
                for node in nodes.skip(offset).take(PAGE_SIZE) {
                    let detail = self.snapshot.proxies.proxies.get(node);
                    let delay_value = self
                        .node_delays
                        .get(node)
                        .copied()
                        .or_else(|| detail.and_then(|p| p.history.last()).map(|d| d.delay));
                    let delay = delay_value
                        .map(|delay| {
                            if delay == 0 {
                                "超时".into()
                            } else {
                                format!("{delay} ms")
                            }
                        })
                        .unwrap_or_else(|| "未测速".into());
                    let selected = proxy.now == *node;
                    let kind = detail.map(|p| p.kind.as_str()).unwrap_or("节点");
                    contents =
                        contents.push(
                            container(
                                aligned_row![
                                    column![
                                        self.label(format!(
                                            "{}{}",
                                            if selected { "● " } else { "○ " },
                                            node
                                        ))
                                        .size(self.scaled(12))
                                        .color(if selected { ACCENT } else { self.foreground() }),
                                        aligned_row![
                                            self.label(kind).size(self.scaled(10)),
                                            self.label(delay).size(self.scaled(10)).color(
                                                delay_value
                                                    .map(latency_color)
                                                    .unwrap_or(self.foreground())
                                            )
                                        ]
                                        .spacing(6)
                                    ]
                                    .spacing(4),
                                    Space::new().width(Length::Fill),
                                    self.action(
                                        "测速",
                                        Action::Delay(node.clone()),
                                        self.snapshot.running
                                    ),
                                    self.action(
                                        if selected { "已选择" } else { "选择" },
                                        Action::Select(group.clone(), node.clone()),
                                        self.snapshot.running
                                            && proxy.kind == "Selector"
                                            && !selected
                                    )
                                ]
                                .spacing(7),
                            )
                            .padding([7, 20])
                            .width(Length::Fill),
                        );
                }
                if total > PAGE_SIZE {
                    contents = contents.push(
                        aligned_row![
                            self.label(format!(
                                "{} 个节点 · {}–{}",
                                total,
                                offset + 1,
                                (offset + PAGE_SIZE).min(total)
                            ))
                            .size(self.scaled(10))
                            .color(self.foreground()),
                            Space::new().width(Length::Fill),
                            button(self.label("上一页")).on_press_maybe(
                                (offset > 0).then(|| Message::GroupPage(group.clone(), false))
                            ),
                            button(self.label("下一页")).on_press_maybe(
                                (offset + PAGE_SIZE < total)
                                    .then(|| Message::GroupPage(group.clone(), true))
                            )
                        ]
                        .spacing(7),
                    );
                }
            }
            list = list.push(
                container(contents)
                    .padding(6)
                    .width(Length::Fill)
                    .style(panel),
            );
        }
        if self.snapshot.proxies.proxies.is_empty() {
            list = list.push(
                self.label("策略组正在加载；默认直连配置无需选择节点。")
                    .color(self.foreground()),
            );
        }
        column![
            self.search("搜索策略组或节点 · 搜索时自动展开"),
            scrollable(list).height(Length::Fill)
        ]
        .spacing(8)
        .into()
    }

    fn websites(&self) -> Element<'_, Message> {
        let query = self.query.to_lowercase();
        let services = ip_check::services();
        let mut list = column![].spacing(4);
        let mut visible = 0;
        for (index, service) in services.iter().enumerate() {
            let result = self
                .site_results
                .iter()
                .find(|(id, _)| id == &service.id)
                .map(|(_, result)| result);
            let busy = self.site_busy.contains(&service.id);
            let queued = self.site_queue.contains(&service.id);
            let (summary, country, delay, color) = match result {
                Some(Ok(result)) => (
                    result.summary.clone(),
                    result.country.clone(),
                    format!("{} ms", result.millis),
                    match result.state {
                        ip_check::State::Confirmed => ACCENT,
                        ip_check::State::Reachable => ACCENT,
                        ip_check::State::Restricted => latency_color(0),
                        ip_check::State::Unknown => latency_color(500),
                    },
                ),
                Some(Err(_)) => (
                    "超时或失败".into(),
                    "未获取".into(),
                    "—".into(),
                    latency_color(0),
                ),
                None => ("未检测".into(), "—".into(), "—".into(), self.foreground()),
            };
            if !format!("{} {} {} {}", service.name, service.group, summary, country)
                .to_lowercase()
                .contains(&query)
            {
                continue;
            }
            visible += 1;
            let state = if busy {
                "检测中…".to_owned()
            } else if queued {
                "等待检测".to_owned()
            } else {
                summary
            };
            let (country_text, flag) = flags::label(&country);
            let mut country_row = aligned_row![].spacing(4);
            if let Some(flag) = flag {
                country_row = country_row.push(image(flag).width(20).height(20));
            }
            country_row =
                country_row.push(self.label(country_text.to_owned()).size(self.scaled(10)));
            let entry = column![
                aligned_row![
                    self.label(format!("{:03}", index + 1))
                        .size(self.scaled(10))
                        .width(34),
                    self.label(&service.name)
                        .size(self.scaled(11))
                        .width(Length::FillPortion(3)),
                    self.label(&service.group).size(self.scaled(10)).width(60),
                    self.label(state)
                        .size(self.scaled(10))
                        .color(if busy || queued {
                            self.foreground()
                        } else {
                            color
                        })
                        .width(Length::FillPortion(3)),
                    country_row.width(Length::FillPortion(2)),
                    self.label(delay)
                        .size(self.scaled(10))
                        .color(match result {
                            Some(Ok(result)) if !busy && !queued =>
                                latency_color(result.millis.min(u128::from(u32::MAX)) as u32),
                            Some(Err(_)) if !busy && !queued => latency_color(0),
                            _ => self.foreground(),
                        })
                        .width(70)
                ]
                .spacing(6)
            ]
            .spacing(6);
            list = list.push(container(entry).padding(8).style(panel));
        }
        if visible == 0 {
            list = list.push(self.label("没有匹配的检测项目。"));
        }
        column![
            aligned_row![
                self.search("搜索平台、地区或检测结果"),
                self.label(format!(
                    "已完成 {} / {}",
                    self.site_results.len(),
                    services.len()
                ))
                .size(self.scaled(10)),
                button(self.label("一键检测"))
                    .padding([5, 10])
                    .style(rounded_primary)
                    .on_press_maybe(
                        (self.snapshot.running
                            && self.site_busy.is_empty()
                            && self.site_queue.is_empty())
                        .then_some(Message::SiteAll)
                    ),
                button(self.label("取消"))
                    .padding([5, 10])
                    .style(rounded_secondary)
                    .on_press_maybe(
                        (!self.site_busy.is_empty() || !self.site_queue.is_empty())
                            .then_some(Message::SiteCancel)
                    )
            ]
            .spacing(8),
            aligned_row![
                self.label("序号").size(self.scaled(10)).width(34),
                self.label("平台 / IP")
                    .size(self.scaled(10))
                    .width(Length::FillPortion(3)),
                self.label("分类").size(self.scaled(10)).width(60),
                self.label("检测结果")
                    .size(self.scaled(10))
                    .width(Length::FillPortion(3)),
                self.label("识别地区")
                    .size(self.scaled(10))
                    .width(Length::FillPortion(2)),
                self.label("延迟").size(self.scaled(10)).width(70)
            ]
            .spacing(6)
            .padding([0, 8]),
            scrollable(list).height(Length::Fill)
        ]
        .spacing(8)
        .into()
    }

    fn profiles_view(&self) -> Element<'_, Message> {
        let form = container(
            column![
                self.label("添加配置").size(self.scaled(12)),
                text_input("订阅名称", &self.profile_name)
                    .style(rounded_input)
                    .size(self.scaled(12))
                    .on_input(Message::ProfileName)
                    .padding(8),
                text_input(
                    "https://订阅地址 或 本地 YAML 文件完整路径",
                    &self.profile_source
                )
                .style(rounded_input)
                .size(self.scaled(12))
                .on_input(Message::ProfileSource)
                .padding(8),
                aligned_row![
                    self.action(
                        "导入配置",
                        Action::Import(
                            self.profile_name.clone(),
                            self.profile_source.trim().into()
                        ),
                        !self.profile_source.trim().is_empty()
                    ),
                    self.action("打开配置目录", Action::OpenProfiles, true)
                ]
                .spacing(8)
            ]
            .spacing(8),
        )
        .padding(14)
        .style(panel);
        let mut list = column![].spacing(8);
        for profile in &self.profiles {
            let active = self.settings.active_profile.as_ref() == Some(&profile.id);
            let remote =
                profile.source.starts_with("http://") || profile.source.starts_with("https://");
            let update_buttons: Element<'_, Message> = if remote {
                aligned_row![
                    self.action(
                        "更新",
                        Action::UpdateProfile(profile.id.clone(), false),
                        true
                    ),
                    self.action(
                        "更新（代理）",
                        Action::UpdateProfile(profile.id.clone(), true),
                        true
                    )
                ]
                .spacing(7)
                .into()
            } else {
                Space::new().width(0).into()
            };
            let source = if profile.source.starts_with("http") {
                reqwest::Url::parse(&profile.source)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    .unwrap_or_else(|| "远程订阅".into())
            } else {
                "本地文件".into()
            };
            list = list.push(
                container(
                    column![
                        aligned_row![
                            self.label(format!(
                                "{}{}",
                                if active { "● " } else { "" },
                                profile.name
                            ))
                            .size(self.scaled(12)),
                            Space::new().width(Length::Fill),
                            update_buttons,
                            if profile.is_default() {
                                Space::new().width(0).into()
                            } else {
                                self.action("删除", Action::DeleteProfile(profile.id.clone()), true)
                            },
                            self.action(
                                if active { "使用中" } else { "使用" },
                                Action::Activate(profile.id.clone()),
                                !active
                            )
                        ]
                        .spacing(7),
                        self.label(format!(
                            "{source} · 更新于 {}",
                            presentation::timestamp(profile.updated)
                        ))
                        .size(self.scaled(10))
                        .color(self.foreground()),
                        self.label(presentation::subscription_usage(profile.usage.as_deref()))
                            .size(self.scaled(10))
                            .color(self.foreground())
                    ]
                    .spacing(7),
                )
                .padding(13)
                .style(panel),
            );
        }
        column![form, scrollable(list).height(Length::Fill)]
            .spacing(14)
            .into()
    }

    fn connections(&self) -> Element<'_, Message> {
        let query = self.query.to_lowercase();
        let entries = self.snapshot.connections.connections.iter().filter(|c| {
            query.is_empty()
                || query.split_whitespace().all(|word| {
                    [
                        &c.metadata.host,
                        &c.metadata.destination_ip,
                        &c.metadata.process,
                        &c.rule,
                    ]
                    .into_iter()
                    .chain(c.chains.iter())
                    .any(|field| contains_query(field, word))
                })
        });
        let total = entries.clone().count();
        let offset = self.list_offset.min(last_page_offset(total));
        let mut list = column![].spacing(6);
        for connection in entries.skip(offset).take(PAGE_SIZE) {
            let m = &connection.metadata;
            let host = if m.host.is_empty() {
                &m.destination_ip
            } else {
                &m.host
            };
            list = list.push(
                container(
                    aligned_row![
                        column![
                            self.label(format!("{}:{} · {}", host, m.destination_port, m.network))
                                .size(self.scaled(12)),
                            self.label(format!(
                                "{} · {} → {}",
                                if m.process.is_empty() {
                                    "未知进程"
                                } else {
                                    &m.process
                                },
                                connection.rule,
                                connection.chains.join(" → ")
                            ))
                            .size(self.scaled(10))
                            .color(self.foreground()),
                            self.label(format!(
                                "↑ {}   ↓ {}   · {}",
                                bytes(connection.upload),
                                bytes(connection.download),
                                presentation::date_text(&connection.start)
                            ))
                            .size(self.scaled(10))
                            .color(self.foreground())
                        ]
                        .spacing(4),
                        Space::new().width(Length::Fill),
                        self.action(
                            "关闭",
                            Action::Close(connection.id.clone()),
                            self.snapshot.running
                        )
                    ]
                    .spacing(6),
                )
                .padding(10)
                .style(panel),
            );
        }
        if total == 0 {
            list = list.push(self.label("当前没有匹配的连接。").color(self.foreground()));
        }
        column![
            self.search("搜索域名、IP、进程或规则"),
            scrollable(list).height(Length::Fill),
            self.list_pager(total, offset)
        ]
        .spacing(8)
        .into()
    }

    fn rules(&self) -> Element<'_, Message> {
        let query = self.query.to_lowercase();
        let entries = self
            .snapshot
            .rules
            .rules
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                query.is_empty()
                    || query.split_whitespace().all(|word| {
                        [&r.kind, &r.payload, &r.proxy]
                            .into_iter()
                            .any(|field| contains_query(field, word))
                    })
            });
        let total = entries.clone().count();
        let offset = self.list_offset.min(last_page_offset(total));
        let mut list = column![].spacing(4);
        for (index, rule) in entries.skip(offset).take(PAGE_SIZE) {
            list = list.push(
                container(
                    aligned_row![
                        self.label(format!("{:04}", index + 1))
                            .size(self.scaled(10))
                            .color(self.foreground())
                            .width(50),
                        self.label(&rule.kind).size(self.scaled(11)).width(150),
                        self.label(&rule.payload)
                            .size(self.scaled(11))
                            .width(Length::Fill),
                        self.label(&rule.proxy)
                            .size(self.scaled(11))
                            .color(ACCENT)
                            .width(160)
                    ]
                    .spacing(8),
                )
                .padding(8)
                .style(panel),
            );
        }
        column![
            self.search("搜索规则类型、内容或策略"),
            scrollable(list).height(Length::Fill),
            self.list_pager(total, offset)
        ]
        .spacing(8)
        .into()
    }

    fn list_pager(&self, total: usize, offset: usize) -> Element<'_, Message> {
        aligned_row![
            self.label(if total == 0 {
                "0 条".into()
            } else {
                format!(
                    "{} 条 · {}–{}",
                    total,
                    offset + 1,
                    (offset + PAGE_SIZE).min(total)
                )
            })
            .size(self.scaled(10)),
            Space::new().width(Length::Fill),
            button(self.label("上一页")).on_press_maybe(
                (offset > 0).then_some(Message::ListPage(offset.saturating_sub(PAGE_SIZE)))
            ),
            button(self.label("下一页")).on_press_maybe(
                (offset + PAGE_SIZE < total).then_some(Message::ListPage(offset + PAGE_SIZE))
            )
        ]
        .spacing(8)
        .into()
    }

    fn logs(&self) -> Element<'_, Message> {
        let query = self.query.to_lowercase();
        let mut list = column![].spacing(4);
        for line in self
            .snapshot
            .logs
            .iter()
            .rev()
            .filter(|l| contains_query(l, &query))
            .take(150)
        {
            list = list.push(
                container(self.label(line.as_ref()).size(self.scaled(10)))
                    .padding(6)
                    .width(Length::Fill)
                    .style(panel),
            );
        }
        column![
            aligned_row![
                self.search("搜索日志 / error / warning"),
                self.action("清空", Action::ClearLogs, true),
                self.action("导出", Action::ExportLogs, true)
            ]
            .spacing(7),
            scrollable(list).height(Length::Fill)
        ]
        .spacing(8)
        .into()
    }

    fn tests(&self) -> Element<'_, Message> {
        let available = !self.probing && self.snapshot.running;
        let network = container(
            column![
                self.label("网站与出口 IP").size(self.scaled(12)),
                text_input("https://example.com", &self.test_url)
                    .style(rounded_input)
                    .size(self.scaled(12))
                    .on_input(Message::TestUrl)
                    .padding(8),
                aligned_row![
                    button(self.label("测试网站响应"))
                        .padding(7)
                        .on_press_maybe(available.then_some(Message::Probe(false))),
                    button(self.label("查询出口 IP"))
                        .padding(7)
                        .on_press_maybe(available.then_some(Message::Probe(true)))
                ]
                .spacing(8)
            ]
            .spacing(10),
        )
        .padding(14)
        .style(panel);
        let dns = container(
            column![
                self.label("系统 DNS 查询").size(self.scaled(12)),
                aligned_row![
                    text_input("github.com", &self.dns_host)
                        .style(rounded_input)
                        .size(self.scaled(12))
                        .on_input(Message::DnsHost)
                        .padding(8),
                    button(self.label("解析域名"))
                        .padding(8)
                        .on_press_maybe((!self.probing).then_some(Message::Dns))
                ]
                .spacing(7)
            ]
            .spacing(8),
        )
        .padding(14)
        .style(panel);
        let mut results = column![
            self.label(if self.probing {
                "正在测试…"
            } else {
                "测试记录"
            })
            .size(self.scaled(12))
        ]
        .spacing(8);
        for result in self.test_results.iter().rev() {
            results = results.push(self.label(result).size(self.scaled(11)));
        }
        column![network, dns, scrollable(results).height(Length::Fill)]
            .spacing(14)
            .into()
    }

    fn ports_dirty(&self) -> bool {
        self.controller_port.trim().parse::<u16>() != Ok(self.settings.controller_port)
            || self.mixed_port.trim().parse::<u16>() != Ok(self.settings.mixed_port)
    }

    fn settings_view(&self) -> Element<'_, Message> {
        let updates = column![
            self.label(format!("客户端更新 · 当前版本 {}", clash_of_rust::VERSION)),
            self.label(if self.updates.checking {
                "正在检查更新…"
            } else if self.updates.status.is_empty() {
                "等待首次检查"
            } else {
                &self.updates.status
            })
            .size(self.scaled(11)),
            aligned_row![
                button(self.label("检查更新"))
                    .style(rounded_primary)
                    .padding([4, 10])
                    .on_press_maybe(
                        (!self.updates.checking && !self.updates.busy() && !self.exiting)
                            .then_some(Message::CheckUpdate)
                    ),
                button(self.label("查看 Release"))
                    .style(rounded_secondary)
                    .padding([4, 10])
                    .on_press(Message::OpenRelease)
            ]
            .spacing(8)
        ]
        .spacing(12);
        let mut ports = column![
            self.label("端口").size(self.scaled(12)),
            aligned_row![
                column![
                    self.label("控制接口")
                        .size(self.scaled(12))
                        .color(self.foreground()),
                    text_input("9090", &self.controller_port)
                        .size(self.scaled(12))
                        .style(rounded_input)
                        .on_input(Message::ControllerPort)
                        .padding(9)
                ]
                .spacing(6),
                column![
                    self.label("HTTP / SOCKS 混合端口")
                        .size(self.scaled(12))
                        .color(self.foreground()),
                    text_input("7897", &self.mixed_port)
                        .size(self.scaled(12))
                        .style(rounded_input)
                        .on_input(Message::MixedPort)
                        .padding(9)
                ]
                .spacing(6)
            ]
            .spacing(16),
        ]
        .spacing(12);
        if self.ports_dirty() {
            ports = ports.push(self.action(
                "保存设置并重启内核",
                Action::Save(self.settings.clone()),
                self.testing_groups.is_empty(),
            ));
        }
        let appearance = column![
            self.label("外观").size(self.scaled(12)),
            aligned_row![
                button(self.label("深色").size(self.scaled(12)))
                    .padding([7, 16])
                    .style(if self.dark {
                        rounded_primary
                    } else {
                        rounded_secondary
                    })
                    .on_press(Message::Dark(true)),
                button(self.label("浅色").size(self.scaled(12)))
                    .padding([7, 16])
                    .style(if self.dark {
                        rounded_secondary
                    } else {
                        rounded_primary
                    })
                    .on_press(Message::Dark(false))
            ]
            .spacing(8),
            aligned_row![
                self.label("窗口大小"),
                text_input("宽", &self.window_width)
                    .style(rounded_input)
                    .size(self.scaled(12))
                    .padding(7)
                    .width(100)
                    .on_input(Message::WindowWidth),
                self.label("×"),
                text_input("高", &self.window_height)
                    .style(rounded_input)
                    .size(self.scaled(12))
                    .padding(7)
                    .width(100)
                    .on_input(Message::WindowHeight),
                self.label("像素"),
                button(self.label("恢复默认").size(self.scaled(12)))
                    .padding([7, 10])
                    .style(rounded_secondary)
                    .on_press(Message::ResetWindowSize)
            ]
            .spacing(8),
        ]
        .spacing(12);
        let geo = column![
            self.label("Geo 数据").size(self.scaled(12)),
            self.label(presentation::geo_version(&self.geo_status))
                .size(self.scaled(11))
                .color(self.foreground()),
            self.action("更新所有 Geo 数据", Action::UpdateGeo, true)
        ]
        .spacing(12);
        scrollable(
            column![
                container(updates)
                    .padding(10)
                    .width(Length::Fill)
                    .style(panel),
                container(ports)
                    .padding(10)
                    .width(Length::Fill)
                    .style(panel),
                container(appearance)
                    .padding(10)
                    .width(Length::Fill)
                    .style(panel),
                container(
                    aligned_row![
                        self.label("开机自启（后台静默启动）"),
                        Space::new().width(Length::Fill),
                        self.action(
                            if self.autostart { "关闭" } else { "开启" },
                            Action::Autostart(!self.autostart),
                            true
                        )
                    ]
                    .spacing(8)
                )
                .padding(10)
                .width(Length::Fill)
                .style(panel),
                container(
                    column![
                        self.label("定时测速"),
                        aligned_row![
                            text_input("间隔（分钟）", &self.interval_input)
                                .style(rounded_input)
                                .size(self.scaled(12))
                                .padding(7)
                                .on_input(Message::IntervalInput)
                                .width(140),
                            self.label("分钟 · 0 表示关闭"),
                            button(self.label("保存间隔"))
                                .style(rounded_primary)
                                .padding([4, 10])
                                .on_press_maybe(
                                    (self.interval_input.trim().parse::<u32>().ok()
                                        != Some(self.settings.delay_interval_minutes)
                                        && !self.working)
                                        .then_some(Message::SaveInterval)
                                )
                        ]
                        .spacing(8)
                        .align_y(iced::alignment::Vertical::Center),
                    ]
                    .spacing(8)
                )
                .padding(10)
                .width(Length::Fill)
                .style(panel),
                container(geo).padding(10).width(Length::Fill).style(panel)
            ]
            .spacing(8),
        )
        .height(Length::Fill)
        .into()
    }
}

fn bytes(value: u64) -> String {
    if value < 1024 {
        format!("{value} B")
    } else if value < 1024 * 1024 {
        format!("{:.1} KiB", value as f64 / 1024.0)
    } else if value < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", value as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GiB", value as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn latency_color(delay: u32) -> Color {
    match delay {
        1..=299 => ACCENT,
        300..=999 => Color::from_rgb(0.93, 0.73, 0.25),
        _ => Color::from_rgb(0.96, 0.34, 0.36),
    }
}

fn panel(theme: &Theme) -> container::Style {
    let mut style = container::rounded_box(theme);
    style.border.radius = 16.0.into();
    style
}

fn rounded_primary(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::primary(theme, status);
    style.border.radius = 12.0.into();
    if matches!(theme, Theme::TokyoNight) {
        style.text_color = Color::WHITE;
    }
    style
}
fn rounded_secondary(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::secondary(theme, status);
    style.border.radius = 12.0.into();
    if matches!(theme, Theme::TokyoNight) {
        style.text_color = Color::WHITE;
    }
    style
}
fn rounded_text(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::text(theme, status);
    style.border.radius = 12.0.into();
    if matches!(theme, Theme::TokyoNight) {
        style.text_color = Color::WHITE;
    }
    style
}
fn rounded_input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let mut style = text_input::default(theme, status);
    style.border.radius = 10.0.into();
    if matches!(theme, Theme::TokyoNight) {
        style.value = Color::WHITE;
        style.placeholder = Color::from_rgb(0.56, 0.59, 0.64);
        style.icon = Color::WHITE;
    }
    style
}

fn sidebar_panel(theme: &Theme) -> container::Style {
    let mut style = panel(theme);
    style.border.radius = iced::border::Radius {
        top_left: 0.0,
        top_right: 16.0,
        bottom_right: 16.0,
        bottom_left: 0.0,
    };
    style
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn pagination_bounds_rows_without_losing_the_last_page() {
        for (total, expected) in [
            (0, 0),
            (1, 0),
            (60, 0),
            (61, 60),
            (120, 60),
            (100_000, 99_960),
        ] {
            let offset = last_page_offset(total);
            assert_eq!(offset, expected);
            assert!((0..total).skip(offset).take(PAGE_SIZE).count() <= PAGE_SIZE);
            if total > 0 {
                assert!(offset < total);
            }
        }
    }

    #[test]
    fn searching_supports_ascii_unicode_and_empty_queries() {
        assert!(contains_query("EXAMPLE.COM", "example"));
        assert!(contains_query("香港节点", "香港"));
        assert!(contains_query("École", "école"));
        assert!(contains_query("", ""));
        assert!(!contains_query("short", "much longer"));
    }
}
