//! Native rule manager; only the visible subscription and one page of widgets
//! are retained. Rule changes remain drafts until the core validates them.
use super::*;
use rule_manager::{CustomRule, Document, Row, Source};

const TYPES: [&str; 18] = [
    "DOMAIN",
    "DOMAIN-SUFFIX",
    "DOMAIN-KEYWORD",
    "DOMAIN-REGEX",
    "GEOSITE",
    "GEOIP",
    "IP-CIDR",
    "IP-CIDR6",
    "IP-ASN",
    "SRC-IP-CIDR",
    "SRC-PORT",
    "DST-PORT",
    "NETWORK",
    "IN-PORT",
    "IN-TYPE",
    "RULE-SET",
    "DSCP",
    "MATCH",
];
const SHORTCUT_LABELS: [&str; 5] = [
    "拦截广告",
    "本地域名直连",
    "私有 IP 直连",
    "国内域名直连",
    "国内 IP 直连",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceFilter {
    #[default]
    All,
    Custom,
    Shortcut,
    Subscription,
}
impl std::fmt::Display for SourceFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::All => "全部来源",
            Self::Custom => "自定义",
            Self::Shortcut => "快捷规则",
            Self::Subscription => "订阅",
        })
    }
}
impl SourceFilter {
    fn matches(self, source: Source) -> bool {
        self == Self::All
            || matches!(
                (self, source),
                (Self::Custom, Source::Custom)
                    | (Self::Shortcut, Source::Shortcut)
                    | (Self::Subscription, Source::Subscription)
            )
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileChoice {
    pub id: String,
    pub name: String,
}
impl std::fmt::Display for ProfileChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}
#[derive(Debug, Clone)]
pub enum Event {
    Profile(ProfileChoice),
    ConfirmProfile,
    CancelProfile,
    Collapse,
    Toggle(usize, bool),
    Filter(SourceFilter),
    Select(String),
    Add,
    Edit,
    Delete,
    Enable,
    Move(isize),
    MoveOpen,
    MoveInput(String),
    MoveTo,
    MoveEdge(bool),
    Restore,
    Discard,
    Apply,
    CloseEditor,
    TextMode(bool),
    Raw(String),
    Kind(String),
    Payload(String),
    Policy(String),
    NoResolve(bool),
    Enabled(bool),
    Position(String),
    SaveEditor,
}
#[derive(Debug)]
pub struct Editor {
    id: Option<String>,
    raw: String,
    text_mode: bool,
    kind: String,
    payload: String,
    policy: String,
    no_resolve: bool,
    enabled: bool,
    position: String,
    error: String,
}
impl Editor {
    fn new(rule: Option<&CustomRule>, position: usize) -> Self {
        let raw = rule.map_or("DOMAIN,,DIRECT", |r| r.rule.as_str());
        let parts = rule_manager::parts(raw).ok();
        let kind = parts.as_ref().map_or("DOMAIN", |p| p.kind).to_owned();
        Self {
            id: rule.map(|r| r.id.clone()),
            raw: raw.into(),
            text_mode: !TYPES.contains(&kind.as_str()),
            payload: parts.as_ref().map_or("", |p| p.payload).into(),
            policy: parts.as_ref().map_or("DIRECT", |p| p.policy).into(),
            no_resolve: parts.as_ref().is_some_and(|p| p.no_resolve),
            kind,
            enabled: rule.is_none_or(|r| r.enabled),
            position: position.to_string(),
            error: String::new(),
        }
    }
    fn form_raw(&self) -> String {
        let mut raw = if self.kind == "MATCH" {
            format!("MATCH,{}", self.policy.trim())
        } else {
            format!(
                "{},{},{}",
                self.kind,
                self.payload.trim(),
                self.policy.trim()
            )
        };
        if self.no_resolve && self.kind != "MATCH" {
            raw.push_str(",no-resolve");
        }
        raw
    }
    fn set_mode(&mut self, text: bool) {
        if text == self.text_mode {
            return;
        }
        if text {
            self.raw = self.form_raw();
        } else {
            match rule_manager::parts(&self.raw) {
                Ok(p) if TYPES.contains(&p.kind) => {
                    self.kind = p.kind.into();
                    self.payload = p.payload.into();
                    self.policy = p.policy.into();
                    self.no_resolve = p.no_resolve;
                }
                Ok(_) => {
                    self.error = "逻辑规则等复杂类型请使用规则文本编辑。".into();
                    return;
                }
                Err(e) => {
                    self.error = e.to_string();
                    return;
                }
            }
        }
        self.text_mode = text;
        self.error.clear();
    }
}
#[derive(Debug)]
pub struct State {
    pub document: Option<Arc<Document>>,
    pub draft: ProfileRules,
    saved: ProfileRules,
    rows: Vec<Row>,
    warnings: Vec<String>,
    selected: Option<String>,
    filter: SourceFilter,
    expanded: bool,
    editor: Option<Editor>,
    move_open: bool,
    move_position: String,
    pending_profile: Option<ProfileChoice>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            document: None,
            draft: Default::default(),
            saved: Default::default(),
            rows: Vec::new(),
            warnings: Vec::new(),
            selected: None,
            filter: Default::default(),
            expanded: true,
            editor: None,
            move_open: false,
            move_position: String::new(),
            pending_profile: None,
        }
    }
}
impl State {
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }
    pub fn has_editor(&self) -> bool {
        self.editor.is_some()
    }
    pub fn dirty(&self) -> bool {
        self.draft != self.saved
    }
    pub fn load(&mut self, document: Arc<Document>, rules: ProfileRules) {
        self.document = Some(document);
        self.draft = rules.clone();
        self.saved = rules;
        self.selected = None;
        self.editor = None;
        self.move_open = false;
        self.pending_profile = None;
        self.rebuild();
    }
    pub fn release_document(&mut self) {
        self.document = None;
        self.rows = Vec::new();
        self.warnings = Vec::new();
    }
    fn rebuild(&mut self) {
        if let Some(doc) = &self.document {
            (self.rows, self.warnings) = doc.rows(&self.draft);
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|id| !self.rows.iter().any(|r| &r.id == id))
        {
            self.selected = None;
        }
    }
    fn selected_index(&self) -> Option<usize> {
        self.selected
            .as_ref()
            .and_then(|id| self.rows.iter().position(|r| &r.id == id))
    }
    fn selected_custom(&self) -> Option<&CustomRule> {
        let row = &self.rows[self.selected_index()?];
        self.draft
            .custom
            .iter()
            .find(|r| row.id == format!("c:{}", r.id))
    }
    fn move_selected(&mut self, target: usize) {
        if let Some(at) = self.selected_index() {
            let target = target.min(self.rows.len().saturating_sub(1));
            if at != target {
                let row = self.rows.remove(at);
                self.rows.insert(target, row);
                self.draft.order = self.rows.iter().map(|r| r.id.clone()).collect();
                self.rebuild();
            }
        }
    }
    fn save_editor(&mut self) -> anyhow::Result<()> {
        let editor = self.editor.as_ref().context("请先打开规则编辑器")?;
        let raw = if editor.text_mode {
            editor.raw.trim().to_owned()
        } else {
            editor.form_raw()
        };
        rule_manager::parts(&raw)?;
        let position = editor
            .position
            .trim()
            .parse::<usize>()
            .context("插入位置应为正整数")?;
        let max = self.rows.len() + usize::from(editor.id.is_none());
        if position == 0 || position > max.max(1) {
            anyhow::bail!("位置应为 1–{}", max.max(1));
        }
        if editor.enabled
            && self.rows.iter().any(|r| {
                r.enabled
                    && r.raw.trim() == raw
                    && editor
                        .id
                        .as_ref()
                        .is_none_or(|id| r.id != format!("c:{id}"))
            })
        {
            anyhow::bail!("已有相同的启用规则，请修改原规则或调整顺序。");
        }
        let id = editor
            .id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let custom = CustomRule {
            id: id.clone(),
            rule: raw,
            enabled: editor.enabled,
        };
        if let Some(previous) = self.draft.custom.iter_mut().find(|r| r.id == id) {
            *previous = custom;
        } else {
            if self.draft.custom.len() >= 1000 {
                anyhow::bail!("自定义规则不能超过 1000 条");
            }
            self.draft.custom.push(custom);
        }
        self.selected = Some(format!("c:{id}"));
        self.rebuild();
        self.move_selected(position - 1);
        self.editor = None;
        Ok(())
    }
}
use anyhow::Context;

