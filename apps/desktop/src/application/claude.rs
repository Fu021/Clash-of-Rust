//! Claude report state, classification and cancelable diagnostic tasks.
use super::*;
use clash_of_rust::claude_check::{
    self as check, Environment, Evidence, Group, Report, Signal, browser::Session,
};

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
    pub protection_pending: Option<bool>,
    pub results_page: usize,
    pub results_filter: Option<ResultKind>,
    pub audit: Option<check::audit::Audit>,
    pub(super) audit_busy: bool,
    pub(super) scan: Option<check::audit::Scan>,
    pub guide: Option<Guide>,
    pub(super) protection_opened: Option<std::time::Instant>,
    pub reports: [Option<Report>; 4],
    pub session: Option<Session>,
    pub(super) guard_status: String,
    pub(super) generation: u64,
    pub(super) aborters: Vec<AbortHandle>,
    pub(super) local_busy: bool,
    pub(super) network_busy: bool,
    pub(super) browser_busy: bool,
    pub(super) importing: bool,
    pub(super) session_starting: bool,
    pub status: String,
    pub error: bool,
    pub(super) reputation_key: String,
    pub(super) additional_busy: bool,
}
pub(super) enum Entry<'a> {
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
    pub(super) fn protection_wait(&self) -> u64 {
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
    pub(super) fn entries(&self) -> Vec<Entry<'_>> {
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
    pub(super) fn kind(&self, entry: &Entry<'_>) -> ResultKind {
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
    pub(super) fn counts(&self, entries: &[Entry<'_>]) -> [usize; 3] {
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
    pub(super) fn differs(&self, report: &Report, signal: &Signal) -> bool {
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
    pub(super) fn selected(&self) -> impl Iterator<Item = &Report> {
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
                return desktop_runtime::clipboard::write(check::browser::site_script());
            }
            Event::Import => {
                let generation = self.claude.generation;
                return desktop_runtime::clipboard::read()
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
}
#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        App::initial_state(Err(anyhow::anyhow!("isolated")), None)
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
            profile_import: false,
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
        assert!(app.claude.entries().is_empty());
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
        for mode in ["rule", "global", "direct"] {
            let _ = app.dispatch(Action::Mode(mode.into()));
            assert!(app.queued_actions.is_empty());
            assert!(app.notice.contains("锁定当前运行模式"));
            assert_eq!(app.settings.claude_protection.as_ref(), Some(&saved));
        }
    }
}
