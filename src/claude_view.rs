//! Native Iced region report with automatic browser sampling and client guides.
use super::*;
use clash_of_rust::claude_check::{
    self as check, Environment, Evidence, Group, Report, Signal, browser::Session,
};
use iced::widget::column;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guide {
    Supplement,
    Protection,
    Diagnostics,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultKind {
    Normal,
    Risk,
    Unknown,
}
impl ResultKind {
    fn name(self) -> &'static str {
        match self {
            Self::Normal => "正常项目",
            Self::Risk => "风险项目",
            Self::Unknown => "辅助线索与未确认项目",
        }
    }
}
#[derive(Debug, Clone)]
pub enum Event {
    Start,
    Protection(bool),
    ConfirmProtection,
    ReputationKey(String),
    SaveReputationKey,
    Additional(check::additional::Kind),
    AdditionalResult(u64, Result<Vec<Signal>, String>),
    Cancel,
    ResultsPage(usize),
    ResultsFilter(Option<ResultKind>),
    Guide(Option<Guide>),
    Browser,
    BrowserOpened(u64, Result<(), String>),
    Audit(u64, check::audit::Audit),
    CloseSession,
    Session(u64, Result<Session, String>),
    Local(u64, Report),
    Network(u64, Result<Report, String>),
    CopySiteScript,
    Import,
    Clipboard(u64, Option<String>),
    Imported(u64, Result<Report, String>),
    Export,
    Completed(Result<String, String>),
    Tick,
}

