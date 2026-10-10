#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[path = "presentation.rs"]
mod presentation;
#[path = "typography.rs"]
mod typography;
#[cfg(feature = "ui-preview")]
#[path = "ui_preview.rs"]
pub(crate) mod ui_preview;
#[path = "ui_style.rs"]
mod ui_style;

use clash_of_rust::{
    config::{Profile, Settings, Store},
    engine::{Engine, ProxyMode, Scope, Snapshot},
    flags, icons, ip_check,
    ip_report::{self, CategoryFilter, RegionFilter, ResultSlot, Status, StatusFilter},
    platform, probe,
    proxy_order::{self, NodeSort},
    tray, update,
};
use futures_util::{
    SinkExt,
    future::{AbortHandle, Abortable},
};
use iced::{
    Color, Element, Length, Subscription, Task, Theme,
    widget::{
        Space, button, column, container, image, pick_list, progress_bar, scrollable, stack, text,
        text_input,
    },
};
use std::{
    cell::RefCell,
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
    #[cfg(windows)]
    if platform::autostart_helper_main() {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if platform::tun_helper_main() {
        return Ok(());
    }
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
    // Publish the native icon before cold font loading and Geo integrity checks.
    let tray_startup = std::sync::Mutex::new(Some(tray::Startup::new(Duration::from_secs(
        if background_start() { 120 } else { 5 },
    ))));
    #[cfg(windows)]
    std::thread::spawn(platform::migrate_autostart);
    typography::initialize();
    iced::application(
        move || App::new(tray_startup.lock().unwrap().take()),
        App::update,
        App::view,
    )
    .title("Clash of Rust · 原生代理客户端")
    .theme(App::theme)
    .subscription(App::subscription)
    .settings(iced::Settings {
        default_text_size: iced::Pixels(15.0),
        ..Default::default()
    })
    .default_font(typography::ENGLISH_FONT)
    .window(iced::window::Settings {
        size: DEFAULT_WINDOW_SIZE,
        min_size: Some(MIN_WINDOW_SIZE),
        icon: Some(icons::window()),
        exit_on_close_request: false,
        visible: !background_start(),
        #[cfg(target_os = "linux")]
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: "clash-of-rust".into(),
            ..Default::default()
        },
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

const PAGE_SIZE: usize = 60;
const GROUP_PAGE_SIZE: usize = 4;

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
    fn description(self) -> &'static str {
        match self {
            Self::Home => "运行状态与流量概览",
            Self::Proxies => "决策组与节点选择",
            Self::Profiles => "导入、更新与切换配置",
            Self::Connections => "查看当前连接与流量去向",
            Self::Rules => "当前配置的匹配顺序与转发策略",
            Self::Logs => "内核与客户端的运行记录",
            Self::Tests => "网站响应、出口信息与 DNS 检查",
            Self::Websites => "当前出口与平台检测结果",
            Self::Settings => "客户端、网络与外观设置",
        }
    }
}

#[derive(Clone)]
struct CoreHandle(Arc<Mutex<Engine>>);
impl std::fmt::Debug for CoreHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CoreHandle")
    }
}

#[derive(Debug, Clone)]
enum Action {
    Refresh,
    Start,
    SavePorts(u16, u16),
    Theme(bool),
    Autostart(bool),
    DelayInterval(u32),
    NodeSort(NodeSort),
    Import(String, String),
    Activate(String),
    UpdateProfile(String),
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

// The message can be cloned; only one receiver may consume the owned installer.
type DownloadResult = Arc<std::sync::Mutex<Option<Result<update::Downloaded, String>>>>;

#[derive(Debug, Clone)]
enum Message {
    RestartCore,
    CoreReady(Result<CoreHandle, String>),
    DismissNotice,
    FocusSearch,
    Navigate(Page),
    Query(String),
    NodeSort(NodeSort),
    SiteCategory(CategoryFilter),
    SiteStatus(StatusFilter),
    SiteRegion(RegionFilter),
    SiteDetail(usize),
    SiteSummaryFilter(CategoryFilter, StatusFilter),
    SiteResetFilters,
    SiteReportCategories,
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
    InstallUpdate,
    CancelUpdateDownload,
    UpdateProgress(u64, update::Progress),
    UpdateDownloaded(u64, DownloadResult),
    UpdateHandoff(Result<update::InstallSession, String>),
    OpenRelease,
    ReleaseOpened(Result<(), String>),
    Tick,
    ToggleGroup(String),
    GroupPage(String, bool),
    GroupDelay(String),
    GroupDelayDone(u64, String, Result<BTreeMap<String, u32>, String>),
    Tray(tray::Command),
    ShowWindow(Option<iced::window::Id>),
    SiteAll,
    SiteCancel,
    IntervalInput(String),
    SaveInterval,
    PeriodicDelay,
    PeriodicDone(u64, Result<BTreeMap<String, u32>, String>),
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
    node_sort: NodeSort,
    proxy_order: RefCell<BTreeMap<clash_of_rust::api::Text, Vec<usize>>>,
    delay_generation: u64,
    delay_aborters: BTreeMap<String, AbortHandle>,
    periodic_aborter: Option<AbortHandle>,
    tray: Option<tray::Guard>,
    tray_startup: Option<tray::Startup>,
    tray_close_requested: bool,
    site_queue: VecDeque<String>,
    interval_input: String,
    periodic_testing: bool,
    site_results: Vec<ResultSlot>,
    site_category: CategoryFilter,
    site_status: StatusFilter,
    site_region: RegionFilter,
    site_detail: Option<usize>,
    site_report_categories: bool,
    site_generation: u64,
    site_aborters: BTreeMap<String, AbortHandle>,
    site_busy: BTreeSet<String>,
    exiting: bool,
    notice: String,
    error: bool,
    core_failure: Option<String>,
    notice_visible: bool,
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
            Action::SavePorts(controller, mixed) => {
                engine.save_ports(controller, mixed).await?;
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
            Action::NodeSort(node_sort) => {
                let settings = Settings {
                    node_sort,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("节点排序已保存".into())
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
            Action::UpdateProfile(id) => {
                let profile = engine
                    .profiles
                    .iter()
                    .find(|p| p.id == id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("配置不存在"))?;
                engine
                    .import(profile.name, profile.source, Some(profile.id))
                    .await?;
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
    fn new(tray_startup: Option<tray::Startup>) -> (Self, Task<Message>) {
        Self::with_engine(startup_store().and_then(Engine::new), tray_startup)
    }

    fn with_engine(
        created: anyhow::Result<Engine>,
        mut tray_startup: Option<tray::Startup>,
    ) -> (Self, Task<Message>) {
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
        let (tray, notice, error) = match tray_startup.as_ref().and_then(tray::Startup::poll) {
            Some(Ok(tray)) => {
                tray_startup = None;
                (Some(tray), notice, error)
            }
            Some(Err(e)) => {
                tray_startup = None;
                (
                    None,
                    format!("{notice} 托盘不可用：{e}；关闭窗口将退出。"),
                    true,
                )
            }
            None => (None, notice, error),
        };
        let core_failure = engine.is_none().then(|| notice.clone());
        let mut app = Self {
            node_sort: settings.node_sort,
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
            core_failure,
            notice_visible: error,
            page: Page::Home,
            snapshot: Snapshot::default(),
            busy: false,
            working: false,
            elevating: false,
            pending_refresh: false,
            queued_actions: VecDeque::new(),
            probing: false,
            visible: !background_start() || (tray.is_none() && tray_startup.is_none()),
            hidden_ticks: 0,
            expanded: BTreeSet::new(),
            group_offsets: BTreeMap::new(),
            testing_groups: BTreeSet::new(),
            node_delays: BTreeMap::new(),
            proxy_order: RefCell::new(BTreeMap::new()),
            delay_generation: 0,
            delay_aborters: BTreeMap::new(),
            periodic_aborter: None,
            tray,
            tray_startup,
            tray_close_requested: false,
            site_queue: VecDeque::new(),
            site_results: vec![],
            site_category: CategoryFilter::All,
            site_status: StatusFilter::All,
            site_region: RegionFilter::All,
            site_detail: None,
            site_report_categories: false,
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
            if background_start() && app.visible {
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
        if size <= 10 { 13.0 } else { size as f32 * 1.25 }
    }

    fn label<'a>(&self, value: impl text::IntoFragment<'a>) -> iced::widget::Text<'a> {
        text(value)
            .font(typography::ENGLISH_FONT)
            .size(self.scaled(12))
    }

    fn accent(&self) -> Color {
        ui_style::tone(&self.theme(), ui_style::Tone::Accent)
    }
    fn secondary(&self) -> Color {
        ui_style::secondary(&self.theme())
    }
    fn latency_color(&self, delay: u32) -> Color {
        ui_style::tone(
            &self.theme(),
            match delay {
                1..=299 => ui_style::Tone::Success,
                300..=999 => ui_style::Tone::Warning,
                _ => ui_style::Tone::Danger,
            },
        )
    }
    fn caption<'a>(&self, value: impl text::IntoFragment<'a>) -> iced::widget::Text<'a> {
        self.label(value).size(13).color(self.secondary())
    }
    fn title<'a>(&self, value: impl text::IntoFragment<'a>) -> iced::widget::Text<'a> {
        self.label(value).size(17)
    }
    fn ready_to_retry(&self) -> bool {
        !self.snapshot.running
            && !self.busy
            && !self.working
            && !self.elevating
            && !self.exiting
            && !self.updates.busy()
    }

    fn selection<'a, T: Clone + PartialEq + std::fmt::Display + 'a>(
        &self,
        options: impl std::borrow::Borrow<[T]> + 'a,
        selected: T,
        on_select: impl Fn(T) -> Message + 'a,
        width: u32,
    ) -> Element<'a, Message> {
        let caption = self
            .label(selected.to_string())
            .size(13)
            .wrapping(text::Wrapping::None);
        let field = pick_list(options, Some(selected), on_select)
            .font(typography::ENGLISH_FONT)
            .text_shaping(text::Shaping::Advanced)
            .text_size(self.scaled(10))
            .padding([8, 9])
            .width(width)
            .style(selection_field);
        // tiny-skia's cached text damage bounds ignore vertical alignment.
        // A normal Text paragraph tracks the actual caption bounds, so changing
        // selection clears the complete old caption without repainting the window.
        // The native pick list still owns pointer/keyboard input and its menu.
        stack![
            field,
            container(caption)
                .width(Length::Fill)
                .height(Length::Fill)
                .clip(true)
                .align_y(iced::alignment::Vertical::Center)
                .padding(iced::Padding {
                    top: 8.0,
                    bottom: 8.0,
                    left: 9.0,
                    right: 9.0 + self.scaled(10),
                })
        ]
        .width(width)
        .into()
    }

