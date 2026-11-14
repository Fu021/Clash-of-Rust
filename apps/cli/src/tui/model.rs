use super::input::Key;
use anyhow::{Result, ensure};
use clash_client_runtime::{
    control::{Meta, Owner, Poll},
    engine::{Scope, Snapshot},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Home,
    Nodes,
    Connections,
    Network,
}

impl Page {
    pub fn scope(self) -> Scope {
        match self {
            Self::Home => Scope::Home,
            Self::Nodes => Scope::Proxies,
            Self::Connections => Scope::Connections,
            Self::Network => Scope::Other,
        }
    }
}

#[derive(Clone)]
pub(super) struct Target {
    pub profile: Option<String>,
    pub group: String,
    pub node: String,
}

impl Target {
    pub fn validate(&self, poll: &Poll) -> Result<()> {
        ensure!(poll.snapshot.running, "内核已停止，未切换节点");
        ensure!(
            poll.meta.settings.claude_protection.is_none(),
            "Claude 保护期间不能切换节点"
        );
        ensure!(
            poll.meta.settings.active_profile == self.profile,
            "订阅已经变化，请重新选择节点"
        );
        let group = poll.snapshot.proxies.proxies.get(self.group.as_str());
        ensure!(
            group.is_some_and(|group| group.kind.as_str() == "Selector"
                && group.all.iter().any(|node| node.as_str() == self.node)),
            "策略组或节点已经变化，请刷新列表"
        );
        Ok(())
    }
}

pub(super) enum Action {
    None,
    Refresh,
    Select(Target),
    CheckNetwork(clash_client_core::network_tools::Tool, String),
    CancelNetwork,
}

pub(super) struct State {
    pub page: Page,
    pub poll: Option<Poll>,
    pub error: Option<String>,
    pub query: String,
    pub searching: bool,
    pub selected: usize,
    pub group: Option<String>,
    pub confirmation: Option<Target>,
    pub message: Option<String>,
    pub saving: bool,
    pub demo: bool,
    pub network_results: Vec<String>,
    pub network_tool: usize,
}

impl State {
    pub fn new(demo: bool) -> Self {
        Self {
            page: Page::Home,
            poll: demo.then(demo_poll),
            error: None,
            query: String::new(),
            searching: false,
            selected: 0,
            group: None,
            confirmation: None,
            message: None,
            saving: false,
            demo,
            network_results: Vec::new(),
            network_tool: 0,
        }
    }

    pub fn accept(&mut self, poll: Option<Poll>) {
        let changed = self
            .poll
            .as_ref()
            .map(|poll| &poll.meta.settings.active_profile)
            != poll.as_ref().map(|poll| &poll.meta.settings.active_profile);
        self.poll = poll;
        self.error = None;
        if changed {
            self.group = None;
            self.selected = 0;
            self.confirmation = None;
        }
        if self.page != Page::Network {
            self.selected = self.selected.min(self.items().len().saturating_sub(1));
        }
    }

    pub fn failure(&mut self, message: String) {
        self.error = Some(message);
        self.confirmation = None;
    }

    pub fn release_page(&mut self) {
        self.poll = self.demo.then(demo_poll);
        self.group = None;
        self.query = if self.page == Page::Network {
            "github.com".into()
        } else {
            String::new()
        };
        self.searching = false;
        self.selected = 0;
        self.confirmation = None;
        self.message = None;
        self.network_results.clear();
    }

