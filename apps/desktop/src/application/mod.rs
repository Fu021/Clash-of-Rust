//! Application state and effects, independent of a GUI framework.
mod commands;
mod connection_lookup;
mod runtime;
use commands::{Action, Reply, execute};
mod bindings;
mod bootstrap;
mod claude;
mod format;
mod interaction;
mod logs;
#[cfg(target_os = "macos")]
#[path = "../platform/macos/desktop.rs"]
mod mac_desktop;
mod notice;
mod presentation;
mod rules;
#[cfg(test)]
mod ui_testing;
#[cfg(test)]
mod ui_tests;
mod window_state;
mod wsl;
use clash_of_rust::{
    config::{AccentColor, Profile, Settings, Store, ThemeMode},
    connection::{self, Field as ConnectionField, Sort as ConnectionSort},
    engine::{Engine, ProxyMode, Scope, Snapshot},
    ip_check,
    ip_report::{self, CategoryFilter, RegionFilter, ResultSlot, Status, StatusFilter},
    platform, probe,
    proxy_order::{self, NodeSort},
    rule_manager::{self, ProfileRules},
    tray, update,
};
use claude as claude_view;
use futures_util::future::{AbortHandle, Abortable};
use interaction as desktop_view;
use rules as rules_view;
use runtime as desktop_runtime;
use runtime::Task;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;
const DEFAULT_WINDOW_SIZE: desktop_runtime::Size = desktop_runtime::Size::new(950.0, 700.0);
const MIN_WINDOW_SIZE: desktop_runtime::Size = desktop_runtime::Size::new(800.0, 450.0);
pub(crate) use bootstrap::run;
fn tray_unavailable(error: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("菜单栏图标不可用：{error}")
    } else {
        format!("托盘不可用：{error}；关闭窗口将退出。")
    }
}
fn update_restart() -> bool {
    std::env::args_os().any(|arg| arg == "--update-result")
}

fn background_start() -> bool {
    std::env::args_os().any(|arg| arg == "--background")
}
fn startup_store() -> anyhow::Result<Store> {
    Store::discover()
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
    Claude,
    Settings,
    Wsl,
}