    fn foreground(&self) -> Color {
        self.theme().palette().text
    }

    fn theme(&self) -> Theme {
        ui_style::theme(self.dark)
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
            iced::keyboard::on_key_press(|key, modifiers| {
                if modifiers.control() {
                    if let iced::keyboard::Key::Character(value) = key {
                        if value.eq_ignore_ascii_case("f") {
                            return Some(Message::FocusSearch);
                        }
                        if let Ok(index) = value.parse::<usize>() {
                            if (1..=Page::ALL.len()).contains(&index) {
                                return Some(Message::Navigate(Page::ALL[index - 1]));
                            }
                        }
                    }
                }
                None
            }),
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
                | Action::SavePorts(..)
                | Action::Activate(_)
                | Action::UpdateProfile(_)
                | Action::DeleteProfile(_)
                | Action::UpdateGeo
                | Action::Mode(_)
                | Action::ProxyMode(_)
                | Action::Select(_, _)
        ) {
            self.cancel_delay_checks();
            self.node_delays.clear();
            self.proxy_order.get_mut().clear();
            self.cancel_site_checks();
            self.site_results.clear();
            self.reset_site_filters();
        }
        self.busy = true;
        self.working = !matches!(action, Action::Refresh);
        Task::perform(execute(engine, action, scope), |reply| {
            Message::Finished(Box::new(reply))
        })
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FocusSearch => return iced::widget::operation::focus("page-search"),
            Message::DismissNotice => {
                self.notice_visible = false;
                self.updates.failure = None;
            }
            Message::RestartCore => {
                if !self.ready_to_retry() {
                    return Task::none();
                }
                self.notice_visible = false;
                if self.engine.is_some() {
                    return self.dispatch(Action::Start);
                }
                self.busy = true;
                self.working = true;
                return Task::perform(
                    async {
                        tokio::task::spawn_blocking(|| startup_store().and_then(Engine::new))
                            .await
                            .map_err(|error| format!("初始化任务失败：{error}"))?
                            .map(|engine| CoreHandle(Arc::new(Mutex::new(engine))))
                            .map_err(|error| format!("初始化失败：{error:#}"))
                    },
                    Message::CoreReady,
                );
            }
            Message::CoreReady(result) => {
                self.busy = false;
                self.working = false;
                if self.exiting {
                    return Task::none();
                }
                match result {
                    Ok(handle) => {
                        let engine = handle.0;
                        // Initialization completed outside the GUI thread. A newly
                        // initialized engine has no running process yet.
                        if let Ok(engine) = engine.try_lock() {
                            self.settings = engine.settings.clone();
                            self.profiles = engine.profiles.clone();
                            self.geo_status = engine.geo_manifest.version.clone();
                            self.controller_port = self.settings.controller_port.to_string();
                            self.mixed_port = self.settings.mixed_port.to_string();
                            self.interval_input = self.settings.delay_interval_minutes.to_string();
                            self.node_sort = self.settings.node_sort;
                        }
                        self.engine = Some(engine);
                        return self.dispatch(Action::Start);
                    }
                    Err(error) => {
                        self.core_failure = Some(error.clone());
                        self.notice = error;
                        self.error = true;
                        self.notice_visible = true;
                    }
                }
            }
            Message::CheckUpdate => return self.check_update(),
            Message::UpdateChecked(result) => self.updates.finish(result),
            Message::InstallUpdate => {
                if self.exiting || self.updates.busy() || self.updates.checking {
                    return Task::none();
                }
                let Some(available) = self.updates.available.clone() else {
                    return Task::none();
                };
                if let Err(error) = update::ensure_installable() {
                    self.updates.fail(error.to_string());
                    return Task::none();
                }
                let (id, registration) = self.updates.begin_download(
                    available
                        .package
                        .as_ref()
                        .map_or(1, |package| package.asset.size),
                );
                let port = self.snapshot.running.then_some(self.settings.mixed_port);
                return Task::run(
                    iced::stream::channel(8, async move |mut output| {
                        let (sender, mut progress) = tokio::sync::mpsc::channel(4);
                        let download = futures_util::future::Abortable::new(
                            update::download(available, port, move |value| {
                                let _ = sender.try_send(value);
                            }),
                            registration,
                        );
                        tokio::pin!(download);
                        let downloaded = loop {
                            tokio::select! {
                                result = &mut download => break result,
                                Some(value) = progress.recv() => { let _ = output.send(Message::UpdateProgress(id, value)).await; }
                            }
                        };
                        if let Ok(result) = downloaded {
                            let result = Arc::new(std::sync::Mutex::new(Some(
                                result.map_err(|error| error.to_string()),
                            )));
                            let _ = output.send(Message::UpdateDownloaded(id, result)).await;
                        }
                    }),
                    |message| message,
                );
            }
            Message::CancelUpdateDownload => {
                self.updates.cancel_download();
            }
            Message::UpdateProgress(id, progress) => {
                if self.updates.accepts_download(id) {
                    self.updates.progress = Some(progress);
                }
            }
            Message::UpdateDownloaded(id, result) => {
                let result = result.lock().expect("download result lock").take();
                // A canceled or superseded result is dropped here, including its
                // temporary directory. Installation starts only after this check.
                if !self.updates.finish_download(id) {
                    return Task::none();
                }
                match result {
                    Some(Ok(downloaded)) => {
                        self.updates.install_pending = true;
                        return Task::perform(
                            async move {
                                match tokio::task::spawn_blocking(move || {
                                    update::handoff(downloaded)
                                })
                                .await
                                {
                                    Ok(result) => result.map_err(|error| error.to_string()),
                                    Err(_) => Err("安装助手无法启动，请重试".into()),
                                }
                            },
                            Message::UpdateHandoff,
                        );
                    }
                    Some(Err(reason)) => self.updates.fail(reason),
                    None => return Task::none(),
                }
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
                    self.notice_visible = true;
                }
            }
            Message::Navigate(page) => {
                self.page = page;
                self.query.clear();
                self.list_offset = 0;
                self.snapshot.retain_scope(page.scope());
                self.proxy_order.get_mut().clear();
                self.site_detail = None;
                return self.dispatch(Action::Refresh);
            }
            Message::Query(value) => {
                self.query = value;
                self.group_offsets.clear();
                self.list_offset = 0;
                self.proxy_order.get_mut().clear();
                self.site_detail = None;
            }
            Message::NodeSort(mode) => {
                self.node_sort = mode;
                self.group_offsets.clear();
                self.proxy_order.get_mut().clear();
                return self.dispatch(Action::NodeSort(mode));
            }
            Message::SiteCategory(category) => {
                self.site_category = category;
                self.list_offset = 0;
                self.site_detail = None;
            }
            Message::SiteStatus(status) => {
                self.site_status = status;
                self.list_offset = 0;
                self.site_detail = None;
            }
            Message::SiteRegion(region) => {
                self.site_region = region;
                self.list_offset = 0;
                self.site_detail = None;
            }
            Message::SiteSummaryFilter(category, status) => {
                self.reset_site_filters();
                self.list_offset = 0;
                self.query.clear();
                self.site_category = category;
                self.site_status = status;
            }
            Message::SiteResetFilters => {
                self.reset_site_filters();
                self.list_offset = 0;
                self.query.clear();
            }
            Message::SiteDetail(index) => {
                self.site_detail = if self.site_detail == Some(index) {
                    None
                } else {
                    Some(index)
                };
            }
            Message::SiteReportCategories => {
                self.site_report_categories = !self.site_report_categories;
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
                    // Refresh native limits with the current display scale.
                    // Startup DPI detection can otherwise leave stale X11 hints.
                    return iced::window::set_min_size(id, Some(MIN_WINDOW_SIZE))
                        .chain(iced::window::resize(id, size));
                }
            }
            Message::ResizeWindow(revision, _, None) => {
                if revision == self.size_revision {
                    self.size_pending = false;
                }
            }
            Message::ReadWindowSize(Some(id)) => {
                return iced::window::size(id).map(Message::WindowSize);
            }
            Message::ReadWindowSize(None) => {}
            Message::WindowSize(size) => self.set_window_size(size),
            Message::Window((id, iced::window::Event::Opened { .. })) => {
                return iced::window::set_min_size(id, Some(MIN_WINDOW_SIZE))
                    .chain(iced::window::resize(id, DEFAULT_WINDOW_SIZE));
            }
            Message::Window((id, iced::window::Event::Rescaled(_))) => {
                return iced::window::set_min_size(id, Some(MIN_WINDOW_SIZE))
                    .chain(iced::window::size(id).map(Message::WindowSize));
            }
            Message::Window((id, iced::window::Event::Resized(size))) => {
                // A resize event can belong to an older native request. It must
                // not cancel a newer input edit or Restore Default command.
                self.set_window_size(size);
                // X11 can update WM_NORMAL_HINTS again while delivering the
                // final resize after a DPI change. Apply limits after that event.
                return iced::window::set_min_size(id, Some(MIN_WINDOW_SIZE));
            }
            Message::TestUrl(value) => self.test_url = value,
            Message::DnsHost(value) => self.dns_host = value,
            Message::Dark(value) => {
                self.dark = value;
                return self.dispatch(Action::Theme(value));
            }
            Message::Action(Action::ProxyMode(ProxyMode::Tun))
                if cfg!(windows) && !platform::is_elevated() =>
            {
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
                        self.notice_visible = true;
                        self.notice = error;
                    }
                }
            }
            Message::Action(Action::SavePorts(..)) => {
                let ports = self.controller_port.parse::<u16>().and_then(|controller| {
                    self.mixed_port
                        .parse::<u16>()
                        .map(|mixed| (controller, mixed))
                });
                match ports {
                    Ok((controller_port, mixed_port)) => {
                        return self.dispatch(Action::SavePorts(controller_port, mixed_port));
                    }
                    Err(_) => {
                        self.error = true;
                        self.notice_visible = true;
                        self.notice = "端口必须为 1–65535 的整数".into();
                    }
                }
            }
            Message::Action(action) => {
                self.proxy_order.get_mut().clear();
                if let Action::Delay(node) = &action {
                    self.node_delays.remove(node);
                }
                if matches!(action, Action::Activate(_) | Action::UpdateProfile(_)) {
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
                    || self.periodic_testing
                    || self.testing_groups.len() >= 2
                    || !self.testing_groups.insert(group.clone())
                {
                    return Task::none();
                }
                let engine = self.engine.clone().unwrap();
                let label = group.clone();
                let generation = self.delay_generation;
                let (handle, registration) = AbortHandle::new_pair();
                self.delay_aborters.insert(group.clone(), handle);
                return Task::perform(
                    async move {
                        Abortable::new(
                            async move {
                                let api = engine.lock().await.api.clone();
                                api.delay_group(&group).await.map_err(|e| e.to_string())
                            },
                            registration,
                        )
                        .await
                        .map_err(|_| "测速已取消".into())
                        .and_then(|result| result)
                    },
                    move |result| Message::GroupDelayDone(generation, label.clone(), result),
                );
            }
            Message::GroupDelayDone(generation, group, result) => {
                if generation != self.delay_generation
                    || self.delay_aborters.remove(&group).is_none()
                {
                    return Task::none();
                }
                self.testing_groups.remove(&group);
                match result {
                    Ok(delays) => {
                        let good = delays.values().filter(|d| **d > 0).count();
                        self.notice = format!(
                            "{group} 测速完成：{good}/{} 个节点可达，其余超时或失败",
                            delays.len()
                        );
                        self.error = false;
                        self.node_delays.extend(delays);
                        self.proxy_order.get_mut().clear();
                    }
                    Err(error) => {
                        self.notice = format!("{group} 测速失败：{error}");
                        self.error = true;
                        self.notice_visible = true;
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
                    self.notice_visible = true;
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
                let generation = self.delay_generation;
                let (handle, registration) = AbortHandle::new_pair();
                self.periodic_aborter = Some(handle);
                return Task::perform(
                    async move {
                        Abortable::new(
                            async move {
                                let api = engine.lock().await.api.clone();
                                api.delay_all_nodes()
                                    .await
                                    .map_err(|error| error.to_string())
                            },
                            registration,
                        )
                        .await
                        .map_err(|_| "测速已取消".into())
                        .and_then(|result| result)
                    },
                    move |result| Message::PeriodicDone(generation, result),
                );
            }
            Message::PeriodicDone(generation, result) => {
                if generation != self.delay_generation || self.periodic_aborter.take().is_none() {
                    return Task::none();
                }
                self.periodic_testing = false;
                if let Ok(delays) = result {
                    self.node_delays = delays;
                    self.proxy_order.get_mut().clear();
                }
                if self.page == Page::Proxies {
                    return self.dispatch(Action::Refresh);
                }
            }

            Message::SiteAll => {
                if !self.snapshot.running
                    || self.working
                    || self.elevating
                    || self.exiting
                    || !self.site_busy.is_empty()
                {
                    return Task::none();
                }
                self.site_results.clear();
                self.reset_site_filters();
                self.list_offset = 0;
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
                    || self.working
                    || self.elevating
                    || self.exiting
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
                if let Some(index) = ip_check::services()
                    .iter()
                    .position(|service| service.id == url)
                {
                    self.site_results
                        .resize_with(ip_check::services().len(), || None);
                    self.site_results[index] = Some(result);
                }
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
                if let Some(startup) = &self.tray_startup {
                    if let Some(result) = startup.poll() {
                        self.tray_startup = None;
                        match result {
                            Ok(tray) => {
                                tray.update(&self.snapshot);
                                self.tray = Some(tray);
                                self.tray_close_requested = false;
                            }
                            Err(error) => {
                                self.error = true;
                                self.notice_visible = true;
                                self.notice = format!("托盘不可用：{error}；关闭窗口将退出。");
                                if !self.visible {
                                    return iced::window::latest().map(Message::ShowWindow);
                                }
                            }
                        }
                    } else if !self.visible && startup.elapsed() >= Duration::from_secs(3) {
                        // Keep an app with no usable tray accessible while retrying.
                        return iced::window::latest().map(Message::ShowWindow);
                    }
                }
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
                if self.snapshot.running && !reply.running {
                    self.cancel_delay_checks();
                    self.node_delays.clear();
                    self.proxy_order.get_mut().clear();
                    self.expanded.clear();
                    self.group_offsets.clear();
                    self.cancel_site_checks();
                }
                if reply.running {
                    self.core_failure = None;
                } else if let Err(error) = &reply.notice {
                    self.core_failure = Some(error.clone());
                } else if self.snapshot.running {
                    self.core_failure = Some("内核已退出，请查看日志或重试启动。".into());
                }
                self.snapshot.running = reply.running;
                if let Some(mut snapshot) = reply.snapshot {
                    if reply.scope == Scope::Proxies && snapshot.running {
                        self.proxy_order.get_mut().clear();
                        let proxies = &snapshot.proxies.proxies;
                        self.expanded
                            .retain(|name| proxies.contains_key(name.as_str()));
                        self.group_offsets
                            .retain(|name, _| proxies.contains_key(name.as_str()));
                        self.node_delays
                            .retain(|name, _| proxies.contains_key(name.as_str()));
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
                        self.notice_visible = true;
                    }
                    Err(error) => {
                        self.notice = error;
                        self.error = true;
                        self.notice_visible = true;
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
                    self.notice_visible = true;
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
                if background_start() && self.tray_startup.is_some() && !self.tray_close_requested {
                    self.tray_close_requested = true;
                    self.notice = "桌面托盘正在初始化，请稍候；再次关闭窗口可退出程序。".into();
                    return Task::none();
                }
                return self.exit();
            }
            Message::ExitFinished(result) => match result {
                Ok(()) => return iced::exit(),
                Err(error) => {
                    self.exiting = false;
                    self.error = true;
                    self.notice_visible = true;
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
        self.cancel_delay_checks();
        self.cancel_site_checks();
        self.tray_startup = None;
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

    fn cancel_delay_checks(&mut self) {
        self.delay_generation = self.delay_generation.wrapping_add(1);
        for (_, handle) in std::mem::take(&mut self.delay_aborters) {
            handle.abort();
        }
        if let Some(handle) = self.periodic_aborter.take() {
            handle.abort();
        }
        self.testing_groups.clear();
        self.periodic_testing = false;
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
        if !self.size_pending
            && size.width.is_finite()
            && size.height.is_finite()
            && size.width > 0.0
            && size.height > 0.0
        {
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
        let style = if matches!(action, Action::DeleteProfile(_) | Action::Close(_)) {
            ui_style::danger_button
        } else if matches!(
            action,
            Action::Import(..) | Action::SavePorts(..) | Action::Activate(_)
        ) {
            rounded_primary
        } else {
            rounded_secondary
        };
        button(self.label(label).size(self.scaled(12)))
            .padding([7, 12])
            .style(style)
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
                    .wrapping(text::Wrapping::None)
                    .width(Length::Fill)
            ]
            .spacing(6),
            Space::new().height(if compact { 6 } else { 18 })
        ]
        .spacing(if compact { 2 } else { 5 });
        for (index, page) in Page::ALL.into_iter().enumerate() {
            nav = nav.push(
                button(
                    aligned_row![
                        container(Space::new().width(3).height(20)).style(move |theme: &Theme| {
                            container::Style {
                                background: (self.page == page)
                                    .then(|| ui_style::tone(theme, ui_style::Tone::Accent).into()),
                                ..Default::default()
                            }
                        }),
                        image(ui_style::nav_icon(index, self.dark))
                            .width(18)
                            .height(18),
                        self.label(page.name()).size(15)
                    ]
                    .spacing(9),
                )
                .width(Length::Fill)
                .padding([if compact { 6 } else { 9 }, 5])
                .style(if self.page == page {
                    ui_style::selected_button
                } else {
                    rounded_text
                })
                .on_press(Message::Navigate(page)),
            );
        }
        nav = nav
            .push(Space::new().height(Length::Fill))
            .push(self.caption(clash_of_rust::VERSION));
        let sidebar = container(nav)
            .width(168)
            .height(Length::Fill)
            .padding(12)
            .style(sidebar_panel);
        let status = if self.exiting {
            "正在退出…"
        } else if self.working || self.elevating {
            "正在处理…"
        } else if self.snapshot.running {
            "● 内核运行中"
        } else {
            "● 内核未启动"
        };
        let tone = if self.working || self.elevating {
            ui_style::Tone::Warning
        } else if self.snapshot.running {
            ui_style::Tone::Success
        } else {
            ui_style::Tone::Danger
        };
        let active = self
            .profiles
            .iter()
            .find(|p| Some(&p.id) == self.settings.active_profile.as_ref())
            .map(|p| p.name.as_str())
            .unwrap_or("选择配置");
        let header = aligned_row![
            column![
                self.label(self.page.name()).size(24),
                self.caption(self.page.description())
            ]
            .spacing(3),
            Space::new().width(Length::Fill),
            column![
                container(self.label(status).size(13))
                    .padding([5, 9])
                    .style(move |theme: &Theme| ui_style::badge(theme, tone)),
                button(self.caption(active))
                    .padding([3, 8])
                    .style(rounded_text)
                    .on_press(Message::Navigate(Page::Profiles))
            ]
            .spacing(3)
            .align_x(iced::alignment::Horizontal::Right)
        ]
        .spacing(8);
        let mut body = column![header].spacing(if compact { 8 } else { 12 });
        if !self.snapshot.running {
            let starting = self.working || self.elevating;
            let reason = if starting {
                "正在初始化配置并启动内核，请稍候…"
            } else {
                self.core_failure
                    .as_deref()
                    .unwrap_or("内核尚未运行，可以重试启动或查看日志。")
            };
            body = body.push(
                container(
                    aligned_row![
                        column![
                            self.title(if starting {
                                "内核正在启动"
                            } else {
                                "内核启动失败或已退出"
                            }),
                            scrollable(
                                self.label(reason)
                                    .size(13)
                                    .wrapping(text::Wrapping::WordOrGlyph)
                            )
                            .height(36)
                        ]
                        .spacing(4)
                        .width(Length::Fill),
                        button(self.label(if starting {
                            "启动中…"
                        } else {
                            "重启内核"
                        }))
                        .padding([8, 12])
                        .style(rounded_primary)
                        .on_press_maybe(self.ready_to_retry().then_some(Message::RestartCore)),
                        button(self.label("查看日志"))
                            .padding([8, 10])
                            .style(rounded_secondary)
                            .on_press(Message::Navigate(Page::Logs))
                    ]
                    .spacing(10),
                )
                .padding(if compact { 10 } else { 14 })
                .width(Length::Fill)
                .style(move |theme: &Theme| {
                    ui_style::badge(
                        theme,
                        if starting {
                            ui_style::Tone::Warning
                        } else {
                            ui_style::Tone::Danger
                        },
                    )
                }),
            );
        }
        let update_status = if self.updates.failure.is_some() {
            self.updates.failure.as_deref()
        } else if self.updates.install_pending || self.updates.installing.is_some() {
            Some("正在安装更新，请完成系统授权…")
        } else {
            None
        };
        if let Some(notice) = update_status.or_else(|| {
            (self.notice_visible && (self.snapshot.running || self.core_failure.is_none()))
                .then_some(self.notice.as_str())
        }) {
            let failed = self.error || self.updates.failure.is_some();
            body = body.push(
                container(
                    aligned_row![
                        self.label(notice).size(13).width(Length::Fill),
                        button(self.label("关闭").size(13))
                            .style(rounded_text)
                            .on_press(Message::DismissNotice)
                    ]
                    .spacing(8),
                )
                .padding(9)
                .width(Length::Fill)
                .style(move |theme: &Theme| {
                    ui_style::badge(
                        theme,
                        if failed {
                            ui_style::Tone::Danger
                        } else {
                            ui_style::Tone::Muted
                        },
                    )
                }),
            );
        }
        if let Some(update) = &self.updates.available {
            body = body.push(
                container(
                    aligned_row![
                        self.label(format!("发现新版本 {}", update.version))
                            .color(self.accent()),
                        Space::new().width(Length::Fill),
                        button(self.label("更新"))
                            .style(rounded_primary)
                            .padding([8, 12])
                            .on_press_maybe(
                                (!self.updates.busy() && !self.updates.checking && !self.exiting)
                                    .then_some(Message::InstallUpdate)
                            ),
                        if self.updates.progress.is_some() {
                            button(self.label("取消下载"))
                                .style(rounded_secondary)
                                .padding([8, 12])
                                .on_press(Message::CancelUpdateDownload)
                                .into()
                        } else {
                            Element::from(Space::new().width(0))
                        }
                    ]
                    .spacing(8),
                )
                .padding(12)
                .width(Length::Fill)
                .style(panel),
            );
        }
        if let Some(progress) = self.updates.progress {
            let percent =
                (progress.received as f64 / progress.total.max(1) as f64 * 100.0).min(100.0);
            body = body.push(
                column![
                    self.caption(format!(
                        "正在下载更新 · {percent:.0}%（{:.1} / {:.1} MiB）",
                        progress.received as f64 / 1048576.0,
                        progress.total as f64 / 1048576.0
                    )),
                    progress_bar(0.0..=100.0, percent as f32)
                ]
                .spacing(4),
            );
        }
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
        aligned_row![
            sidebar,
            container(body.push(content))
                .padding(if compact { 10 } else { 16 })
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
                        .color(self.secondary()),
                    Space::new().width(Length::Fill),
                    self.label(detail)
                        .size(self.scaled(10))
                        .color(self.secondary())
                ]
                .spacing(4),
                self.label(value).size(self.scaled(24)).color(self.accent()),
            ]
            .spacing(10),
        )
        .padding(16)
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
        .spacing(12);
        for (name, mode) in [("规则", "rule"), ("全局", "global"), ("直连", "direct")] {
            modes = modes.push(
                button(self.label(name).size(self.scaled(12)))
                    .padding([8, 14])
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
        let mut options = aligned_row![].spacing(12);
        for (label, mode) in [
            ("系统代理", ProxyMode::System),
            ("TUN模式", ProxyMode::Tun),
            ("关闭", ProxyMode::Off),
        ] {
            options = options.push(
                button(self.label(label).size(self.scaled(12)))
                    .padding([8, 14])
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
            .color(self.secondary())
        ]
        .spacing(10);
        scrollable(
            column![
                stats,
                container(modes)
                    .padding(16)
                    .width(Length::Fill)
                    .style(panel),
                container(ingress)
                    .padding(16)
                    .width(Length::Fill)
                    .style(panel),
                totals,
                footer
            ]
            .spacing(12),
        )
        .height(Length::Fill)
        .into()
    }

    fn search(&self, placeholder: &'static str) -> Element<'_, Message> {
        text_input(placeholder, &self.query)
            .id("page-search")
            .style(rounded_input)
            .size(self.scaled(12))
            .on_input(Message::Query)
            .padding(8)
            .into()
    }

    fn proxies(&self) -> Element<'_, Message> {
        let query = self.query.to_lowercase();
        let mut list = column![].spacing(8);
        let groups = self
            .snapshot
            .proxies
            .proxies
            .iter()
            .filter(|(group, proxy)| {
                !proxy.all.is_empty()
                    && (contains_query(group, &query)
                        || proxy.all.iter().any(|node| contains_query(node, &query)))
            });
        let group_total = groups.clone().count();
        let group_offset = self
            .list_offset
            .min(group_total.saturating_sub(1) / GROUP_PAGE_SIZE * GROUP_PAGE_SIZE);
        let visible_groups: Vec<_> = groups.skip(group_offset).take(GROUP_PAGE_SIZE).collect();
        let mut orders = self.proxy_order.borrow_mut();
        orders.retain(|name, _| {
            visible_groups.iter().any(|(group, _)| {
                *group == name && (self.expanded.contains(name.as_str()) || !query.is_empty())
            })
        });
        for (group, proxy) in visible_groups {
            let group_match = contains_query(group, &query);
            let expanded = self.expanded.contains(group.as_str()) || !query.is_empty();
            let mut contents = column![
                aligned_row![
                    button(
                        aligned_row![
                            self.label(if expanded { "▼" } else { "▶" })
                                .size(self.scaled(12)),
                            column![
                                self.label(group.as_str()).size(self.scaled(12)),
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
                    .on_press(Message::ToggleGroup(group.to_string())),
                    button(
                        self.label(if self.testing_groups.contains(group.as_str()) {
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
                            && !self.testing_groups.contains(group.as_str()))
                        .then(|| Message::GroupDelay(group.to_string()))
                    )
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center)
            ]
            .spacing(4);
            if expanded {
                let indices = orders.entry(group.clone()).or_insert_with(|| {
                    proxy_order::sorted_indices(
                        &proxy.all,
                        self.node_sort,
                        |node| self.node_delay(node),
                        |node| group_match || contains_query(node, &query),
                    )
                });
                let total = indices.len();
                let offset = self
                    .group_offsets
                    .get(group.as_str())
                    .copied()
                    .unwrap_or(0)
                    .min(last_page_offset(total));
                let end = (offset + PAGE_SIZE).min(total);
                for pair in indices[offset..end].chunks(2) {
                    let mut row = aligned_row![].spacing(10);
                    for &index in pair {
                        row = row.push(self.node_card(
                            group.as_str(),
                            proxy,
                            proxy.all[index].as_str(),
                        ));
                    }
                    if pair.len() == 1 {
                        row = row.push(Space::new().width(Length::FillPortion(1)));
                    }
                    contents = contents.push(row);
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
                                (offset > 0).then(|| Message::GroupPage(group.to_string(), false))
                            ),
                            button(self.label("下一页")).on_press_maybe(
                                (offset + PAGE_SIZE < total)
                                    .then(|| Message::GroupPage(group.to_string(), true))
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
            aligned_row![
                self.search("搜索策略组或节点 · 搜索时自动展开"),
                self.selection(NodeSort::ALL, self.node_sort, Message::NodeSort, 185)
            ]
            .spacing(8),
            scrollable(list).height(Length::Fill),
            self.pager(group_total, group_offset, GROUP_PAGE_SIZE)
        ]
        .spacing(8)
        .into()
    }

    fn node_card<'a>(
        &'a self,
        group: &'a str,
        proxy: &'a clash_of_rust::api::Proxy,
        node: &'a str,
    ) -> Element<'a, Message> {
        let detail = self.snapshot.proxies.proxies.get(node);
        let delay = self.node_delay(node);
        let selected = proxy.now.as_str() == node;
        let selectable = self.snapshot.running
            && !self.working
            && !self.elevating
            && !self.exiting
            && proxy.kind == "Selector"
            && !selected;
        let label = delay
            .map(|delay| {
                if delay == 0 {
                    "超时".into()
                } else {
                    format!("{delay} ms")
                }
            })
            .unwrap_or_else(|| "未测速".into());
        container(
            column![
                button(
                    column![
                        aligned_row![
                            self.label(node).size(15).width(Length::Fill),
                            self.label(label).size(13).color(
                                delay
                                    .map(|d| self.latency_color(d))
                                    .unwrap_or(self.secondary())
                            )
                        ]
                        .spacing(6),
                        aligned_row![
                            self.caption(detail.map(|p| p.kind.as_str()).unwrap_or("节点")),
                            Space::new().width(Length::Fill),
                            self.label(if selected {
                                "✓ 已选"
                            } else {
                                "选择节点"
                            })
                            .size(13)
                            .color(if selected {
                                self.accent()
                            } else {
                                self.secondary()
                            })
                        ]
                        .spacing(6)
                    ]
                    .spacing(5)
                )
                .padding(4)
                .width(Length::Fill)
                .style(rounded_text)
                .on_press_maybe(
                    selectable.then(|| Message::Action(Action::Select(group.into(), node.into())))
                ),
                aligned_row![
                    Space::new().width(Length::Fill),
                    self.action("测速", Action::Delay(node.into()), self.snapshot.running)
                ]
            ]
            .spacing(3),
        )
        .padding(8)
        .width(Length::FillPortion(1))
        .style(if selected {
            ui_style::selected_panel
        } else {
            panel
        })
        .into()
    }

    fn node_delay(&self, name: &str) -> Option<u32> {
        self.node_delays.get(name).copied().or_else(|| {
            self.snapshot
                .proxies
                .proxies
                .get(name)
                .and_then(|proxy| proxy.delay)
        })
    }

    fn reset_site_filters(&mut self) {
        self.site_category = CategoryFilter::All;
        self.site_status = StatusFilter::All;
        self.site_region = RegionFilter::All;
        self.site_detail = None;
    }

    fn site_result(&self, index: usize) -> Option<&Result<ip_check::CheckResult, String>> {
        self.site_results.get(index).and_then(Option::as_ref)
    }

    fn site_matches(&self, index: usize, query: &str) -> bool {
        let service = &ip_check::services()[index];
        let result = self.site_result(index);
        if let CategoryFilter::Group(group) = self.site_category
            && service.group != group
        {
            return false;
        }
        if let StatusFilter::Status(status) = self.site_status
            && Status::of(result) != status
        {
            return false;
        }
        match self.site_region {
            RegionFilter::All => {}
            RegionFilter::Unidentified if ip_report::region(result).is_some() => return false,
            RegionFilter::Country(code) if ip_report::region(result) != Some(code) => return false,
            _ => {}
        }
        contains_query(&service.name, query)
            || contains_query(&service.group, query)
            || match result {
                Some(Ok(result)) => {
                    contains_query(&result.summary, query) || contains_query(&result.country, query)
                }
                Some(Err(error)) => {
                    contains_query("超时或失败", query) || contains_query(error, query)
                }
                None => contains_query("未检测 等待检测 检测中", query),
            }
    }

    fn site_status_color(&self, status: Status) -> Color {
        match status {
            Status::Available | Status::Identified => self.latency_color(1),
            Status::Restricted | Status::Failed => self.latency_color(0),
            Status::Partial | Status::Reachable | Status::Unknown => self.latency_color(500),
            Status::Untested => self.secondary(),
        }
    }

    fn site_report(&self, report: &ip_report::Report, region_count: usize) -> Element<'_, Message> {
        let completed = report.totals.completed();
        let compact = self.window_height.parse::<u16>().unwrap_or(700) < 560;
        let show_categories = !compact || self.site_report_categories;
        let mut statuses = aligned_row![].spacing(6);
        for status in Status::ALL {
            let count = report.totals.count(status);
            if count == 0 {
                continue;
            }
            let ink = self.site_status_color(status);
            statuses = statuses.push(
                button(self.label(format!("{status} {count}")).size(13).color(ink))
                    .padding([5, 8])
                    .style(if self.site_status == StatusFilter::Status(status) {
                        ui_style::selected_button
                    } else {
                        rounded_secondary
                    })
                    .on_press(Message::SiteSummaryFilter(
                        CategoryFilter::All,
                        StatusFilter::Status(status),
                    )),
            );
        }
        let mut categories = column![].spacing(8);
        if show_categories {
            for pair in report.categories.chunks(2) {
                let mut row = iced::widget::row![].spacing(8);
                for category in pair {
                    let mut states = aligned_row![].spacing(3);
                    for status in Status::ALL {
                        let count = category.counts.count(status);
                        if count == 0 {
                            continue;
                        }
                        states = states.push(
                            button(
                                self.label(format!("{status} {count}"))
                                    .size(13)
                                    .color(self.site_status_color(status)),
                            )
                            .padding([3, 4])
                            .style(rounded_text)
                            .on_press(Message::SiteSummaryFilter(
                                CategoryFilter::Group(category.name),
                                StatusFilter::Status(status),
                            )),
                        );
                    }
                    let selected = self.site_category == CategoryFilter::Group(category.name);
                    row = row.push(
                        container(
                            column![
                                button(
                                    aligned_row![
                                        self.label(category.name).size(15),
                                        Space::new().width(Length::Fill),
                                        self.label(format!(
                                            "{}/{}",
                                            category.counts.non_red(),
                                            category.counts.total()
                                        ))
                                        .size(14)
                                    ]
                                    .spacing(6)
                                )
                                .width(Length::Fill)
                                .padding(4)
                                .style(rounded_text)
                                .on_press(
                                    Message::SiteSummaryFilter(
                                        CategoryFilter::Group(category.name),
                                        StatusFilter::All
                                    )
                                ),
                                states.wrap()
                            ]
                            .spacing(3),
                        )
                        .padding(6)
                        .width(Length::FillPortion(1))
                        .style(if selected {
                            ui_style::selected_panel
                        } else {
                            panel
                        }),
                    );
                }
                if pair.len() == 1 {
                    row = row.push(Space::new().width(Length::FillPortion(1)));
                }
                categories = categories.push(row);
            }
        }
        let toggle: Element<'_, Message> = if compact {
            button(
                self.label(if show_categories {
                    "收起分类"
                } else {
                    "分类总结"
                })
                .size(13),
            )
            .padding([4, 6])
            .style(rounded_secondary)
            .on_press(Message::SiteReportCategories)
            .into()
        } else {
            Space::new().width(0).into()
        };
        let mut body = column![
            aligned_row![
                self.title(if completed == report.totals.total() {
                    "检测总结"
                } else {
                    "检测总结 · 部分完成"
                }),
                Space::new().width(Length::Fill),
                self.caption(format!("完成 {completed} / {}", report.totals.total())),
                toggle
            ]
            .spacing(8),
            statuses.wrap()
        ]
        .spacing(8);
        if !compact {
            let exit = self.site_result(0).and_then(|result| result.as_ref().ok());
            body = body.push(self.caption(format!(
                "出口 IP：{} · {} · 识别到 {region_count} 个地区",
                exit.map_or("未获取", |r| r.summary.as_str()),
                exit.map_or("未获取", |r| flags::country_text(&r.country))
            )));
        }
        if show_categories {
            body = body.push(scrollable(categories).height(if compact { 65 } else { 100 }));
        }
        container(body)
            .padding(if compact { 10 } else { 12 })
            .width(Length::Fill)
            .style(panel)
            .into()
    }

    fn websites(&self) -> Element<'_, Message> {
        let query = self.query.to_lowercase();
        let services = ip_check::services();
        let report = ip_report::Report::from_results(&self.site_results);
        let codes: BTreeSet<_> = self
            .site_results
            .iter()
            .filter_map(|result| ip_report::region(result.as_ref()))
            .collect();
        let matching = (0..services.len()).filter(|&index| self.site_matches(index, &query));
        let visible = matching.clone().count();
        let offset = self.list_offset.min(last_page_offset(visible));
        let mut list = column![].spacing(4);
        for index in matching.skip(offset).take(PAGE_SIZE) {
            let service = &services[index];
            let result = self.site_result(index);
            let busy = self.site_busy.contains(&service.id);
            let queued = self.site_queue.contains(&service.id);
            let result_color = self.site_status_color(Status::of(result));
            let (summary, country, delay, color) = match result {
                Some(Ok(result)) => (
                    result.summary.as_str(),
                    result.country.as_str(),
                    format!("{} ms", result.millis),
                    result_color,
                ),
                Some(Err(_)) => ("超时或失败", "未获取", "—".into(), self.latency_color(0)),
                None => ("未检测", "—", "—".into(), self.foreground()),
            };
            let state = if busy {
                "检测中…"
            } else if queued {
                "等待检测"
            } else {
                summary
            };
            let (country_text, flag) = flags::label(country);
            let mut country_row = aligned_row![].spacing(4);
            if let Some(flag) = flag {
                country_row = country_row.push(image(flag).width(20).height(20));
            }
            country_row = country_row.push(self.label(country_text).size(self.scaled(10)));
            let mut entry = column![
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
                                self.latency_color(result.millis.min(u128::from(u32::MAX)) as u32),
                            Some(Err(_)) if !busy && !queued => self.latency_color(0),
                            _ => self.foreground(),
                        })
                        .width(70),
                    button(
                        self.label(if self.site_detail == Some(index) {
                            "收起"
                        } else {
                            "详情"
                        })
                        .size(self.scaled(10))
                    )
                    .padding([3, 5])
                    .style(rounded_text)
                    .on_press_maybe(result.is_some().then_some(Message::SiteDetail(index)))
                ]
                .spacing(6)
            ]
            .spacing(6);
            if self.site_detail == Some(index) {
                let detail = match result {
                    Some(Ok(result)) => Some(result.detail.as_str()),
                    Some(Err(error)) => Some(error.as_str()),
                    None => None,
                };
                if let Some(detail) = detail {
                    entry =
                        entry.push(self.label(detail).size(self.scaled(10)).width(Length::Fill));
                }
            }
            list = list.push(container(entry).padding(8).style(panel));
        }
        if visible == 0 {
            list = list.push(self.label("没有匹配的检测项目。"));
        }
        let mut page = column![
            aligned_row![
                self.search("搜索平台、地区或检测结果"),
                self.label(format!(
                    "已完成 {} / {}",
                    report.totals.completed(),
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
            .spacing(8)
        ]
        .spacing(8);
        if report.totals.completed() > 0 && self.site_busy.is_empty() && self.site_queue.is_empty()
        {
            page = page.push(self.site_report(&report, codes.len()));
        }
        let categories: Vec<_> = std::iter::once(CategoryFilter::All)
            .chain(
                report
                    .categories
                    .iter()
                    .map(|category| CategoryFilter::Group(category.name)),
            )
            .collect();
        let statuses: Vec<_> = std::iter::once(StatusFilter::All)
            .chain(Status::ALL.into_iter().map(StatusFilter::Status))
            .collect();
        let regions: Vec<_> = [RegionFilter::All, RegionFilter::Unidentified]
            .into_iter()
            .chain(codes.into_iter().map(RegionFilter::Country))
            .collect();
        page = page.push(
            aligned_row![
                self.selection(categories, self.site_category, Message::SiteCategory, 110),
                self.selection(statuses, self.site_status, Message::SiteStatus, 150),
                self.selection(regions, self.site_region, Message::SiteRegion, 150),
                button(self.label("清除筛选").size(self.scaled(10)))
                    .padding([5, 8])
                    .style(rounded_secondary)
                    .on_press(Message::SiteResetFilters),
                Space::new().width(Length::Fill),
                self.label(format!("匹配 {visible} 项"))
                    .size(self.scaled(10))
            ]
            .spacing(6)
            .wrap(),
        );
        page.push(
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
                self.label("延迟").size(self.scaled(10)).width(70),
                Space::new().width(43)
            ]
            .spacing(6)
            .padding([0, 8]),
        )
        .push(scrollable(list).height(Length::Fill))
        .push(self.pager(visible, offset, PAGE_SIZE))
        .into()
    }

    fn profiles_view(&self) -> Element<'_, Message> {
        let form = container(
            column![
                self.title("导入订阅"),
                self.caption("配置名称"),
                text_input("例如：我的订阅", &self.profile_name)
                    .style(rounded_input)
                    .size(15)
                    .on_input(Message::ProfileName)
                    .padding(9),
                self.caption("订阅链接或本地 YAML 路径"),
                text_input("https://订阅地址 或 本地文件完整路径", &self.profile_source)
                    .style(rounded_input)
                    .size(15)
                    .on_input(Message::ProfileSource)
                    .padding(9),
                aligned_row![
                    self.action(
                        "导入",
                        Action::Import(
                            self.profile_name.clone(),
                            self.profile_source.trim().into()
                        ),
                        !self.profile_source.trim().is_empty()
                    ),
                    self.action("打开配置目录", Action::OpenProfiles, true)
                ]
                .spacing(10)
            ]
            .spacing(8),
        )
        .padding(16)
        .width(Length::Fill)
        .style(panel);
        let mut list = column![].spacing(10);
        for profile in &self.profiles {
            let active = self.settings.active_profile.as_ref() == Some(&profile.id);
            let remote =
                profile.source.starts_with("http://") || profile.source.starts_with("https://");
            let update: Element<'_, Message> = if remote {
                self.action("更新", Action::UpdateProfile(profile.id.clone()), true)
            } else {
                Space::new().width(0).into()
            };
            // Show the host, not subscription credentials or private URL tokens.
            let source = if remote {
                reqwest::Url::parse(&profile.source)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .unwrap_or_else(|| "远程订阅".into())
            } else {
                "本地配置".into()
            };
            list = list.push(
                container(
                    column![
                        aligned_row![
                            self.title(&profile.name).width(Length::Fill),
                            if active {
                                container(self.label("● 使用中").size(13))
                                    .padding([4, 8])
                                    .style(|theme: &Theme| {
                                        ui_style::badge(theme, ui_style::Tone::Success)
                                    })
                                    .into()
                            } else {
                                Element::from(Space::new().width(0))
                            },
                            self.action(
                                if active { "已启用" } else { "启用" },
                                Action::Activate(profile.id.clone()),
                                !active
                            ),
                            update,
                            if profile.is_default() {
                                Space::new().width(0).into()
                            } else {
                                self.action("删除", Action::DeleteProfile(profile.id.clone()), true)
                            }
                        ]
                        .spacing(8),
                        self.caption(format!(
                            "来源：{source} · 更新于 {}",
                            presentation::timestamp(profile.updated)
                        ))
                        .wrapping(text::Wrapping::WordOrGlyph),
                        self.label(presentation::subscription_usage(profile.usage.as_deref()))
                            .size(14)
                    ]
                    .spacing(10),
                )
                .padding(16)
                .width(Length::Fill)
                .style(if active {
                    ui_style::selected_panel
                } else {
                    panel
                }),
            );
        }
        if self.profiles.is_empty() {
            list = list.push(self.empty_state(
                "还没有订阅配置",
                "导入订阅链接或本地 YAML 文件后，即可启用配置。",
            ));
        }
        scrollable(column![form, self.title("当前订阅与配置"), list].spacing(14))
            .height(Length::Fill)
            .into()
    }

    fn connections(&self) -> Element<'_, Message> {
        let query = self.query.to_lowercase();
        let entries = self.snapshot.connections.connections.iter().filter(|c| {
            query.is_empty()
                || query.split_whitespace().all(|word| {
                    [
                        c.metadata.host.as_ref(),
                        c.metadata.destination_ip.as_ref(),
                        c.metadata.process.as_str(),
                        c.rule.as_str(),
                    ]
                    .into_iter()
                    .chain(c.chains.iter().map(|name| name.as_str()))
                    .any(|field| contains_query(field, word))
                })
        });
        let total = entries.clone().count();
        let offset = self.list_offset.min(last_page_offset(total));
        let mut list = column![].spacing(0);
        for (index, connection) in entries.skip(offset).take(PAGE_SIZE).enumerate() {
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
                            self.label(format!("{}:{}", host, m.destination_port))
                                .size(14)
                                .wrapping(text::Wrapping::WordOrGlyph),
                            self.caption(format!(
                                "{} · {}",
                                m.network,
                                if m.process.is_empty() {
                                    "未知进程"
                                } else {
                                    &m.process
                                }
                            ))
                        ]
                        .spacing(4)
                        .width(Length::FillPortion(2)),
                        column![
                            self.label(connection.rule.as_str()).size(14),
                            self.caption(connection.chains.join(" → "))
                                .wrapping(text::Wrapping::WordOrGlyph)
                        ]
                        .spacing(4)
                        .width(Length::FillPortion(2)),
                        column![
                            self.label(format!("↑ {}", bytes(connection.upload)))
                                .size(13),
                            self.label(format!("↓ {}", bytes(connection.download)))
                                .size(13)
                        ]
                        .spacing(4)
                        .width(104),
                        self.caption(presentation::date_text(&connection.start))
                            .width(94),
                        self.action(
                            "关闭",
                            Action::Close(connection.id.to_string()),
                            self.snapshot.running
                        )
                    ]
                    .spacing(10),
                )
                .padding(10)
                .width(Length::Fill)
                .style(move |theme: &Theme| ui_style::table_row(theme, index % 2 == 1)),
            );
        }
        if total == 0 {
            list = list
                .push(self.empty_state("没有匹配的连接", "尝试更改搜索条件，或等待新的网络请求。"));
        }
        column![
            aligned_row![
                self.search("搜索域名、IP、进程或规则"),
                self.caption(format!("匹配 {total} 条"))
            ]
            .spacing(10),
            self.table_header(
                aligned_row![
                    self.label("目标 / 进程")
                        .size(13)
                        .width(Length::FillPortion(2)),
                    self.label("规则 / 代理链")
                        .size(13)
                        .width(Length::FillPortion(2)),
                    self.label("上传 / 下载").size(13).width(104),
                    self.label("开始时间").size(13).width(94),
                    self.label("操作").size(13).width(54)
                ]
                .spacing(10)
                .into()
            ),
            container(scrollable(list).height(Length::Fill))
                .width(Length::Fill)
                .style(panel),
            self.list_pager(total, offset)
        ]
        .spacing(10)
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
                        [r.kind.as_str(), r.payload.as_ref(), r.proxy.as_str()]
                            .into_iter()
                            .any(|field| contains_query(field, word))
                    })
            });
        let total = entries.clone().count();
        let offset = self.list_offset.min(last_page_offset(total));
        let mut list = column![].spacing(0);
        for (row, (index, rule)) in entries.skip(offset).take(PAGE_SIZE).enumerate() {
            list = list.push(
                container(
                    aligned_row![
                        self.caption(format!("{:04}", index + 1)).width(44),
                        self.label(rule.kind.as_str()).size(13).width(145),
                        self.label(rule.payload.as_ref())
                            .size(14)
                            .wrapping(text::Wrapping::WordOrGlyph)
                            .width(Length::Fill),
                        self.label(rule.proxy.as_str())
                            .size(14)
                            .color(self.accent())
                            .width(120)
                    ]
                    .spacing(10),
                )
                .padding([12, 10])
                .width(Length::Fill)
                .style(move |theme: &Theme| ui_style::table_row(theme, row % 2 == 1)),
            );
        }
        if total == 0 {
            list = list.push(self.empty_state(
                "没有匹配的规则",
                "尝试更改搜索条件，或在内核启动后加载当前配置。",
            ));
        }
        column![
            aligned_row![
                self.search("搜索规则类型、内容或策略"),
                self.caption(format!("匹配 {total} 条"))
            ]
            .spacing(10),
            self.table_header(
                aligned_row![
                    self.label("序号").size(13).width(44),
                    self.label("类型").size(13).width(145),
                    self.label("规则内容").size(13).width(Length::Fill),
                    self.label("策略").size(13).width(120)
                ]
                .spacing(10)
                .into()
            ),
            container(scrollable(list).height(Length::Fill))
                .width(Length::Fill)
                .style(panel),
            self.list_pager(total, offset)
        ]
        .spacing(10)
        .into()
    }

    fn table_header<'a>(&self, content: Element<'a, Message>) -> Element<'a, Message> {
        container(content)
            .padding(10)
            .width(Length::Fill)
            .style(|theme: &Theme| ui_style::table_row(theme, true))
            .into()
    }
    fn empty_state<'a>(&self, title: &'a str, detail: &'a str) -> Element<'a, Message> {
        container(column![self.title(title), self.caption(detail)].spacing(8))
            .padding(24)
            .width(Length::Fill)
            .style(panel)
            .into()
    }

    fn list_pager(&self, total: usize, offset: usize) -> Element<'_, Message> {
        self.pager(total, offset, PAGE_SIZE)
    }

    fn pager(&self, total: usize, offset: usize, page_size: usize) -> Element<'_, Message> {
        aligned_row![
            self.label(if total == 0 {
                "0 条".into()
            } else {
                format!(
                    "{} 条 · {}–{}",
                    total,
                    offset + 1,
                    (offset + page_size).min(total)
                )
            })
            .size(self.scaled(10)),
            Space::new().width(Length::Fill),
            button(self.label("上一页"))
                .padding([8, 12])
                .style(rounded_secondary)
                .on_press_maybe(
                    (offset > 0).then_some(Message::ListPage(offset.saturating_sub(page_size)))
                ),
            button(self.label("下一页"))
                .padding([8, 12])
                .style(rounded_secondary)
                .on_press_maybe(
                    (offset + page_size < total).then_some(Message::ListPage(offset + page_size))
                )
        ]
        .spacing(8)
        .into()
    }

    fn logs(&self) -> Element<'_, Message> {
        let query = self.query.to_lowercase();
        let mut list = column![].spacing(0);
        let mut visible = 0;
        for (index, line) in self
            .snapshot
            .logs
            .iter()
            .rev()
            .filter(|line| contains_query(line, &query))
            .take(150)
            .enumerate()
        {
            visible += 1;
            let (level, tone) = if contains_query(line, "error") || line.contains("失败") {
                ("ERROR", ui_style::Tone::Danger)
            } else if contains_query(line, "warn") {
                ("WARN", ui_style::Tone::Warning)
            } else {
                ("INFO", ui_style::Tone::Success)
            };
            list = list.push(
                container(
                    aligned_row![
                        container(self.label(level).size(12))
                            .padding([4, 6])
                            .width(68)
                            .style(move |theme: &Theme| ui_style::badge(theme, tone)),
                        self.label(line.as_ref())
                            .size(13)
                            .wrapping(text::Wrapping::WordOrGlyph)
                            .width(Length::Fill)
                    ]
                    .spacing(10),
                )
                .padding(10)
                .width(Length::Fill)
                .style(move |theme: &Theme| ui_style::table_row(theme, index % 2 == 1)),
            );
        }
        if visible == 0 {
            list = list.push(self.empty_state(
                "暂无匹配的日志",
                "新记录会显示在上方，可以搜索错误或导出完整记录。",
            ));
        }
        column![
            aligned_row![
                self.search("搜索日志 / error / warning"),
                self.action("清空", Action::ClearLogs, true),
                self.action("导出", Action::ExportLogs, true)
            ]
            .spacing(10),
            self.caption("最新记录在上方 · 最多保留 500 条，显示最近匹配的 150 条"),
            self.table_header(
                aligned_row![
                    self.label("级别").size(13).width(68),
                    self.label("日志内容").size(13)
                ]
                .spacing(10)
                .into()
            ),
            container(scrollable(list).height(Length::Fill))
                .width(Length::Fill)
                .style(panel)
        ]
        .spacing(10)
        .into()
    }

    fn tests(&self) -> Element<'_, Message> {
        let available = !self.probing && self.snapshot.running && !self.working && !self.exiting;
        let network = container(
            column![
                self.title("网站与出口 IP"),
                self.caption("测试地址"),
                text_input("https://example.com", &self.test_url)
                    .style(rounded_input)
                    .size(15)
                    .on_input(Message::TestUrl)
                    .padding(9),
                aligned_row![
                    button(self.label("测试网站响应"))
                        .style(rounded_primary)
                        .padding([8, 12])
                        .on_press_maybe(available.then_some(Message::Probe(false))),
                    button(self.label("查询出口 IP"))
                        .style(rounded_secondary)
                        .padding([8, 12])
                        .on_press_maybe(available.then_some(Message::Probe(true)))
                ]
                .spacing(10)
            ]
            .spacing(10),
        )
        .padding(16)
        .width(Length::Fill)
        .style(panel);
        let dns = container(
            column![
                self.title("系统 DNS 查询"),
                self.caption("域名"),
                aligned_row![
                    text_input("github.com", &self.dns_host)
                        .style(rounded_input)
                        .size(15)
                        .on_input(Message::DnsHost)
                        .padding(9),
                    button(self.label("解析域名"))
                        .style(rounded_primary)
                        .padding([8, 12])
                        .on_press_maybe((!self.probing && !self.exiting).then_some(Message::Dns))
                ]
                .spacing(10)
            ]
            .spacing(10),
        )
        .padding(16)
        .width(Length::Fill)
        .style(panel);
        let mut results = column![self.title(if self.probing {
            "正在测试…"
        } else {
            "测试记录"
        })]
        .spacing(10);
        for result in self.test_results.iter().rev() {
            let failed = result.starts_with("测试失败：") || result.starts_with("DNS 查询失败：");
            results = results.push(
                container(
                    aligned_row![
                        container(self.label(if failed { "失败" } else { "完成" }).size(13))
                            .padding([4, 8])
                            .style(move |theme: &Theme| ui_style::badge(
                                theme,
                                if failed {
                                    ui_style::Tone::Danger
                                } else {
                                    ui_style::Tone::Success
                                }
                            )),
                        self.label(result)
                            .size(14)
                            .wrapping(text::Wrapping::WordOrGlyph)
                            .width(Length::Fill)
                    ]
                    .spacing(10),
                )
                .padding(12)
                .width(Length::Fill)
                .style(panel),
            );
        }
        if self.test_results.is_empty() {
            results = results.push(self.caption(
                "执行测试后，结果会显示在这里；网站和出口查询使用当前代理，DNS 使用系统解析。",
            ));
        }
        scrollable(column![network, dns, results].spacing(14))
            .height(Length::Fill)
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
                    .padding([8, 12])
                    .on_press_maybe(
                        (!self.updates.checking && !self.updates.busy() && !self.exiting)
                            .then_some(Message::CheckUpdate)
                    ),
                button(self.label("查看 Release"))
                    .style(rounded_secondary)
                    .padding([8, 12])
                    .on_press(Message::OpenRelease)
            ]
            .spacing(12)
        ]
        .spacing(12);
        let mut ports = column![
            self.title("网络端口"),
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
                Action::SavePorts(0, 0),
                self.testing_groups.is_empty(),
            ));
        }
        let appearance = column![
            self.title("外观"),
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
            .spacing(12),
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
            .spacing(12),
        ]
        .spacing(12);
        let geo = column![
            self.title("Geo 数据"),
            self.label(presentation::geo_version(&self.geo_status))
                .size(self.scaled(11))
                .color(self.foreground()),
            self.action("更新所有 Geo 数据", Action::UpdateGeo, true)
        ]
        .spacing(12);
        scrollable(
            column![
                container(updates)
                    .padding(16)
                    .width(Length::Fill)
                    .style(panel),
                container(ports)
                    .padding(16)
                    .width(Length::Fill)
                    .style(panel),
                container(appearance)
                    .padding(16)
                    .width(Length::Fill)
                    .style(panel),
                container(
                    aligned_row![
                        column![self.title("启动"), self.caption("开机自启 · 后台静默启动")]
                            .spacing(6),
                        Space::new().width(Length::Fill),
                        self.action(
                            if self.autostart { "关闭" } else { "开启" },
                            Action::Autostart(!self.autostart),
                            true
                        )
                    ]
                    .spacing(12)
                )
                .padding(16)
                .width(Length::Fill)
                .style(panel),
                container(
                    column![
                        self.title("定时测速"),
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
                                .padding([8, 12])
                                .on_press_maybe(
                                    (self.interval_input.trim().parse::<u32>().ok()
                                        != Some(self.settings.delay_interval_minutes)
                                        && !self.working)
                                        .then_some(Message::SaveInterval)
                                )
                        ]
                        .spacing(12)
                        .align_y(iced::alignment::Vertical::Center),
                    ]
                    .spacing(12)
                )
                .padding(16)
                .width(Length::Fill)
                .style(panel),
                container(geo).padding(16).width(Length::Fill).style(panel)
            ]
            .spacing(12),
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

