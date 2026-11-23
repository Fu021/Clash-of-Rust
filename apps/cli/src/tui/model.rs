use super::form::{self, Form};
use super::input::Key;
use anyhow::{Context, Result, ensure};
use clash_client_core::config::{Language, LanguagePreference};
use clash_client_core::proxy_order::{self, NodeSort};
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
    Settings,
    Tools,
    Profiles,
    Rules,
}

impl Page {
    pub const ALL: [Self; 8] = [
        Self::Home,
        Self::Nodes,
        Self::Profiles,
        Self::Connections,
        Self::Rules,
        Self::Network,
        Self::Settings,
        Self::Tools,
    ];

    pub fn title(self, language: Language) -> &'static str {
        let (chinese, english) = match self {
            Self::Home => ("首页", "Home"),
            Self::Nodes => ("节点", "Nodes"),
            Self::Profiles => ("订阅", "Subscriptions"),
            Self::Connections => ("连接", "Connections"),
            Self::Rules => ("规则", "Rules"),
            Self::Network => ("工具", "Network"),
            Self::Settings => ("设置", "Settings"),
            Self::Tools => ("操作", "Commands"),
        };
        language.text(chinese, english)
    }

    pub fn scope(self) -> Scope {
        match self {
            Self::Home => Scope::Home,
            Self::Nodes => Scope::Proxies,
            Self::Connections => Scope::Connections,
            Self::Network | Self::Settings | Self::Tools | Self::Profiles | Self::Rules => {
                Scope::Other
            }
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
    #[cfg(test)]
    pub fn validate(&self, poll: &Poll) -> Result<()> {
        self.validate_in(poll, Language::SimplifiedChinese)
    }

    pub fn validate_in(&self, poll: &Poll, language: Language) -> Result<()> {
        ensure!(
            poll.snapshot.running,
            "{}",
            language.text(
                "内核已停止，未切换节点",
                "Core stopped; node was not changed"
            )
        );
        ensure!(
            poll.meta.settings.claude_protection.is_none(),
            "{}",
            language.text(
                "Claude 保护期间不能切换节点",
                "Nodes cannot be changed while Claude protection is active"
            )
        );
        ensure!(
            poll.meta.settings.active_profile == self.profile,
            "{}",
            language.text(
                "订阅已经变化，请重新选择节点",
                "The subscription changed; select the node again"
            )
        );
        let group = poll.snapshot.proxies.proxies.get(self.group.as_str());
        let group = group.context(language.text(
            "策略组已不存在，请重新选择",
            "The group no longer exists; select it again",
        ))?;
        ensure!(
            group.kind.as_str() == "Selector",
            "{}",
            language.text(
                "自动策略组由内核选择节点，不能手动固定成员；请在上一级手动组选择 AUTO 或具体节点",
                "Automatic groups select nodes themselves; choose a node in the parent selector"
            )
        );
        ensure!(
            group.all.iter().any(|node| node.as_str() == self.node),
            "{}",
            language.text(
                "节点已不在这个策略组中，请重新选择",
                "The node is no longer in this group; select it again"
            )
        );
        Ok(())
    }
}

pub(super) enum Action {
    None,
    Refresh,
    Select(Target),
    SetNodeSort(NodeSort),
    CheckNetwork(
        clash_client_core::network_tools::Tool,
        String,
        clash_client_core::network_tools::Options,
    ),
    CancelNetwork,
    Execute(crate::args::Args),
    ApplySetting(crate::args::Args),
    SaveFakeIp(Box<clash_client_runtime::control::Command>),
    SaveRules(Box<clash_client_runtime::control::Command>),
    CheckUpdate,
    InstallUpdate(Box<clash_client_runtime::update::Available>),
    ScrollDetail(bool),
    ScrollNetwork(bool),
    ReviewForm,
}

pub(super) struct State {
    pub page: Page,
    pub language: Language,
    pub language_preference: LanguagePreference,
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
    pub network_outcome: Option<Result<clash_client_core::network_tools::Report>>,
    pub network_tool: usize,
    pub network_options: clash_client_core::network_tools::Options,
    pub form: Option<Form>,
    pub fake_ip: Option<super::fake_ip::Editor>,
    pub execution: Option<crate::args::Args>,
    pub connection: Option<String>,
    pub detail_offset: usize,
    pub shortcuts: Option<super::catalog::Shortcuts>,
    pub rules: Option<super::rules::Browser>,
    pub shortcuts_open: bool,
    pub updates: super::home::Updates,
}

impl State {
    pub fn new(demo: bool) -> Self {
        Self {
            page: Page::Home,
            language: Language::SimplifiedChinese,
            language_preference: LanguagePreference::System,
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
            network_outcome: None,
            network_tool: 0,
            network_options: Default::default(),
            form: None,
            fake_ip: None,
            execution: None,
            connection: None,
            detail_offset: 0,
            shortcuts: None,
            rules: None,
            shortcuts_open: false,
            updates: Default::default(),
        }
    }

    pub fn accept(&mut self, poll: Option<Poll>) {
        let selected_node = self.selected_node();
        if let Some(poll) = &poll {
            self.language_preference = poll.meta.settings.language;
        }
        let changed = self
            .poll
            .as_ref()
            .map(|poll| &poll.meta.settings.active_profile)
            != poll.as_ref().map(|poll| &poll.meta.settings.active_profile);
        self.poll = poll;
        if let Some(editor) = &mut self.fake_ip
            && let Some(poll) = &self.poll
        {
            editor.sync(&poll.meta.settings);
        }
        self.error = None;
        if changed {
            let rule_draft = self.rules.as_ref().is_some_and(super::rules::Browser::busy);
            self.shortcuts = None;
            self.rules = None;
            let preserve_form = self.form.as_ref().is_some_and(|form| {
                form.network.is_some()
                    || form
                        .setting
                        .as_deref()
                        .is_some_and(|key| !key.starts_with("shortcut_"))
            });
            if !preserve_form {
                self.message = if self.form.is_some() || self.confirmation.is_some() || rule_draft {
                    Some(
                        self.language
                            .text(
                                "后台或当前订阅已变化，已取消旧表单，请重新选择",
                                "Backend or subscription changed; select again",
                            )
                            .into(),
                    )
                } else {
                    None
                };
            }
            self.group = None;
            self.selected = 0;
            if self.page == Page::Profiles {
                self.selected = self
                    .poll
                    .as_ref()
                    .and_then(|poll| {
                        self.items().iter().position(|id| {
                            Some(*id) == poll.meta.settings.active_profile.as_deref()
                        })
                    })
                    .unwrap_or(0);
            }
            self.confirmation = None;
            if !preserve_form {
                self.form = None;
                self.execution = None;
            }
        }
        if self.page != Page::Network {
            let selected = {
                let items = self.items();
                let selected = if changed {
                    None
                } else {
                    selected_node
                        .as_deref()
                        .and_then(|name| items.iter().position(|item| *item == name))
                };
                selected.unwrap_or(self.selected.min(items.len().saturating_sub(1)))
            };
            self.selected = selected;
        }
    }

    pub fn failure(&mut self, message: String) {
        self.error = Some(message);
        self.confirmation = None;
        self.execution = None;
    }

    pub fn network_report(&self) -> Option<&clash_client_core::network_tools::Report> {
        self.network_outcome.as_ref()?.as_ref().ok()
    }

    pub fn clear_network_outcome(&mut self) {
        self.network_results.clear();
        self.network_outcome = None;
    }

    pub fn set_language(&mut self, language: Language) {
        if self.page == Page::Profiles {
            let selected = self.selected_node();
            self.language = language;
            self.selected = 0;
            self.restore_selected_node(selected.as_deref());
        } else {
            self.language = language;
        }
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
        self.clear_network_outcome();
        self.network_options = Default::default();
        self.form = None;
        self.fake_ip = None;
        self.execution = None;
        self.connection = None;
        self.detail_offset = 0;
        self.shortcuts = None;
        self.rules = None;
        self.shortcuts_open = false;
        super::rules::sync(self);
    }

    pub fn items(&self) -> Vec<&str> {
        if self.page == Page::Home {
            return super::home::items(self);
        }
        let query = self.query.to_lowercase();
        if self.page == Page::Settings {
            return super::settings::KEYS
                .iter()
                .copied()
                .filter(|key| super::settings::matches_query(key, &query, self.language))
                .collect();
        }
        if self.page == Page::Rules {
            if !self.shortcuts_open {
                return self
                    .rules
                    .as_ref()
                    .map_or_else(Vec::new, |rules| rules.items(&query, self.language));
            }
            return super::settings::RULE_KEYS
                .iter()
                .copied()
                .filter(|key| {
                    key.contains(&query)
                        || super::settings::title(key, self.language)
                            .to_lowercase()
                            .contains(&query)
                })
                .collect();
        }
        if self.page == Page::Tools {
            return form::commands()
                .filter(|spec| {
                    spec.path.contains(&query)
                        || spec
                            .localized_description(self.language)
                            .to_lowercase()
                            .contains(&query)
                })
                .map(|spec| spec.path)
                .collect();
        }
        let Some(poll) = &self.poll else {
            return Vec::new();
        };
        match self.page {
            Page::Network => Vec::new(),
            Page::Home => unreachable!(),
            Page::Settings | Page::Tools | Page::Rules => unreachable!(),
            Page::Profiles => poll
                .meta
                .profiles
                .iter()
                .filter(|profile| {
                    let display_name = profile.display_name(self.language);
                    profile.id.to_lowercase().contains(&query)
                        || profile.matches_name(display_name, &query)
                })
                .map(|profile| profile.id.as_str())
                .collect(),
            Page::Nodes => {
                let proxies = &poll.snapshot.proxies.proxies;
                match self.group.as_deref().and_then(|group| proxies.get(group)) {
                    Some(group) => proxy_order::sorted_indices(
                        &group.all,
                        poll.meta.settings.tui_node_sort,
                        |name| proxies.get(name).and_then(|proxy| proxy.delay),
                        |name| name.to_lowercase().contains(&query),
                    )
                    .into_iter()
                    .map(|index| group.all[index].as_str())
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
                    clash_client_core::connection::matches_query(connection, &query)
                })
                .map(|connection| connection.id.as_ref())
                .collect(),
        }
    }

    pub fn entering_text(&self) -> bool {
        self.searching
            || self.form.as_ref().is_some_and(|form| form.editing)
            || self.fake_ip.as_ref().is_some_and(|editor| editor.editing)
            || self
                .rules
                .as_ref()
                .and_then(|rules| rules.input.as_ref())
                .is_some_and(|input| input.editing)
    }

    fn selected_node(&self) -> Option<String> {
        if self.page == Page::Profiles || self.page == Page::Nodes && self.group.is_some() {
            self.items()
                .get(self.selected)
                .map(|name| (*name).to_owned())
        } else {
            None
        }
    }

    fn restore_selected_node(&mut self, name: Option<&str>) {
        if let Some(name) = name {
            let index = self.items().iter().position(|item| *item == name);
            if let Some(index) = index {
                self.selected = index;
            }
        }
    }

    pub fn key(&mut self, key: Key) -> Action {
        if self.fake_ip.is_some() {
            return super::fake_ip::key(self, key);
        }
        if let Some(form) = &mut self.form {
            if form.editing {
                match key {
                    Key::Escape => {
                        form.editing = false;
                        form.draft.clear();
                    }
                    Key::Enter => form.apply_edit(),
                    Key::Back => {
                        form.draft.pop();
                    }
                    Key::Clear => form.draft.clear(),
                    Key::Char(character) => form.append(&character.to_string()),
                    Key::Paste(text) => form.append(&text),
                    _ => {}
                }
                return Action::None;
            }
            if self.execution.is_some() {
                return match key {
                    Key::Enter => {
                        let args = self.execution.take().unwrap();
                        if self.demo {
                            self.message = Some(
                                self.language
                                    .text(
                                        "演示模式：未执行操作或保存设置",
                                        "Demo mode: no command run or settings saved",
                                    )
                                    .into(),
                            );
                            Action::None
                        } else if form.setting.is_some() && self.error.is_some() {
                            self.message = Some(
                                self.language
                                    .text(
                                        "后台状态过期，草稿保留；等待读取恢复后再确认",
                                        "Backend state is stale; draft retained until it reconnects",
                                    )
                                    .into(),
                            );
                            Action::None
                        } else if form.setting.is_some() && super::settings::inline(&args) {
                            Action::ApplySetting(args)
                        } else {
                            Action::Execute(args)
                        }
                    }
                    Key::Escape | Key::Back => {
                        self.execution = None;
                        Action::None
                    }
                    _ => Action::None,
                };
            }
            match key {
                Key::Up => form.navigate(true),
                Key::Down | Key::Tab => form.navigate(false),
                Key::Left => {
                    form.cycle(true);
                }
                Key::Right | Key::Char(' ') => {
                    form.cycle(false);
                }
                Key::Escape => {
                    self.form = None;
                }
                Key::Enter if form.selected < form.fields.len() => {
                    if form.fields[form.selected].choices.is_empty() {
                        form.begin_edit();
                    } else if form.setting.is_some() {
                        if self.error.is_some() {
                            self.message = Some(
                                self.language
                                    .text(
                                        "后台状态过期，草稿保留；等待读取恢复后再保存",
                                        "Backend state is stale; draft retained until it reconnects",
                                    )
                                    .into(),
                            );
                        } else {
                            return Action::ReviewForm;
                        }
                    } else {
                        form.navigate(false);
                    }
                }
                Key::Enter if form.setting.is_some() && self.error.is_some() => {
                    self.message = Some(
                        self.language
                            .text(
                                "后台状态过期，草稿保留；等待读取恢复后再保存",
                                "Backend state is stale; draft retained until it reconnects",
                            )
                            .into(),
                    );
                }
                Key::Enter => return Action::ReviewForm,
                _ => {}
            }
            return Action::None;
        }
        if self.searching {
            let selected_node = self.selected_node();
            match key {
                Key::Escape | Key::Enter => self.searching = false,
                Key::Back => {
                    self.query.pop();
                }
                Key::Clear => self.query.clear(),
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
            self.restore_selected_node(selected_node.as_deref());
            return Action::None;
        }
        if self.page == Page::Rules
            && !self.shortcuts_open
            && self.rules.as_ref().is_some_and(super::rules::Browser::busy)
        {
            return super::rules::key(self, key).unwrap_or(Action::None);
        }
        if self.updates.pending.is_some() {
            return super::home::confirm(self, key);
        }
        if self.confirmation.is_some() {
            return match key {
                Key::Enter => {
                    let target = self.confirmation.take().unwrap();
                    if self.demo {
                        self.message = Some(
                            self.language
                                .text(
                                    "演示模式：未提交节点切换",
                                    "Demo mode: node selection not submitted",
                                )
                                .into(),
                        );
                        Action::None
                    } else if self.error.is_some() {
                        self.message = Some(
                            self.language
                                .text(
                                    "后台状态过期，请先刷新",
                                    "Backend state is stale; refresh first",
                                )
                                .into(),
                        );
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
        if let Some(connection) = &self.connection {
            match key {
                Key::Up => {
                    return Action::ScrollDetail(false);
                }
                Key::Down => {
                    return Action::ScrollDetail(true);
                }
                Key::Char('x' | 'd') => {
                    let path = if key == Key::Char('x') {
                        "connection close"
                    } else {
                        "connection show"
                    };
                    let mut form = Form::new_in(
                        form::commands().find(|spec| spec.path == path).unwrap(),
                        self.language,
                    );
                    form.fields[0].value.clone_from(connection);
                    form.selected = form.fields.len();
                    self.form = Some(form);
                    return Action::None;
                }
                _ => {}
            }
        }
        match key {
            Key::Char('e') if self.page == Page::Network => {
                if self.demo {
                    self.message = Some(
                        self.language
                            .text("演示模式：未导出文件", "Demo mode: no file exported")
                            .into(),
                    );
                } else if self.network_report().is_some() {
                    self.message = None;
                    self.form = Some(super::network::export_editor(self.language));
                } else {
                    self.message = Some(
                        self.language
                            .text(
                                "请先完成一次检查，再导出本次结果",
                                "Run a check before exporting its result",
                            )
                            .into(),
                    );
                }
            }
            Key::Char('o') if self.page == Page::Network => {
                self.message = None;
                self.form = Some(super::network::editor(
                    clash_client_core::network_tools::Tool::ALL[self.network_tool],
                    &self.query,
                    &self.network_options,
                    self.language,
                ));
                return Action::CancelNetwork;
            }
            Key::Char('s') if self.page == Page::Nodes => {
                let Some(poll) = &self.poll else {
                    self.message = Some(
                        self.language
                            .text(
                                "后台未运行，无法保存节点排序",
                                "Backend is not running; node order was not saved",
                            )
                            .into(),
                    );
                    return Action::None;
                };
                let mode = match poll.meta.settings.tui_node_sort {
                    NodeSort::Name => NodeSort::LatencyAscending,
                    NodeSort::LatencyAscending => NodeSort::LatencyDescending,
                    NodeSort::LatencyDescending => NodeSort::Name,
                };
                if self.error.is_some() {
                    self.message = Some(
                        self.language
                            .text(
                                "后台状态过期，请先刷新",
                                "Backend state is stale; refresh first",
                            )
                            .into(),
                    );
                } else if self.demo {
                    let selected_node = self.selected_node();
                    self.poll.as_mut().unwrap().meta.settings.tui_node_sort = mode;
                    self.restore_selected_node(selected_node.as_deref());
                } else {
                    self.message = None;
                    return Action::SetNodeSort(mode);
                }
            }
            Key::Left | Key::Right | Key::Char('[' | ']') if self.page == Page::Network => {
                let count = clash_client_core::network_tools::Tool::ALL.len();
                self.network_tool = if matches!(key, Key::Right | Key::Char(']')) {
                    (self.network_tool + 1) % count
                } else {
                    (self.network_tool + count - 1) % count
                };
                self.selected = 0;
                self.clear_network_outcome();
                self.network_options = Default::default();
                self.message = None;
                return Action::CancelNetwork;
            }
            Key::Escape if self.page == Page::Network => {
                self.clear_network_outcome();
                self.selected = 0;
                self.message = None;
                return Action::CancelNetwork;
            }
            Key::Tab => {
                let index = Page::ALL
                    .iter()
                    .position(|page| *page == self.page)
                    .unwrap();
                self.page = Page::ALL[(index + 1) % Page::ALL.len()];
            }
            Key::Char(number @ '1'..='8') => {
                self.page = Page::ALL[number as usize - '1' as usize];
            }
            Key::Char('/') if self.page != Page::Home => {
                if self.page == Page::Network {
                    self.query.clear();
                    self.clear_network_outcome();
                }
                self.searching = true;
            }
            Key::Char('r') => return Action::Refresh,
            Key::Up if self.page == Page::Network => return Action::ScrollNetwork(false),
            Key::Down if self.page == Page::Network => return Action::ScrollNetwork(true),
            Key::Up => self.selected = self.selected.saturating_sub(1),
            Key::Down => {
                let count = self.items().len();
                self.selected = (self.selected + 1).min(count.saturating_sub(1));
            }
            Key::Back | Key::Escape => {
                if self.page == Page::Rules {
                    self.shortcuts_open = false;
                    super::rules::sync(self);
                }
                self.connection = None;
                self.detail_offset = 0;
                self.group = None;
                self.query.clear();
                self.selected = 0;
            }
            Key::Enter if self.page == Page::Nodes => self.enter_node(),
            Key::Enter if self.page == Page::Home => return super::home::open(self),
            Key::Enter if self.page == Page::Connections => {
                self.connection = self.items().get(self.selected).map(|id| (*id).to_owned());
                self.detail_offset = 0;
            }
            Key::Enter if self.page == Page::Tools => {
                if let Some(path) = self.items().get(self.selected)
                    && let Some(spec) = form::commands().find(|spec| spec.path == *path)
                {
                    self.form = Some(Form::new_in(spec, self.language));
                }
            }
            Key::Enter if self.page == Page::Rules && !self.shortcuts_open => {
                super::rules::open(self);
            }
            Key::Enter if matches!(self.page, Page::Settings | Page::Rules | Page::Profiles) => {
                self.edit_setting()
            }
            Key::Enter if self.page == Page::Network => {
                if self.query.trim().is_empty() {
                    self.error = Some(
                        self.language
                            .text(
                                "请输入域名、IP 或网址",
                                "Enter a domain, IP address, or URL",
                            )
                            .into(),
                    );
                    return Action::None;
                }
                self.selected = 0;
                self.clear_network_outcome();
                self.message = None;
                if self.demo {
                    if self.network_tool != 0 {
                        let tool = clash_client_core::network_tools::Tool::ALL[self.network_tool];
                        self.network_results = vec![
                            format!(
                                "{} · {}",
                                self.language.text("目标（演示）", "Target (demo)"),
                                self.query
                            ),
                            format!(
                                "{} · {}",
                                super::network::title(tool, self.language),
                                super::network::hint(tool, self.language)
                            ),
                            self.language
                                .text(
                                    "演示模式 · 未运行网络或系统探针",
                                    "Demo mode · No network or system probes run",
                                )
                                .into(),
                        ];
                        return Action::None;
                    }
                    self.network_results = vec![
                        format!("{} · {}", self.language.text("目标（演示）", "Target (demo)"), self.query),
                        format!("{} · 198.18.0.12", self.language.text("系统 DNS", "System DNS")),
                        format!("{} · 203.0.113.42", self.language.text("内核 DNS", "Core DNS")),
                        self.language.text("本机代理 · 端口可连接 · 1 ms", "Local proxy · Port reachable · 1 ms").into(),
                        "HTTPS · HTTP 200 · 128 ms".into(),
                        "HTTP · HTTP 301 · 96 ms".into(),
                        self.language.text("采用协议 · HTTPS", "Protocol · HTTPS").into(),
                        self.language.text(
                            "实际线路 · PROXY → AUTO → 🇺🇸 美国 02 · 命中 DomainSuffix (github.com)",
                            "Route · PROXY → AUTO → 🇺🇸 美国 02 · Matched DomainSuffix (github.com)",
                        ).into(),
                    ];
                } else {
                    return Action::CheckNetwork(
                        clash_client_core::network_tools::Tool::ALL[self.network_tool],
                        self.query.clone(),
                        self.network_options.clone(),
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
        if poll
            .snapshot
            .proxies
            .proxies
            .get(group.as_str())
            .is_some_and(|group| group.kind.as_str() != "Selector")
        {
            self.message = None;
            return;
        }
        let target = Target {
            profile: poll.meta.settings.active_profile.clone(),
            group: group.clone(),
            node: name,
        };
        match target.validate_in(poll, self.language) {
            Ok(()) if self.error.is_none() => {
                self.message = None;
                self.confirmation = Some(target);
            }
            Ok(()) => {
                self.message = Some(
                    self.language
                        .text(
                            "后台状态过期，请先刷新",
                            "Backend state is stale; refresh first",
                        )
                        .into(),
                );
            }
            Err(error) => self.message = Some(error.to_string()),
        }
    }

    fn edit_setting(&mut self) {
        let Some(item) = self
            .items()
            .get(self.selected)
            .map(|item| (*item).to_owned())
        else {
            return;
        };
        let Some(poll) = &self.poll else {
            self.message = Some(
                self.language
                    .text(
                        "后台未运行；请在「操作」页启动或读取配置",
                        "Backend is not running; start it or load a configuration under Commands",
                    )
                    .into(),
            );
            return;
        };
        if self.error.is_some() {
            self.message = Some(
                self.language
                    .text(
                        "后台状态过期，请先刷新",
                        "Backend state is stale; refresh first",
                    )
                    .into(),
            );
            return;
        }
        let key = if self.page == Page::Profiles {
            "active_profile"
        } else {
            &item
        };
        if key == "fake_ip_exclusions" {
            self.fake_ip = Some(super::fake_ip::Editor::new(&poll.meta.settings));
            self.message = None;
            return;
        }
        let form = if self.page == Page::Rules {
            super::catalog::open_shortcut(self, key)
        } else {
            super::settings::open(poll, key, self.language)
        };
        match form {
            Ok(mut form) => {
                if self.page == Page::Profiles {
                    form.fields[0].value = item;
                }
                self.form = Some(form);
                self.message = None;
            }
            Err(error) => self.message = Some(format!("{error:#}")),
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
        {"id":"demo-1","metadata":{"host":"github.com","sniffHost":"api.github.com","destinationPort":"443","network":"tcp"},
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