impl Page {
    const ALL: [Self; 11] = [
        Self::Home,
        Self::Proxies,
        Self::Profiles,
        Self::Connections,
        Self::Rules,
        Self::Logs,
        Self::Tests,
        Self::Websites,
        Self::Claude,
        Self::Settings,
        Self::Wsl,
    ];
    fn scope(self) -> Scope {
        match self {
            Self::Home => Scope::Home,
            Self::Proxies => Scope::Proxies,
            Self::Connections => Scope::Connections,
            Self::Rules => Scope::Rules,
            Self::Logs => Scope::Logs,
            Self::Claude => Scope::Claude,
            _ => Scope::Other,
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

// The message can be cloned; only one receiver may consume the owned installer.
type DownloadResult = Arc<std::sync::Mutex<Option<Result<update::Downloaded, String>>>>;
#[cfg(target_os = "macos")]
type FilePickSession = Arc<std::sync::Mutex<Option<clash_of_rust::file_dialog::Session>>>;

#[derive(Debug, Clone)]
enum Message {
    Key(desktop_runtime::keyboard::Event),
    SystemTheme(desktop_runtime::theme::Mode),
    Appearance(ThemeMode),
    TextScale(u16),
    TraySpeed(bool),
    Confirm,
    Noop,
    CancelConfirm,
    ConnectionDetail(String),
    CloseConnectionDetail,
    LogLevel(Option<logs::Level>),
    LogDetail(String),
    CloseLogDetail,
    ConnectionResolved(u64, Result<String, String>),
    Copy(String),
    EditProfile(String),
    EditName(String),
    EditSource(String),
    SaveProfileEdit,
    CloseProfileEdit,
    ImportProfile,
    ImportDialog(bool),
    AccentDialog(bool),
    PickFile(bool),
    FilePicked(bool, Result<Option<std::path::PathBuf>, String>),
    #[cfg(target_os = "macos")]
    FilePickStarted(bool, FilePickSession),
    Claude(claude_view::Event),
    Wsl(wsl::Event),
    RestartCore,
    CoreReady(Result<CoreHandle, String>),
    DismissNotice,
    NoticeExpired(u64),
    SystemNotification,
    NotificationFinished(Result<(), String>),
    Navigate(Page),
    Query(String),
    NodeSort(NodeSort),
    ConnectionSort(ConnectionField),
    SiteCategory(CategoryFilter),
    SiteStatus(StatusFilter),
    SiteRegion(RegionFilter),
    SiteDetail(usize),
    SiteSummaryFilter(CategoryFilter, StatusFilter),
    SiteResetFilters,
    SiteReport,
    SiteRetest,
    ListPage(usize),
    ProfileName(String),
    ProfileSource(String),
    ControllerPort(String),
    MixedPort(String),
    SavePorts,
    Rules(rules_view::Event),
    WindowWidth(String),
    WindowHeight(String),
    ResetWindowSize,
    ApplyWindowSize(u64, desktop_runtime::Size),
    ResizeWindow(
        u64,
        desktop_runtime::Size,
        Option<desktop_runtime::window::Id>,
    ),
    VerifyWindowSize(u64, desktop_runtime::Size, desktop_runtime::window::Id, u8),
    RetryWindowSize(u64, desktop_runtime::Size, desktop_runtime::window::Id, u8),
    WindowSizeConfirmed(
        u64,
        desktop_runtime::Size,
        desktop_runtime::window::Id,
        u8,
        desktop_runtime::Size,
        bool,
        Option<bool>,
    ),
    #[cfg(target_os = "linux")]
    WindowCreated(desktop_runtime::window::Id),
    ReadWindowSize(Option<desktop_runtime::window::Id>),
    WindowSize(desktop_runtime::Size),
    CaptureWindowState(u64, desktop_runtime::window::Id),
    WindowStateCaptured(u64, desktop_runtime::Size, bool, Option<bool>),
    WindowStateSaved(Result<(), String>),
    TestUrl(String),
    DnsHost(String),
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
    #[cfg(target_os = "macos")]
    MacTrayPumped(Result<mac_desktop::Events, String>),
    ToggleGroup(String),
    GroupPage(String, bool),
    GroupDelay(String),
    GroupDelayDone(u64, String, Result<BTreeMap<String, u32>, String>),
    Tray(tray::Command),
    ShowWindow(Option<desktop_runtime::window::Id>),
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
    Window((desktop_runtime::window::Id, desktop_runtime::window::Event)),
    ExitWindowState(Option<(desktop_runtime::Size, bool, Option<bool>)>),
    ExitFinished(Result<(), String>),
}

struct App {
    confirmation: Option<desktop_view::Confirmation>,
    profile_editor: Option<desktop_view::ProfileEditor>,
    import_open: bool,
    accent_open: bool,
    connection_detail: Option<String>,
    log_level: Option<logs::Level>,
    log_detail: Option<String>,
    connection_lookup: connection_lookup::Lookup,
    picking_file: bool,
    picking_profile: Option<String>,
    system_dark: bool,
    engine: Option<Arc<Mutex<Engine>>>,
    page: Page,
    settings: Settings,
    rule_state: rules_view::State,
    claude: claude_view::State,
    wsl: wsl::State,
    profiles: Vec<Profile>,
    snapshot: Snapshot,
    busy: bool,
    working: bool,
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
    connection_sort: ConnectionSort,
    connection_order: RefCell<Option<Vec<usize>>>,
    delay_generation: u64,
    delay_aborters: BTreeMap<String, AbortHandle>,
    periodic_aborter: Option<AbortHandle>,
    tray: Option<tray::Guard>,
    tray_startup: Option<tray::Startup>,
    tray_close_requested: bool,
    #[cfg(target_os = "macos")]
    mac_reopen_ready: bool,
    site_queue: VecDeque<String>,
    interval_input: String,
    periodic_testing: bool,
    site_results: Vec<ResultSlot>,
    site_category: CategoryFilter,
    site_status: StatusFilter,
    site_region: RegionFilter,
    site_detail: Option<usize>,
    site_show_results: bool,
    site_has_run: bool,
    site_generation: u64,
    site_aborters: BTreeMap<String, AbortHandle>,
    site_busy: BTreeSet<String>,
    exiting: bool,
    notice: String,
    error: bool,
    core_failure: Option<String>,
    notice_visible: bool,
    notice_started: Option<std::time::Instant>,
    notice_revision: u64,
    notice_timer: Option<desktop_runtime::task::Handle>,
    notification_busy: bool,
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
    window_opening: bool,
    window_state: window_state::State,
    window_state_revision: u64,
    window_state_debounce: Option<desktop_runtime::task::Handle>,
    dark: bool,
    autostart: bool,
    test_url: String,
    dns_host: String,
    test_results: Vec<String>,
    geo_status: String,
    updates: update::State,
}

impl App {
    fn new(
        tray_startup: Option<tray::Startup>,
        window_state: window_state::State,
    ) -> (Self, Task<Message>) {
        let mut app =
            Self::initial_state(Err(anyhow::anyhow!("正在初始化内核与配置…")), tray_startup);
        // Read the small settings file before creating the first surface. Core,
        // Geo and network initialization remain on the background worker.
        if let Ok(settings) = startup_store().and_then(|store| store.load_settings()) {
            app.system_dark = settings.dark;
            app.settings = settings;
            app.sync_appearance();
        }
        app.set_window_size(window_state.size());
        app.window_state = window_state;
        app.core_failure = None;
        app.error = false;
        app.notice_visible = false;
        app.busy = true;
        app.working = true;
        let task = Task::perform(
            async {
                tokio::task::spawn_blocking(|| startup_store().and_then(Engine::new))
                    .await
                    .map_err(|error| format!("初始化任务失败：{error}"))?
                    .map(|engine| CoreHandle(Arc::new(Mutex::new(engine))))
                    .map_err(|error| format!("初始化失败：{error:#}"))
            },
            Message::CoreReady,
        );
        (
            app,
            Task::batch([
                task,
                desktop_runtime::window::latest().map(Message::ReadWindowSize),
                desktop_runtime::system::theme().map(Message::SystemTheme),
                Task::perform(
                    async {
                        if let Err(error) = platform::notification::initialize().await {
                            eprintln!("系统通知授权未开启：{error:#}");
                        }
                    },
                    |_| Message::Tick,
                ),
            ]),
        )
    }

    fn initial_state(
        created: anyhow::Result<Engine>,
        mut tray_startup: Option<tray::Startup>,
    ) -> Self {
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
                (None, format!("{notice} {}", tray_unavailable(&e)), true)
            }
            None => (None, notice, error),
        };
        let core_failure = engine.is_none().then(|| notice.clone());
        let mut app = Self {
            confirmation: None,
            profile_editor: None,
            import_open: false,
            accent_open: false,
            connection_detail: None,
            log_level: None,
            log_detail: None,
            connection_lookup: connection_lookup::Lookup::default(),
            picking_file: false,
            picking_profile: None,
            system_dark: settings.dark,
            rule_state: rules_view::State::default(),
            claude: claude_view::State::default(),
            wsl: wsl::State::new(settings.wsl.clone()),
            node_sort: settings.node_sort,
            controller_port: settings.controller_port.to_string(),
            interval_input: settings.delay_interval_minutes.to_string(),
            periodic_testing: false,
            mixed_port: settings.mixed_port.to_string(),
            window_width: format!("{:.0}", DEFAULT_WINDOW_SIZE.width),
            window_height: format!("{:.0}", DEFAULT_WINDOW_SIZE.height),
            size_revision: 0,
            size_pending: false,
            window_opening: false,
            window_state: window_state::State::default(),
            window_state_revision: 0,
            window_state_debounce: None,
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
            notice_started: error.then(std::time::Instant::now),
            notice_revision: 0,
            notice_timer: None,
            notification_busy: false,
            page: Page::Home,
            snapshot: Snapshot::default(),
            busy: false,
            working: false,
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
            connection_sort: ConnectionSort::default(),
            connection_order: RefCell::new(None),
            delay_generation: 0,
            delay_aborters: BTreeMap::new(),
            periodic_aborter: None,
            tray,
            tray_startup,
            tray_close_requested: false,
            #[cfg(target_os = "macos")]
            mac_reopen_ready: false,
            site_queue: VecDeque::new(),
            site_results: vec![],
            site_category: CategoryFilter::All,
            site_status: StatusFilter::All,
            site_region: RegionFilter::All,
            site_detail: None,
            site_show_results: false,
            site_has_run: false,
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
        app.updates.status = update::FORK_UPDATE_NOTICE.into();
        if let Some((message, error)) = update::startup_outcome() {
            if error {
                app.updates.failure = Some(message);
            } else {
                app.updates.outcome = Some(message);
            }
            app.error = error;
        }
        app
    }

    fn ready_to_retry(&self) -> bool {
        !self.snapshot.running
            && !self.busy
            && !self.working
            && !self.exiting
            && !self.updates.busy()
    }

    fn check_update(&mut self) -> Task<Message> {
        if !update::SELF_UPDATE_ENABLED {
            self.updates.checking = false;
            self.updates.available = None;
            self.updates.status = update::FORK_UPDATE_NOTICE.into();
            return Task::none();
        }
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

    fn dispatch(&mut self, mut action: Action) -> Task<Message> {
        if matches!(action, Action::Refresh)
            && self.visible
            && self.page == Page::Rules
            && self.rule_state.document.is_none()
            && let Some(id) = self.settings.active_profile.clone()
        {
            action = Action::LoadRules(id);
        }
        let scope = if self.visible {
            self.page.scope()
        } else {
            Scope::Other
        };
        self.dispatch_scope(action, scope)
    }

    fn dispatch_scope(&mut self, action: Action, scope: Scope) -> Task<Message> {
        if matches!(action, Action::Select(..))
            && (self.settings.claude_protection.is_some()
                || self.claude.protection_pending.is_some())
        {
            self.show_notice(
                "Claude 保护已锁定当前节点，请先关闭保护再切换节点".into(),
                false,
            );
            return Task::none();
        }
        if (self.updates.install_pending || self.updates.installing.is_some())
            && !matches!(action, Action::Refresh)
        {
            return Task::none();
        }
        if !self.exiting {
            // Coalesce rapid appearance clicks while a network operation owns the engine.
            self.queued_actions.retain(|queued| {
                !matches!(
                    (&action, queued),
                    (Action::Appearance(_), Action::Appearance(_))
                        | (Action::Accent(_), Action::Accent(_))
                        | (Action::TextScale(_), Action::TextScale(_))
                )
            });
            if (self.engine.is_some() || self.busy)
                && self.queued_actions.len() < 16
                && action.preview_appearance(&mut self.settings)
            {
                for queued in &self.queued_actions {
                    queued.preview_appearance(&mut self.settings);
                }
                self.sync_appearance();
            }
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
                | Action::DnsOverride(_)
                | Action::Ipv6(_)
                | Action::Udp(_)
                | Action::ClaudeProtection(_)
                | Action::SaveRules(..)
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
            self.claude.close();
            self.claude.reports = Default::default();
            self.claude.status = "网络配置已改变，请重新检测地区线索。".into();
        }
        self.busy = true;
        self.working = !matches!(action, Action::Refresh);
        if let Action::ClaudeProtection(enabled) = &action {
            self.claude.protection_pending = Some(*enabled);
        }
        Task::perform(execute(engine, action, scope, self.visible), |reply| {
            Message::Finished(Box::new(reply))
        })
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let notice_revision = self.notice_revision;
        #[cfg(target_os = "macos")]
        let pump = matches!(
            &message,
            Message::Tick | Message::Window((_, desktop_runtime::window::Event::Opened { .. }))
        );
        let task = self.update_message(message);
        let task = self.schedule_notice(notice_revision, task);
        #[cfg(target_os = "macos")]
        if pump {
            return Task::batch([
                task,
                desktop_runtime::native::call(|_| {
                    tray::pump();
                    Message::MacTrayPumped(mac_desktop::pump())
                }),
            ]);
        }
        task
    }

    fn update_message(&mut self, message: Message) -> Task<Message> {
        if let Some(task) = self.desktop_message(&message) {
            return task;
        }
        match message {
            Message::Noop => {}
            Message::Key(_)
            | Message::SystemTheme(_)
            | Message::Appearance(_)
            | Message::TextScale(_)
            | Message::TraySpeed(_)
            | Message::Confirm
            | Message::CancelConfirm
            | Message::ConnectionDetail(_)
            | Message::ConnectionResolved(..)
            | Message::LogLevel(_)
            | Message::LogDetail(_)
            | Message::CloseLogDetail
            | Message::CloseConnectionDetail
            | Message::Copy(_)
            | Message::EditProfile(_)
            | Message::EditName(_)
            | Message::EditSource(_)
            | Message::SaveProfileEdit
            | Message::CloseProfileEdit
            | Message::ImportProfile
            | Message::ImportDialog(_)
            | Message::AccentDialog(_)
            | Message::PickFile(_)
            | Message::FilePicked(..) => {}
            #[cfg(target_os = "macos")]
            Message::FilePickStarted(..) => {}
            Message::Claude(event) => return self.claude_event(event),
            Message::Wsl(event) => return self.wsl_event(event),
            #[cfg(target_os = "macos")]
            Message::MacTrayPumped(result) => match result {
                Ok(events) => {
                    self.mac_reopen_ready = true;
                    if events.quit && !self.exiting {
                        return self.request_exit();
                    }
                    if events.reopen && !self.exiting {
                        return desktop_runtime::window::latest().map(Message::ShowWindow);
                    }
                }
                Err(error) => {
                    if self.notice != error {
                        eprintln!("Dock 窗口恢复不可用：{error}");
                        self.show_notice(error, true);
                    }
                }
            },
            Message::DismissNotice => {
                self.dismiss_notice();
                self.updates.failure = None;
            }
            Message::NoticeExpired(revision) => {
                if revision == self.notice_revision {
                    self.dismiss_notice();
                }
            }
            Message::SystemNotification => {
                if self.notification_busy || self.exiting {
                    return Task::none();
                }
                self.notification_busy = true;
                return Task::perform(
                    async {
                        platform::notification::show("Clash of Rust", "喵!")
                            .await
                            .map_err(|error| format!("系统通知失败：{error:#}"))
                    },
                    Message::NotificationFinished,
                );
            }
            Message::NotificationFinished(result) => {
                self.notification_busy = false;
                if let Err(error) = result {
                    self.show_notice(error, true);
                }
            }
            Message::RestartCore => {
                if !self.ready_to_retry() {
                    return Task::none();
                }
                self.dismiss_notice();
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
                            // Loading the ordinary core must not discard an
                            // independent WSL operation or a user's new draft.
                            self.wsl.load_preferences(self.settings.wsl.clone());
                            self.rule_state = rules_view::State::default();
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
                        self.show_notice(error, true);
                        return self.check_update();
                    }
                }
            }
            Message::CheckUpdate => return self.check_update(),
            Message::UpdateChecked(result) => {
                if update::SELF_UPDATE_ENABLED {
                    self.updates.finish(result);
                }
            }
            Message::InstallUpdate => {
                if !update::SELF_UPDATE_ENABLED {
                    self.updates.available = None;
                    self.updates.status = update::FORK_UPDATE_NOTICE.into();
                    return Task::none();
                }
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
                if let Some(download) = self.updates.cached_for(&available.version) {
                    return self.install_download(download);
                }
                let (id, registration) = self.updates.begin_download(
                    available
                        .package
                        .as_ref()
                        .map_or(1, |package| package.asset.size),
                );
                let port = self.snapshot.running.then_some(self.settings.mixed_port);
                return Task::run(
                    desktop_runtime::stream::channel(8, async move |output| {
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
                        return self.install_download(Arc::new(downloaded));
                    }
                    Some(Err(reason)) => self.updates.fail(reason),
                    None => return Task::none(),
                }
            }
            Message::UpdateHandoff(result) => {
                self.updates.install_pending = false;
                match result {
                    Ok(directory) => {
                        let shutdown = directory.requires_shutdown();
                        self.updates.installing = Some(directory);
                        if shutdown {
                            return self.exit();
                        }
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
                    self.show_notice(format!("无法打开 Release 页面：{error}"), true);
                }
            }
            Message::Navigate(page) => {
                if self.page == Page::Claude && page != Page::Claude {
                    self.claude.close();
                }
                if self.page == Page::Rules
                    && page != Page::Rules
                    && !self.rule_state.dirty()
                    && !self.rule_state.has_editor()
                {
                    self.rule_state.release_document();
                }
                self.page = page;
                self.log_detail = None;
                self.clear_connection_detail();
                self.query.clear();
                self.list_offset = 0;
                self.snapshot.retain_scope(page.scope());
                self.proxy_order.get_mut().clear();
                *self.connection_order.get_mut() = None;
                self.site_detail = None;
                if page == Page::Rules {
                    if let Some(doc) = &self.rule_state.document {
                        let id = doc.profile_id.clone();
                        return self.dispatch_scope(Action::LoadRules(id), Scope::Rules);
                    }
                    if let Some(id) = self.settings.active_profile.clone() {
                        return self.dispatch_scope(Action::LoadRules(id), Scope::Rules);
                    }
                }
                return self.dispatch(Action::Refresh);
            }
            Message::Query(value) => {
                if self.page == Page::Rules {
                    self.rule_state.clear_selection();
                }
                self.query = value;
                self.group_offsets.clear();
                self.list_offset = 0;
                self.proxy_order.get_mut().clear();
                *self.connection_order.get_mut() = None;
                self.site_detail = None;
            }
            Message::ConnectionSort(field) => {
                self.connection_sort.toggle(field);
                *self.connection_order.get_mut() = None;
                self.list_offset = 0;
            }
            Message::NodeSort(mode) => {
                self.node_sort = mode;
                self.group_offsets.clear();
                self.proxy_order.get_mut().clear();
                return self.dispatch(Action::NodeSort(mode));
            }
            Message::SiteCategory(category) => {
                self.site_category = category;
                self.site_show_results = true;
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
                self.site_show_results = true;
            }
            Message::SiteResetFilters => {
                let category = self.site_category;
                self.reset_site_filters();
                self.site_category = category;
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
            Message::SiteReport => {
                self.reset_site_filters();
                self.query.clear();
                self.list_offset = 0;
                self.site_show_results = false;
            }
            Message::ListPage(offset) => self.list_offset = offset,
            Message::ProfileName(value) => self.profile_name = value,
            Message::ProfileSource(value) => self.profile_source = value,
            Message::ControllerPort(value) => self.controller_port = value,
            Message::MixedPort(value) => self.mixed_port = value,
            Message::Rules(event) => return self.update_rules(event),
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
                    return desktop_runtime::window::latest().then(move |id| match id {
                        Some(id) => desktop_runtime::window::fit_size(id, size)
                            .map(move |size| Message::ResizeWindow(revision, size, Some(id))),
                        None => Task::done(Message::ResizeWindow(revision, size, None)),
                    });
                }
            }
            Message::ResizeWindow(revision, size, Some(id)) => {
                if revision == self.size_revision {
                    self.size_pending = true;
                    self.window_width = format!("{:.0}", size.width);
                    self.window_height = format!("{:.0}", size.height);
                    self.window_state_revision = self.window_state_revision.wrapping_add(1);
                    if let Some(handle) = self.window_state_debounce.take() {
                        handle.abort();
                    }
                    // Drop provisional DPI-dependent limits before requesting
                    // a resize. Winit applies the request asynchronously; keep
                    // old native events out of the saved state until confirmed.
                    return desktop_runtime::window::set_min_size(id, None)
                        .chain(desktop_runtime::window::maximize(id, false))
                        .chain(desktop_runtime::window::resize(id, size))
                        .chain(verify_window_size(revision, size, id, 0));
                }
            }
            Message::ResizeWindow(revision, _, None) => {
                if revision == self.size_revision {
                    self.size_pending = false;
                }
            }
            Message::VerifyWindowSize(revision, size, id, attempt) => {
                if revision == self.size_revision && self.size_pending && !self.exiting {
                    return read_window_state(id).map(move |(actual, maximized, minimized)| {
                        Message::WindowSizeConfirmed(
                            revision, size, id, attempt, actual, maximized, minimized,
                        )
                    });
                }
                if !self.size_pending && !self.exiting {
                    return desktop_runtime::window::set_min_size(id, Some(MIN_WINDOW_SIZE));
                }
            }
            Message::RetryWindowSize(revision, size, id, attempt) => {
                if revision == self.size_revision && self.size_pending && !self.exiting {
                    return desktop_runtime::window::resize(id, size)
                        .chain(verify_window_size(revision, size, id, attempt));
                }
                if !self.size_pending && !self.exiting {
                    return desktop_runtime::window::set_min_size(id, Some(MIN_WINDOW_SIZE));
                }
            }
            Message::WindowSizeConfirmed(
                revision,
                size,
                id,
                attempt,
                actual,
                maximized,
                minimized,
            ) => {
                if revision != self.size_revision || !self.size_pending || self.exiting {
                    if !self.size_pending && !self.exiting {
                        return desktop_runtime::window::set_min_size(id, Some(MIN_WINDOW_SIZE));
                    }
                    return Task::none();
                }
                if !window_size_matches(size, actual) && attempt < 2 {
                    // A stale scale can map the request to the unchanged physical
                    // size, producing no Resized event. First request a distinct
                    // size to refresh the viewport, then apply the exact target.
                    let provisional = desktop_runtime::Size::new(
                        if size.width >= 16384.0 {
                            size.width - 2.0
                        } else {
                            size.width + 2.0
                        },
                        size.height,
                    );
                    return desktop_runtime::window::resize(id, provisional).chain(Task::perform(
                        async { tokio::time::sleep(Duration::from_millis(100)).await },
                        move |_| Message::RetryWindowSize(revision, size, id, attempt + 1),
                    ));
                }
                self.size_pending = false;
                self.set_window_size(actual);
                let save = if window_size_matches(size, actual)
                    && self.window_state.capture(actual, maximized, minimized)
                {
                    let state = self.window_state.clone();
                    Task::perform(
                        async move { state.save().await.map_err(|error| format!("{error:#}")) },
                        Message::WindowStateSaved,
                    )
                } else {
                    Task::none()
                };
                return Task::batch([
                    desktop_runtime::window::set_min_size(id, Some(MIN_WINDOW_SIZE)),
                    save,
                ]);
            }
            Message::ReadWindowSize(Some(id)) => {
                return desktop_runtime::window::size(id).map(Message::WindowSize);
            }
            Message::ReadWindowSize(None) => {}
            Message::WindowSize(size) => self.set_window_size(size),
            Message::CaptureWindowState(revision, id) => {
                if revision == self.window_state_revision && !self.exiting {
                    self.window_state_debounce = None;
                    return read_window_state(id).map(move |(size, maximized, minimized)| {
                        Message::WindowStateCaptured(revision, size, maximized, minimized)
                    });
                }
            }
            Message::WindowStateCaptured(revision, size, maximized, minimized) => {
                if revision == self.window_state_revision
                    && !self.size_pending
                    && !self.exiting
                    && self.window_state.capture(size, maximized, minimized)
                {
                    let state = self.window_state.clone();
                    return Task::perform(
                        async move { state.save().await.map_err(|error| format!("{error:#}")) },
                        Message::WindowStateSaved,
                    );
                }
            }
            Message::WindowStateSaved(result) => {
                if let Err(error) = result {
                    self.show_notice(format!("无法保存窗口尺寸：{error}"), true);
                }
            }
            Message::Window((id, desktop_runtime::window::Event::Opened { .. })) => {
                self.window_opening = false;
                self.size_revision = self.size_revision.wrapping_add(1);
                return self.update(Message::ResizeWindow(
                    self.size_revision,
                    self.window_state.size(),
                    Some(id),
                ));
            }
            Message::Window((id, desktop_runtime::window::Event::Rescaled)) => {
                if self.size_pending {
                    return Task::none();
                }
                return desktop_runtime::window::set_min_size(id, Some(MIN_WINDOW_SIZE))
                    .chain(desktop_runtime::window::size(id).map(Message::WindowSize));
            }
            Message::Window((id, desktop_runtime::window::Event::Resized(size))) => {
                // A resize event can belong to an older native request. It must
                // not cancel a newer input edit or Restore Default command.
                self.set_window_size(size);
                // X11 can update WM_NORMAL_HINTS again while delivering the
                // final resize after a DPI change. Apply limits after that event.
                return Task::batch([
                    if self.size_pending {
                        Task::none()
                    } else {
                        desktop_runtime::window::set_min_size(id, Some(MIN_WINDOW_SIZE))
                    },
                    self.schedule_window_state(id),
                ]);
            }
            Message::TestUrl(value) => self.test_url = value,
            Message::DnsHost(value) => self.dns_host = value,
            Message::Action(Action::DeleteProfile(id)) => {
                return self.confirm_delete(id);
            }
            Message::SavePorts => {
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
                        self.show_notice("端口必须为 1–65535 的整数".into(), true);
                    }
                }
            }
            Message::Action(action) => {
                if let Action::Delay(node) = &action
                    && self
                        .snapshot
                        .proxies
                        .proxies
                        .get(node.as_str())
                        .is_some_and(|proxy| matches!(proxy.kind.as_str(), "URLTest" | "Fallback"))
                {
                    return self.update(Message::GroupDelay(node.clone()));
                }
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
                        self.show_notice(
                            format!(
                                "{group} 测速完成：{good}/{} 个节点可达，其余超时或失败",
                                delays.len()
                            ),
                            false,
                        );
                        self.node_delays.extend(delays);
                        self.proxy_order.get_mut().clear();
                    }
                    Err(error) => {
                        self.show_notice(format!("{group} 测速失败：{error}"), true);
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
                    self.show_notice("请输入 0–1440 分钟，0 表示关闭定时测速".into(), true);
                }
            },
            Message::PeriodicDelay => {
                if !self.snapshot.running
                    || self.working
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
                if !self.can_check_sites() {
                    return Task::none();
                }
                self.site_results.clear();
                self.reset_site_filters();
                self.query.clear();
                self.site_show_results = false;
                self.site_has_run = true;
                self.list_offset = 0;
                self.site_generation = self.site_generation.wrapping_add(1);
                self.site_queue = ip_check::services()
                    .iter()
                    .map(|item| item.id.clone())
                    .collect();
                return self.start_site_queue();
            }
            Message::SiteRetest => {
                if !self.can_check_sites() {
                    return Task::none();
                }
                if self.site_category == CategoryFilter::All {
                    return self.update(Message::SiteAll);
                }
                let CategoryFilter::Group(group) = self.site_category else {
                    unreachable!();
                };
                self.reset_site_filters();
                self.site_category = CategoryFilter::Group(group);
                self.query.clear();
                self.site_results
                    .resize_with(ip_check::services().len(), || None);
                self.site_generation = self.site_generation.wrapping_add(1);
                self.site_has_run = true;
                self.site_detail = None;
                self.list_offset = 0;
                self.site_queue = ip_check::services()
                    .iter()
                    .enumerate()
                    .filter(|(_, service)| service.group == group)
                    .map(|(index, service)| {
                        self.site_results[index] = None;
                        service.id.clone()
                    })
                    .collect();
                return self.start_site_queue();
            }
            Message::SiteCancel => {
                self.cancel_site_checks();
            }
            Message::SiteProbe(url) => {
                if !self.snapshot.running
                    || self.working
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
                    tray::Command::Show => {
                        return desktop_runtime::window::latest().map(Message::ShowWindow);
                    }
                    tray::Command::Exit => return self.request_exit(),
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
                    tray::Command::RefreshNodes => return self.dispatch(Action::RefreshTray),
                    tray::Command::Select(group, node) => {
                        return self.dispatch(Action::Select(group, node));
                    }
                }
            }
            Message::ShowWindow(Some(id)) => {
                self.visible = true;
                let refresh = self.dispatch(Action::Refresh);
                return Task::batch([
                    desktop_runtime::window::set_mode(id, desktop_runtime::window::Mode::Windowed)
                        .chain(desktop_runtime::window::minimize(id, false))
                        .chain(desktop_runtime::window::gain_focus(id)),
                    refresh,
                ]);
            }
            Message::ShowWindow(None) => {
                return self.open_window();
            }
            #[cfg(target_os = "linux")]
            Message::WindowCreated(_id) => {
                self.window_opening = false;
            }
            Message::Tick => {
                if let Some(startup) = &self.tray_startup {
                    if let Some(result) = startup.poll() {
                        self.tray_startup = None;
                        match result {
                            Ok(tray) => {
                                tray.update(&self.snapshot);
                                tray.speed(&self.snapshot, self.settings.tray_speed);
                                self.tray = Some(tray);
                                self.tray_close_requested = false;
                            }
                            Err(error) => {
                                self.show_notice(tray_unavailable(&error), true);
                                if !self.visible {
                                    return desktop_runtime::window::latest()
                                        .map(Message::ShowWindow);
                                }
                            }
                        }
                    } else if !self.visible && startup.elapsed() >= Duration::from_secs(3) {
                        // Keep an app with no usable tray accessible while retrying.
                        return desktop_runtime::window::latest().map(Message::ShowWindow);
                    }
                }
                if self.updates.install_pending || self.updates.installing.is_some() {
                    self.updates.poll_install_stage();
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
                    return desktop_runtime::window::latest().map(Message::ShowWindow);
                }
                if !self.visible && !self.settings.tray_speed {
                    self.hidden_ticks = (self.hidden_ticks + 1) % 5;
                    if self.hidden_ticks != 0 {
                        return Task::none();
                    }
                }
                if (self.snapshot.running || self.page == Page::Logs) && !self.busy {
                    let scope = if !self.visible || self.page == Page::Rules {
                        Scope::Other
                    } else {
                        self.page.scope()
                    };
                    return self.dispatch_scope(Action::Refresh, scope);
                }
            }
            Message::Finished(reply) => {
                self.claude.protection_pending = None;
                let reply = *reply;
                let protection_changed = reply.protection_changed;
                if reply.profile_edit
                    && let Err(error) = &reply.notice
                    && let Some(editor) = &mut self.profile_editor
                {
                    editor.error = error.clone();
                }
                if reply.profile_import && reply.notice.is_ok() {
                    self.import_open = false;
                    self.profile_name.clear();
                    self.profile_source.clear();
                }
                self.busy = false;
                self.working = false;
                self.autostart = reply.autostart;
                if let Some(doc) = reply.rule_document
                    && self.page == Page::Rules
                {
                    if reply.rules_applied
                        || (!self.rule_state.dirty() && !self.rule_state.has_editor())
                    {
                        let rules = ProfileRules::for_profile(
                            &reply.settings,
                            &doc.profile_id,
                            doc.builtin,
                        );
                        self.rule_state.load(doc, rules);
                    } else {
                        self.rule_state.refresh_document(doc);
                    }
                }
                self.settings = reply.settings;
                // A completed Geo/network operation predates the queued appearance choices.
                for queued in &self.queued_actions {
                    queued.preview_appearance(&mut self.settings);
                }
                if !self
                    .queued_actions
                    .iter()
                    .any(|action| matches!(action, Action::NodeSort(_)))
                    && self.node_sort != self.settings.node_sort
                {
                    self.node_sort = self.settings.node_sort;
                    self.proxy_order.get_mut().clear();
                    self.group_offsets.clear();
                }
                self.sync_appearance();
                if let Some(nodes) = reply.tray_nodes
                    && let Some(tray) = &self.tray
                {
                    tray.nodes(nodes);
                }
                if let Some(profiles) = reply.profiles {
                    self.profiles = profiles;
                    if self
                        .rule_state
                        .document
                        .as_ref()
                        .is_some_and(|doc| !self.profiles.iter().any(|p| p.id == doc.profile_id))
                    {
                        self.rule_state = rules_view::State::default();
                    }
                }
                self.geo_status = reply.geo_status;
                if self.snapshot.running && !reply.running {
                    self.cancel_delay_checks();
                    self.node_delays.clear();
                    self.proxy_order.get_mut().clear();
                    self.expanded.clear();
                    self.group_offsets.clear();
                    self.cancel_site_checks();
                    self.claude.close();
                    self.claude.status =
                        "内核已停止；已获得的报告保留，网络结果可能已过期。".into();
                }
                if reply.running {
                    self.core_failure = None;
                } else if reply.start_requested || self.snapshot.running {
                    self.core_failure = Some(
                        reply
                            .notice
                            .as_ref()
                            .err()
                            .cloned()
                            .unwrap_or_else(|| "内核已退出，请查看日志或重试启动。".into()),
                    );
                }
                self.snapshot.running = reply.running;
                if let Some(mut snapshot) = reply.snapshot {
                    if let Some(protection) = &self.settings.claude_protection {
                        snapshot.proxies.proxies.remove(protection.route.as_str());
                        for proxy in snapshot.proxies.proxies.values_mut() {
                            proxy.all.retain(|name| name.as_str() != protection.route);
                        }
                    }
                    if let Some(guard) = &snapshot.claude_guard {
                        self.claude.guard_report(guard);
                    } else if reply.scope == Scope::Claude {
                        self.claude.clear_guard();
                    }
                    if let (Some(now), Some(previous)) = (
                        snapshot.connections_sampled_at,
                        self.snapshot.connections_sampled_at,
                    ) && let Some(elapsed) = now.checked_duration_since(previous)
                    {
                        connection::sample_rates(
                            &mut snapshot.connections.connections,
                            &self.snapshot.connections.connections,
                            elapsed,
                        );
                    }
                    if reply.scope == Scope::Proxies && snapshot.running {
                        self.proxy_order.get_mut().clear();
                        let proxies = &snapshot.proxies.proxies;
                        self.expanded
                            .retain(|name| proxies.contains_key(name.as_str()));
                        self.group_offsets
                            .retain(|name, _| proxies.contains_key(name.as_str()));
                        let members = proxies
                            .values()
                            .flat_map(|p| p.all.iter().map(|n| n.as_str()))
                            .collect::<BTreeSet<_>>();
                        self.node_delays.retain(|name, _| {
                            proxies.contains_key(name.as_str()) || members.contains(name.as_str())
                        });
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
                        snapshot.connections_sampled_at =
                            self.snapshot.connections_sampled_at.take();
                        snapshot.connection_count = self.snapshot.connection_count;
                    }
                    self.snapshot = snapshot;
                    *self.connection_order.get_mut() = None;
                    self.snapshot.retain_scope(if self.visible {
                        self.page.scope()
                    } else {
                        Scope::Other
                    });
                }
                if let Some(tray) = &self.tray {
                    tray.update(&self.snapshot);
                    tray.speed(&self.snapshot, self.settings.tray_speed);
                }
                match reply.notice {
                    Ok(notice) if !notice.is_empty() => {
                        if notice.starts_with("订阅信息已保存") {
                            self.profile_editor = None;
                        }
                        self.show_notice(notice, false);
                    }
                    Err(error) => {
                        self.show_notice(error, true);
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
                let protection_test = if protection_changed
                    && self.settings.claude_protection.is_some()
                    && self.page == Page::Claude
                    && !self.error
                {
                    self.claude_event(claude_view::Event::Start)
                } else {
                    Task::none()
                };
                return Task::batch([update_task, next_task, protection_test]);
            }
            Message::Probe(exit_ip) => {
                if self.probing {
                    return Task::none();
                }
                if !self.snapshot.running {
                    self.show_notice("请先启动内核".into(), true);
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
            Message::Window((_, desktop_runtime::window::Event::Focused)) => {
                self.visible = true;
                return self.dispatch(Action::Refresh);
            }
            Message::Window((id, desktop_runtime::window::Event::CloseRequested)) => {
                let can_reopen = self.tray.is_some();
                #[cfg(target_os = "macos")]
                let can_reopen = can_reopen || self.mac_reopen_ready;
                if can_reopen {
                    self.visible = false;
                    self.snapshot.retain_scope(Scope::Other);
                    if !self.rule_state.dirty() && !self.rule_state.has_editor() {
                        self.rule_state.release_document();
                    }
                    *self.connection_order.get_mut() = None;
                    let stop_stream = self.dispatch_scope(Action::Refresh, Scope::Other);
                    return Task::batch([
                        desktop_runtime::window::set_mode(
                            id,
                            desktop_runtime::window::Mode::Hidden,
                        ),
                        stop_stream,
                    ]);
                }
                if background_start() && self.tray_startup.is_some() && !self.tray_close_requested {
                    self.tray_close_requested = true;
                    self.show_notice(
                        "桌面托盘正在初始化，请稍候；再次关闭窗口可退出程序。".into(),
                        false,
                    );
                    return Task::none();
                }
                return self.request_exit();
            }
            Message::ExitWindowState(captured) => {
                if let Some((size, maximized, minimized)) = captured {
                    self.window_state.capture(size, maximized, minimized);
                }
                let state = self.window_state.clone();
                let engine = self.engine.clone();
                return Task::perform(
                    async move {
                        if let Err(error) = state.save().await {
                            eprintln!("无法保存窗口尺寸：{error:#}");
                        }
                        if let Some(engine) = engine {
                            engine.lock().await.stop().await.map_err(|e| e.to_string())
                        } else {
                            Ok(())
                        }
                    },
                    Message::ExitFinished,
                );
            }
            Message::ExitFinished(result) => match result {
                Ok(()) => return desktop_runtime::exit(),
                Err(error) => {
                    self.exiting = false;
                    self.show_notice(format!("退出失败：{error}"), true);
                    if self.updates.busy() {
                        if let Some(session) = &self.updates.installing {
                            session.cancel();
                        }
                        self.updates.progress = None;
                        self.updates.failure =
                            Some("更新失败：客户端无法正常退出，请检查代理恢复或内核状态".into());
                    }
                }
            },
        }
        Task::none()
    }

    fn install_download(&mut self, downloaded: Arc<update::Downloaded>) -> Task<Message> {
        if !update::SELF_UPDATE_ENABLED {
            self.updates.available = None;
            self.updates.status = update::FORK_UPDATE_NOTICE.into();
            return Task::none();
        }
        self.updates.prepare_install(downloaded.clone());
        Task::perform(
            async move {
                match tokio::task::spawn_blocking(move || update::handoff(downloaded)).await {
                    Ok(result) => result.map_err(|error| error.to_string()),
                    Err(_) => Err("安装助手无法启动，请重试".into()),
                }
            },
            Message::UpdateHandoff,
        )
    }

    fn exit(&mut self) -> Task<Message> {
        if self.exiting {
            return Task::none();
        }
        self.cancel_delay_checks();
        self.cancel_site_checks();
        self.clear_connection_detail();
        self.claude.shutdown();
        self.tray_startup = None;
        self.exiting = true;
        if let Some(handle) = self.window_state_debounce.take() {
            handle.abort();
        }
        desktop_runtime::window::latest().then(|id| match id {
            Some(id) => read_window_state(id).map(|state| Message::ExitWindowState(Some(state))),
            None => Task::done(Message::ExitWindowState(None)),
        })
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

    fn set_window_size(&mut self, size: desktop_runtime::Size) {
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

    fn open_window(&mut self) -> Task<Message> {
        if self.exiting || self.window_opening {
            return Task::none();
        }
        self.window_opening = true;
        self.visible = true;
        let (_, open) = desktop_runtime::window::open(desktop_runtime::window::Settings {
            size: self.window_state.size(),
            visible: true,
        });
        Task::batch([
            open.map(|id| {
                #[cfg(target_os = "linux")]
                {
                    Message::WindowCreated(id)
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = id;
                    Message::Noop
                }
            }),
            self.dispatch(Action::Refresh),
        ])
    }

    fn schedule_window_state(&mut self, id: desktop_runtime::window::Id) -> Task<Message> {
        self.window_state_revision = self.window_state_revision.wrapping_add(1);
        if let Some(handle) = self.window_state_debounce.take() {
            handle.abort();
        }
        if self.exiting {
            return Task::none();
        }
        let revision = self.window_state_revision;
        let (task, handle) = Task::perform(
            async { tokio::time::sleep(Duration::from_millis(350)).await },
            move |_| Message::CaptureWindowState(revision, id),
        )
        .abortable();
        self.window_state_debounce = Some(handle);
        task
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
        let size = desktop_runtime::Size::new(
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

    fn sites_running(&self) -> bool {
        !self.site_busy.is_empty() || !self.site_queue.is_empty()
    }

    fn can_check_sites(&self) -> bool {
        self.snapshot.running && !self.working && !self.exiting && !self.sites_running()
    }

    fn ports_dirty(&self) -> bool {
        self.controller_port.trim().parse::<u16>() != Ok(self.settings.controller_port)
            || self.mixed_port.trim().parse::<u16>() != Ok(self.settings.mixed_port)
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

fn read_window_state(
    id: desktop_runtime::window::Id,
) -> Task<(desktop_runtime::Size, bool, Option<bool>)> {
    desktop_runtime::window::is_maximized(id).then(move |maximized| {
        desktop_runtime::window::is_minimized(id).then(move |minimized| {
            desktop_runtime::window::size(id).map(move |size| (size, maximized, minimized))
        })
    })
}

fn verify_window_size(
    revision: u64,
    size: desktop_runtime::Size,
    id: desktop_runtime::window::Id,
    attempt: u8,
) -> Task<Message> {
    Task::perform(
        async { tokio::time::sleep(Duration::from_millis(100)).await },
        move |_| Message::VerifyWindowSize(revision, size, id, attempt),
    )
}

fn window_size_matches(desired: desktop_runtime::Size, actual: desktop_runtime::Size) -> bool {
    actual.width.is_finite()
        && actual.height.is_finite()
        && (desired.width - actual.width).abs() <= 1.0
        && (desired.height - actual.height).abs() <= 1.0
}

#[cfg(test)]
mod window_tests {
    use super::*;

    fn app() -> App {
        // Exercise real message handling without loading a store, starting the
        // core, or executing asynchronous update/network tasks.
        App::initial_state(Err(anyhow::anyhow!("geometry test")), None)
    }

    #[test]
    fn old_native_resize_does_not_cancel_restore_default() {
        let mut app = app();
        let id = desktop_runtime::window::Id::unique();
        let _ = app.update(Message::ResetWindowSize);
        let revision = app.size_revision;
        let _ = app.update(Message::Window((
            id,
            desktop_runtime::window::Event::Resized(desktop_runtime::Size::new(1200.0, 700.0)),
        )));
        assert_eq!(app.size_revision, revision);
        assert!(app.size_pending);
        assert_eq!(app.window_width, "950");
        let _ = app.update(Message::ResizeWindow(
            revision,
            DEFAULT_WINDOW_SIZE,
            Some(id),
        ));
        assert!(app.size_pending);
        let _ = app.update(Message::WindowSizeConfirmed(
            revision,
            DEFAULT_WINDOW_SIZE,
            id,
            0,
            DEFAULT_WINDOW_SIZE,
            false,
            Some(false),
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
        let id = desktop_runtime::window::Id::unique();
        let _ = app.update(Message::WindowWidth("1400".into()));
        let obsolete = app.size_revision;
        let _ = app.update(Message::ResetWindowSize);
        for window in [None, Some(id)] {
            let _ = app.update(Message::ResizeWindow(
                obsolete,
                desktop_runtime::Size::new(1400.0, 700.0),
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
            desktop_runtime::Size::ZERO,
            desktop_runtime::Size::new(f32::NAN, 700.0),
            desktop_runtime::Size::new(950.0, f32::INFINITY),
        ] {
            let _ = app.update(Message::WindowSize(size));
            assert_eq!(
                (app.window_width.as_str(), app.window_height.as_str()),
                ("950", "700")
            );
        }
    }

    #[test]
    fn stale_or_maximized_geometry_cannot_replace_last_normal_dimensions() {
        let mut app = app();
        let normal = desktop_runtime::Size::new(900.0, 500.0);
        let _ = app.update(Message::WindowStateCaptured(0, normal, false, Some(false)));
        assert_eq!(app.window_state.size(), normal);
        let _ = app.schedule_window_state(desktop_runtime::window::Id::unique());
        let current = app.window_state_revision;
        let large = desktop_runtime::Size::new(1920.0, 1080.0);
        let _ = app.update(Message::WindowStateCaptured(0, large, false, Some(false)));
        let _ = app.update(Message::WindowStateCaptured(
            current,
            large,
            true,
            Some(false),
        ));
        let _ = app.update(Message::WindowStateCaptured(
            current,
            normal,
            false,
            Some(true),
        ));
        assert_eq!(app.window_state.size(), normal);
        assert_eq!(app.window_state_revision, current);
    }

    #[test]
    fn geometry_capture_during_pending_size_edit_keeps_last_normal_dimensions() {
        let mut app = app();
        let _ = app.update(Message::WindowWidth("1400".into()));
        let _ = app.update(Message::WindowStateCaptured(
            0,
            desktop_runtime::Size::new(1200.0, 700.0),
            false,
            Some(false),
        ));
        assert!(app.size_pending);
        assert_eq!(app.window_state.size(), DEFAULT_WINDOW_SIZE);
    }

    #[test]
    fn first_resize_with_provisional_dpi_is_retried_without_saving_it() {
        let mut app = app();
        let id = desktop_runtime::window::Id::unique();
        let _ = app.update(Message::ResetWindowSize);
        let revision = app.size_revision;
        let _ = app.update(Message::ResizeWindow(
            revision,
            DEFAULT_WINDOW_SIZE,
            Some(id),
        ));
        let inflated = desktop_runtime::Size::new(1900.0, 1400.0);
        let _ = app.update(Message::Window((
            id,
            desktop_runtime::window::Event::Resized(inflated),
        )));
        let _ = app.update(Message::WindowSizeConfirmed(
            revision,
            DEFAULT_WINDOW_SIZE,
            id,
            0,
            inflated,
            false,
            Some(false),
        ));
        assert!(app.size_pending);
        assert_eq!(app.window_state.size(), DEFAULT_WINDOW_SIZE);
        assert_eq!(
            (app.window_width.as_str(), app.window_height.as_str()),
            ("950", "700")
        );
        let _ = app.update(Message::WindowSizeConfirmed(
            revision,
            DEFAULT_WINDOW_SIZE,
            id,
            1,
            DEFAULT_WINDOW_SIZE,
            false,
            Some(false),
        ));
        assert!(!app.size_pending);
        assert_eq!(app.window_state.size(), DEFAULT_WINDOW_SIZE);
    }

    #[test]
    fn obsolete_resize_confirmation_cannot_finish_a_newer_edit() {
        let mut app = app();
        let id = desktop_runtime::window::Id::unique();
        let _ = app.update(Message::ResetWindowSize);
        let obsolete = app.size_revision;
        let _ = app.update(Message::WindowWidth("1200".into()));
        let _ = app.update(Message::WindowSizeConfirmed(
            obsolete,
            DEFAULT_WINDOW_SIZE,
            id,
            0,
            DEFAULT_WINDOW_SIZE,
            false,
            Some(false),
        ));
        assert!(app.size_pending);
        assert_eq!(app.window_width, "1200");
    }

    #[test]
    fn resize_confirmation_tolerates_fractional_dpi_rounding() {
        assert!(window_size_matches(
            DEFAULT_WINDOW_SIZE,
            desktop_runtime::Size::new(950.4, 699.6)
        ));
        assert!(!window_size_matches(
            DEFAULT_WINDOW_SIZE,
            desktop_runtime::Size::new(1425.0, 1050.0)
        ));
        assert!(!window_size_matches(
            DEFAULT_WINDOW_SIZE,
            desktop_runtime::Size::new(f32::NAN, 700.0)
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn background_show_opens_only_one_window_with_saved_logical_geometry() {
        let mut app = app();
        let normal = desktop_runtime::Size::new(1100.0, 650.0);
        app.window_state.capture(normal, false, None);
        app.visible = false;
        let _ = app.update(Message::ShowWindow(None));
        assert!(app.visible);
        assert!(app.window_opening);
        let _ = app.update(Message::ShowWindow(None));
        assert!(app.window_opening);
        assert_eq!(app.window_state.size(), normal);
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn connection_search_includes_sniffed_domain_and_matched_rule_payload() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("view test")), None);
        app.snapshot.connections = serde_json::from_value(serde_json::json!({"connections":[
            {"id":"sniffed", "metadata":{"host":"", "sniffHost":"mail.google.com", "destinationIP":"192.0.2.1"}, "rule":"DomainSuffix", "rulePayload":"google.com", "chains":["DIRECT"]},
            {"id":"payload", "metadata":{"host":"192.0.2.2"}, "rule":"DomainSuffix", "rulePayload":"google.com", "chains":["Proxy"]},
            {"id":"unrelated", "metadata":{"host":"example.test"}, "rule":"Match", "chains":["Proxy"]}
        ]})).unwrap();
        for (query, expected) in [
            ("MAIL.GOOGLE.COM", vec![0]),
            ("google.com", vec![0, 1]),
            ("google.com direct", vec![0]),
            ("192.0.2.2", vec![1]),
        ] {
            let _ = app.update(Message::Query(query.into()));
            {
                let _view = app.connection_rows();
            }
            assert_eq!(app.connection_order.borrow().as_ref().unwrap(), &expected);
        }
    }

    #[test]
    fn connection_sort_applies_to_all_filtered_rows_before_pagination() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("view test")), None);
        app.snapshot.connections = serde_json::from_value(serde_json::json!({"connections":
            (0..125).map(|index| serde_json::json!({"id": index.to_string(),
                "metadata": {"host": if index == 124 { "other.test" } else { "example.test" }, "process": "hidden-process"},
                "download": index, "rule":"MATCH"})).collect::<Vec<_>>()
        })).unwrap();
        let _ = app.update(Message::Query("example.test".into()));
        let _ = app.update(Message::ConnectionSort(ConnectionField::DownloadTotal));
        {
            let _view = app.connection_rows();
        }
        {
            let order = app.connection_order.borrow();
            let order = order.as_ref().unwrap();
            assert_eq!(order.len(), 124);
            assert_eq!(order[0], 123);
            assert_eq!(order[PAGE_SIZE], 63);
        }
        let _ = app.update(Message::ConnectionSort(ConnectionField::DownloadTotal));
        {
            let _view = app.connection_rows();
        }
        assert_eq!(app.connection_order.borrow().as_ref().unwrap()[0], 0);
        let _ = app.update(Message::Query("hidden-process".into()));
        {
            let _view = app.connection_rows();
        }
        assert!(app.connection_order.borrow().as_ref().unwrap().is_empty());
    }

    #[test]
    fn proxy_widget_count_is_bounded_across_many_expanded_groups() {
        fn nodes(groups: usize) -> usize {
            let mut app = App::initial_state(Err(anyhow::anyhow!("view test")), None);
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
            let (_, groups) = app.proxy_rows();
            assert!(groups.len() <= GROUP_PAGE_SIZE);
            groups
                .iter()
                .map(|(_, pairs)| {
                    pairs
                        .iter()
                        .map(|pair| usize::from(pair.left.valid) + usize::from(pair.right.valid))
                        .sum::<usize>()
                })
                .sum::<usize>()
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
        App::initial_state(Err(anyhow::anyhow!("state test")), None)
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
            profile_edit: false,
            profile_import: false,
            scope: Scope::Home,
            start_requested: true,
            settings: app.settings.clone(),
            profiles: None,
            rule_document: None,
            rules_applied: false,
            snapshot: Some(Snapshot {
                running: true,
                ..Snapshot::default()
            }),
            notice: Ok("内核已启动".into()),
            running: true,
            geo_status: String::new(),
            autostart: false,
            protection_changed: false,
            tray_nodes: None,
        })));
        assert!(app.core_failure.is_none());
        let _ = app.update(Message::RestartCore);
        assert!(!app.busy && app.queued_actions.is_empty());
    }
    #[test]
    fn settings_errors_preserve_the_original_core_failure() {
        let mut app = app();
        app.core_failure = Some("内核程序被系统阻止".into());
        let _ = app.update(Message::Finished(Box::new(Reply {
            profile_edit: false,
            profile_import: false,
            scope: Scope::Other,
            start_requested: false,
            settings: app.settings.clone(),
            profiles: None,
            rule_document: None,
            rules_applied: false,
            snapshot: None,
            notice: Err("无法修改开机启动登录任务：拒绝访问 (0x80070005)".into()),
            running: false,
            geo_status: String::new(),
            autostart: false,
            protection_changed: false,
            tray_nodes: None,
        })));
        assert_eq!(app.core_failure.as_deref(), Some("内核程序被系统阻止"));
        assert!(app.notice.contains("开机启动"));
        assert!(app.error && app.notice_visible);
        let _ = app.update(Message::Finished(Box::new(Reply {
            profile_edit: false,
            profile_import: false,
            scope: Scope::Home,
            start_requested: true,
            settings: app.settings.clone(),
            profiles: None,
            rule_document: None,
            rules_applied: false,
            snapshot: None,
            notice: Err("无法启动内核程序：拒绝访问".into()),
            running: false,
            geo_status: String::new(),
            autostart: false,
            protection_changed: false,
            tray_nodes: None,
        })));
        assert_eq!(
            app.core_failure.as_deref(),
            Some("无法启动内核程序：拒绝访问")
        );
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
        let _ = app.update(Message::SavePorts);
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
        App::initial_state(Err(anyhow::anyhow!("isolated dev.2 test")), None)
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
        assert!(app.site_show_results);
        assert!(app.site_matches(2, ""));
        assert_eq!(
            ip_report::Report::from_results(&app.site_results)
                .totals
                .completed(),
            2
        );
    }

    #[test]
    fn report_navigation_and_filter_reset_keep_results_and_category_context() {
        let mut app = app();
        app.site_results
            .resize_with(ip_check::services().len(), || None);
        app.site_results[1] = Some(result(ip_check::State::Confirmed, "美国"));
        let _ = app.update(Message::SiteSummaryFilter(
            CategoryFilter::Group("AI"),
            StatusFilter::All,
        ));
        let _ = app.update(Message::Query("chatgpt".into()));
        let _ = app.update(Message::SiteStatus(StatusFilter::Status(Status::Available)));
        let _ = app.update(Message::SiteRegion(RegionFilter::Country("US")));
        let _ = app.update(Message::SiteResetFilters);
        assert_eq!(app.site_category, CategoryFilter::Group("AI"));
        assert!(app.site_show_results);
        assert!(app.query.is_empty());
        assert_eq!(app.site_status, StatusFilter::All);
        assert_eq!(app.site_region, RegionFilter::All);
        assert_eq!(
            ip_report::Report::from_results(&app.site_results)
                .totals
                .completed(),
            1
        );
        let _ = app.update(Message::SiteReport);
        assert!(!app.site_show_results);
        assert_eq!(app.site_category, CategoryFilter::All);
        assert_eq!(
            ip_report::Report::from_results(&app.site_results)
                .totals
                .completed(),
            1
        );
    }

    #[test]
    fn category_retest_preserves_other_results_and_return_does_not_cancel_running_checks() {
        let mut app = app();
        app.snapshot.running = true;
        app.site_results = ip_check::services()
            .iter()
            .map(|_| Some(result(ip_check::State::Confirmed, "美国")))
            .collect();
        let _ = app.update(Message::SiteSummaryFilter(
            CategoryFilter::Group("AI"),
            StatusFilter::All,
        ));
        let _ = app.update(Message::SiteRetest);
        assert!(app.site_show_results);
        assert!(app.site_has_run);
        assert_eq!(app.site_category, CategoryFilter::Group("AI"));
        for (index, service) in ip_check::services().iter().enumerate() {
            assert_eq!(app.site_result(index).is_none(), service.group == "AI");
        }
        let checking: BTreeSet<_> = app
            .site_busy
            .iter()
            .chain(app.site_queue.iter())
            .cloned()
            .collect();
        assert_eq!(checking.len(), 5);
        assert!(
            ip_check::services()
                .iter()
                .filter(|service| checking.contains(&service.id))
                .all(|service| service.group == "AI")
        );
        let generation = app.site_generation;
        let _ = app.update(Message::SiteReport);
        assert!(!app.site_show_results);
        assert!(app.sites_running());
        assert_eq!(generation, app.site_generation);
        let _ = app.update(Message::SiteCancel);
        assert!(!app.sites_running());
        let _ = app.update(Message::SiteDone(
            generation,
            ip_check::services()[1].id.clone(),
            result(ip_check::State::Confirmed, "美国"),
        ));
        assert!(app.site_result(1).is_none());
    }

    #[test]
    fn one_key_check_opens_live_report_and_disabled_core_keeps_existing_results() {
        let mut app = app();
        app.site_results
            .push(Some(result(ip_check::State::Identified, "美国")));
        app.site_show_results = true;
        let _ = app.update(Message::SiteAll);
        assert!(app.site_show_results);
        assert_eq!(app.site_results.len(), 1);
        app.snapshot.running = true;
        let _ = app.update(Message::SiteAll);
        assert!(!app.site_show_results);
        assert!(app.site_has_run);
        assert!(app.sites_running());
        assert!(app.site_results.is_empty());
        assert_eq!(app.site_category, CategoryFilter::All);
        assert_eq!(
            app.site_busy.len() + app.site_queue.len(),
            ip_check::services().len()
        );
        let generation = app.site_generation;
        let _ = app.update(Message::SiteAll);
        assert_eq!(generation, app.site_generation);
        app.cancel_site_checks();
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
        drop(app.proxy_rows());
        assert_eq!(app.proxy_order.borrow().len(), GROUP_PAGE_SIZE);
        let old_pointer = app.proxy_order.borrow().values().next().unwrap().as_ptr();
        drop(app.proxy_rows());
        assert_eq!(
            app.proxy_order.borrow().values().next().unwrap().as_ptr(),
            old_pointer
        );
        app.group_offsets.insert("group-0".into(), 60);
        let _ = app.update(Message::NodeSort(NodeSort::Name));
        assert!(app.group_offsets.is_empty());
        assert!(app.proxy_order.borrow().is_empty());
        drop(app.proxy_rows());
        assert_eq!(app.proxy_order.borrow().len(), GROUP_PAGE_SIZE);
        let _ = app.update(Message::Navigate(Page::Home));
        assert!(app.proxy_order.borrow().is_empty());
    }

    #[test]
    fn old_settings_receive_default_sort_and_new_preferences_round_trip() {
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json.as_object_mut().unwrap().remove("node_sort");
        let settings: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.node_sort, NodeSort::Name);
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