fn panel(theme: &Theme) -> container::Style {
    ui_style::panel(theme)
}
fn selection_field(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let mut style = pick_list::default(theme, status);
    style.text_color = Color::TRANSPARENT;
    style.placeholder_color = Color::TRANSPARENT;
    style.background = ui_style::surface(theme).into();
    style.border = iced::Border {
        color: iced::color!(0x64748B),
        width: 1.0,
        radius: 6.0.into(),
    };
    style.handle_color = ui_style::secondary(theme);
    style
}
fn rounded_primary(theme: &Theme, status: button::Status) -> button::Style {
    ui_style::primary(theme, status)
}
fn rounded_secondary(theme: &Theme, status: button::Status) -> button::Style {
    ui_style::secondary_button(theme, status)
}
fn rounded_text(theme: &Theme, status: button::Status) -> button::Style {
    ui_style::text_button(theme, status)
}
fn rounded_input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    ui_style::input(theme, status)
}
fn sidebar_panel(theme: &Theme) -> container::Style {
    ui_style::sidebar(theme)
}

#[cfg(test)]
mod window_tests {
    use super::*;

    fn app() -> App {
        // Exercise real message handling without loading a store, starting the
        // core, or executing asynchronous update/network tasks.
        App::with_engine(Err(anyhow::anyhow!("geometry test")), None).0
    }