    pub fn items(&self) -> Vec<&str> {
        let Some(poll) = &self.poll else {
            return Vec::new();
        };
        let query = self.query.to_lowercase();
        match self.page {
            Page::Home | Page::Network => Vec::new(),
            Page::Nodes => {
                let proxies = &poll.snapshot.proxies.proxies;
                match self.group.as_deref().and_then(|group| proxies.get(group)) {
                    Some(group) => group
                        .all
                        .iter()
                        .map(|name| name.as_str())
                        .filter(|name| name.to_lowercase().contains(&query))
                        .collect(),
                    None => proxies
                        .iter()
                        .filter(|(_, proxy)| !proxy.all.is_empty())
                        .map(|(name, _)| name.as_str())
                        .filter(|name| name.to_lowercase().contains(&query))
                        .collect(),
                }
            }
            Page::Connections => poll
                .snapshot
                .connections
                .connections
                .iter()
                .filter(|connection| {
                    connection.metadata.host.to_lowercase().contains(&query)
                        || connection.metadata.destination_ip.contains(&query)
                })
                .map(|connection| connection.id.as_ref())
                .collect(),
        }
    }

    pub fn key(&mut self, key: Key) -> Action {
        if self.searching {
            match key {
                Key::Escape | Key::Enter => self.searching = false,
                Key::Back => {
                    self.query.pop();
                }
                Key::Char(character)
                    if self.query.len() + character.len_utf8() <= 256
                        && !character.is_control() =>
                {
                    self.query.push(character);
                }
                Key::Paste(text) => {
                    for character in text.chars().filter(|character| !character.is_control()) {
                        if self.query.len() + character.len_utf8() > 256 {
                            break;
                        }
                        self.query.push(character);
                    }
                }
                _ => {}
            }
            self.selected = 0;
            return Action::None;
        }
        if self.confirmation.is_some() {
            return match key {
                Key::Enter => {
                    let target = self.confirmation.take().unwrap();
                    if self.demo {
                        self.message = Some("演示模式：未提交节点切换".into());
                        Action::None
                    } else if self.error.is_some() {
                        self.message = Some("后台状态过期，请先刷新".into());
                        Action::None
                    } else {
                        Action::Select(target)
                    }
                }
                Key::Escape | Key::Back => {
                    self.confirmation = None;
                    Action::None
                }
                _ => Action::None,
            };
        }
        match key {
            Key::Char('[' | ']') if self.page == Page::Network => {
                let count = clash_client_core::network_tools::Tool::ALL.len();
                self.network_tool = if key == Key::Char(']') {
                    (self.network_tool + 1) % count
                } else {
                    (self.network_tool + count - 1) % count
                };
                self.selected = 0;
                self.network_results.clear();
                return Action::CancelNetwork;
            }
            Key::Escape if self.page == Page::Network => {
                self.network_results.clear();
                self.selected = 0;
                return Action::CancelNetwork;
            }
            Key::Tab => {
                self.page = match self.page {
                    Page::Home => Page::Nodes,
                    Page::Nodes => Page::Connections,
                    Page::Connections => Page::Network,
                    Page::Network => Page::Home,
                };
            }
            Key::Char('1') => self.page = Page::Home,
            Key::Char('2') => self.page = Page::Nodes,
            Key::Char('3') => self.page = Page::Connections,
            Key::Char('4') => self.page = Page::Network,
            Key::Char('/') if self.page != Page::Home => {
                if self.page == Page::Network {
                    self.query.clear();
                    self.network_results.clear();
                }
                self.searching = true;
            }
            Key::Char('r') => return Action::Refresh,
            Key::Up => self.selected = self.selected.saturating_sub(1),
            Key::Down => {
                let count = if self.page == Page::Network {
                    self.network_results.len()
                } else {
                    self.items().len()
                };
                self.selected = (self.selected + 1).min(count.saturating_sub(1));
            }
            Key::Back | Key::Escape => {
                self.group = None;
                self.query.clear();
                self.selected = 0;
            }
            Key::Enter if self.page == Page::Nodes => self.enter_node(),
            Key::Enter if self.page == Page::Network => {
                self.selected = 0;
                self.network_results.clear();
                if self.demo {
                    if self.network_tool != 0 {
                        let tool = clash_client_core::network_tools::Tool::ALL[self.network_tool];
                        self.network_results = vec![
                            format!("目标（演示） · {}", self.query),
                            format!("{} · {}", tool.title(), tool.hint()),
                            "演示模式 · 未运行网络或系统探针".into(),
                        ];
                        return Action::None;
                    }
                    self.network_results = vec![
                        format!("目标（演示） · {}", self.query),
                        "系统 DNS · 198.18.0.12".into(),
                        "内核 DNS · 203.0.113.42".into(),
                        "本机代理 · 端口可连接 · 1 ms".into(),
                        "HTTPS · HTTP 200 · 128 ms".into(),
                        "HTTP · HTTP 301 · 96 ms".into(),
                        "采用协议 · HTTPS".into(),
                        "实际线路 · PROXY → AUTO → 🇺🇸 美国 02 · 命中 DomainSuffix (github.com)"
                            .into(),
                    ];
                } else {
                    return Action::CheckNetwork(
                        clash_client_core::network_tools::Tool::ALL[self.network_tool],
                        self.query.clone(),
                    );
                }
            }
            _ => {}
        }
        Action::None
    }

