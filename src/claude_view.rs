//! Native Iced region report and opt-in browser/terminal collection controls.
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
#[derive(Debug, Clone)]
pub enum Event {
    Start,
    Cancel,
    Tab(Tab),
    Detail(Option<Group>),
    Browser,
    CodeCommand,
    CloseSession,
    Session(u64, bool, Result<Session, String>),
    Local(u64, Report),
    Network(u64, Result<Report, String>),
    CopySiteScript,
    Import,
    Clipboard(u64, Option<String>),
    Imported(u64, Result<Report, String>),
    Export,
    Completed(Result<String, String>),
    Observe,
    Tick,
    Routes(u64, Result<Vec<Route>, String>),
}

#[derive(Default)]
pub struct State {
    pub tab: Tab,
    pub detail: Option<Group>,
    pub reports: [Option<Report>; 4],
    pub session: Option<Session>,
    pub observing: Option<Instant>,
    generation: u64,
    aborters: Vec<AbortHandle>,
    local_busy: bool,
    network_busy: bool,
    importing: bool,
    session_starting: bool,
    routes_busy: bool,
    pub status: String,
    pub error: bool,
    command: String,
    observed: Vec<Route>,
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
        self.local_busy || self.network_busy || self.importing
    }
    pub fn close(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        for aborter in self.aborters.drain(..) {
            aborter.abort();
        }
        self.session = None;
        self.command.clear();
        self.observing = None;
        self.session_starting = false;
        self.routes_busy = false;
        self.local_busy = false;
        self.network_busy = false;
        self.importing = false;
    }
    fn store(&mut self, report: Report) {
        let index = index(report.environment);
        self.reports[index] = Some(report);
    }
    fn selected(&self) -> impl Iterator<Item = &Report> {
        self.reports
            .iter()
            .flatten()
            .filter(|r| self.tab.environment().is_none_or(|e| r.environment == e))
    }
    fn finish_observation(&mut self) {
        self.observing = None;
        let report = self.reports[index(Environment::Desktop)].get_or_insert_with(|| {
            Report::new(Environment::Desktop, "本机系统与用户触发的连接观测")
        });
        report.routes = self.observed.clone();
        report
            .signals
            .retain(|signal| signal.title != "Desktop 请求观测");
        report.signals.push(if report.routes.is_empty() {
            Signal::unknown(Group::Desktop, "Desktop 请求观测", "内核连接记录", "观测期间没有捕获到 Claude 域名连接；请求可能未经过此内核或连接已结束，不能当作已确认直连。")
        } else {
            Signal::new(Group::Desktop, "Desktop 请求观测", format!("记录 {} 条域名与线路组合", report.routes.len()), "用户操作 Desktop 期间的内核连接记录", Evidence::Measured, "按 Claude 域名与观测时间关联，不按进程识别。只能确认记录到的规则与代理链，不能证明全部来自 Desktop，也不据此推断远端出口国家。")
        });
        self.status = "请求观测结束；域名、规则与代理链见详情。".into();
    }
}