    #[test]
    fn old_native_resize_does_not_cancel_restore_default() {
        let mut app = app();
        let id = iced::window::Id::unique();
        let _ = app.update(Message::ResetWindowSize);
        let revision = app.size_revision;
        let _ = app.update(Message::Window((
            id,
            iced::window::Event::Resized(iced::Size::new(1200.0, 700.0)),
        )));
        assert_eq!(app.size_revision, revision);
        assert!(app.size_pending);
        assert_eq!(app.window_width, "950");
        let _ = app.update(Message::ResizeWindow(
            revision,
            DEFAULT_WINDOW_SIZE,
            Some(id),
        ));
        assert!(!app.size_pending);
        assert_eq!(
            (app.window_width.as_str(), app.window_height.as_str()),
            ("950", "700")
        );
    }

    #[test]
    fn obsolete_async_resize_cannot_overwrite_newer_input() {
        let mut app = app();
        let id = iced::window::Id::unique();
        let _ = app.update(Message::WindowWidth("1400".into()));
        let obsolete = app.size_revision;
        let _ = app.update(Message::ResetWindowSize);
        for window in [None, Some(id)] {
            let _ = app.update(Message::ResizeWindow(
                obsolete,
                iced::Size::new(1400.0, 700.0),
                window,
            ));
            assert!(app.size_pending);
            assert_eq!(app.window_width, "950");
        }
    }