impl App {
    pub(crate) fn update_rules(&mut self, event: Event) -> Task<Message> {
        if self.working || self.elevating || self.exiting {
            return Task::none();
        }
        let state = &mut self.rule_state;
        match event {
            Event::Profile(choice) => {
                if state
                    .document
                    .as_ref()
                    .is_some_and(|d| d.profile_id == choice.id)
                {
                    return Task::none();
                }
                if state.dirty() || state.editor.is_some() {
                    state.pending_profile = Some(choice);
                } else {
                    state.release_document();
                    return self.dispatch_scope(Action::LoadRules(choice.id), Scope::Rules);
                }
            }
            Event::ConfirmProfile => {
                if let Some(choice) = state.pending_profile.take() {
                    *state = State::default();
                    return self.dispatch_scope(Action::LoadRules(choice.id), Scope::Rules);
                }
            }
            Event::CancelProfile => state.pending_profile = None,
            Event::Collapse => state.expanded = !state.expanded,
            Event::Filter(filter) => {
                state.filter = filter;
                state.selected = None;
                self.list_offset = 0;
            }
            Event::Toggle(index, enabled) => {
                if state
                    .document
                    .as_ref()
                    .is_some_and(|d| !d.builtin && !d.included[index])
                {
                    state.draft.enabled[index] = enabled;
                    state.rebuild();
                }
            }
            Event::Select(id) => state.selected = Some(id),
            Event::Add => {
                state.editor = Some(Editor::new(
                    None,
                    state.selected_index().map_or(1, |i| i + 1),
                ))
            }
            Event::Edit => {
                if let Some(rule) = state.selected_custom() {
                    state.editor = Some(Editor::new(
                        Some(rule),
                        state.selected_index().unwrap_or(0) + 1,
                    ));
                }
            }
            Event::Delete => {
                if let Some(id) = state.selected_custom().map(|r| r.id.clone()) {
                    state.draft.custom.retain(|r| r.id != id);
                    state.draft.order.retain(|key| key != &format!("c:{id}"));
                    state.rebuild();
                }
            }
            Event::Enable => {
                if let Some(id) = state.selected_custom().map(|r| r.id.clone())
                    && let Some(rule) = state.draft.custom.iter_mut().find(|r| r.id == id)
                {
                    rule.enabled = !rule.enabled;
                    state.rebuild();
                }
            }
            Event::Move(delta) => {
                if let Some(at) = state.selected_index() {
                    state.move_selected(at.saturating_add_signed(delta));
                }
            }
            Event::MoveOpen => {
                state.move_open = true;
                state.move_position = state.selected_index().map_or(1, |i| i + 1).to_string();
            }
            Event::MoveInput(value) => state.move_position = value,
            Event::MoveTo => {
                if let Ok(position) = state.move_position.trim().parse::<usize>()
                    && position > 0
                    && position <= state.rows.len()
                {
                    state.move_selected(position - 1);
                    state.move_open = false;
                }
            }
            Event::MoveEdge(bottom) => {
                state.move_selected(if bottom {
                    state.rows.len().saturating_sub(1)
                } else {
                    0
                });
                state.move_open = false;
            }
            Event::Restore => {
                state.draft.order.clear();
                state.rebuild();
            }
            Event::Discard => {
                state.draft = state.saved.clone();
                state.editor = None;
                state.move_open = false;
                state.rebuild();
            }
            Event::Apply => {
                if state.dirty()
                    && state.editor.is_none()
                    && let Some(doc) = &state.document
                {
                    return self.dispatch_scope(
                        Action::SaveRules(doc.profile_id.clone(), state.draft.clone()),
                        Scope::Rules,
                    );
                }
            }
            Event::CloseEditor => {
                state.editor = None;
                state.move_open = false;
            }
            Event::TextMode(text) => {
                if let Some(editor) = &mut state.editor {
                    editor.set_mode(text);
                }
            }
            Event::Raw(value) => {
                if let Some(editor) = &mut state.editor {
                    editor.raw = value;
                }
            }
            Event::Kind(value) => {
                if let Some(editor) = &mut state.editor {
                    editor.kind = value;
                    if !matches!(
                        editor.kind.as_str(),
                        "GEOIP" | "IP-CIDR" | "IP-CIDR6" | "IP-ASN" | "RULE-SET"
                    ) {
                        editor.no_resolve = false;
                    }
                }
            }
            Event::Payload(value) => {
                if let Some(editor) = &mut state.editor {
                    editor.payload = value;
                }
            }
            Event::Policy(value) => {
                if let Some(editor) = &mut state.editor {
                    editor.policy = value;
                }
            }
            Event::NoResolve(value) => {
                if let Some(editor) = &mut state.editor {
                    editor.no_resolve = value;
                }
            }
            Event::Enabled(value) => {
                if let Some(editor) = &mut state.editor {
                    editor.enabled = value;
                }
            }
            Event::Position(value) => {
                if let Some(editor) = &mut state.editor {
                    editor.position = value;
                }
            }
            Event::SaveEditor => {
                if let Err(error) = state.save_editor()
                    && let Some(editor) = &mut state.editor
                {
                    editor.error = error.to_string();
                }
            }
        }
        Task::none()
    }
    fn rule_button<'a>(
        &self,
        label: &'a str,
        event: Event,
        enabled: bool,
        primary: bool,
    ) -> Element<'a, Message> {
        button(self.label(label).size(12))
            .padding([7, 10])
            .style(if primary {
                rounded_primary
            } else {
                rounded_secondary
            })
            .on_press_maybe(
                (enabled && !self.working && !self.elevating && !self.exiting)
                    .then_some(Message::Rules(event)),
            )
            .into()
    }
    fn rule_input<'a>(
        &self,
        placeholder: &'a str,
        value: &'a str,
        event: fn(String) -> Event,
    ) -> Element<'a, Message> {
        text_input(placeholder, value)
            .font(typography::ENGLISH_FONT)
            .size(13)
            .padding(9)
            .style(rounded_input)
            .on_input(move |v| Message::Rules(event(v)))
            .into()
    }
    pub(crate) fn rules(&self) -> Element<'_, Message> {
        let state = &self.rule_state;
        let Some(doc) = &state.document else {
            return self.empty_state(
                "规则管理",
                if self.working {
                    "正在加载订阅规则…"
                } else {
                    "请先添加或选择订阅配置。"
                },
            );
        };
        let editable = !self.working && !self.exiting && !self.elevating;
        let compact = self.window_height.parse::<u16>().unwrap_or(700) < 560;
        let choices: Vec<_> = self
            .profiles
            .iter()
            .map(|p| ProfileChoice {
                id: p.id.clone(),
                name: p.name.clone(),
            })
            .collect();
        let selected = choices
            .iter()
            .find(|p| p.id == doc.profile_id)
            .cloned()
            .unwrap_or(ProfileChoice {
                id: doc.profile_id.clone(),
                name: "订阅配置".into(),
            });
        let profile_header = aligned_row![
            self.selection(
                choices,
                selected,
                |p| Message::Rules(Event::Profile(p)),
                195
            ),
            self.caption(
                if self.settings.active_profile.as_deref() == Some(&doc.profile_id) {
                    "当前使用中"
                } else {
                    "未启用 · 修改仅保存到此订阅"
                }
            ),
            Space::new().width(Length::Fill),
            self.rule_button("新增规则", Event::Add, true, true)
        ]
        .spacing(8);
        let enabled = if doc.builtin {
            0
        } else {
            state
                .draft
                .enabled
                .iter()
                .enumerate()
                .filter(|(i, on)| **on || doc.included[*i])
                .count()
        };
        let expanded = state.expanded;
        let mut shortcuts = column![
            aligned_row![
                self.title("快捷规则"),
                self.caption(format!("已启用 {enabled}/5")),
                Space::new().width(Length::Fill),
                self.rule_button(
                    if expanded { "收起" } else { "展开" },
                    Event::Collapse,
                    true,
                    false
                )
            ]
            .spacing(8)
        ]
        .spacing(8);
        // Compact windows keep the card scrollable so no toolbar or report is covered.
        let show_shortcuts = state.expanded;
        if show_shortcuts {
            let mut options = column![self.caption(if doc.builtin {
                "默认直连配置保留原规则；快捷规则适用于订阅。"
            } else {
                "仅补充订阅缺少的规则，补充的规则优先匹配。"
            })]
            .spacing(8);
            for pair in [0, 1, 2, 3, 4].chunks(2) {
                let mut row = aligned_row![].spacing(12);
                for &i in pair {
                    let mut label = SHORTCUT_LABELS[i].to_owned();
                    if doc.included[i] {
                        label.push_str(" · 订阅已包含");
                    }
                    row = row.push(
                        column![
                            checkbox(!doc.builtin && (state.draft.enabled[i] || doc.included[i]))
                                .label(label)
                                .text_size(13)
                                .size(16)
                                .on_toggle_maybe(
                                    (editable && !doc.builtin && !doc.included[i])
                                        .then_some(move |on| Message::Rules(Event::Toggle(i, on)))
                                ),
                            self.caption(clash_of_rust::config::SUBSCRIPTION_RULES[i])
                                .size(11)
                                .wrapping(text::Wrapping::WordOrGlyph)
                        ]
                        .spacing(3)
                        .width(Length::Fill),
                    );
                }
                if pair.len() == 1 {
                    row = row.push(Space::new().width(Length::Fill));
                }
                options = options.push(row);
            }
            if compact {
                shortcuts = shortcuts.push(page_scroll(options).height(75));
            } else {
                shortcuts = shortcuts.push(options);
            }
        }
        let query = self.query.to_lowercase();
        let entries = state.rows.iter().enumerate().filter(|(_, r)| {
            state.filter.matches(r.source)
                && (query.is_empty()
                    || query
                        .split_whitespace()
                        .all(|word| contains_query(&r.raw, word)))
        });
        let total = entries.clone().count();
        let offset = self.list_offset.min(last_page_offset(total));
        let selected_at = state.selected_index();
        let custom = state.selected_custom();
        let mut tools = aligned_row![
            self.caption(selected_at.map_or_else(
                || "选择一条规则调整顺序".into(),
                |i| format!("已选择第 {} 条", i + 1)
            )),
            Space::new().width(Length::Fill),
            self.rule_button(
                "上移",
                Event::Move(-1),
                selected_at.is_some_and(|i| i > 0),
                false
            ),
            self.rule_button(
                "下移",
                Event::Move(1),
                selected_at.is_some_and(|i| i + 1 < state.rows.len()),
                false
            ),
            self.rule_button("移到…", Event::MoveOpen, selected_at.is_some(), false),
            self.rule_button("编辑", Event::Edit, custom.is_some(), false),
            self.rule_button(
                custom.map_or("停用", |r| if r.enabled { "停用" } else { "启用" }),
                Event::Enable,
                custom.is_some(),
                false
            ),
            self.rule_button("删除", Event::Delete, custom.is_some(), false)
        ]
        .spacing(5);
        if compact {
            tools = tools.spacing(3);
        }
        let mut list = column![].spacing(2);
        for (display_row, (index, rule)) in entries.skip(offset).take(PAGE_SIZE).enumerate() {
            let parts = rule_manager::parts(&rule.raw).ok();
            let active = state.selected.as_ref() == Some(&rule.id);
            let raw = parts.as_ref().map_or(rule.raw.as_ref(), |p| {
                if p.payload.is_empty() {
                    "所有剩余连接"
                } else {
                    p.payload
                }
            });
            let source = if !rule.enabled {
                format!("{} · 停用", rule.source)
            } else {
                rule.source.to_string()
            };
            let content = aligned_row![
                self.caption(format!("{:04}", index + 1)).width(42),
                self.label(parts.as_ref().map_or("规则", |p| p.kind))
                    .size(12)
                    .width(if compact { 95 } else { 115 }),
                self.label(raw)
                    .size(13)
                    .wrapping(text::Wrapping::WordOrGlyph)
                    .width(Length::Fill),
                self.label(parts.as_ref().map_or("", |p| p.policy))
                    .size(13)
                    .color(if rule.enabled {
                        self.accent()
                    } else {
                        self.secondary()
                    })
                    .width(85),
                self.caption(source).size(11).width(90)
            ]
            .spacing(7);
            list = list.push(
                button(content)
                    .padding([9, 8])
                    .width(Length::Fill)
                    .style(move |theme: &Theme, _| {
                        let row = if active {
                            ui_style::selected_panel(theme)
                        } else {
                            ui_style::table_row(theme, display_row % 2 == 1)
                        };
                        button::Style {
                            background: row.background,
                            border: row.border,
                            text_color: theme.palette().text,
                            ..button::Style::default()
                        }
                    })
                    .on_press_maybe(
                        editable.then(|| Message::Rules(Event::Select(rule.id.clone()))),
                    ),
            );
        }
        if total == 0 {
            list = list
                .push(self.empty_state("没有匹配的规则", "调整搜索和来源筛选，或新增自定义规则。"));
        }
        let mut page = column![
            profile_header,
            container(shortcuts)
                .padding(12)
                .width(Length::Fill)
                .style(panel),
            aligned_row![
                self.search("搜索规则类型、内容或策略"),
                self.selection(
                    [
                        SourceFilter::All,
                        SourceFilter::Custom,
                        SourceFilter::Shortcut,
                        SourceFilter::Subscription
                    ],
                    state.filter,
                    |f| Message::Rules(Event::Filter(f)),
                    115
                ),
                self.rule_button(
                    "恢复订阅顺序",
                    Event::Restore,
                    !state.draft.order.is_empty(),
                    false
                )
            ]
            .spacing(8),
            tools,
            self.table_header(
                aligned_row![
                    self.label("顺序").size(12).width(42),
                    self.label("类型")
                        .size(12)
                        .width(if compact { 95 } else { 115 }),
                    self.label("规则内容").size(12).width(Length::Fill),
                    self.label("策略").size(12).width(85),
                    self.label("来源").size(12).width(90)
                ]
                .spacing(7)
                .into()
            ),
            container(page_scroll(list).height(Length::Fill))
                .width(Length::Fill)
                .style(panel),
            self.list_pager(total, offset)
        ]
        .spacing(if compact { 5 } else { 8 });
        if !state.warnings.is_empty() {
            page = page.push(
                container(page_scroll(self.caption(state.warnings.join("\n")).size(12)).height(40))
                    .padding(8)
                    .width(Length::Fill)
                    .style(|t| ui_style::badge(t, ui_style::Tone::Warning)),
            );
        }
        page = page.push(
            aligned_row![
                column![
                    self.label(if self.working {
                        "正在校验并应用…"
                    } else if state.dirty() {
                        "有未应用的修改"
                    } else {
                        "规则已保存"
                    })
                    .size(13)
                    .color(if state.dirty() {
                        ui_style::tone(&self.theme(), ui_style::Tone::Warning)
                    } else {
                        self.secondary()
                    }),
                    self.caption("应用前校验配置 · 订阅更新后保留自定义规则和排序")
                        .size(11)
                ]
                .spacing(3)
                .width(Length::Fill),
                self.rule_button("放弃修改", Event::Discard, state.dirty(), false),
                self.rule_button(
                    "应用修改",
                    Event::Apply,
                    state.dirty() && self.engine.is_some(),
                    true
                )
            ]
            .spacing(8),
        );
        let page: Element<'_, Message> = page.into();
        let dialog: Option<Element<'_, Message>> = if let Some(choice) = &state.pending_profile {
            Some(
                column![
                    self.title("切换订阅？"),
                    self.label(format!("切换到“{}”会放弃当前未应用的修改。", choice.name))
                        .size(14),
                    aligned_row![
                        Space::new().width(Length::Fill),
                        self.rule_button("继续编辑", Event::CancelProfile, true, false),
                        self.rule_button("放弃并切换", Event::ConfirmProfile, true, true)
                    ]
                    .spacing(8)
                ]
                .spacing(16)
                .into(),
            )
        } else if state.move_open {
            Some(
                column![
                    self.title("移动规则"),
                    self.caption(format!(
                        "指定位置 1–{}，包含当前筛选未显示的规则。",
                        state.rows.len()
                    )),
                    self.rule_input("规则位置", &state.move_position, Event::MoveInput),
                    aligned_row![
                        self.rule_button("移到顶部", Event::MoveEdge(false), true, false),
                        self.rule_button("移到底部", Event::MoveEdge(true), true, false),
                        Space::new().width(Length::Fill),
                        self.rule_button("取消", Event::CloseEditor, true, false),
                        self.rule_button(
                            "移动",
                            Event::MoveTo,
                            state
                                .move_position
                                .parse::<usize>()
                                .is_ok_and(|i| i > 0 && i <= state.rows.len()),
                            true
                        )
                    ]
                    .spacing(8)
                ]
                .spacing(12)
                .into(),
            )
        } else {
            state.editor.as_ref().map(|e| self.rule_editor(e, doc))
        };
        if let Some(dialog) = dialog {
            let overlay = container(container(dialog).padding(22).width(550).style(panel))
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(|_| container::Style {
                    background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.52).into()),
                    ..container::Style::default()
                });
            stack![page, iced::widget::opaque(overlay)].into()
        } else {
            page
        }
    }
    fn rule_editor<'a>(&self, editor: &'a Editor, doc: &'a Document) -> Element<'a, Message> {
        let mut body = column![
            self.title(if editor.id.is_some() {
                "编辑自定义规则"
            } else {
                "新增自定义规则"
            }),
            aligned_row![
                self.rule_button("表单", Event::TextMode(false), true, !editor.text_mode),
                self.rule_button("规则文本", Event::TextMode(true), true, editor.text_mode)
            ]
            .spacing(8)
        ]
        .spacing(12);
        if editor.text_mode {
            body = body
                .push(self.rule_input("DOMAIN,example.com,DIRECT", &editor.raw, Event::Raw))
                .push(self.caption("支持完整 mihomo 规则；复杂逻辑规则使用文本编辑。"));
        } else {
            body = body.push(
                aligned_row![
                    self.caption("规则类型").width(75),
                    self.selection(
                        TYPES.map(str::to_owned),
                        editor.kind.clone(),
                        |kind| Message::Rules(Event::Kind(kind)),
                        190
                    )
                ]
                .spacing(8),
            );
            if editor.kind != "MATCH" {
                body = body.push(self.rule_input(
                    "规则内容，如 example.com 或 192.168.0.0/16",
                    &editor.payload,
                    Event::Payload,
                ));
            }
            body = body.push(
                aligned_row![
                    self.caption("策略").width(75),
                    self.selection(
                        doc.policies.clone(),
                        editor.policy.clone(),
                        |policy| Message::Rules(Event::Policy(policy)),
                        250
                    )
                ]
                .spacing(8),
            );
            if matches!(
                editor.kind.as_str(),
                "GEOIP" | "IP-CIDR" | "IP-CIDR6" | "IP-ASN" | "RULE-SET"
            ) {
                body = body.push(
                    checkbox(editor.no_resolve)
                        .label("no-resolve · 不主动解析域名")
                        .text_size(13)
                        .on_toggle(|v| Message::Rules(Event::NoResolve(v))),
                );
            }
            body = body.push(
                self.caption(editor.form_raw())
                    .wrapping(text::Wrapping::WordOrGlyph),
            );
        }
        body = body.push(
            aligned_row![
                self.caption("插入位置").width(75),
                self.rule_input("从 1 开始的顺序", &editor.position, Event::Position),
                checkbox(editor.enabled)
                    .label("启用")
                    .text_size(13)
                    .on_toggle(|v| Message::Rules(Event::Enabled(v)))
            ]
            .spacing(10),
        );
        if !editor.error.is_empty() {
            body = body.push(
                self.label(&editor.error)
                    .size(13)
                    .color(ui_style::tone(&self.theme(), ui_style::Tone::Danger)),
            );
        }
        body = body.push(self.caption(
            "修改保存为草稿，应用后生效。订阅原有规则可排序，编辑与删除仅作用于自定义规则。",
        ));
        body = body.push(
            aligned_row![
                Space::new().width(Length::Fill),
                self.rule_button("取消", Event::CloseEditor, true, false),
                self.rule_button("保存到草稿", Event::SaveEditor, true, true)
            ]
            .spacing(8),
        );
        page_scroll(body).height(Length::Shrink).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> State {
        let doc = Document::from_raw(
            "test",
            false,
            "proxies: []\nrules: [\"DOMAIN,a.test,DIRECT\", \"MATCH,DIRECT\"]",
        )
        .unwrap();
        let mut state = State::default();
        state.load(Arc::new(doc), ProfileRules::default());
        state
    }
    #[test]
    fn editing_is_a_draft_and_reordering_uses_full_list() {
        let mut state = state();
        state.editor = Some(Editor::new(None, 1));
        let e = state.editor.as_mut().unwrap();
        e.payload = "b.test".into();
        state.save_editor().unwrap();
        assert!(state.dirty());
        assert_eq!(state.saved.custom.len(), 0);
        assert_eq!(state.rows[0].source, Source::Custom);
        state.move_selected(1);
        assert_eq!(state.rows[1].source, Source::Custom);
        state.draft = state.saved.clone();
        state.rebuild();
        assert!(!state.dirty());
        assert_eq!(state.rows.len(), 2);
    }
    #[test]
    fn invalid_or_duplicate_editor_keeps_draft_intact() {
        let mut state = state();
        state.editor = Some(Editor::new(None, 1));
        assert!(state.save_editor().is_err());
        assert!(!state.dirty());
        state.editor.as_mut().unwrap().payload = "a.test".into();
        assert!(state.save_editor().is_err());
        assert!(!state.dirty());
    }
}