impl App {
    pub(crate) fn claude_event(&mut self, event: Event) -> Task<Message> {
        let wrap = |event| Message::Claude(event);
        match event {
            Event::Start => {
                if self.claude.busy() || self.exiting {
                    return Task::none();
                }
                self.claude.close();
                self.claude.reports = Default::default();
                self.claude.observed.clear();
                self.claude.error = false;
                self.claude.status = "正在采集本机与内核地区线索…".into();
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
                return Task::batch([local, network]);
            }
            Event::Cancel => {
                if self.claude.observing.is_some() {
                    self.claude.finish_observation();
                }
                self.claude.close();
                self.claude.status = "已停止检测，已获得的报告保留。".into();
            }
            Event::Tab(tab) => {
                self.claude.tab = tab;
                self.claude.detail = None;
            }
            Event::Detail(group) => self.claude.detail = group,
            Event::Local(generation, report) if generation == self.claude.generation => {
                self.claude.local_busy = false;
                self.claude.store(report);
            }
            Event::Network(generation, result) if generation == self.claude.generation => {
                self.claude.network_busy = false;
                match result {
                    Ok(report) => {
                        self.claude.store(report);
                        self.claude.status =
                            "本机与内核采样完成；请补充常用浏览器和 Code 终端的检测。".into();
                    }
                    Err(error) => {
                        self.claude.status = error;
                        self.claude.error = true;
                    }
                }
            }
            Event::Browser | Event::CodeCommand => {
                let code = matches!(event, Event::CodeCommand);
                if let Some(session) = self.claude.session.as_ref().filter(|s| !s.finished()) {
                    if code {
                        match std::env::current_exe() {
                            Ok(executable) => {
                                self.claude.command = check::system::terminal_command(
                                    &executable,
                                    &session.submit_url(),
                                );
                                self.claude.status =
                                    "命令已复制；请在启动 Code 的同一终端、同一项目目录运行。"
                                        .into();
                                return iced::clipboard::write(self.claude.command.clone());
                            }
                            Err(_) => {
                                self.claude.status = "无法获得应用程序路径。".into();
                                self.claude.error = true;
                            }
                        }
                    } else {
                        let url = session.url.clone();
                        return Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || platform::open_diagnostic(&url))
                                    .await
                                    .map_err(|e| e.to_string())?
                                    .map(|_| "已打开浏览器入口，请在常用浏览器中完成检测。".into())
                                    .map_err(|e| e.to_string())
                            },
                            move |result| wrap(Event::Completed(result)),
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
                        move |result| wrap(Event::Session(generation, code, result)),
                    );
                }
            }
            Event::Session(generation, code, result) if generation == self.claude.generation => {
                self.claude.session_starting = false;
                match result {
                    Ok(session) => {
                        self.claude.session = Some(session);
                        return self.claude_event(if code {
                            Event::CodeCommand
                        } else {
                            Event::Browser
                        });
                    }
                    Err(error) => {
                        self.claude.status = error;
                        self.claude.error = true;
                    }
                }
            }
            Event::CloseSession => {
                self.claude.session = None;
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
            Event::Observe => {
                if !self.snapshot.running || self.claude.busy() {
                    return Task::none();
                }
                self.claude.observed.clear();
                self.claude.observing = Some(Instant::now() + Duration::from_secs(20));
                self.claude.status = "接下来 20 秒请只操作 Claude Desktop，访问或发送一条消息；将按域名记录经过内核的连接。".into();
            }
            Event::Tick => {
                if !self.visible || self.page != Page::Claude || self.exiting {
                    self.claude.close();
                    return Task::none();
                }
                if let Some(session) = &self.claude.session {
                    if let Some(report) = session.next_report() {
                        self.claude.store(report);
                        self.claude.error = false;
                        self.claude.status = "已收到实际浏览器或 Code 终端的采样报告。".into();
                    } else if session.finished() {
                        self.claude.session = None;
                        self.claude.command.clear();
                        self.claude.status = "回传入口已到期，请重新打开。".into();
                    }
                }
                if let Some(deadline) = self.claude.observing {
                    if Instant::now() >= deadline {
                        self.claude.finish_observation();
                    } else if !self.claude.routes_busy
                        && let Some(engine) = self.engine.clone()
                    {
                        self.claude.routes_busy = true;
                        let generation = self.claude.generation;
                        return Task::perform(
                            async move {
                                let api = engine.lock().await.api.clone();
                                let connections: check::ObservedConnections =
                                    api.get("connections").await.map_err(|_| {
                                        "无法读取内核连接，未确认客户端是否经过此内核。".to_string()
                                    })?;
                                Ok(connections.connections)
                            },
                            move |result| wrap(Event::Routes(generation, result)),
                        );
                    }
                }
            }
            Event::Routes(generation, result) if generation == self.claude.generation => {
                self.claude.routes_busy = false;
                if self.claude.observing.is_some() {
                    match result {
                        Ok(routes) => {
                            for route in routes {
                                if self.claude.observed.len() < 32
                                    && !self.claude.observed.iter().any(|r| {
                                        r.host == route.host
                                            && r.rule == route.rule
                                            && r.chain == route.chain
                                    })
                                {
                                    self.claude.observed.push(route);
                                }
                            }
                        }
                        Err(message) => {
                            self.claude.finish_observation();
                            self.claude.status = message;
                            self.claude.error = true;
                        }
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
                button(self.label(tab.name()).size(13))
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
                .size(13)
            )
            .style(ui_style::primary)
            .padding([7, 12])
            .on_press_maybe((!state.busy()).then_some(event(Event::Start))),
            button(self.label("导出报告").size(13))
                .style(ui_style::secondary_button)
                .padding([7, 10])
                .on_press_maybe(
                    (has_reports && self.engine.is_some()).then_some(event(Event::Export))
                )
        ]
        .spacing(8);
        let mut body = column![controls].spacing(10);
        if state.busy() || state.observing.is_some() {
            let progress = if let Some(deadline) = state.observing {
                1.0 - deadline
                    .saturating_duration_since(Instant::now())
                    .as_secs_f32()
                    / 20.0
            } else {
                (usize::from(!state.local_busy) + usize::from(!state.network_busy)) as f32 / 2.0
            };
            body = body
                .push(aligned_row![
                    self.caption(if state.observing.is_some() {
                        "正在观测 Desktop 请求…"
                    } else {
                        "正在检测地区线索…"
                    }),
                    Space::new().width(Length::Fill),
                    button(self.label("停止").size(12))
                        .style(ui_style::text_button)
                        .on_press(event(Event::Cancel))
                ])
                .push(progress_bar(0.0..=1.0, progress).girth(4));
        }
        if !state.status.is_empty() {
            body = body.push(self.label(&state.status).size(12).color(if state.error {
                ui_style::tone(&self.theme(), ui_style::Tone::Danger)
            } else {
                self.secondary()
            }));
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
            "先检测本机与内核，再补充浏览器与终端采样".into()
        };
        body = body.push(
            container(
                column![
                    self.label("地区线索报告").size(16),
                    self.label(summary).size(20),
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
            button(self.label("浏览器检测").size(12))
                .style(ui_style::secondary_button)
                .on_press_maybe((!state.session_starting).then_some(event(Event::Browser))),
            button(self.label("复制 Code 检测命令").size(12))
                .style(ui_style::secondary_button)
                .on_press_maybe((!state.session_starting).then_some(event(Event::CodeCommand))),
            button(self.label("导入剪贴板采样").size(12))
                .style(ui_style::secondary_button)
                .on_press_maybe((!state.importing).then_some(event(Event::Import)))
        ]
        .spacing(6);
        body = body.push(actions);
        if state.session.is_some() {
            body = body.push(aligned_row![
                self.caption("本机回传入口已开启；离开此页即关闭，最长 20 分钟。"),
                Space::new().width(Length::Fill),
                button(self.label("关闭入口").size(12))
                    .style(ui_style::text_button)
                    .on_press(event(Event::CloseSession))
            ]);
        }
        if !state.command.is_empty() {
            body = body
                .push(
                    text_input("在启动 Code 的终端运行", &state.command)
                        .size(12)
                        .style(ui_style::input),
                )
                .push(
                    self.caption(
                        "请在 Code 使用的项目目录中运行；不会读取账号、Cookie 或 API 密钥。",
                    ),
                );
        }
        if state.tab == Tab::Web {
            body = body.push(aligned_row![button(self.label("复制 Claude 页面采样脚本").size(12)).style(ui_style::secondary_button).on_press(event(Event::CopySiteScript)), Space::new().width(Length::Fill)]).push(self.caption("可在 Claude 网页开发者工具的 Console 运行，再导入脚本复制的 JSON，以确认该站点的时区、权限与同域出口。"));
        }
        if state.tab == Tab::Desktop {
            body = body.push(
                aligned_row![
                    button(self.label("记录 Desktop 请求").size(12))
                        .style(ui_style::secondary_button)
                        .on_press_maybe(
                            (self.snapshot.running && !state.busy() && state.observing.is_none())
                                .then_some(event(Event::Observe))
                        ),
                    self.caption("20 秒内只操作 Desktop；按域名和时间关联，无法确认来源进程。")
                ]
                .spacing(8),
            );
        }
        let mut results = column![].spacing(10);
        if let Some(group) = state.detail {
            results = results.push(
                aligned_row![
                    button(self.label("‹ 返回报告").size(12))
                        .style(ui_style::text_button)
                        .on_press(event(Event::Detail(None))),
                    self.label(group.name()).size(18)
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
                            self.label(&signal.title).size(15),
                            Space::new().width(Length::Fill),
                            container(
                                self.label(if state.differs(report, signal) {
                                    "存在差异"
                                } else {
                                    signal.evidence.name()
                                })
                                .size(11)
                            )
                            .padding([3, 7])
                            .style(move |theme: &Theme| ui_style::badge(theme, tone))
                        ],
                        self.label(&signal.value).size(14),
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
                                    self.label(&route.host).size(14),
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
            results = results.push(self.caption("网页版检测需要在常用浏览器中完成，Code 检测需要在实际终端运行；Desktop 的系统线索与请求观测分别展示。"));
        }
        body.push(scrollable(results).height(Length::Fill))
            .height(Length::Fill)
            .into()
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
        let tone = if differences > 0 {
            ui_style::Tone::Warning
        } else if measured > 0 {
            ui_style::Tone::Success
        } else {
            ui_style::Tone::Muted
        };
        let status = if differences > 0 {
            "存在差异"
        } else if measured > 0 {
            "已有实测"
        } else {
            "待补充"
        };
        let mut lines = column![aligned_row![
            self.label(group.name()).size(15),
            Space::new().width(Length::Fill),
            container(self.label(status).size(11))
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
                .size(12)
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
    });
    desktop.routes.push(Route {
        host: "api.anthropic.com".into(),
        rule: "MATCH".into(),
        chain: "Proxy → 美国节点1".into(),
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
        state.finish_observation();
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
