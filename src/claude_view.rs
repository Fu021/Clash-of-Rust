//! Native Iced region report with automatic browser sampling and client guides.
use super::*;
use clash_of_rust::claude_check::{
    self as check, Environment, Evidence, Group, Report, Route, Signal, browser::Session,
};
use iced::widget::column;
use std::time::Instant;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Overview,
    Web,
    Desktop,
    Code,
}
impl Tab {
    const ALL: [Self; 4] = [Self::Overview, Self::Web, Self::Desktop, Self::Code];
    fn name(self) -> &'static str {
        match self {
            Self::Overview => "总览",
            Self::Web => "网页版",
            Self::Desktop => "Desktop",
            Self::Code => "Code",
        }
    }
    fn environment(self) -> Option<Environment> {
        match self {
            Self::Overview => None,
            Self::Web => Some(Environment::Web),
            Self::Desktop => Some(Environment::Desktop),
            Self::Code => Some(Environment::Code),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guide {
    Clients,
    Code,
    Desktop,
}
#[derive(Debug, Clone, Copy)]
pub enum SessionAction {
    Browser,
    CopyCode,
    AutoCode,
}
#[derive(Debug, Clone)]
pub enum Event {
    Start,
    Cancel,
    Tab(Tab),
    Detail(Option<Group>),
    Guide(Option<Guide>),
    Browser,
    BrowserOpened(u64, Result<(), String>),
    CodeCommand,
    CodeAuto,
    CodeDirectory(String),
    Terminal(u64, Result<Arc<check::terminal::Launch>, String>),
    CloseSession,
    Session(u64, SessionAction, Result<Session, String>),
    Local(u64, Report),
    Network(u64, Result<Report, String>),
    CopySiteScript,
    Import,
    Clipboard(u64, Option<String>),
    Imported(u64, Result<Report, String>),
    Export,
    Completed(Result<String, String>),
    Monitor(bool),
    MonitorStarted(u64, Result<check::observe::Session, String>),
    Tick,
}

#[derive(Default)]
pub struct State {
    pub tab: Tab,
    pub detail: Option<Group>,
    pub guide: Option<Guide>,
    pub reports: [Option<Report>; 4],
    pub session: Option<Session>,
    pub monitor_enabled: bool,
    monitor_starting: bool,
    monitor: Option<check::observe::Session>,
    monitor_generation: u64,
    monitor_revision: u64,
    monitor_key: Option<(u16, String)>,
    monitor_status: String,
    generation: u64,
    aborters: Vec<AbortHandle>,
    local_busy: bool,
    network_busy: bool,
    browser_busy: bool,
    code_busy: bool,
    code_deadline: Option<Instant>,
    terminal_starting: bool,
    terminal: Option<Arc<check::terminal::Launch>>,
    project_dir: String,
    importing: bool,
    session_starting: bool,
    pub status: String,
    pub error: bool,
    command: String,
}
fn index(environment: Environment) -> usize {
    match environment {
        Environment::Core => 0,
        Environment::Web => 1,
        Environment::Desktop => 2,
        Environment::Code => 3,
    }
}
impl State {
    fn differs(&self, report: &Report, signal: &Signal) -> bool {
        report.differs(signal)
            || (!signal.country.is_empty()
                && self.reports[index(Environment::Core)]
                    .as_ref()
                    .and_then(Report::baseline_country)
                    .is_some_and(|reference| reference != signal.country))
    }
    pub fn busy(&self) -> bool {
        self.local_busy
            || self.network_busy
            || self.browser_busy
            || self.code_busy
            || self.session_starting
            || self.importing
    }
    pub fn close(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        for aborter in self.aborters.drain(..) {
            aborter.abort();
        }
        self.session = None;
        self.guide = None;
        self.command.clear();
        self.terminal = None;
        self.terminal_starting = false;
        self.code_busy = false;
        self.code_deadline = None;
        self.session_starting = false;
        self.local_busy = false;
        self.network_busy = false;
        self.browser_busy = false;
        self.importing = false;
    }
    pub fn shutdown(&mut self) {
        self.close();
        self.stop_monitor();
    }
    fn stop_monitor(&mut self) {
        self.monitor_enabled = false;
        if let Some(snapshot) = self.monitor.as_ref().and_then(|s| s.after(0)) {
            self.monitor_report(snapshot);
        }
        self.monitor = None;
        self.monitor_starting = false;
        self.monitor_key = None;
        self.monitor_generation = self.monitor_generation.wrapping_add(1);
        self.monitor_status = "实时检测已停止，记录保留。".into();
    }
    fn store(&mut self, mut report: Report) {
        if report.environment == Environment::Web {
            self.browser_busy = false;
        }
        if report.environment == Environment::Code {
            self.code_busy = false;
            self.code_deadline = None;
        }
        if report.environment == Environment::Desktop
            && self.monitor_enabled
            && let Some(previous) = &mut self.reports[index(Environment::Desktop)]
        {
            report.signals.extend(
                previous
                    .signals
                    .iter()
                    .filter(|s| s.title == "Claude 请求实时检测")
                    .cloned(),
            );
            report.routes = std::mem::take(&mut previous.routes);
        }
        let index = index(report.environment);
        self.reports[index] = Some(report);
    }
    fn sampling_status(&mut self) {
        if self.error {
            return;
        }
        self.status = if self.code_busy {
            "正在检测新终端环境，等待 Code 报告回传…"
        } else if self.local_busy || self.network_busy {
            "正在采集本机与内核地区线索…"
        } else if self.browser_busy {
            "浏览器正在自动检测，等待报告回传…"
        } else if self.reports[index(Environment::Web)].is_some() {
            "自动检测完成；可继续检测 Code 或开启 Claude 请求实时检测。"
        } else {
            "本机与内核采样完成；浏览器采样尚未完成，可重新检测。"
        }
        .into();
    }
    fn selected(&self) -> impl Iterator<Item = &Report> {
        self.reports
            .iter()
            .flatten()
            .filter(|r| self.tab.environment().is_none_or(|e| r.environment == e))
    }
    fn monitor_report(&mut self, snapshot: check::observe::Snapshot) {
        self.monitor_revision = snapshot.revision;
        self.monitor_status = format!(
            "{} · 累计 {} 条事件 · 失败 {} · 保留最近 32 条明细",
            if !self.monitor_enabled {
                "已停止"
            } else if snapshot.connected {
                "实时检测中"
            } else {
                "等待内核事件流，断开后自动重连"
            },
            snapshot.total,
            snapshot.failures
        );
        let report = self.reports[index(Environment::Desktop)].get_or_insert_with(|| {
            Report::new(Environment::Desktop, "本机系统与 Claude 域名实时连接事件")
        });
        report.generated_at = chrono::Utc::now().to_rfc3339();
        report.routes = snapshot.routes.into_iter().rev().collect();
        report
            .signals
            .retain(|signal| signal.title != "Claude 请求实时检测");
        report.signals.push(if report.routes.is_empty() {
            Signal::unknown(Group::Desktop, "Claude 请求实时检测", "内核实时事件流", "尚未捕获内核识别到的 Claude 域名连接；请求可能未经过此内核或没有产生连接事件，不能当作已确认直连。")
        } else {
            Signal::new(Group::Desktop, "Claude 请求实时检测", self.monitor_status.clone(), "内核实时连接事件，适用于网页、Desktop 与 Code", Evidence::Measured, "记录内核识别到的 Claude 域名、时间、匹配规则、线路与连接结果。不会读取 HTTPS 请求内容；不按进程识别，不据此推断远端出口国家。")
        });
    }
}

impl App {
    fn start_claude_monitor(&mut self) -> Task<Message> {
        self.claude.monitor_generation = self.claude.monitor_generation.wrapping_add(1);
        self.claude.monitor = None;
        self.claude.monitor_revision = 0;
        self.claude.monitor_starting = true;
        self.claude.monitor_key =
            Some((self.settings.controller_port, self.settings.secret.clone()));
        self.claude.monitor_status = "正在连接内核实时事件流…".into();
        let generation = self.claude.monitor_generation;
        let settings = self.settings.clone();
        Task::perform(
            async move {
                clash_of_rust::api::Api::new(&settings)
                    .map(check::observe::Session::start)
                    .map_err(|e| e.to_string())
            },
            move |result| Message::Claude(Event::MonitorStarted(generation, result)),
        )
    }
    pub(crate) fn claude_event(&mut self, event: Event) -> Task<Message> {
        let wrap = |event| Message::Claude(event);
        match event {
            Event::Start => {
                if self.claude.busy() || self.exiting {
                    return Task::none();
                }
                self.claude.close();
                self.claude.reports = Default::default();
                self.claude.monitor_revision = 0;
                self.claude.error = false;
                self.claude.status = "正在采集地区线索，并自动打开浏览器检测…".into();
                self.claude.guide = Some(Guide::Clients);
                self.claude.local_busy = true;
                self.claude.network_busy = true;
                let generation = self.claude.generation;
                let local =
                    Task::perform(check::system::local(Environment::Desktop), move |report| {
                        wrap(Event::Local(generation, report))
                    });
                let port = self.snapshot.running.then_some(self.settings.mixed_port);
                let ipv6 = self.settings.ipv6;
                let dns_override = self.settings.dns_override;
                let (aborter, registration) = AbortHandle::new_pair();
                self.claude.aborters.push(aborter);
                let network = Task::perform(
                    async move {
                        Abortable::new(async move {
                        let mut report = Report::new(Environment::Core, "显式通过本应用内核探测；不代表浏览器或其他客户端实际出口");
                        report.signals.push(Signal::new(Group::Dns, "客户端配置", format!("DNS 覆盖{} · IPv6 {}", if dns_override { "开启" } else { "关闭" }, if ipv6 { "开启" } else { "关闭" }), "本应用设置", Evidence::Measured, "配置状态与 DNS、IPv6 实测分别展示。"));
                        if let Some(port) = port {
                            let client = check::client(Some(port)).map_err(|e| e.to_string())?;
                            check::network(&client, &mut report, "通过内核混合端口的探测请求；不会回退到直连").await;
                            report.signals.extend(check::system_dns(&client, "本机系统解析器；结果查询通过内核").await);
                        } else {
                            report.signals.push(Signal::unknown(Group::Exit, "内核出口", "内核探测", "内核未运行，没有回退到直连；仍可检测浏览器和 Code 的实际环境。"));
                        }
                        Ok::<_, String>(report)
                    }, registration).await.unwrap_or_else(|_| Err("检测已取消".into()))
                    },
                    move |result| wrap(Event::Network(generation, result)),
                );
                let browser = self.claude_event(Event::Browser);
                return Task::batch([local, network, browser]);
            }
            Event::Cancel => {
                self.claude.close();
                self.claude.status = "已停止检测，已获得的报告保留。".into();
            }
            Event::Tab(tab) => {
                self.claude.tab = tab;
                self.claude.detail = None;
            }
            Event::Detail(group) => self.claude.detail = group,
            Event::CodeDirectory(value) => self.claude.project_dir = value,
            Event::Guide(guide) => {
                self.claude.guide = guide;
                match guide {
                    Some(Guide::Code) => self.claude.tab = Tab::Code,
                    Some(Guide::Desktop) => self.claude.tab = Tab::Desktop,
                    _ => {}
                }
                self.claude.detail = None;
                if guide == Some(Guide::Code) {
                    return iced::widget::operation::focus("claude-code-directory");
                }
            }
            Event::Local(generation, report) if generation == self.claude.generation => {
                self.claude.local_busy = false;
                self.claude.store(report);
                self.claude.sampling_status();
            }
            Event::Network(generation, result) if generation == self.claude.generation => {
                self.claude.network_busy = false;
                match result {
                    Ok(report) => {
                        self.claude.store(report);
                        self.claude.sampling_status();
                    }
                    Err(error) => {
                        self.claude.status = error;
                        self.claude.error = true;
                    }
                }
            }
            Event::Browser | Event::CodeCommand | Event::CodeAuto => {
                if self.exiting
                    || self.claude.session_starting
                    || (matches!(event, Event::CodeAuto) && self.claude.terminal_starting)
                {
                    return Task::none();
                }
                let action = match event {
                    Event::CodeCommand => SessionAction::CopyCode,
                    Event::CodeAuto => SessionAction::AutoCode,
                    _ => SessionAction::Browser,
                };
                if matches!(action, SessionAction::Browser) {
                    self.claude.browser_busy = true;
                } else if matches!(action, SessionAction::AutoCode) && !self.claude.code_busy {
                    self.claude.code_busy = true;
                    self.claude.code_deadline = Some(Instant::now() + Duration::from_secs(120));
                    self.claude.reports[index(Environment::Code)] = None;
                    self.claude.error = false;
                }
                if let Some(session) = self.claude.session.as_ref().filter(|s| !s.finished()) {
                    if !matches!(action, SessionAction::Browser) {
                        match std::env::current_exe() {
                            Ok(executable) => {
                                if matches!(action, SessionAction::AutoCode) {
                                    let submit = session.submit_url();
                                    let directory = self.claude.project_dir.clone();
                                    let generation = self.claude.generation;
                                    self.claude.terminal_starting = true;
                                    self.claude.status = "正在打开终端，自动检测 Code 环境…".into();
                                    return Task::perform(
                                        async move {
                                            check::terminal::launch(check::terminal::Request {
                                                executable: &executable,
                                                submit: &submit,
                                                directory: &directory,
                                                offline: false,
                                                keep_open: true,
                                            })
                                            .await
                                            .map(Arc::new)
                                            .map_err(|e| e.to_string())
                                        },
                                        move |result| wrap(Event::Terminal(generation, result)),
                                    );
                                }
                                self.claude.command = check::system::terminal_command(
                                    &executable,
                                    &session.submit_url(),
                                );
                                self.claude.status =
                                    "命令已复制；请在启动 Code 的同一终端、同一项目目录运行。"
                                        .into();
                                self.claude.error = false;
                                return iced::clipboard::write(self.claude.command.clone());
                            }
                            Err(_) => {
                                self.claude.code_busy = false;
                                self.claude.status = "无法获得应用程序路径。".into();
                                self.claude.error = true;
                            }
                        }
                    } else {
                        let url = session.url.clone();
                        let generation = self.claude.generation;
                        return Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || platform::open_diagnostic(&url))
                                    .await
                                    .map_err(|e| e.to_string())?
                                    .map_err(|e| e.to_string())
                            },
                            move |result| wrap(Event::BrowserOpened(generation, result)),
                        );
                    }
                } else if !self.claude.session_starting {
                    self.claude.session_starting = true;
                    let generation = self.claude.generation;
                    let port = self.snapshot.running.then_some(self.settings.mixed_port);
                    return Task::perform(
                        async move {
                            Session::start(check::client(port).map_err(|e| e.to_string())?)
                                .await
                                .map_err(|e| e.to_string())
                        },
                        move |result| wrap(Event::Session(generation, action, result)),
                    );
                }
            }
            Event::BrowserOpened(generation, result)
                if generation == self.claude.generation
                    && self.claude.browser_busy
                    && self.claude.session.is_some() =>
            {
                match result {
                    Ok(()) => self.claude.sampling_status(),
                    Err(error) => {
                        self.claude.browser_busy = false;
                        self.claude.status = format!("无法自动打开浏览器：{error}");
                        self.claude.error = true;
                    }
                }
            }
            Event::Session(generation, action, result)
                if generation == self.claude.generation && self.claude.session_starting =>
            {
                self.claude.session_starting = false;
                match result {
                    Ok(session) => {
                        self.claude.session = Some(session);
                        return self.claude_event(match action {
                            SessionAction::CopyCode => Event::CodeCommand,
                            SessionAction::AutoCode => Event::CodeAuto,
                            SessionAction::Browser => Event::Browser,
                        });
                    }
                    Err(error) => {
                        if matches!(action, SessionAction::Browser) {
                            self.claude.browser_busy = false;
                        } else if matches!(action, SessionAction::AutoCode) {
                            self.claude.code_busy = false;
                        }
                        self.claude.status = error;
                        self.claude.error = true;
                    }
                }
            }
            Event::Terminal(generation, result)
                if generation == self.claude.generation
                    && self.claude.terminal_starting
                    && self.claude.session.is_some() =>
            {
                self.claude.terminal_starting = false;
                match result {
                    Ok(launch) => {
                        self.claude.terminal = Some(launch);
                        if self.claude.code_busy {
                            self.claude.sampling_status();
                        }
                    }
                    Err(error) => {
                        self.claude.code_busy = false;
                        self.claude.status =
                            format!("自动终端检测失败：{error}。可复制命令手动检测。");
                        self.claude.error = true;
                    }
                }
            }
            Event::CloseSession => {
                self.claude.session = None;
                self.claude.browser_busy = false;
                self.claude.code_busy = false;
                self.claude.session_starting = false;
                self.claude.terminal_starting = false;
                self.claude.terminal = None;
                self.claude.command.clear();
                self.claude.status = "浏览器与终端回传入口已关闭。".into();
            }
            Event::CopySiteScript => {
                self.claude.status = "脚本已复制：在 Claude 网页的开发者工具 Console 中运行，再导入脚本复制的 JSON。脚本不读取账号或 Cookie。".into();
                return iced::clipboard::write(check::browser::site_script());
            }
            Event::Import => {
                let generation = self.claude.generation;
                return iced::clipboard::read()
                    .map(move |text| wrap(Event::Clipboard(generation, text)));
            }
            Event::Clipboard(generation, text) if generation == self.claude.generation => {
                let Some(text) = text.filter(|s| s.len() <= check::REPORT_LIMIT) else {
                    self.claude.status = "剪贴板为空或内容超过 64 KiB；请复制采样 JSON。".into();
                    self.claude.error = true;
                    return Task::none();
                };
                self.claude.importing = true;
                let port = self.snapshot.running.then_some(self.settings.mixed_port);
                return Task::perform(
                    async move {
                        let json: serde_json::Value = serde_json::from_str(&text)
                            .map_err(|_| "剪贴板不是有效的检测 JSON".to_string())?;
                        if json.get("kind").and_then(|v| v.as_str()) == Some("browser") {
                            let data = serde_json::from_value(json)
                                .map_err(|_| "浏览器采样格式无效".to_string())?;
                            check::browser::report(
                                data,
                                &check::client(port).map_err(|e| e.to_string())?,
                            )
                            .await
                            .map_err(|e| e.to_string())
                        } else {
                            let report: Report = serde_json::from_value(json)
                                .map_err(|_| "检测报告格式无效".to_string())?;
                            if report.environment != Environment::Code {
                                return Err("请导入浏览器采样或 Code 终端报告".into());
                            }
                            report.validate().map_err(|e| e.to_string())?;
                            Ok(report)
                        }
                    },
                    move |result| wrap(Event::Imported(generation, result)),
                );
            }
            Event::Imported(generation, result) if generation == self.claude.generation => {
                self.claude.importing = false;
                match result {
                    Ok(report) => {
                        self.claude.store(report);
                        self.claude.error = false;
                        self.claude.status = "采样报告已导入；来源与实测范围见各项详情。".into();
                    }
                    Err(error) => {
                        self.claude.status = error;
                        self.claude.error = true;
                    }
                }
            }
            Event::Export => {
                let Some(engine) = self.engine.clone() else {
                    return Task::none();
                };
                let bytes = serde_json::to_vec_pretty(
                    &serde_json::json!({"schema":1,"version":clash_of_rust::VERSION,"reports":self.claude.reports}),
                );
                return Task::perform(
                    async move {
                        let bytes = bytes.map_err(|e| e.to_string())?;
                        let path =
                            engine
                                .lock()
                                .await
                                .store
                                .root
                                .join("diagnostics")
                                .join(format!(
                                    "claude-region-{}.json",
                                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                                ));
                        tokio::task::spawn_blocking(move || {
                            clash_of_rust::config::atomic_write(&path, &bytes)?;
                            Ok::<_, anyhow::Error>(format!("报告已导出：{}", path.display()))
                        })
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())
                    },
                    move |result| wrap(Event::Completed(result)),
                );
            }
            Event::Completed(result) => match result {
                Ok(message) => {
                    self.claude.status = message;
                    self.claude.error = false;
                }
                Err(message) => {
                    self.claude.status = message;
                    self.claude.error = true;
                }
            },
            Event::Monitor(enabled) => {
                if self.exiting {
                    return Task::none();
                }
                if !enabled {
                    self.claude.stop_monitor();
                } else if self.snapshot.running {
                    self.claude.monitor_enabled = true;
                    self.claude.guide = Some(Guide::Desktop);
                    self.claude.tab = Tab::Desktop;
                    self.claude.detail = None;
                    return self.start_claude_monitor();
                } else {
                    self.claude.guide = Some(Guide::Desktop);
                    self.claude.monitor_status = "请先启动内核，再开启实时检测。".into();
                }
            }
            Event::MonitorStarted(generation, result)
                if generation == self.claude.monitor_generation
                    && self.claude.monitor_enabled
                    && self.claude.monitor_starting =>
            {
                self.claude.monitor_starting = false;
                match result {
                    Ok(session) => self.claude.monitor = Some(session),
                    Err(error) => {
                        self.claude.stop_monitor();
                        self.claude.monitor_status = format!("无法启动实时检测：{error}");
                    }
                }
            }
            Event::Tick => {
                if self.exiting {
                    self.claude.shutdown();
                    return Task::none();
                }
                if self.claude.monitor_enabled {
                    let key = (self.settings.controller_port, self.settings.secret.clone());
                    if self.snapshot.running && self.claude.monitor_key.as_ref() != Some(&key) {
                        return self.start_claude_monitor();
                    }
                    if let Some(snapshot) = self
                        .claude
                        .monitor
                        .as_ref()
                        .and_then(|s| s.after(self.claude.monitor_revision))
                    {
                        self.claude.monitor_report(snapshot);
                    }
                }
                if !self.visible || self.page != Page::Claude {
                    self.claude.close();
                    return Task::none();
                }
                if self.claude.code_busy
                    && self
                        .claude
                        .code_deadline
                        .is_some_and(|t| Instant::now() >= t)
                {
                    self.claude.code_busy = false;
                    self.claude.code_deadline = None;
                    self.claude.status =
                        "终端未及时回传。请查看已打开的终端，或复制命令在实际 Code 终端中重试。"
                            .into();
                    self.claude.error = true;
                }
                if let Some(session) = &self.claude.session {
                    if let Some(report) = session.next_report() {
                        let web = report.environment == Environment::Web;
                        self.claude.store(report);
                        self.claude.error = false;
                        if web {
                            self.claude.sampling_status();
                        } else {
                            self.claude.status = "已收到 Code 终端的采样报告。".into();
                        }
                    } else if session.finished() {
                        self.claude.session = None;
                        self.claude.browser_busy = false;
                        self.claude.code_busy = false;
                        self.claude.terminal = None;
                        self.claude.command.clear();
                        self.claude.status = "回传入口已到期，请重新打开。".into();
                    }
                }
            }
            _ => {}
        }
        Task::none()
    }

    pub(crate) fn claude_view(&self) -> Element<'_, Message> {
        let event = |event| Message::Claude(event);
        let state = &self.claude;
        let mut tabs = aligned_row![].spacing(5);
        for tab in Tab::ALL {
            tabs = tabs.push(
                button(self.label(tab.name()).size(self.font_size(13)))
                    .style(if state.tab == tab {
                        ui_style::selected_button
                    } else {
                        ui_style::secondary_button
                    })
                    .padding([7, 11])
                    .on_press(event(Event::Tab(tab))),
            );
        }
        let has_reports = state.reports.iter().any(Option::is_some);
        let controls = aligned_row![
            tabs,
            Space::new().width(Length::Fill),
            button(
                self.label(if has_reports {
                    "重新检测"
                } else {
                    "开始检测"
                })
                .size(self.font_size(13))
            )
            .style(ui_style::primary)
            .padding([7, 12])
            .on_press_maybe((!state.busy()).then_some(event(Event::Start))),
            button(self.label("导出报告").size(self.font_size(13)))
                .style(ui_style::secondary_button)
                .padding([7, 10])
                .on_press_maybe(
                    (has_reports && self.engine.is_some()).then_some(event(Event::Export))
                )
        ]
        .spacing(8);
        let mut body = column![controls].spacing(10);
        if state.busy() {
            let progress = if state.code_busy {
                0.0
            } else {
                (usize::from(!state.local_busy)
                    + usize::from(!state.network_busy)
                    + usize::from(!state.browser_busy && !state.session_starting))
                    as f32
                    / 3.0
            };
            body = body
                .push(aligned_row![
                    self.caption(if state.code_busy {
                        "正在检测新终端环境…"
                    } else {
                        "正在检测地区线索…"
                    }),
                    Space::new().width(Length::Fill),
                    button(self.label("停止").size(self.font_size(12)))
                        .style(ui_style::text_button)
                        .on_press(event(Event::Cancel))
                ])
                .push(progress_bar(0.0..=1.0, progress).girth(4));
        }
        if !state.status.is_empty() {
            body = body.push(self.label(&state.status).size(self.font_size(12)).color(
                if state.error {
                    ui_style::tone(&self.theme(), ui_style::Tone::Danger)
                } else {
                    self.secondary()
                },
            ));
        }
        let (measured, differences, unknown) = state
            .selected()
            .flat_map(|report| report.signals.iter().map(move |signal| (report, signal)))
            .fold((0, 0, 0), |sum, (report, signal)| {
                (
                    sum.0 + usize::from(signal.evidence == Evidence::Measured),
                    sum.1 + usize::from(state.differs(report, signal)),
                    sum.2 + usize::from(signal.evidence == Evidence::Unknown),
                )
            });
        let summary = if has_reports {
            format!("实测 {measured} · 环境差异 {differences} · 未确认 {unknown}")
        } else {
            "本机、内核与浏览器自动检测；Code 与实时请求按需开启".into()
        };
        body = body.push(
            container(
                column![
                    self.label("地区线索报告").size(self.font_size(16)),
                    self.label(summary).size(self.font_size(20)),
                    self.caption(
                        "分别展示实测与辅助线索；语言、字体和地区差异不直接判定服务资格。"
                    )
                ]
                .spacing(5),
            )
            .padding(13)
            .width(Length::Fill)
            .style(ui_style::panel),
        );
        let actions = aligned_row![
            button(self.label("Code 检测").size(self.font_size(12)))
                .style(ui_style::secondary_button)
                .on_press(event(Event::Guide(Some(Guide::Code)))),
            checkbox(state.monitor_enabled)
                .label("实时检测 Claude 请求")
                .text_size(self.scaled(12))
                .on_toggle_maybe(
                    (!self.exiting && (self.snapshot.running || state.monitor_enabled))
                        .then_some(|enabled| Message::Claude(Event::Monitor(enabled)))
                )
        ]
        .spacing(6);
        body = body.push(actions);
        if !state.monitor_status.is_empty() {
            body = body.push(self.caption(&state.monitor_status));
        }
        if state.session.is_some() {
            body = body.push(aligned_row![
                self.caption("本机回传入口已开启；离开此页即关闭，最长 20 分钟。"),
                Space::new().width(Length::Fill),
                button(self.label("关闭入口").size(self.font_size(12)))
                    .style(ui_style::text_button)
                    .on_press_maybe(
                        (!state.session_starting && !state.terminal_starting)
                            .then_some(event(Event::CloseSession))
                    )
            ]);
        }
        if state.tab == Tab::Web {
            body = body.push(aligned_row![
                button(self.label("复制 Claude 页面采样脚本").size(self.font_size(12)))
                    .style(ui_style::secondary_button).on_press(event(Event::CopySiteScript)),
                button(self.label("导入剪贴板采样").size(self.font_size(12)))
                    .style(ui_style::secondary_button)
                    .on_press_maybe((!state.importing).then_some(event(Event::Import))),
                Space::new().width(Length::Fill)
            ].spacing(6)).push(self.caption("可在 Claude 网页开发者工具的 Console 运行，再导入脚本复制的 JSON，以确认该站点的时区、权限与同域出口。"));
        }
        let mut results = column![].spacing(10);
        if let Some(group) = state.detail {
            results = results.push(
                aligned_row![
                    button(self.label("‹ 返回报告").size(self.font_size(12)))
                        .style(ui_style::text_button)
                        .on_press(event(Event::Detail(None))),
                    self.label(group.name()).size(self.font_size(18))
                ]
                .spacing(10),
            );
            for report in state.selected() {
                let signals: Vec<_> = report.signals.iter().filter(|s| s.group == group).collect();
                if signals.is_empty() && !(group == Group::Desktop && !report.routes.is_empty()) {
                    continue;
                }
                results = results.push(self.caption(format!(
                    "{} · {}",
                    report.environment.name(),
                    report.generated_at
                )));
                for signal in signals {
                    let tone = if state.differs(report, signal) {
                        ui_style::Tone::Warning
                    } else if signal.evidence == Evidence::Measured {
                        ui_style::Tone::Success
                    } else {
                        ui_style::Tone::Muted
                    };
                    let mut item = column![
                        aligned_row![
                            self.label(&signal.title).size(self.font_size(15)),
                            Space::new().width(Length::Fill),
                            container(
                                self.label(if state.differs(report, signal) {
                                    "存在差异"
                                } else {
                                    signal.evidence.name()
                                })
                                .size(self.font_size(11))
                            )
                            .padding([3, 7])
                            .style(move |theme: &Theme| ui_style::badge(theme, tone))
                        ],
                        self.label(&signal.value).size(self.font_size(14)),
                        self.caption(format!("来源：{}", signal.source)),
                        self.caption(&signal.detail)
                    ]
                    .spacing(5);
                    if !signal.advice.is_empty() {
                        item = item.push(self.caption(format!("建议：{}", signal.advice)));
                    }
                    results = results.push(
                        container(item)
                            .padding(12)
                            .width(Length::Fill)
                            .style(ui_style::panel),
                    );
                }
                if group == Group::Desktop {
                    for route in &report.routes {
                        results = results.push(
                            container(
                                column![
                                    self.label(&route.host).size(self.font_size(14)),
                                    self.caption(format!(
                                        "{}{}{}",
                                        route.time,
                                        if route.time.is_empty() || route.outcome.is_empty() {
                                            ""
                                        } else {
                                            " · "
                                        },
                                        route.outcome
                                    )),
                                    self.caption(format!("规则：{}", route.rule)),
                                    self.caption(format!("线路：{}", route.chain))
                                ]
                                .spacing(4),
                            )
                            .padding(12)
                            .width(Length::Fill)
                            .style(ui_style::panel),
                        );
                    }
                }
            }
        } else {
            let groups: &[Group] = match state.tab {
                Tab::Overview => &Group::ALL,
                Tab::Web => &[Group::Exit, Group::Browser, Group::WebRtc, Group::Dns],
                Tab::Desktop => &[Group::Desktop],
                Tab::Code => &[Group::Exit, Group::Dns, Group::Code],
            };
            for pair in groups.chunks(2) {
                results = results.push(if pair.len() == 1 {
                    self.claude_card(pair[0])
                } else {
                    aligned_row![self.claude_card(pair[0]), self.claude_card(pair[1])]
                        .spacing(10)
                        .into()
                });
            }
        }
        if !has_reports {
            results = results.push(self.caption("开始检测会自动打开默认浏览器；Code 可自动打开终端采样，也可开启 Claude 请求实时检测。"));
        }
        body.push(page_scroll(results).height(Length::Fill))
            .height(Length::Fill)
            .into()
    }

    pub(crate) fn claude_dialog(&self) -> Option<Element<'_, Message>> {
        let guide = self.claude.guide?;
        let event = |event| Message::Claude(event);
        let close = button(self.label("关闭"))
            .style(ui_style::secondary_button)
            .padding([8, 12])
            .on_press(event(Event::Guide(None)));
        let mut body = match guide {
            Guide::Clients => column![
                self.title("还需完成客户端检测"),
                self.label("本机、内核和默认浏览器会自动检测。Code 可自动打开终端补充采样；开启实时请求检测后，会持续记录经过内核的 Claude 域名连接。"),
                self.caption("点击下方按钮查看操作步骤，也可以稍后从检测页继续。"),
                aligned_row![
                    button(self.label("Code 检测")).style(ui_style::primary).padding([8, 12])
                        .on_press(event(Event::Guide(Some(Guide::Code)))),
                    button(self.label("实时请求检测")).style(ui_style::primary).padding([8, 12])
                        .on_press(event(Event::Guide(Some(Guide::Desktop)))),
                    Space::new().width(Length::Fill),
                    close
                ].spacing(8)
            ],
            Guide::Code => {
                let mut content = column![
                    self.title("Code 检测"),
                    self.label("选择项目目录后，点击“自动检测”，程序会打开终端并执行检测，结果自动回传。留空使用用户目录。"),
                    text_input("项目目录（可选）", &self.claude.project_dir)
                        .id("claude-code-directory")
                        .padding(9).size(self.font_size(14)).style(ui_style::input)
                        .on_input_maybe((!self.claude.code_busy && !self.claude.session_starting)
                            .then_some(|value| Message::Claude(Event::CodeDirectory(value))))
                        .on_submit_maybe((!self.claude.code_busy && !self.claude.session_starting && !self.exiting)
                            .then_some(event(Event::CodeAuto))),
                    self.caption("新终端会加载自己的启动配置。要核对已运行的 Code，可复制命令，在启动 Code 的同一终端、同一项目目录运行，并结合 /status 查看。"),
                    self.caption("保持本应用的检测页打开。不会读取账号、Cookie 或 API 密钥。")
                ].spacing(12);
                if !self.claude.command.is_empty() {
                    content = content.push(self.caption("命令已复制到剪贴板：")).push(
                        text_input("检测命令", &self.claude.command)
                            .size(self.font_size(12)).style(ui_style::input)
                    );
                }
                if self.claude.code_busy {
                    content = content.push(self.caption("正在自动检测，等待终端回传…"));
                } else if self.claude.reports[index(Environment::Code)].is_some() {
                    content = content.push(self.caption("Code 报告已回传，可关闭此窗口查看结果。"));
                }
                content.push(aligned_row![
                    button(self.label("复制命令")).style(ui_style::secondary_button).padding([8, 12])
                        .on_press_maybe((!self.claude.session_starting && !self.exiting)
                            .then_some(event(Event::CodeCommand))),
                    Space::new().width(Length::Fill),
                    close,
                    button(self.label(if self.claude.code_busy { "正在检测…" } else { "自动检测" }))
                        .style(ui_style::primary).padding([8, 12])
                        .on_press_maybe((!self.claude.code_busy && !self.claude.session_starting && !self.exiting)
                            .then_some(event(Event::CodeAuto)))
                ].spacing(8))
            }
            Guide::Desktop => {
                let mut content = column![
                    self.title("Claude 请求实时检测"),
                    self.label("开启后，持续记录内核识别到的 Claude 域名连接，适用于网页版、Desktop 和 Code。可正常使用客户端，无需限定在 20 秒内操作。"),
                    self.label("在 Desktop 分类查看时间、域名、匹配规则、线路与连接结果。关闭后保留记录；切换页面或隐藏窗口不会停止实时检测。"),
                    self.caption("保留最近 32 条明细与累计数量。不按进程区分，也不读取 HTTPS 请求内容。"),
                    checkbox(self.claude.monitor_enabled).label("实时检测 Claude 请求")
                        .text_size(self.scaled(14))
                        .on_toggle_maybe((!self.exiting && (self.snapshot.running || self.claude.monitor_enabled))
                            .then_some(|enabled| Message::Claude(Event::Monitor(enabled))))
                ].spacing(12);
                if !self.snapshot.running {
                    content = content.push(self.caption("请先启动内核，再开始观测。"));
                }
                if !self.claude.monitor_status.is_empty() { content = content.push(self.caption(&self.claude.monitor_status)); }
                content.push(aligned_row![
                    Space::new().width(Length::Fill),
                    close
                ].spacing(8))
            }
        }.spacing(14);
        if self.claude.error {
            body = body.push(
                self.label(&self.claude.status)
                    .color(ui_style::tone(&self.theme(), ui_style::Tone::Danger)),
            );
        }
        Some(body.into())
    }

    fn claude_card(&self, group: Group) -> Element<'_, Message> {
        let reports: Vec<_> = self.claude.selected().collect();
        let entries: Vec<_> = reports
            .iter()
            .flat_map(|r| {
                r.signals
                    .iter()
                    .filter(move |s| s.group == group)
                    .map(move |s| (*r, s))
            })
            .collect();
        let differences = entries
            .iter()
            .filter(|(r, s)| self.claude.differs(r, s))
            .count();
        let measured = entries
            .iter()
            .filter(|(_, s)| s.evidence == Evidence::Measured)
            .count();
        let partial = measured < entries.len();
        let tone = if differences > 0 {
            ui_style::Tone::Warning
        } else if measured > 0 && !partial {
            ui_style::Tone::Success
        } else {
            ui_style::Tone::Muted
        };
        let status = if differences > 0 {
            "存在差异"
        } else if measured > 0 && partial {
            "部分确认"
        } else if measured > 0 {
            "已检测"
        } else {
            "待补充"
        };
        let mut lines = column![aligned_row![
            self.label(group.name()).size(self.font_size(15)),
            Space::new().width(Length::Fill),
            container(self.label(status).size(self.font_size(11)))
                .padding([3, 6])
                .style(move |theme: &Theme| ui_style::badge(theme, tone))
        ]]
        .spacing(6);
        if entries.is_empty() {
            lines = lines.push(self.caption(match group {
                Group::Browser | Group::WebRtc => "请在常用浏览器中完成检测",
                Group::Code => "复制命令，在实际 Code 终端运行",
                Group::Desktop => "检测系统线索，另行观测客户端请求",
                _ => "检测结果会标明来源与实测范围",
            }));
        }
        for (_, signal) in entries.iter().take(3) {
            lines = lines.push(self.caption(format!("{}：{}", signal.title, signal.value)));
        }
        lines = lines.push(aligned_row![
            Space::new().width(Length::Fill),
            self.label(format!("查看详情 · {} 项 ›", entries.len()))
                .size(self.font_size(12))
                .color(ui_style::tone(&self.theme(), ui_style::Tone::Accent))
        ]);
        button(container(lines).padding(10).width(Length::Fill))
            .style(ui_style::secondary_button)
            .width(Length::Fill)
            .padding(0)
            .on_press(Message::Claude(Event::Detail(Some(group))))
            .into()
    }
}