#[derive(Default)]
pub struct State {
    pub results_page: usize,
    pub results_filter: Option<ResultKind>,
    pub audit: Option<check::audit::Audit>,
    audit_busy: bool,
    scan: Option<check::audit::Scan>,
    pub guide: Option<Guide>,
    protection_opened: Option<std::time::Instant>,
    pub reports: [Option<Report>; 4],
    pub session: Option<Session>,
    guard_status: String,
    generation: u64,
    aborters: Vec<AbortHandle>,
    local_busy: bool,
    network_busy: bool,
    browser_busy: bool,
    importing: bool,
    session_starting: bool,
    pub status: String,
    pub error: bool,
    reputation_key: String,
    additional_busy: bool,
}
enum Entry<'a> {
    Config(&'a check::audit::Finding),
    Clean(&'a str),
    Signal(&'a Report, &'a Signal),
    Route(&'a check::Route),
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
    fn protection_wait(&self) -> u64 {
        self.protection_opened.map_or(3, |opened| {
            let remaining = std::time::Duration::from_secs(3).saturating_sub(opened.elapsed());
            remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
        })
    }
    pub(crate) fn clear_guard(&mut self) {
        self.guard_status.clear();
    }
    pub(crate) fn guard_report(&mut self, snapshot: &check::guard::Snapshot) {
        self.guard_status = format!(
            "{} · 已关闭 {} 条异常连接",
            if snapshot.connected {
                "后台实时保护中"
            } else {
                "实时保护暂未确认"
            },
            snapshot.blocked
        );
        let mut report = Report::new(
            Environment::Desktop,
            "Claude 保护的后台监测与拦截记录；共享服务无法按进程归属",
        );
        report.routes = snapshot.routes.iter().cloned().collect();
        let mut signal = Signal::new(
            Group::Desktop,
            "实时保护与异常拦截",
            format!(
                "已观察 {} 次请求 · 已关闭 {} 条异常连接 · 系统提示请求 {} 次",
                snapshot.total, snapshot.blocked, snapshot.alerts
            ),
            "内核事件与连接控制接口",
            if snapshot.connected {
                Evidence::Measured
            } else {
                Evidence::Unknown
            },
            "持续在后台运行；只处理 Claude 和保护规则关联的服务，偏离指定节点的存续连接会被关闭。未经过内核、已结束的请求及 HTTPS 内容无法据此检查。",
        );
        signal.risk = snapshot.blocked > 0 || !snapshot.error.is_empty();
        if !snapshot.error.is_empty() {
            signal.detail.push_str(&format!(" {}", snapshot.error));
        }
        report.signals.push(signal);
        let current = self.reports[index(Environment::Desktop)]
            .get_or_insert_with(|| Report::new(Environment::Desktop, "本机环境与实时保护"));
        current.routes = report.routes;
        current.signals.retain(|s| s.title != "实时保护与异常拦截");
        current.signals.extend(report.signals);
    }
    fn entries(&self) -> Vec<Entry<'_>> {
        let mut entries = Vec::new();
        if let Some(audit) = &self.audit {
            entries.extend(audit.findings.iter().map(Entry::Config));
            entries.extend(audit.clean_files.iter().map(|path| Entry::Clean(path)));
        }
        entries.extend(
            self.selected()
                .flat_map(|r| r.routes.iter().map(Entry::Route)),
        );
        let mut signals: Vec<_> = self
            .selected()
            .flat_map(|r| r.signals.iter().map(move |s| (r, s)))
            .collect();
        signals.sort_by_key(|(r, s)| {
            if self.differs(r, s) {
                0
            } else if s.group == Group::Exit {
                1
            } else {
                2
            }
        });
        entries.extend(signals.into_iter().map(|(r, s)| Entry::Signal(r, s)));
        entries
    }
    fn kind(&self, entry: &Entry<'_>) -> ResultKind {
        match entry {
            Entry::Clean(_) => ResultKind::Normal,
            Entry::Config(f) => {
                if f.level == check::audit::Level::Incomplete {
                    ResultKind::Unknown
                } else {
                    ResultKind::Risk
                }
            }
            Entry::Signal(r, s) => {
                if s.risk {
                    ResultKind::Risk
                } else if s.evidence != Evidence::Measured
                    || matches!(
                        s.title.as_str(),
                        "实时保护与异常拦截" | "Claude 请求实时检测"
                    )
                {
                    ResultKind::Unknown
                } else if matches!(s.group, Group::Exit | Group::WebRtc | Group::Dns)
                    && self.differs(r, s)
                {
                    ResultKind::Risk
                } else {
                    ResultKind::Normal
                }
            }
            Entry::Route(r) => {
                if r.risky() {
                    ResultKind::Risk
                } else {
                    ResultKind::Unknown
                }
            }
        }
    }
    fn counts(&self, entries: &[Entry<'_>]) -> [usize; 3] {
        let mut counts = [0; 3];
        for entry in entries {
            counts[self.kind(entry) as usize] += 1;
        }
        if let Some(audit) = &self.audit {
            counts[0] += audit.clean_count.saturating_sub(audit.clean_files.len());
            counts[1] += (audit.issues + audit.reviews).saturating_sub(
                audit
                    .findings
                    .iter()
                    .filter(|f| f.level != check::audit::Level::Incomplete)
                    .count(),
            );
            counts[2] += audit.unreadable.saturating_sub(
                audit
                    .findings
                    .iter()
                    .filter(|f| f.level == check::audit::Level::Incomplete)
                    .count(),
            );
        }
        counts
    }
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
            || self.audit_busy
            || self.session_starting
            || self.importing
            || self.additional_busy
    }
    pub fn close(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        for aborter in self.aborters.drain(..) {
            aborter.abort();
        }
        self.session = None;
        self.guide = None;
        self.protection_opened = None;
        if let Some(scan) = self.scan.take() {
            scan.cancel();
        }
        self.audit_busy = false;
        self.session_starting = false;
        self.local_busy = false;
        self.network_busy = false;
        self.browser_busy = false;
        self.importing = false;
        self.additional_busy = false;
    }
    pub fn shutdown(&mut self) {
        self.close();
    }
    fn store(&mut self, mut report: Report) {
        if report.environment == Environment::Web {
            self.browser_busy = false;
        }
        if report.environment == Environment::Desktop
            && let Some(previous) = &mut self.reports[index(Environment::Desktop)]
        {
            report.signals.extend(
                previous
                    .signals
                    .iter()
                    .filter(|s| s.title == "实时保护与异常拦截")
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
        self.status = if self.audit_busy {
            "正在扫描 Claude 项目配置…"
        } else if self.local_busy || self.network_busy {
            "正在检测本机与网络地区线索…"
        } else if self.browser_busy {
            "浏览器正在自动检测，等待报告回传…"
        } else if self.reports[index(Environment::Web)].is_some() {
            "检测完成，配置风险和地区线索已列在下方。"
        } else {
            "配置与本机采样完成；浏览器采样尚未完成，可重新检测。"
        }
        .into();
    }
    fn selected(&self) -> impl Iterator<Item = &Report> {
        self.reports.iter().flatten()
    }
}

impl App {
    pub(crate) fn claude_event(&mut self, event: Event) -> Task<Message> {
        let wrap = |event| Message::Claude(event);
        match event {
            Event::Additional(kind) => {
                if self.claude.busy() || self.working || self.exiting {
                    return Task::none();
                }
                let Some(port) = self.snapshot.running.then_some(self.settings.mixed_port) else {
                    self.claude.status = "请先启动内核并完成一次检测。".into();
                    self.claude.error = true;
                    return Task::none();
                };
                let ip = self.claude.reports[index(Environment::Core)]
                    .as_ref()
                    .and_then(|report| {
                        report
                            .signals
                            .iter()
                            .find(|s| !s.ip.is_empty() && s.group == Group::Exit)
                    })
                    .map(|signal| signal.ip.clone())
                    .unwrap_or_default();
                self.claude.guide = None;
                self.claude.additional_busy = true;
                self.claude.error = false;
                self.claude.status = format!("正在进行{}…", kind.title());
                let generation = self.claude.generation;
                let (aborter, registration) = AbortHandle::new_pair();
                self.claude.aborters.push(aborter);
                return Task::perform(
                    async move {
                        Abortable::new(
                            async move {
                                let client =
                                    check::client(Some(port)).map_err(|e| e.to_string())?;
                                check::additional::run(&client, kind, &ip)
                                    .await
                                    .map_err(|e| e.to_string())
                            },
                            registration,
                        )
                        .await
                        .unwrap_or_else(|_| Err("补充检测已取消".into()))
                    },
                    move |result| wrap(Event::AdditionalResult(generation, result)),
                );
            }
            Event::AdditionalResult(generation, result) if generation == self.claude.generation => {
                self.claude.additional_busy = false;
                match result {
                    Ok(signals) => {
                        let report = self.claude.reports[index(Environment::Core)]
                            .get_or_insert_with(|| {
                                Report::new(Environment::Core, "内核探测与补充检测")
                            });
                        report
                            .signals
                            .retain(|old| !signals.iter().any(|new| new.title == old.title));
                        let remaining = 80usize.saturating_sub(report.signals.len());
                        report.signals.extend(signals.into_iter().take(remaining));
                        self.claude.status = "补充检测完成，结果已加入报告。".into();
                    }
                    Err(error) => {
                        self.claude.status = error;
                        self.claude.error = true;
                    }
                }
            }
            Event::ReputationKey(value) => self.claude.reputation_key = value,
            Event::SaveReputationKey => {
                let key = self.claude.reputation_key.trim().to_owned();
                self.claude.reputation_key.clear();
                self.claude.guide = None;
                return self.dispatch(Action::ReputationKey((!key.is_empty()).then_some(key)));
            }
            Event::Protection(enabled) => {
                if self.working || self.exiting || self.claude.busy() {
                    return Task::none();
                }
                if enabled {
                    self.claude.guide = Some(Guide::Protection);
                    self.claude.protection_opened = Some(std::time::Instant::now());
                } else {
                    return self.dispatch(Action::ClaudeProtection(false));
                }
            }
            Event::ConfirmProtection => {
                if self.claude.guide != Some(Guide::Protection)
                    || self.claude.protection_wait() > 0
                    || self.working
                    || self.exiting
                    || self.claude.busy()
                {
                    return Task::none();
                }
                self.claude.guide = None;
                self.claude.protection_opened = None;
                return self.dispatch(Action::ClaudeProtection(true));
            }
            Event::Start => {
                if self.claude.busy() || self.exiting {
                    return Task::none();
                }
                self.claude.close();
                self.claude.reports = Default::default();
                self.claude.error = false;
                self.claude.status = "正在扫描项目配置，并自动打开浏览器检测…".into();
                self.claude.results_page = 0;
                self.claude.results_filter = None;
                self.claude.audit = None;
                self.claude.audit_busy = true;
                self.claude.local_busy = true;
                self.claude.network_busy = true;
                let generation = self.claude.generation;
                let local =
                    Task::perform(check::system::local(Environment::Desktop), move |report| {
                        wrap(Event::Local(generation, report))
                    });
                let port = self.snapshot.running.then_some(self.settings.mixed_port);
                let ipv6 = self.settings.ipv6;
                let udp = self.settings.udp;
                let dns_override = self.settings.dns_override;
                let detection_settings = self.settings.clone();
                let (aborter, registration) = AbortHandle::new_pair();
                self.claude.aborters.push(aborter);
                let network = Task::perform(
                    async move {
                        Abortable::new(async move {
                        let mut report = Report::new(Environment::Core, "显式通过本应用内核探测；不代表浏览器或其他客户端实际出口");
                        report.signals.push(Signal::new(Group::Dns, "保存的网络偏好", format!("DNS 覆盖{} · IPv6 {} · UDP {}", if dns_override { "开启" } else { "关闭" }, if ipv6 { "开启" } else { "关闭" }, if udp { "开启" } else { "关闭" }), "本应用设置", Evidence::Measured, "这些是保留的用户偏好；Claude 保护使用临时覆盖，实际状态见内核与网络实测。"));
                        if let Some(port) = port {
                            let client = check::client(Some(port)).map_err(|e| e.to_string())?;
                            check::network(&client, &mut report, "通过内核混合端口的探测请求；不会回退到直连").await;
                            report.signals.extend(check::enhanced::configuration(&clash_of_rust::api::Api::new(&detection_settings).map_err(|e|e.to_string())?, &detection_settings).await);
                            if let Ok(routes)=clash_of_rust::api::Api::new(&detection_settings).map_err(|e|e.to_string())?.get::<check::ObservedConnections>("connections").await {report.routes=routes.connections;}
                            report.signals.extend(check::system_dns(&client, "本机系统解析器；结果查询通过内核").await);
                            check::enhanced::extend(&client,&mut report,detection_settings.ip_reputation_key.as_deref()).await;
                        } else {
                            report.signals.push(Signal::unknown(Group::Exit, "内核出口", "内核探测", "内核未运行，没有回退到直连；仍可检测浏览器和 Code 的实际环境。"));
                        }
                        Ok::<_, String>(report)
                    }, registration).await.unwrap_or_else(|_| Err("检测已取消".into()))
                    },
                    move |result| wrap(Event::Network(generation, result)),
                );
                let cancel = check::audit::Scan::default();
                self.claude.scan = Some(cancel.clone());
                let mixed_port = self.settings.mixed_port;
                let audit = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            check::audit::local(mixed_port, &cancel)
                        })
                        .await
                        .unwrap_or_else(|_| check::audit::Audit {
                            incomplete: true,
                            ..Default::default()
                        })
                    },
                    move |report| wrap(Event::Audit(generation, report)),
                );
                let browser = self.claude_event(Event::Browser);
                return Task::batch([local, network, audit, browser]);
            }
            Event::Cancel => {
                self.claude.close();
                self.claude.status = "已停止检测，已获得的报告保留。".into();
            }
            Event::ResultsPage(page) => self.claude.results_page = page,
            Event::ResultsFilter(filter) => {
                self.claude.results_filter = filter;
                self.claude.results_page = 0;
            }
            Event::Guide(guide) => {
                if matches!(guide, Some(Guide::Diagnostics)) {
                    self.claude.reputation_key =
                        self.settings.ip_reputation_key.clone().unwrap_or_default();
                } else {
                    self.claude.reputation_key.clear();
                }
                self.claude.guide = guide;
            }
            Event::Audit(generation, audit) if generation == self.claude.generation => {
                self.claude.audit_busy = false;
                self.claude.scan = None;
                self.claude.audit = Some(audit);
                self.claude.sampling_status();
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
            Event::Browser => {
                if self.exiting || self.claude.session_starting {
                    return Task::none();
                }
                self.claude.browser_busy = true;
                if let Some(session) = self.claude.session.as_ref().filter(|s| !s.finished()) {
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
                self.claude.session_starting = true;
                let generation = self.claude.generation;
                let port = self.snapshot.running.then_some(self.settings.mixed_port);
                return Task::perform(
                    async move {
                        Session::start(check::client(port).map_err(|e| e.to_string())?)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    move |result| wrap(Event::Session(generation, result)),
                );
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
            Event::Session(generation, result)
                if generation == self.claude.generation && self.claude.session_starting =>
            {
                self.claude.session_starting = false;
                match result {
                    Ok(session) => {
                        self.claude.session = Some(session);
                        return self.claude_event(Event::Browser);
                    }
                    Err(error) => {
                        self.claude.browser_busy = false;
                        self.claude.status = error;
                        self.claude.error = true;
                    }
                }
            }
            Event::CloseSession => {
                self.claude.session = None;
                self.claude.browser_busy = false;
                self.claude.session_starting = false;
                self.claude.status = "浏览器回传入口已关闭。".into();
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
                    &serde_json::json!({"schema":1,"version":clash_of_rust::VERSION,"reports":self.claude.reports,"configuration_audit":self.claude.audit}),
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
            Event::Tick => {
                if self.exiting {
                    self.claude.shutdown();
                    return Task::none();
                }
                if !self.visible || self.page != Page::Claude {
                    self.claude.close();
                    return Task::none();
                }
                if let Some(session) = &self.claude.session {
                    if let Some(report) = session.next_report() {
                        let web = report.environment == Environment::Web;
                        self.claude.store(report);
                        self.claude.error = false;
                        if web {
                            self.claude.sampling_status();
                        } else {
                            self.claude.status = "已收到补充采样报告。".into();
                        }
                    } else if session.finished() {
                        self.claude.session = None;
                        self.claude.browser_busy = false;
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
        let has_reports = state.audit.is_some() || state.reports.iter().any(Option::is_some);
        let mut body = column![
            aligned_row![
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
                Space::new().width(Length::Fill),
                button(self.label("检测选项").size(self.font_size(12)))
                    .style(ui_style::text_button)
                    .on_press(event(Event::Guide(Some(Guide::Diagnostics)))),
                button(self.label("导出报告").size(self.font_size(13)))
                    .style(ui_style::secondary_button)
                    .padding([7, 10])
                    .on_press_maybe(
                        (has_reports && self.engine.is_some()).then_some(event(Event::Export))
                    )
            ]
            .spacing(8)
        ]
        .spacing(8);
        let protection = self.settings.claude_protection.as_ref();
        body = body.push(
            container(column![
                checkbox(protection.is_some()).label("Claude 保护")
                    .text_size(self.scaled(13))
                    .on_toggle_maybe((!self.working && !self.exiting && !state.busy() && (self.snapshot.running || protection.is_some()))
                        .then_some(|enabled| Message::Claude(Event::Protection(enabled)))),
                self.caption(if let Some(protection) = protection {
                    format!("使用节点：{} · TUN、规则、代理 DNS 与实时拦截已开启；关闭后恢复原状态。", protection.node)
                } else { "一键启用 TUN、广泛规则、代理 DNS 与实时拦截；可能影响其他应用与国内网站。".into() }),
            ].spacing(5)).padding(10).width(Length::Fill).style(ui_style::panel)
        );
        if state.busy() {
            let progress = (usize::from(!state.local_busy)
                + usize::from(!state.network_busy)
                + usize::from(!state.audit_busy)
                + usize::from(!state.browser_busy && !state.session_starting))
                as f32
                / 4.0;
            body = body
                .push(aligned_row![
                    self.caption("正在扫描配置与检测地区线索…"),
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
        if let Some(audit) = &state.audit {
            body = body.push(self.caption(format!(
                "已检查 {} 个已记录项目 · {} 份配置{}{}",
                audit.projects,
                audit.files,
                if audit.missing_projects > 0 {
                    format!(" · {} 个目录已不存在", audit.missing_projects)
                } else {
                    String::new()
                },
                if audit.incomplete {
                    " · 扫描未完成，见下方说明"
                } else {
                    ""
                }
            )));
        }
        if !state.guard_status.is_empty() {
            body = body.push(self.caption(&state.guard_status));
        }
        if state.session.is_some() {
            body = body.push(aligned_row![
                self.caption("浏览器正在自动采样；离开此页会关闭回传入口。"),
                Space::new().width(Length::Fill),
                button(self.label("关闭入口").size(self.font_size(12)))
                    .style(ui_style::text_button)
                    .on_press(event(Event::CloseSession))
            ]);
        }
        let mut entries = state.entries();
        let counts = state.counts(&entries);
        if state.results_filter.is_none() {
            body = body.push(
                aligned_row![
                    self.claude_count_card(
                        ResultKind::Normal,
                        counts[0],
                        "已确认且未发现上述问题",
                        ui_style::Tone::Success
                    ),
                    self.claude_count_card(
                        ResultKind::Risk,
                        counts[1],
                        "配置覆盖或网络差异，点击查看",
                        ui_style::Tone::Warning
                    )
                ]
                .spacing(10),
            );
            if counts[2] > 0 {
                body = body.push(
                    button(
                        self.label(format!("另有 {} 项辅助线索或未确认结果 ›", counts[2]))
                            .size(self.font_size(12)),
                    )
                    .style(ui_style::text_button)
                    .on_press(event(Event::ResultsFilter(Some(ResultKind::Unknown)))),
                );
            }
            if let Some(audit) = &state.audit
                && audit.incomplete
            {
                body = body.push(self.caption("部分配置尚未完成检查，不能据此确认没有风险。"));
            }
            return body
                .push(self.caption(
                    "配置扫描仅覆盖已记录的项目和可读取文件；检测结果不代表 Claude 服务资格。",
                ))
                .push(Space::new().height(Length::Fill))
                .height(Length::Fill)
                .into();
        }
        let filter = state.results_filter.unwrap();
        entries.retain(|entry| state.kind(entry) == filter);
        body = body.push(
            aligned_row![
                button(self.label("‹ 返回报告").size(self.font_size(12)))
                    .style(ui_style::text_button)
                    .on_press(event(Event::ResultsFilter(None))),
                self.label(filter.name()).size(self.font_size(17)),
                Space::new().width(Length::Fill),
                button(self.label("补充采样").size(self.font_size(12)))
                    .style(ui_style::text_button)
                    .on_press(event(Event::Guide(Some(Guide::Supplement))))
            ]
            .spacing(8),
        );
        let pages = entries.len().div_ceil(20).max(1);
        let page = state.results_page.min(pages - 1);
        let mut results = column![].spacing(8);
        if entries.is_empty() {
            results = results.push(self.caption(if state.busy() {
                "检测结果会逐项显示。"
            } else if has_reports {
                "当前没有可展示的结果。"
            } else {
                "开始检测会自动扫描已记录的 Claude 项目配置，并打开默认浏览器采样。"
            }));
        }
        if let Some(audit) = &state.audit {
            let omitted = if filter == ResultKind::Normal {
                audit.clean_count.saturating_sub(audit.clean_files.len())
            } else {
                audit.omitted
            };
            if omitted > 0 {
                results = results
                    .push(self.caption(format!("明细超过保留上限，另有 {} 项未展开。", omitted)));
            }
        }
        for entry in entries.into_iter().skip(page * 20).take(20) {
            let (title, value, source, detail, advice, badge, tone) = match entry {
                Entry::Config(f) => (
                    format!("配置 · {}", f.field),
                    f.value.clone(),
                    f.path.clone(),
                    f.reason.clone(),
                    f.advice.clone(),
                    f.level.name(),
                    match f.level {
                        check::audit::Level::Issue => ui_style::Tone::Danger,
                        check::audit::Level::Review => ui_style::Tone::Warning,
                        check::audit::Level::Incomplete => ui_style::Tone::Muted,
                    },
                ),
                Entry::Clean(path) => (
                    "配置文件".into(),
                    "未发现上述风险项".into(),
                    path.into(),
                    "已检查代理、API 地址、提供方及证书相关的配置字段。".into(),
                    String::new(),
                    "已检查",
                    ui_style::Tone::Success,
                ),
                Entry::Signal(r, s) => (
                    s.title.clone(),
                    s.value.clone(),
                    s.source.clone(),
                    s.detail.clone(),
                    s.advice.clone(),
                    if state.differs(r, s) {
                        "存在差异"
                    } else {
                        s.evidence.name()
                    },
                    if state.differs(r, s) {
                        ui_style::Tone::Warning
                    } else if s.evidence == Evidence::Measured {
                        ui_style::Tone::Success
                    } else {
                        ui_style::Tone::Muted
                    },
                ),
                Entry::Route(r) => (
                    r.host.clone(),
                    r.outcome.clone(),
                    "内核连接记录".into(),
                    format!("{}\n规则：{}\n线路：{}", r.time, r.rule, r.chain),
                    String::new(),
                    if r.outcome.starts_with("异常连接") {
                        "已拦截"
                    } else if r.risky() {
                        "异常线路"
                    } else {
                        "已选路"
                    },
                    if r.risky() {
                        ui_style::Tone::Danger
                    } else {
                        ui_style::Tone::Muted
                    },
                ),
            };
            let mut item = column![aligned_row![
                self.label(title).size(self.font_size(14)),
                Space::new().width(Length::Fill),
                container(self.label(badge).size(self.font_size(11)))
                    .padding([3, 7])
                    .style(move |theme: &Theme| ui_style::badge(theme, tone))
            ]]
            .spacing(5);
            if !value.is_empty() {
                item = item.push(self.label(value).size(self.font_size(14)));
            }
            item = item
                .push(self.caption(format!("来源：{source}")))
                .push(self.caption(detail));
            if !advice.is_empty() {
                item = item.push(self.caption(format!("建议：{advice}")));
            }
            results = results.push(
                container(item)
                    .padding(11)
                    .width(Length::Fill)
                    .style(ui_style::panel),
            );
        }
        body = body.push(page_scroll(results).height(Length::Fill));
        if pages > 1 {
            body = body.push(
                aligned_row![
                    self.caption(format!("第 {} / {} 页", page + 1, pages)),
                    Space::new().width(Length::Fill),
                    button(self.label("上一页"))
                        .style(ui_style::secondary_button)
                        .on_press_maybe((page > 0).then(|| event(Event::ResultsPage(page - 1)))),
                    button(self.label("下一页"))
                        .style(ui_style::secondary_button)
                        .on_press_maybe(
                            (page + 1 < pages).then(|| event(Event::ResultsPage(page + 1)))
                        )
                ]
                .spacing(8),
            );
        }
        body.height(Length::Fill).into()
    }

    pub(crate) fn claude_dialog(&self) -> Option<Element<'_, Message>> {
        let guide = self.claude.guide?;
        let event = |event| Message::Claude(event);
        let content = match guide {
            Guide::Supplement => column![
                self.title("补充采样"),
                self.label("自动浏览器检测来自本机检测页面。需要核对 Claude 网页本身的设置时，可在该网页的开发者工具 Console 运行采样脚本，再导入结果。"),
                aligned_row![
                    button(self.label("复制网页采样脚本")).style(ui_style::secondary_button)
                        .on_press(event(Event::CopySiteScript)),
                    button(self.label("导入剪贴板采样")).style(ui_style::secondary_button)
                        .on_press_maybe((!self.claude.importing).then_some(event(Event::Import)))
                ].spacing(8)
            ],
            Guide::Protection => column![
                self.title("开启 Claude 保护"),
                self.label("将使用当前具体代理节点，启用 TUN、严格路由、IPv6 接管和 UDP 转发，加入 Claude 与关联服务规则，并让公网 DNS 通过该节点访问加密 DNS。"),
                self.label("广泛规则可能改变其他应用的监控、登录或客服连接；全局 DNS 变化可能影响国内网站与内网服务，节点故障时相关解析也可能失败。"),
                self.caption("节点域名引导解析使用直连加密 DNS，局域网名称保留本地解析。浏览器独立 DNS、时区、语言和账号资格仍需检测确认，不能保证消除所有风险。"),
                self.caption("原代理模式、DNS、UDP、规则和节点选择会保存，重启后仍保留；关闭保护会恢复。首次启用 TUN 可能需要系统授权。"),
                self.label("实时监测到 Claude 或关联服务偏离保护节点时，会关闭对应连接并弹出系统提示；共享服务可能属于其他应用。"),
                aligned_row![
                    Space::new().width(Length::Fill),
                    button(self.label(if self.claude.protection_wait() > 0 {
                        format!("开启并重新检测（{} 秒）", self.claude.protection_wait())
                    } else {
                        "开启并重新检测".into()
                    })).style(ui_style::primary)
                        .on_press_maybe((self.claude.protection_wait() == 0 && !self.working && !self.exiting && !self.claude.busy()).then_some(event(Event::ConfirmProtection)))
                ],
            ],
            Guide::Diagnostics => column![
                self.title("检测服务"),
                self.label("IP 归属、ASN 与地区可匿名查询；完整的机房、VPN、代理、Tor 与滥用标签需要 ipapi.is 的个人 API 密钥。留空时相关项目显示未确认。"),
                text_input("可选：ipapi.is API 密钥",&self.claude.reputation_key)
                    .secure(true).style(rounded_input).on_input(|value|Message::Claude(Event::ReputationKey(value))),
                self.caption("密钥仅保存在本机，以 HTTPS 请求正文提交给所选检测服务；不会交给浏览器、Claude 或写入报告。匿名接口有每日额度，失败不反复重试。"),
                button(self.label("保存")).style(ui_style::primary)
                    .on_press_maybe((!self.working && !self.exiting).then_some(event(Event::SaveReputationKey))),
                self.caption("补充检测需手动触发：注册信息查询公网出口归属；全球探针从最多 20 个远端位置测量出口 IP；小流量测速下载 10 MiB、上传 2 MiB。结果不代表 Claude 的账号资格。"),
                aligned_row![
                    button(self.label("注册信息")).style(ui_style::secondary_button)
                        .on_press_maybe((self.snapshot.running && !self.claude.busy()).then_some(event(Event::Additional(check::additional::Kind::Registration)))),
                    button(self.label("全球探针")).style(ui_style::secondary_button)
                        .on_press_maybe((self.snapshot.running && !self.claude.busy()).then_some(event(Event::Additional(check::additional::Kind::GlobalPing)))),
                    button(self.label("小流量测速")).style(ui_style::secondary_button)
                        .on_press_maybe((self.snapshot.running && !self.claude.busy()).then_some(event(Event::Additional(check::additional::Kind::Speed)))),
                ].spacing(8),
            ],
        };
        Some(
            content
                .spacing(14)
                .push(aligned_row![
                    Space::new().width(Length::Fill),
                    button(self.label("知道了"))
                        .style(ui_style::primary)
                        .padding([8, 12])
                        .on_press(event(Event::Guide(None)))
                ])
                .into(),
        )
    }

    fn claude_count_card(
        &self,
        kind: ResultKind,
        count: usize,
        description: &str,
        tone: ui_style::Tone,
    ) -> Element<'_, Message> {
        button(
            container(
                column![
                    self.label(match kind {
                        ResultKind::Normal => "正常",
                        ResultKind::Risk => "风险",
                        ResultKind::Unknown => "未确认",
                    })
                    .size(self.font_size(15)),
                    self.label(format!("{count} 项"))
                        .size(self.font_size(30))
                        .color(ui_style::tone(&self.theme(), tone)),
                    self.caption(description.to_owned()),
                    self.label("查看具体项目 ›")
                        .size(self.font_size(12))
                        .color(ui_style::tone(&self.theme(), ui_style::Tone::Accent))
                ]
                .spacing(8),
            )
            .padding(18)
            .width(Length::Fill),
        )
        .width(Length::Fill)
        .padding(0)
        .style(ui_style::secondary_button)
        .on_press(Message::Claude(Event::ResultsFilter(Some(kind))))
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
    state.status = "检测完成，点击数量查看具体项目。".into();
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
    desktop.routes.push(check::Route {
        host: "claude.ai".into(),
        rule: "DOMAIN-SUFFIX claude.ai".into(),
        chain: "Claude → 美国节点1".into(),
        time: "2026-10-14T07:15:01Z".into(),
        outcome: "内核已选路".into(),
    });
    desktop.routes.push(check::Route {
        host: "api.anthropic.com".into(),
        rule: "MATCH".into(),
        chain: "Proxy → 美国节点1".into(),
        time: "2026-10-14T07:15:03Z".into(),
        outcome: "内核已选路".into(),
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
    let mut audit = check::audit::Audit {
        projects: 6,
        files: 9,
        ..Default::default()
    };
    audit.findings.push(check::audit::Finding {
        level: check::audit::Level::Review,
        path: "/Users/demo/Projects/website/.claude/settings.local.json".into(),
        field: "NO_PROXY".into(),
        value: "包含 Claude 域名或全部地址".into(),
        reason: "此配置可能让 Claude 请求绕过 HTTP 代理；TUN 是否接管需要结合实时请求确认。".into(),
        advice: "移除不需要的绕过项，并重新启动相应会话验证。".into(),
    });
    audit.findings.push(check::audit::Finding {
        level: check::audit::Level::Review,
        path: "/Users/demo/Projects/tools/.claude/settings.json".into(),
        field: "ANTHROPIC_BASE_URL".into(),
        value: "https://relay.example.com/".into(),
        reason: "API 地址被覆盖；本机官方域名的出口检测不能代表中转服务的远端出口。".into(),
        advice: "确认目标地址是预期服务，并在实际会话中核对来源。".into(),
    });
    audit.reviews = audit.findings.len();
    audit.clean_files = vec!["/Users/demo/.claude/settings.json".into()];
    audit.clean_count = 1;
    state.audit = Some(audit);
    match scene {
        "claude-scanning" => {
            state.reports = Default::default();
            state.audit = None;
            state.audit_busy = true;
            state.local_busy = true;
            state.network_busy = true;
            state.browser_busy = true;
            state.status = "正在扫描项目配置，并自动打开浏览器检测…".into();
        }
        "claude-risk" => state.results_filter = Some(ResultKind::Risk),
        "claude-normal" => state.results_filter = Some(ResultKind::Normal),
        "claude-unknown" => state.results_filter = Some(ResultKind::Unknown),
        "claude-protection-guide" => state.guide = Some(Guide::Protection),
        "claude-live" => {
            state.guard_status = "实时检测中 · 累计 8 条事件 · 失败 0 · 保留最近 32 条明细".into();
        }
        "claude-clean" => {
            state.audit.as_mut().unwrap().findings.clear();
            state.audit.as_mut().unwrap().reviews = 0;
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
    fn finish_local(app: &mut App, generation: u64) {
        let _ = app.claude_event(Event::Local(
            generation,
            Report::new(Environment::Desktop, "test"),
        ));
        let _ = app.claude_event(Event::Network(
            generation,
            Ok(Report::new(Environment::Core, "test")),
        ));
        let _ = app.claude_event(Event::Audit(generation, check::audit::Audit::default()));
    }
    #[test]
    fn protection_confirmation_waits_three_seconds_and_reopening_restarts_it() {
        let mut app = app();
        let _ = app.claude_event(Event::Protection(true));
        assert_eq!(app.claude.protection_wait(), 3);
        let _ = app.claude_event(Event::ConfirmProtection);
        assert_eq!(app.claude.guide, Some(Guide::Protection));
        app.claude.protection_opened =
            Some(std::time::Instant::now() - Duration::from_millis(2900));
        assert_eq!(app.claude.protection_wait(), 1);
        let _ = app.claude_event(Event::ConfirmProtection);
        assert_eq!(app.claude.guide, Some(Guide::Protection));
        app.claude.protection_opened = Some(std::time::Instant::now() - Duration::from_secs(3));
        assert_eq!(app.claude.protection_wait(), 0);
        let _ = app.claude_event(Event::ConfirmProtection);
        assert!(app.claude.guide.is_none());
        app.working = false;
        let _ = app.claude_event(Event::Protection(true));
        assert_eq!(app.claude.protection_wait(), 3);
    }
    #[test]
    fn disabling_protection_keeps_the_report_without_starting_browser_or_audit() {
        let mut app = app();
        app.page = Page::Claude;
        app.updates.started = true;
        app.claude.reports[0] = Some(Report::new(Environment::Core, "preserved"));
        let generation = app.claude.generation;
        let _ = app.update(Message::Finished(Box::new(Reply {
            profile_edit: false,
            scope: Scope::Claude,
            start_requested: false,
            settings: app.settings.clone(),
            profiles: None,
            rule_document: None,
            rules_applied: false,
            snapshot: None,
            notice: Ok("Claude 保护已关闭".into()),
            running: true,
            geo_status: String::new(),
            autostart: false,
            exit_after_start: false,
            protection_changed: true,
            tray_nodes: None,
        })));
        assert_eq!(app.claude.generation, generation);
        assert!(!app.claude.busy() && app.claude.session.is_none());
        assert_eq!(app.claude.reports[0].as_ref().unwrap().context, "preserved");
    }
    #[test]
    fn start_runs_config_audit_and_browser_once_without_a_client_dialog() {
        let mut app = app();
        let _ = app.claude_event(Event::Start);
        assert!(app.claude.audit_busy && app.claude.local_busy && app.claude.network_busy);
        assert!(app.claude.browser_busy && app.claude.session_starting);
        assert!(app.claude.guide.is_none() && app.claude.scan.is_some());
        let generation = app.claude.generation;
        let _ = app.claude_event(Event::Start);
        assert_eq!(app.claude.generation, generation);
    }
    #[test]
    fn cancellation_ignores_old_audit_browser_and_receiver_results() {
        let mut app = app();
        let _ = app.claude_event(Event::Start);
        let old = app.claude.generation;
        let _ = app.claude_event(Event::Cancel);
        let status = app.claude.status.clone();
        let _ = app.claude_event(Event::Audit(old, check::audit::Audit::default()));
        let _ = app.claude_event(Event::BrowserOpened(old, Err("stale".into())));
        let _ = app.claude_event(Event::Session(old, Err("stale".into())));
        assert!(!app.claude.busy() && !app.claude.error);
        assert!(app.claude.audit.is_none() && app.claude.scan.is_none());
        assert_eq!(app.claude.status, status);
    }
    #[test]
    fn failed_browser_receiver_does_not_hide_config_results_or_stick_progress() {
        let mut app = app();
        let _ = app.claude_event(Event::Start);
        let generation = app.claude.generation;
        let _ = app.claude_event(Event::Session(generation, Err("bind failed".into())));
        finish_local(&mut app, generation);
        assert!(!app.claude.busy());
        assert!(app.claude.error && app.claude.audit.is_some());
        assert_eq!(app.claude.status, "bind failed");
    }
    #[tokio::test]
    async fn completion_waits_for_browser_without_late_launch_overwriting_report() {
        let mut app = app();
        app.page = Page::Claude;
        let _ = app.claude_event(Event::Start);
        let generation = app.claude.generation;
        let session = Session::start(check::client(None).unwrap()).await.unwrap();
        let _ = app.claude_event(Event::Session(generation, Ok(session)));
        finish_local(&mut app, generation);
        assert!(app.claude.busy() && app.claude.browser_busy);
        let _ = app.claude_event(Event::Imported(
            generation,
            Ok(Report::new(Environment::Web, "test")),
        ));
        assert!(!app.claude.busy());
        let status = app.claude.status.clone();
        let _ = app.claude_event(Event::BrowserOpened(generation, Ok(())));
        assert_eq!(app.claude.status, status);
    }
    #[test]
    fn auxiliary_preferences_and_unconfirmed_routes_never_count_as_normal_or_risk() {
        let mut state = State::default();
        let mut report = Report::new(Environment::Web, "test");
        report.signals.push(check::system::preference(
            Group::Browser,
            "时区",
            "Asia/Shanghai",
            "test",
            Some("CN"),
        ));
        report.signals.push(Signal::unknown(
            Group::WebRtc,
            "WebRTC",
            "test",
            "未获得候选",
        ));
        report.signals.push(Signal::new(
            Group::Desktop,
            "Claude 请求实时检测",
            "累计 100 条事件 · 失败 50",
            "内核事件流",
            Evidence::Measured,
            "累计统计不能证明连接成功。",
        ));
        report.routes.push(check::Route {
            host: "claude.ai".into(),
            chain: "Claude[node]".into(),
            outcome: "内核已选路".into(),
            ..Default::default()
        });
        state.store(report);
        assert_eq!(state.counts(&state.entries()), [0, 0, 4]);
        let mut core = Report::new(Environment::Core, "test");
        let mut exit = Signal::new(
            Group::Exit,
            "出口",
            "203.0.113.1",
            "test",
            Evidence::Measured,
            "test",
        );
        exit.country = "US".into();
        core.signals.push(exit);
        state.store(core);
        assert_eq!(state.counts(&state.entries()), [1, 0, 4]);
    }
    #[test]
    fn measured_exit_differences_and_reject_or_failed_routes_are_risks() {
        let mut state = State::default();
        for (env, country) in [(Environment::Core, "US"), (Environment::Web, "JP")] {
            let mut report = Report::new(env, "test");
            let mut exit = Signal::new(
                Group::Exit,
                "出口",
                country,
                "test",
                Evidence::Measured,
                "test",
            );
            exit.country = country.into();
            report.signals.push(exit);
            if env == Environment::Web {
                report.routes = vec![
                    check::Route {
                        chain: "Claude[REJECT]".into(),
                        ..Default::default()
                    },
                    check::Route {
                        chain: "Proxy[DIRECT]".into(),
                        ..Default::default()
                    },
                    check::Route {
                        chain: "Proxy[node]".into(),
                        outcome: "连接失败".into(),
                        ..Default::default()
                    },
                ];
            }
            state.store(report);
        }
        assert_eq!(state.counts(&state.entries()), [1, 4, 0]);
    }
    #[test]
    fn filtered_detail_pages_reset_and_counts_include_omitted_config_rows() {
        let mut app = app();
        app.claude.audit = Some(check::audit::Audit {
            clean_count: 700,
            reviews: 1000,
            omitted: 1000,
            ..Default::default()
        });
        assert_eq!(app.claude.counts(&app.claude.entries()), [700, 1000, 0]);
        let _ = app.claude_event(Event::ResultsPage(4));
        let _ = app.claude_event(Event::ResultsFilter(Some(ResultKind::Risk)));
        assert_eq!(app.claude.results_page, 0);
        assert_eq!(app.claude.results_filter, Some(ResultKind::Risk));
        let _ = app.claude_view();
        let _ = app.claude_event(Event::ResultsFilter(None));
        assert!(app.claude.results_filter.is_none());
    }
    #[test]
    fn page_navigation_cancels_sampling_without_clearing_protection() {
        let mut app = app();
        app.page = Page::Claude;
        let id = uuid::Uuid::new_v4().to_string();
        let saved = Arc::new(check::protection::Protection::new(
            id.clone(),
            "node".into(),
            check::protection::Previous {
                mode: "rule".into(),
                proxy_mode: ProxyMode::Off,
                selectors: vec![],
            },
        ));
        app.settings.active_profile = Some(id);
        app.settings.claude_protection = Some(saved.clone());
        let generation = app.claude.generation;
        app.claude.audit_busy = true;
        let _ = app.update(Message::Navigate(Page::Home));
        let _ = app.claude_event(Event::Audit(generation, check::audit::Audit::default()));
        assert_eq!(app.settings.claude_protection.as_ref(), Some(&saved));
        assert!(app.claude.audit.is_none() && !app.claude.busy());
    }
}