    #[test]
    fn invalid_native_sizes_do_not_replace_window_inputs() {
        let mut app = app();
        for size in [
            iced::Size::ZERO,
            iced::Size::new(f32::NAN, 700.0),
            iced::Size::new(950.0, f32::INFINITY),
        ] {
            let _ = app.update(Message::WindowSize(size));
            assert_eq!(
                (app.window_width.as_str(), app.window_height.as_str()),
                ("950", "700")
            );
        }
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn proxy_widget_count_is_bounded_across_many_expanded_groups() {
        fn nodes(groups: usize) -> usize {
            let mut app = App::with_engine(Err(anyhow::anyhow!("view test")), None).0;
            let mut proxies = serde_json::Map::new();
            let names: Vec<_> = (0..100).map(|i| format!("node-{i}")).collect();
            for name in &names {
                proxies.insert(name.clone(), serde_json::json!({"type":"HTTP"}));
            }
            for group in 0..groups {
                proxies.insert(
                    format!("group-{group}"),
                    serde_json::json!({"type":"Selector","now":"node-0","all":names}),
                );
            }
            app.snapshot.proxies =
                serde_json::from_value(serde_json::json!({"proxies":proxies})).unwrap();
            app.query = "node-".into();
            let element = app.proxies();
            let children = element.as_widget().children();
            let mut pending: Vec<_> = children.iter().collect();
            let mut count = 0;
            while let Some(tree) = pending.pop() {
                count += 1;
                pending.extend(tree.children.iter());
            }
            count
        }
        assert_eq!(nodes(5), nodes(500));
    }

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

#[cfg(test)]
mod async_state_tests {
    use super::*;
    fn app() -> App {
        App::with_engine(Err(anyhow::anyhow!("state test")), None).0
    }
    #[test]
    fn retry_after_initialization_failure_is_available_and_not_queued_twice() {
        let mut app = app();
        assert!(app.ready_to_retry());
        assert!(app.core_failure.is_some());
        let _ = app.update(Message::RestartCore);
        assert!(app.busy && app.working);
        let _ = app.update(Message::RestartCore);
        assert!(app.queued_actions.is_empty());
        let _ = app.update(Message::CoreReady(Err("权限不足".into())));
        assert!(app.ready_to_retry());
        assert_eq!(app.core_failure.as_deref(), Some("权限不足"));
        let _ = app.update(Message::Navigate(Page::Settings));
        assert_eq!(app.core_failure.as_deref(), Some("权限不足"));
    }
    #[test]
    fn running_or_exiting_core_cannot_be_retried_and_success_clears_failure() {
        let mut app = app();
        app.exiting = true;
        let _ = app.update(Message::RestartCore);
        assert!(!app.busy);
        app.exiting = false;
        let _ = app.update(Message::Finished(Box::new(Reply {
            scope: Scope::Home,
            settings: app.settings.clone(),
            profiles: None,
            snapshot: Some(Snapshot {
                running: true,
                ..Snapshot::default()
            }),
            notice: Ok("内核已启动".into()),
            running: true,
            geo_status: String::new(),
            autostart: false,
            exit_after_start: false,
        })));
        assert!(app.core_failure.is_none());
        let _ = app.update(Message::RestartCore);
        assert!(!app.busy && app.queued_actions.is_empty());
    }
    #[test]
    fn late_group_result_cannot_remove_new_work_or_restore_old_delays() {
        let mut app = app();
        let (old, _) = AbortHandle::new_pair();
        let old_watch = old.clone();
        app.delay_aborters.insert("group".into(), old);
        let obsolete = app.delay_generation;
        app.cancel_delay_checks();
        assert!(old_watch.is_aborted());
        let (new, _) = AbortHandle::new_pair();
        app.delay_aborters.insert("group".into(), new);
        app.testing_groups.insert("group".into());
        let _ = app.update(Message::GroupDelayDone(
            obsolete,
            "group".into(),
            Ok(BTreeMap::from([("node".into(), 999)])),
        ));
        assert!(app.testing_groups.contains("group"));
        assert!(app.node_delays.is_empty());
        let _ = app.update(Message::GroupDelayDone(
            app.delay_generation,
            "group".into(),
            Ok(BTreeMap::from([("node".into(), 42)])),
        ));
        assert!(app.testing_groups.is_empty());
        assert_eq!(app.node_delays["node"], 42);
    }
    #[test]
    fn late_periodic_result_cannot_replace_current_run() {
        let mut app = app();
        let obsolete = app.delay_generation;
        app.cancel_delay_checks();
        let (handle, _) = AbortHandle::new_pair();
        app.periodic_aborter = Some(handle);
        app.periodic_testing = true;
        let _ = app.update(Message::PeriodicDone(
            obsolete,
            Ok(BTreeMap::from([("node".into(), 999)])),
        ));
        assert!(app.periodic_testing);
        assert!(app.node_delays.is_empty());
        let _ = app.update(Message::PeriodicDone(
            app.delay_generation,
            Ok(BTreeMap::from([("node".into(), 42)])),
        ));
        assert!(!app.periodic_testing);
        assert_eq!(app.node_delays["node"], 42);
    }
    #[test]
    fn queued_port_save_contains_only_ports() {
        let mut app = app();
        app.busy = true;
        app.controller_port = "9999".into();
        app.mixed_port = "8888".into();
        let _ = app.update(Message::Action(Action::SavePorts(0, 0)));
        app.settings.run_mode = "global".into();
        app.settings.delay_interval_minutes = 10;
        assert!(matches!(
            app.queued_actions.pop_front(),
            Some(Action::SavePorts(9999, 8888))
        ));
        assert_eq!(app.settings.run_mode, "global");
        assert_eq!(app.settings.delay_interval_minutes, 10);
    }
}

#[cfg(test)]
mod dev2_tests {
    use super::*;

    fn app() -> App {
        App::with_engine(Err(anyhow::anyhow!("isolated dev.2 test")), None).0
    }

    fn result(state: ip_check::State, country: &str) -> Result<ip_check::CheckResult, String> {
        Ok(ip_check::CheckResult {
            state,
            summary: "可用".into(),
            country: country.into(),
            millis: 20,
            detail: "test evidence".into(),
        })
    }

    #[test]
    fn catalog_slots_replace_results_and_ignore_cancelled_generations() {
        let mut app = app();
        let id = ip_check::services()[1].id.clone();
        let old = app.site_generation;
        app.cancel_site_checks();
        let _ = app.update(Message::SiteDone(
            old,
            id.clone(),
            result(ip_check::State::Confirmed, "日本"),
        ));
        assert!(app.site_results.is_empty());
        let _ = app.update(Message::SiteDone(
            app.site_generation,
            id.clone(),
            result(ip_check::State::Confirmed, "日本"),
        ));
        let _ = app.update(Message::SiteDone(
            app.site_generation,
            id,
            result(ip_check::State::Partial, "日本"),
        ));
        let report = ip_report::Report::from_results(&app.site_results);
        assert_eq!(report.totals.completed(), 1);
        assert_eq!(report.totals.count(Status::Available), 0);
        assert_eq!(report.totals.count(Status::Partial), 1);
    }

    #[test]
    fn combined_filters_use_actual_region_and_summary_click_resets_other_filters() {
        let mut app = app();
        app.site_results.resize_with(183, || None);
        app.site_results[1] = Some(result(ip_check::State::Confirmed, "🇯🇵 日本"));
        app.site_results[2] = Some(result(ip_check::State::Restricted, "美国"));
        app.site_category = CategoryFilter::Group("AI");
        app.site_status = StatusFilter::Status(Status::Available);
        app.site_region = RegionFilter::Country("JP");
        assert!(app.site_matches(1, "chatgpt"));
        assert!(!app.site_matches(2, ""));
        assert!(!app.site_matches(1, "claude"));
        app.query = "old search".into();
        app.list_offset = 120;
        app.site_detail = Some(1);
        let _ = app.update(Message::SiteSummaryFilter(
            CategoryFilter::Group("AI"),
            StatusFilter::Status(Status::Restricted),
        ));
        assert!(app.query.is_empty());
        assert_eq!(app.site_region, RegionFilter::All);
        assert_eq!(app.list_offset, 0);
        assert_eq!(app.site_detail, None);
        assert!(app.site_matches(2, ""));
        assert_eq!(
            ip_report::Report::from_results(&app.site_results)
                .totals
                .completed(),
            2
        );
    }

    #[test]
    fn ordering_cache_is_bounded_reused_and_released_when_leaving_proxy_page() {
        let mut app = app();
        let names: Vec<_> = (0..125).rev().map(|i| format!("node{i}")).collect();
        let mut proxies = serde_json::Map::new();
        for group in 0..12 {
            proxies.insert(
                format!("group-{group}"),
                serde_json::json!({"type":"Selector", "all":names}),
            );
        }
        app.snapshot.proxies =
            serde_json::from_value(serde_json::json!({"proxies":proxies})).unwrap();
        app.query = "node".into();
        drop(app.proxies());
        assert_eq!(app.proxy_order.borrow().len(), GROUP_PAGE_SIZE);
        let old_pointer = app.proxy_order.borrow().values().next().unwrap().as_ptr();
        drop(app.proxies());
        assert_eq!(
            app.proxy_order.borrow().values().next().unwrap().as_ptr(),
            old_pointer
        );
        app.group_offsets.insert("group-0".into(), 60);
        let _ = app.update(Message::NodeSort(NodeSort::Name));
        assert!(app.group_offsets.is_empty());
        assert!(app.proxy_order.borrow().is_empty());
        drop(app.proxies());
        assert_eq!(app.proxy_order.borrow().len(), GROUP_PAGE_SIZE);
        let _ = app.update(Message::Navigate(Page::Home));
        assert!(app.proxy_order.borrow().is_empty());
    }

    #[test]
    fn old_settings_receive_default_sort_and_new_preferences_round_trip() {
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json.as_object_mut().unwrap().remove("node_sort");
        let settings: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.node_sort, NodeSort::LatencyAscending);
        for mode in NodeSort::ALL {
            let settings = Settings {
                node_sort: mode,
                ..Settings::default()
            };
            let decoded: Settings =
                serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
            assert_eq!(decoded.node_sort, mode);
        }
    }
}