#[cfg(feature = "ui-preview")]
#[allow(dead_code)] // Production binaries compiled beside the harness do not use fixtures.
pub(crate) fn fixture_state(scene: &str) -> State {
    let mut state = State::default();
    if scene == "claude-empty" {
        return state;
    }
    state.status = "固定示例数据，用于检查界面布局。".into();
    let mut core = Report::new(Environment::Core, "示例内核探测，不代表真实检测");
    let mut ip = Signal::new(
        Group::Exit,
        "IPv4 出口",
        "203.0.113.42 · 美国",
        "示例 ipify 查询",
        Evidence::Measured,
        "仅测试域名看到的出口，不代替 Claude 目标出口。",
    );
    ip.country = "US".into();
    ip.ip = "203.0.113.42".into();
    core.signals.push(ip.clone());
    core.signals.push(Signal::unknown(
        Group::Exit,
        "IPv6 出口",
        "示例内核探测",
        "未获得有效结果，不判断地区。",
    ));
    core.signals.push(Signal::new(
        Group::Dns,
        "客户端配置",
        "DNS 覆盖开启 · IPv6 开启",
        "示例设置",
        Evidence::Measured,
        "配置状态不代表实际解析器。",
    ));
    core.signals.push(Signal::unknown(
        Group::Dns,
        "实际递归 DNS",
        "示例权威 DNS 测试",
        "权威 DNS 未返回有效记录。",
    ));
    state.store(core);
    let mut web = Report::new(Environment::Web, "示例浏览器本地页面");
    web.signals.push(ip.clone());
    web.signals.push(check::system::preference(
        Group::Browser,
        "浏览器时区",
        "Asia/Shanghai · UTC+8",
        "示例 Intl",
        Some("CN"),
    ));
    web.signals.push(check::system::preference(
        Group::Browser,
        "语言偏好",
        "zh-CN, en-US",
        "示例 navigator.languages",
        Some("CN"),
    ));
    web.signals.push(Signal::new(
        Group::Browser,
        "可识别字体",
        "Noto Sans CJK SC · Source Han Sans SC",
        "示例 Canvas",
        Evidence::Inferred,
        "中文字体全球通用，不按字体判定国家。",
    ));
    web.signals.push(Signal::unknown(
        Group::WebRtc,
        "WebRTC 公网地址",
        "示例浏览器 ICE 收集",
        "本次未获得公网候选，不能确认没有泄漏。",
    ));
    state.store(web);
    let mut desktop = Report::new(Environment::Desktop, "示例系统与用户触发的连接观测");
    desktop.signals.push(check::system::preference(
        Group::Desktop,
        "系统时区",
        "Asia/Shanghai",
        "示例系统设置",
        Some("CN"),
    ));
    desktop.signals.push(Signal::new(
        Group::Desktop,
        "系统代理",
        "http://127.0.0.1:7890/",
        "示例本机设置",
        Evidence::Measured,
        "Desktop 是否遵循此配置需要结合请求观测确认。",
    ));
    desktop.signals.push(Signal::unknown(
        Group::Desktop,
        "Desktop 内部地区环境",
        "示例客户端状态",
        "不能将系统设置直接当作客户端内部环境。",
    ));
    desktop.routes.push(Route {
        host: "claude.ai".into(),
        rule: "DOMAIN-SUFFIX claude.ai".into(),
        chain: "Claude → 美国节点1".into(),
        time: "2026-10-14T07:15:01Z".into(),
        outcome: "连接已建立".into(),
    });
    desktop.routes.push(Route {
        host: "api.anthropic.com".into(),
        rule: "MATCH".into(),
        chain: "Proxy → 美国节点1".into(),
        time: "2026-10-14T07:15:03Z".into(),
        outcome: "连接已建立".into(),
    });
    state.store(desktop);
    let mut code = Report::new(Environment::Code, "示例实际终端采样");
    code.signals.push(ip);
    code.signals.push(Signal::new(
        Group::Code,
        "HTTPS_PROXY",
        "http://127.0.0.1:7890/",
        "示例启动终端",
        Evidence::Measured,
        "代理用户名、密码与查询参数没有进入报告。",
    ));
    code.signals.push(Signal::unknown(
        Group::Code,
        "运行中 Code 会话",
        "示例内部状态",
        "请在 Code /status 中确认托管来源与启动设置。",
    ));
    state.store(code);
    match scene {
        "claude-web" => state.tab = Tab::Web,
        "claude-desktop" => state.tab = Tab::Desktop,
        "claude-code" => state.tab = Tab::Code,
        "claude-details" => state.detail = Some(Group::Browser),
        "claude-start-guide" => {
            state.guide = Some(Guide::Clients);
            state.reports = Default::default();
            state.local_busy = true;
            state.network_busy = true;
            state.browser_busy = true;
            state.status = "正在采集地区线索，并自动打开浏览器检测…".into();
        }
        "claude-code-guide" => {
            state.guide = Some(Guide::Code);
            state.tab = Tab::Code;
        }
        "claude-live" => {
            state.monitor_enabled = true;
            state.monitor_status =
                "实时检测中 · 累计 8 条事件 · 失败 0 · 保留最近 32 条明细".into();
            state.tab = Tab::Desktop;
            state.detail = Some(Group::Desktop);
        }
        "claude-desktop-guide" => {
            state.guide = Some(Guide::Desktop);
            state.tab = Tab::Desktop;
        }
        _ => {}
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        App::with_engine(Err(anyhow::anyhow!("isolated")), None).0
    }
    #[test]
    fn starting_detection_starts_browser_and_dismissing_guide_keeps_sampling() {
        let mut app = app();
        app.page = Page::Claude;
        let _ = app.claude_event(Event::Start);
        assert_eq!(app.claude.guide, Some(Guide::Clients));
        assert!(app.claude.local_busy && app.claude.network_busy);
        assert!(app.claude.browser_busy && app.claude.session_starting);
        let generation = app.claude.generation;
        let _ = app.claude_event(Event::Start);
        assert_eq!(app.claude.generation, generation, "No overlapping runs");
        let _ = app.desktop_message(&Message::Key(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        }));
        assert!(app.claude.guide.is_none());
        assert!(app.claude.busy());
        assert_eq!(app.claude.generation, generation);
    }
    #[test]
    fn cancellation_ignores_old_browser_and_session_callbacks() {
        let mut app = app();
        let _ = app.claude_event(Event::Start);
        let old = app.claude.generation;
        let _ = app.claude_event(Event::Cancel);
        let stopped = app.claude.status.clone();
        let _ = app.claude_event(Event::BrowserOpened(old, Err("stale open error".into())));
        let _ = app.claude_event(Event::Session(
            old,
            SessionAction::Browser,
            Err("stale session error".into()),
        ));
        assert!(!app.claude.busy() && !app.claude.error);
        assert!(app.claude.guide.is_none());
        assert_eq!(app.claude.status, stopped);
        let _ = app.claude_event(Event::Start);
        let _ = app.claude_event(Event::Session(
            old,
            SessionAction::Browser,
            Err("old run".into()),
        ));
        assert!(app.claude.session_starting && app.claude.browser_busy);
        assert!(!app.claude.error);
    }
    #[test]
    fn receiver_failure_does_not_leave_detection_busy_after_local_results() {
        let mut app = app();
        let _ = app.claude_event(Event::Start);
        let generation = app.claude.generation;
        let _ = app.claude_event(Event::Session(
            generation,
            SessionAction::Browser,
            Err("bind failed".into()),
        ));
        let _ = app.claude_event(Event::Local(
            generation,
            Report::new(Environment::Desktop, "test"),
        ));
        let _ = app.claude_event(Event::Network(
            generation,
            Ok(Report::new(Environment::Core, "test")),
        ));
        assert!(!app.claude.busy());
        assert!(app.claude.error);
        assert_eq!(app.claude.status, "bind failed");
        assert!(app.claude.reports[index(Environment::Desktop)].is_some());
        assert!(app.claude.reports[index(Environment::Core)].is_some());
    }
    #[test]
    fn cancelling_automatic_code_ignores_its_pending_receiver() {
        let mut app = app();
        let _ = app.claude_event(Event::CodeAuto);
        let generation = app.claude.generation;
        assert!(app.claude.code_busy && app.claude.session_starting);
        let _ = app.claude_event(Event::Cancel);
        let status = app.claude.status.clone();
        let _ = app.claude_event(Event::Session(
            generation,
            SessionAction::AutoCode,
            Err("old terminal".into()),
        ));
        assert!(!app.claude.busy());
        assert!(app.claude.terminal.is_none());
        assert_eq!(app.claude.status, status);
    }
    #[tokio::test]
    async fn browser_report_is_required_to_finish_automatic_detection() {
        let mut app = app();
        app.page = Page::Claude;
        let _ = app.claude_event(Event::Start);
        let generation = app.claude.generation;
        let session = Session::start(check::client(None).unwrap()).await.unwrap();
        let _ = app.claude_event(Event::Session(
            generation,
            SessionAction::Browser,
            Ok(session),
        ));
        let _ = app.claude_event(Event::Local(
            generation,
            Report::new(Environment::Desktop, "test"),
        ));
        let _ = app.claude_event(Event::Network(
            generation,
            Ok(Report::new(Environment::Core, "test")),
        ));
        assert!(app.claude.busy() && app.claude.browser_busy);
        let _ = app.claude_event(Event::Imported(
            generation,
            Ok(Report::new(Environment::Web, "test")),
        ));
        assert!(!app.claude.busy());
        let status = app.claude.status.clone();
        let _ = app.claude_event(Event::BrowserOpened(generation, Ok(())));
        assert_eq!(
            app.claude.status, status,
            "Late launch result must not overwrite the report"
        );
    }
    #[test]
    fn client_guides_do_not_start_monitor_until_switch_on() {
        let mut app = app();
        app.page = Page::Claude;
        app.snapshot.running = true;
        for guide in [Guide::Clients, Guide::Code, Guide::Desktop] {
            let _ = app.claude_event(Event::Guide(Some(guide)));
            assert!(app.claude_dialog().is_some());
            assert!(!app.claude.monitor_enabled);
            assert!(app.claude.command.is_empty());
        }
        let _ = app.claude_event(Event::Guide(None));
        assert!(app.claude_dialog().is_none());
        let _ = app.claude_event(Event::Guide(Some(Guide::Desktop)));
        let _ = app.claude_event(Event::Monitor(true));
        assert_eq!(app.claude.guide, Some(Guide::Desktop));
        assert!(app.claude.monitor_enabled);
        let epoch = app.claude.monitor_generation;
        let _ = app.update(Message::Navigate(Page::Home));
        assert!(app.claude.monitor_enabled);
        assert_eq!(app.claude.monitor_generation, epoch);
        let _ = app.claude_event(Event::Monitor(false));
        assert!(!app.claude.monitor_enabled);
        assert_eq!(app.claude.tab, Tab::Desktop);
    }
    #[test]
    fn leaving_page_cancels_pending_results_without_retaining_a_server() {
        let mut app = app();
        app.page = Page::Claude;
        let generation = app.claude.generation;
        app.claude.network_busy = true;
        let _ = app.update(Message::Navigate(Page::Home));
        let _ = app.claude_event(Event::Network(
            generation,
            Ok(Report::new(Environment::Core, "stale")),
        ));
        assert!(!app.claude.busy());
        assert!(app.claude.reports.iter().all(Option::is_none));
    }
    #[test]
    fn missing_desktop_connections_remain_unconfirmed() {
        let mut state = State::default();
        state.monitor_report(check::observe::Snapshot::default());
        let report = state.reports[index(Environment::Desktop)].as_ref().unwrap();
        assert_eq!(report.signals[0].evidence, Evidence::Unknown);
        assert!(report.routes.is_empty());
    }
    #[test]
    fn page_navigation_and_views_cover_all_three_environments() {
        let mut app = app();
        app.page = Page::Claude;
        assert_eq!(Page::ALL[8].name(), "A÷检测");
        for tab in Tab::ALL {
            app.claude.tab = tab;
            let _view = app.claude_view();
        }
        assert!(app.claude.session.is_none());
    }
}
