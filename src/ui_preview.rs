//! Deterministic screenshots of the real views. Feature-gated out of installers.
use super::*;

fn fixture(scene: &str) -> App {
    let mut app = App::with_engine(Err(anyhow::anyhow!("isolated preview")), None).0;
    app.snapshot.running = true;
    app.snapshot.mode = "rule".into();
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
    } else if scene == "rules" {
        app.page = Page::Rules;
        app.snapshot.rules = serde_json::from_value(serde_json::json!({"rules":[
            {"type":"DOMAIN-SUFFIX","payload":"github.com","proxy":"代理选择"},
            {"type":"DOMAIN-SUFFIX","payload":"google.com","proxy":"代理选择"},
            {"type":"DOMAIN","payload":"localhost","proxy":"DIRECT"},
            {"type":"IP-CIDR","payload":"192.168.0.0/16","proxy":"DIRECT"},
            {"type":"GEOIP","payload":"CN","proxy":"DIRECT"},
            {"type":"MATCH","payload":"","proxy":"自动选择"}
        ]}))
        .unwrap();
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
    } else if scene == "settings" {
        app.page = Page::Settings;
        app.geo_status = "2026-10-10".into();
        app.updates.status = "已是最新版本".into();
    } else {
        app.page = Page::Websites;
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
            "ip-ai" => app.site_category = CategoryFilter::Group("AI"),
            "ip-restricted" => app.site_status = StatusFilter::Status(Status::Restricted),
            "ip-details" => {
                app.site_category = CategoryFilter::Group("AI");
                app.site_detail = Some(5);
            }
            "ip-partial" => {
                for result in app.site_results.iter_mut().skip(42) {
                    *result = None;
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
        Message::SiteSummaryFilter(..) | Message::SiteResetFilters | Message::NodeSort(_)
    ) {
        eprintln!("preview interaction: {message:?}");
    }
    match message {
        Message::Query(_)
        | Message::FocusSearch
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
        | Message::SiteReportCategories => app.update(message),
        _ => Task::none(),
    }
}

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
