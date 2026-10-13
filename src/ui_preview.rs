//! Deterministic screenshots of the real views. Feature-gated out of installers.
use super::*;

// Run the actual application state machine and real core from a signed fixture
// app. Only input delivery and human confirmation are replaced; installers,
// authorization scripts, socket leases and normal shutdown remain production code.
#[cfg(target_os = "macos")]
struct MacFlow {
    app: App,
    scenario: String,
    phase: u8,
    report: std::path::PathBuf,
}

#[cfg(target_os = "macos")]
impl MacFlow {
    fn begin_update(&mut self, task: Task<Message>) -> Task<Message> {
        self.app.updates.install_pending = true;
        let package =
            std::path::PathBuf::from(std::env::var_os("CLASH_CI_UPDATE_PACKAGE").unwrap());
        let version = std::env::var("CLASH_CI_UPDATE_VERSION").unwrap();
        let install = Task::perform(
            async move {
                let download =
                    update::fixture_download(&package, version).map_err(|e| e.to_string())?;
                tokio::task::spawn_blocking(move || update::handoff(download))
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())
            },
            Message::UpdateHandoff,
        );
        Task::batch([task, install])
    }
    fn record(&self, extra: serde_json::Value) {
        let logs = self
            .app
            .engine
            .as_ref()
            .and_then(|engine| engine.try_lock().ok()?.export_logs().ok())
            .and_then(|path| std::fs::read_to_string(path).ok());
        let payload = serde_json::json!({
            "scenario": self.scenario,
            "phase": self.phase,
            "running": self.app.snapshot.running,
            "tun": self.app.snapshot.tun,
            "exiting": self.app.exiting,
            "error": self.app.error,
            "notice": self.app.notice,
            "failure": self.app.updates.failure,
            "core_logs": logs,
            "system_authorization_sessions": platform::macos::ci_proxy_authorization_creations(),
            "administrator_requests": platform::macos::ci_authorizations(),
            "details": extra,
        });
        clash_of_rust::config::atomic_write(
            &self.report,
            &serde_json::to_vec_pretty(&payload).unwrap(),
        )
        .unwrap();
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        let finished = matches!(&message, Message::Finished(_));
        // Refreshes also produce Finished messages and can complete while an
        // action is queued. Only an action's nonempty notice/error confirms it.
        let action_finished = matches!(
            &message,
            Message::Finished(reply)
                if !matches!(&reply.notice, Ok(notice) if notice.is_empty())
        );
        let handoff = matches!(&message, Message::UpdateHandoff(_));
        let task = self.app.update(message);
        if handoff {
            let tun_to_system = self.phase == 3 && self.scenario.starts_with("update-tun-");
            self.phase = 10;
            self.record(serde_json::json!({
                "directory": self.app.updates.installing.as_ref().map(|session| &session.directory),
                "tun_to_system_before_update": tun_to_system,
            }));
        }
        if self.phase == 0 && finished && self.app.snapshot.running {
            self.phase = 1;
            self.record(serde_json::json!({}));
            if self.scenario == "autostart" {
                return Task::batch([
                    task,
                    self.app.update(Message::Action(Action::Autostart(true))),
                ]);
            }
            if self.scenario.starts_with("update") && !self.scenario.starts_with("update-tun-") {
                return self.begin_update(task);
            }
            return Task::batch([
                task,
                self.app
                    .update(Message::Action(Action::ProxyMode(ProxyMode::Tun))),
            ]);
        }
        if self.scenario == "autostart" && action_finished {
            if self.app.error || (self.phase == 2 && !self.app.autostart) {
                self.phase = 10;
                self.record(serde_json::json!({}));
            } else if self.phase == 1 && self.app.autostart {
                self.phase = 2;
                return Task::batch([
                    task,
                    self.app.update(Message::Action(Action::Autostart(false))),
                ]);
            }
        }
        if self.scenario == "proxy-switches" && action_finished {
            let next = match (
                self.phase,
                self.app.snapshot.tun,
                self.app.settings.proxy_mode,
            ) {
                (1, true, _) => Some(ProxyMode::System),
                (2, false, ProxyMode::System) => Some(ProxyMode::Tun),
                (3, true, _) => Some(ProxyMode::System),
                (4, false, ProxyMode::System) => Some(ProxyMode::Off),
                _ => None,
            };
            if self.app.error
                || (self.phase == 5
                    && !self.app.snapshot.tun
                    && self.app.settings.proxy_mode == ProxyMode::Off)
            {
                self.phase = 10;
                self.record(serde_json::json!({}));
            } else if let Some(mode) = next {
                self.phase += 1;
                return Task::batch([
                    task,
                    self.app.update(Message::Action(Action::ProxyMode(mode))),
                ]);
            }
        }
        if self.scenario.starts_with("update-tun-") && action_finished {
            if self.app.error {
                self.phase = 10;
                self.record(serde_json::json!({}));
            } else if self.phase == 1 && self.app.snapshot.tun {
                self.phase = 2;
                return Task::batch([
                    task,
                    self.app
                        .update(Message::Action(Action::ProxyMode(ProxyMode::System))),
                ]);
            } else if self.phase == 2
                && !self.app.snapshot.tun
                && self.app.settings.proxy_mode == ProxyMode::System
                && !self.app.working
            {
                self.phase = 3;
                return self.begin_update(task);
            }
        }
        if matches!(self.scenario.as_str(), "tun-cancel" | "tun-timeout")
            && self.phase == 1
            && action_finished
        {
            self.phase = 10;
            self.record(serde_json::json!({}));
        } else if self.scenario == "tun" && action_finished {
            if self.app.error {
                self.phase = 10;
                self.record(serde_json::json!({}));
            } else if self.phase == 1 && self.app.snapshot.tun {
                self.phase = 2;
                // Exceed the fixture helper's 5s idle interval while the core is
                // running, then restart it through the actual port-save action.
                let port = std::net::TcpListener::bind("127.0.0.1:0")
                    .unwrap()
                    .local_addr()
                    .unwrap()
                    .port();
                self.app.mixed_port = port.to_string();
                self.record(serde_json::json!({"restart_port": port}));
                return Task::batch([
                    task,
                    Task::perform(
                        async { tokio::time::sleep(Duration::from_secs(6)).await },
                        |_| Message::Action(Action::SavePorts(0, 0)),
                    ),
                ]);
            } else if self.phase == 2 && self.app.snapshot.tun && !self.app.working {
                // Periodic refreshes are also Finished messages; wait for the
                // new mixed port to reach the persisted settings first.
                if self.app.mixed_port == self.app.settings.mixed_port.to_string() {
                    self.phase = 3;
                    return Task::batch([
                        task,
                        self.app
                            .update(Message::Action(Action::ProxyMode(ProxyMode::Off))),
                    ]);
                }
            } else if self.phase == 3 && !self.app.snapshot.tun && !self.app.working {
                self.phase = 10;
                self.record(serde_json::json!({"lease_reused_after_long_run": true}));
                return Task::batch([task, self.app.exit()]);
            }
        }
        task
    }
    fn view(&self) -> Element<'_, Message> {
        self.app.view()
    }
    fn theme(&self) -> Theme {
        self.app.theme()
    }
    fn subscription(&self) -> Subscription<Message> {
        self.app.subscription()
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn run_macos_flow() -> iced::Result {
    if update_restart() {
        return super::main();
    }
    assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
    typography::initialize();
    let scenario = std::env::var("CLASH_CI_FLOW").unwrap();
    let report = std::path::PathBuf::from(std::env::var_os("CLASH_CI_FLOW_REPORT").unwrap());
    iced::application(
        move || {
            let (mut app, task) = App::with_engine(startup_store().and_then(Engine::new), None);
            app.updates.started = true;
            (
                MacFlow {
                    app,
                    scenario: scenario.clone(),
                    report: report.clone(),
                    phase: 0,
                },
                task,
            )
        },
        MacFlow::update,
        MacFlow::view,
    )
    .title("Clash of Rust · native macOS test")
    .theme(MacFlow::theme)
    .subscription(MacFlow::subscription)
    .default_font(typography::ENGLISH_FONT)
    .window(window_settings(DEFAULT_WINDOW_SIZE, true))
    .run()
}

fn fixture(scene: &str) -> App {
    let mut app = App::with_engine(Err(anyhow::anyhow!("isolated preview")), None).0;
    app.snapshot.running = true;
    app.snapshot.mode = "rule".into();
    app.snapshot.active_node = Some("香港节点2".into());
    app.notice = "界面预览：固定演示数据，不代表真实网络检测结果。".into();
    app.error = false;
    app.core_failure = None;
    app.notice_visible = false;
    app.updates.checking = false;
    app.dark = !scene.ends_with("light");
    app.settings.active_profile = Some("preview".into());
    app.profiles.push(Profile {
        id: "preview".into(),
        name: "演示订阅".into(),
        source: String::new(),
        updated: 0,
        usage: None,
    });
    let scene = scene.trim_end_matches("-light").replace("-compact", "");
    let scene = scene.as_str();
    if scene.starts_with("proxies") {
        app.page = Page::Proxies;
        app.node_sort = match scene {
            "proxies-descending" => NodeSort::LatencyDescending,
            "proxies-name" => NodeSort::Name,
            _ => NodeSort::LatencyAscending,
        };
        let names = [
            "香港节点10",
            "美国节点1",
            "香港节点2",
            "新加坡节点1",
            "DIRECT",
            "日本节点1",
            "香港节点1",
            "REJECT",
        ];
        let mut proxies = serde_json::Map::new();
        for (name, kind, delay) in [
            ("香港节点10", "Shadowsocks", Some(85)),
            ("香港节点2", "VLESS", Some(28)),
            ("香港节点1", "Trojan", Some(46)),
            ("日本节点1", "Shadowsocks", Some(110)),
            ("新加坡节点1", "VMess", None),
            ("美国节点1", "VLESS", Some(0)),
            ("DIRECT", "Direct", None),
            ("REJECT", "Reject", None),
        ] {
            proxies.insert(name.into(), serde_json::json!({"type": kind, "history": delay.map(|delay| vec![serde_json::json!({"delay": delay})]).unwrap_or_default()}));
        }
        for (name, kind) in [("手动选择", "Selector"), ("自动选择", "URLTest")] {
            proxies.insert(
                name.into(),
                serde_json::json!({"type":kind,"now":"香港节点2","all":names}),
            );
        }
        app.snapshot.proxies =
            serde_json::from_value(serde_json::json!({"proxies": proxies})).unwrap();
        app.expanded.insert("手动选择".into());
    } else if scene.starts_with("home") {
        app.page = Page::Home;
        app.snapshot.version = "1.19.32".into();
        app.snapshot.upload_rate = 24 * 1024;
        app.snapshot.download_rate = 1200 * 1024;
        app.snapshot.connection_count = 5;
        app.snapshot.connections.upload_total = 4 * 1024 * 1024;
        app.snapshot.connections.download_total = 120 * 1024 * 1024;
        if scene == "home-downloading" {
            app.updates.progress = Some(update::Progress {
                received: 25 * 1024 * 1024,
                total: 100 * 1024 * 1024,
            });
        }
        if scene == "home-failure" || scene == "home-retrying" {
            app.snapshot = Snapshot::default();
            app.core_failure = Some("混合端口 7897 被占用，请调整端口后重试。".into());
            app.working = scene == "home-retrying";
            app.busy = app.working;
        }
    } else if scene == "profiles" {
        app.page = Page::Profiles;
        app.profile_name = "我的订阅".into();
        app.profile_source = "https://example.com/subscription".into();
        app.profiles[0].source = "https://example.com/subscription".into();
        app.profiles[0].usage = Some(
            "upload=104857600;download=2474639360;total=107374182400;expire=1798675200".into(),
        );
        app.profiles[0].updated = 1791590400;
        app.profiles.push(Profile {
            id: "backup".into(),
            name: "备用订阅".into(),
            source: "https://example.org/sub".into(),
            updated: 1791590400,
            usage: None,
        });
        app.profiles.push(Profile {
            id: "default".into(),
            name: "默认直连".into(),
            source: "default.yaml".into(),
            updated: 1791590400,
            usage: None,
        });
    } else if scene == "connections" {
        app.page = Page::Connections;
        app.snapshot.connections=serde_json::from_value(serde_json::json!({"connections":[
            {"id":"1","metadata":{"host":"api.github.com","destinationPort":"443","network":"tcp","process":"firefox"},"rule":"DOMAIN-SUFFIX","chains":["代理选择","香港02"],"upload":24576,"download":184320,"start":"2026-10-10T14:30:12Z"},
            {"id":"2","metadata":{"host":"www.google.com","destinationPort":"443","network":"tcp","process":"chrome"},"rule":"MATCH","chains":["自动选择","香港02"],"upload":18432,"download":1258291,"start":"2026-10-10T14:31:08Z"},
            {"id":"3","metadata":{"host":"example.org","destinationPort":"443","network":"tcp","process":"curl"},"rule":"DOMAIN","chains":["DIRECT"],"upload":2048,"download":34816,"start":"2026-10-10T14:31:40Z"}
        ]})).unwrap();
        let previous = app.snapshot.connections.connections.clone();
        for (connection, upload, download) in [
            (0, 24 * 1024, 256 * 1024),
            (1, 8 * 1024, 2 * 1024 * 1024),
            (2, 0, 0),
        ] {
            app.snapshot.connections.connections[connection].upload += upload;
            app.snapshot.connections.connections[connection].download += download;
        }
        connection::sample_rates(
            &mut app.snapshot.connections.connections,
            &previous,
            Duration::from_secs(1),
        );
    } else if scene.starts_with("rules") {
        app.page = Page::Rules;
        let raw = "proxies: []\nproxy-groups: [{name: 代理选择, type: select, proxies: [DIRECT]}]\nrules:\n  - DOMAIN-SUFFIX,github.com,代理选择\n  - DOMAIN-SUFFIX,google.com,代理选择\n  - DOMAIN,localhost,DIRECT\n  - IP-CIDR,192.168.0.0/16,DIRECT\n  - GEOIP,CN,DIRECT\n  - MATCH,代理选择\n";
        let doc = rule_manager::Document::from_raw("preview", false, raw).unwrap();
        app.rule_state
            .load(Arc::new(doc), Arc::new(ProfileRules::default()));
        if scene != "rules" {
            for event in [
                rules_view::Event::Toggle(0, true),
                rules_view::Event::Toggle(4, true),
                rules_view::Event::Add,
                rules_view::Event::Kind("DOMAIN-SUFFIX".into()),
                rules_view::Event::Payload("example.com".into()),
                rules_view::Event::Policy("代理选择".into()),
                rules_view::Event::SaveEditor,
            ] {
                let _ = app.update(Message::Rules(event));
            }
            if scene == "rules-editor" {
                let _ = app.update(Message::Rules(rules_view::Event::Edit));
            }
            if scene != "rules-collapsed" {
                let _ = app.update(Message::Rules(rules_view::Event::Collapse));
            }
        }
    } else if scene == "logs" {
        app.page = Page::Logs;
        app.snapshot.logs = [
            "[客户端] mihomo 已启动，控制接口可用",
            "[客户端] 配置已更新并生效",
            "[内核] INFO TCP connection established",
            "[内核] WARN DNS query failed, retrying",
            "[内核] ERROR connection timeout: api.example.org:443",
        ]
        .into_iter()
        .map(Arc::from)
        .collect();
    } else if scene == "diagnostics" {
        app.page = Page::Tests;
        app.test_results = vec![
            "网站响应 · HTTP 204 · 42 ms".into(),
            "出口 IP · 203.0.113.42 · 德国".into(),
            "系统 DNS · github.com → 140.82.112.4".into(),
            "测试失败：连接超时，请检查当前节点或重试。".into(),
        ];
    } else if scene.starts_with("claude") {
        app.page = Page::Claude;
        app.snapshot.active_node = Some("美国节点1".into());
        app.claude = claude_view::fixture_state(scene);
    } else if scene.starts_with("settings") {
        app.page = Page::Settings;
        app.geo_status = "2026-10-10".into();
        app.updates.status = "已是最新版本".into();
    } else {
        app.page = Page::Websites;
        app.site_has_run = true;
        for (index, service) in ip_check::services().iter().enumerate() {
            let mut state = if index == 0 {
                ip_check::State::Identified
            } else if index == 1 || index == 2 {
                ip_check::State::Confirmed
            } else if index == 3 {
                ip_check::State::Restricted
            } else if index == 4 {
                ip_check::State::Reachable
            } else if index == 5 {
                ip_check::State::Unknown
            } else {
                match index % 8 {
                    0..=2 => ip_check::State::Confirmed,
                    3 => ip_check::State::Partial,
                    4 => ip_check::State::Reachable,
                    5 => ip_check::State::Identified,
                    6 => ip_check::State::Restricted,
                    _ => ip_check::State::Unknown,
                }
            };
            if service.information_only() && state != ip_check::State::Unknown {
                state = ip_check::State::Identified;
            }
            let country = match service.group.as_str() {
                "日本" => "日本",
                "台湾" => "台湾",
                "香港" => "香港",
                "欧洲" => "英国",
                "大洋洲" => "澳大利亚",
                "韩国" => "韩国",
                "东南亚" => "新加坡",
                "非洲" => "南非",
                "南美" => "巴西",
                _ => "美国",
            };
            let summary = if index == 0 {
                "203.0.113.42"
            } else {
                match state {
                    ip_check::State::Confirmed => "可用",
                    ip_check::State::Partial => "仅自制内容",
                    ip_check::State::Reachable => "网页可达 · 解锁未确认",
                    ip_check::State::Identified => "已识别地区",
                    ip_check::State::Restricted => "地区不支持",
                    ip_check::State::Unknown => "验证拦截，未确认",
                    ip_check::State::Failed => "请求失败",
                }
            };
            let result = if index > 6 && index % 17 == 0 {
                Err("连接超时：平台未在请求时限内返回响应，无法确认可用性。".into())
            } else {
                Ok(ip_check::CheckResult {
                    state,
                    summary: summary.into(),
                    country: if state == ip_check::State::Unknown {
                        "未提供".into()
                    } else {
                        country.into()
                    },
                    millis: 35 + (index as u128 * 7) % 120,
                    detail: "固定演示结果；展开详情可查看平台返回的依据、访问限制或请求错误。"
                        .into(),
                })
            };
            app.site_results.push(Some(result));
        }
        match scene {
            "ip-ai" => {
                app.site_category = CategoryFilter::Group("AI");
                app.site_show_results = true;
            }
            "ip-restricted" => {
                app.site_status = StatusFilter::Status(Status::Restricted);
                app.site_show_results = true;
            }
            "ip-details" => {
                app.site_category = CategoryFilter::Group("AI");
                app.site_detail = Some(5);
                app.site_show_results = true;
            }
            "ip-empty" => {
                app.site_results.clear();
                app.site_has_run = false;
            }
            "ip-running" | "ip-partial" | "ip-category-partial" => {
                for result in app.site_results.iter_mut().skip(42) {
                    *result = None;
                }
                if scene != "ip-partial" {
                    app.site_busy.insert(ip_check::services()[42].id.clone());
                    app.site_queue.extend(
                        ip_check::services()
                            .iter()
                            .skip(43)
                            .map(|service| service.id.clone()),
                    );
                }
                if scene == "ip-category-partial" {
                    app.site_category = CategoryFilter::Group("日本");
                    app.site_show_results = true;
                }
            }
            _ => {}
        }
    }
    app
}

fn update(app: &mut App, message: Message) -> Task<Message> {
    if matches!(
        message,
        Message::SiteSummaryFilter(..)
            | Message::SiteResetFilters
            | Message::SiteReport
            | Message::SiteDetail(_)
            | Message::NodeSort(_)
            | Message::ConnectionSort(_)
    ) {
        eprintln!("preview interaction: {message:?}");
    }
    match message {
        Message::Query(_)
        | Message::Rules(_)
        | Message::NodeSort(_)
        | Message::ToggleGroup(_)
        | Message::GroupPage(..)
        | Message::ListPage(_)
        | Message::SiteCategory(_)
        | Message::SiteStatus(_)
        | Message::SiteRegion(_)
        | Message::SiteDetail(_)
        | Message::SiteSummaryFilter(..)
        | Message::SiteResetFilters
        | Message::SiteReport => app.update(message),
        Message::Claude(event @ (claude_view::Event::Tab(_) | claude_view::Event::Detail(_))) => {
            app.update(Message::Claude(event))
        }
        Message::ConnectionSort(field) => {
            let task = app.update(Message::ConnectionSort(field));
            eprintln!(
                "preview connection sort: {:?}, descending={}",
                app.connection_sort.field, app.connection_sort.descending
            );
            task
        }
        _ => Task::none(),
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn run() -> iced::Result {
    typography::initialize();
    let scene = std::env::var("CLASH_UI_PREVIEW_SCENE").expect("preview scene");
    let size = if scene.contains("-compact") {
        MIN_WINDOW_SIZE
    } else {
        DEFAULT_WINDOW_SIZE
    };
    iced::application(
        move || {
            let mut app = fixture(&scene);
            app.window_width = format!("{}", size.width);
            app.window_height = format!("{}", size.height);
            (app, Task::none())
        },
        update,
        App::view,
    )
    .title("Clash of Rust · UI preview")
    .theme(App::theme)
    .default_font(typography::ENGLISH_FONT)
    .settings(iced::Settings {
        default_text_size: iced::Pixels(15.0),
        ..Default::default()
    })
    .window(iced::window::Settings {
        size,
        min_size: Some(MIN_WINDOW_SIZE),
        ..Default::default()
    })
    .run()
}

// Native screenshots use iced's rendered pixels rather than screen recording.
// This test executable and its fixture data never enter distributed app bundles.
#[cfg(target_os = "macos")]
struct Capture {
    app: App,
}
#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
enum CaptureMessage {
    App(Message),
    Take,
    TrayChecked,
    Captured(iced::window::Screenshot),
}
#[cfg(target_os = "macos")]
fn capture_update(state: &mut Capture, message: CaptureMessage) -> Task<CaptureMessage> {
    match message {
        CaptureMessage::App(message) => update(&mut state.app, message).map(CaptureMessage::App),
        CaptureMessage::Take => {
            if std::env::var("CLASH_UI_PREVIEW_SCENE").as_deref() == Ok("home") {
                return iced::window::latest().and_then(|id| {
                    iced::window::run(id, |_| {
                        // Exercise AppKit creation, all menu/icon states and final
                        // destruction on the real GUI event loop's main thread.
                        let guard = tray::start().expect("create main-thread tray");
                        for (mode, system_proxy, tun) in [
                            ("rule", false, false),
                            ("global", true, false),
                            ("direct", false, true),
                        ] {
                            guard.update(&Snapshot {
                                running: true,
                                mode: mode.into(),
                                system_proxy,
                                tun,
                                ..Default::default()
                            });
                            tray::pump();
                            tray::verify_native_state();
                        }
                        drop(guard);
                        tray::pump();
                        CaptureMessage::TrayChecked
                    })
                });
            }
            iced::window::latest()
                .and_then(|id| iced::window::screenshot(id).map(CaptureMessage::Captured))
        }
        CaptureMessage::TrayChecked => iced::window::latest()
            .and_then(|id| iced::window::screenshot(id).map(CaptureMessage::Captured)),
        CaptureMessage::Captured(screenshot) => {
            let output = std::path::PathBuf::from(
                std::env::var_os("CLASH_UI_PREVIEW_OUTPUT").expect("capture output"),
            );
            std::fs::write(output.with_extension("rgba"), screenshot.as_ref())
                .expect("save rendered pixels");
            std::fs::write(
                output.with_extension("json"),
                serde_json::to_vec(&[screenshot.size.width, screenshot.size.height]).unwrap(),
            )
            .expect("save screenshot size");
            iced::exit()
        }
    }
}
#[cfg(target_os = "macos")]
fn capture_view(state: &Capture) -> Element<'_, CaptureMessage> {
    state.app.view().map(CaptureMessage::App)
}
#[cfg(target_os = "macos")]
pub(crate) fn run() -> iced::Result {
    typography::initialize();
    let scene = std::env::var("CLASH_UI_PREVIEW_SCENE").expect("preview scene");
    iced::application(
        move || {
            (
                Capture {
                    app: fixture(&scene),
                },
                Task::perform(
                    async {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    },
                    |_| CaptureMessage::Take,
                ),
            )
        },
        capture_update,
        capture_view,
    )
    .title("Clash of Rust · UI preview")
    .theme(|state: &Capture| state.app.theme())
    .default_font(typography::ENGLISH_FONT)
    .settings(iced::Settings {
        default_text_size: iced::Pixels(15.0),
        ..Default::default()
    })
    .window(iced::window::Settings {
        size: DEFAULT_WINDOW_SIZE,
        min_size: Some(MIN_WINDOW_SIZE),
        ..Default::default()
    })
    .run()
}