    fn enter_node(&mut self) {
        let Some(name) = self.items().get(self.selected).map(|name| name.to_string()) else {
            return;
        };
        let Some(group) = &self.group else {
            self.group = Some(name);
            self.query.clear();
            self.selected = 0;
            return;
        };
        let poll = self.poll.as_ref().unwrap();
        let target = Target {
            profile: poll.meta.settings.active_profile.clone(),
            group: group.clone(),
            node: name,
        };
        match target.validate(poll) {
            Ok(()) if self.error.is_none() => self.confirmation = Some(target),
            Ok(()) => self.message = Some("后台状态过期，请先刷新".into()),
            Err(error) => self.message = Some(error.to_string()),
        }
    }
}

pub(super) fn demo_poll() -> Poll {
    let proxies = serde_json::from_value(serde_json::json!({"proxies":{
        "PROXY":{"type":"Selector","now":"AUTO","all":["AUTO","🇺🇸 美国 02","🇯🇵 日本 01","DIRECT"]},
        "AUTO":{"type":"URLTest","now":"🇺🇸 美国 02","all":["🇺🇸 美国 02","🇯🇵 日本 01"]},
        "🇺🇸 美国 02":{"type":"Trojan","history":[{"delay":128}]},
        "🇯🇵 日本 01":{"type":"Shadowsocks","history":[{"delay":186}]},
        "DIRECT":{"type":"Direct"}
    }}))
    .unwrap();
    let connections = serde_json::from_value(serde_json::json!({"connections":[
        {"id":"demo-1","metadata":{"host":"github.com","destinationPort":"443","network":"tcp"},
         "chains":["🇺🇸 美国 02","AUTO","PROXY"],"rule":"DomainSuffix","rulePayload":"github.com",
         "download":2453667,"upload":42000},
        {"id":"demo-2","metadata":{"host":"example.com","destinationPort":"443","network":"tcp"},
         "chains":["DIRECT"],"rule":"Match","download":12000,"upload":2000}
    ]}))
    .unwrap();
    Poll {
        meta: Meta {
            view_revision: 0,
            measurements: Vec::new(),
            settings: Default::default(),
            profiles: Vec::new(),
            geo: clash_client_runtime::assets::GeoManifest {
                version: "demo".into(),
                updated: 0,
                installed_at: None,
                files: Vec::new(),
            },
            running: true,
            version: clash_client_core::VERSION.into(),
            owner: Owner::Daemon,
            executable: Default::default(),
        },
        snapshot: Snapshot {
            running: true,
            mode: "rule".into(),
            tun: true,
            active_node: Some("🇺🇸 美国 02".into()),
            active_route: Some(vec!["PROXY".into(), "AUTO".into(), "🇺🇸 美国 02".into()]),
            connection_count: 6,
            download_rate: 22754,
            upload_rate: 141005,
            proxies,
            connections,
            ..Default::default()
        },
    }
}
