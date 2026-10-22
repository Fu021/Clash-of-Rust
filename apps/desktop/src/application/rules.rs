//! Native rule manager; only the visible subscription and one page of widgets
//! are retained. Rule changes remain drafts until the core validates them.
use super::*;
use rule_manager::{CustomRule, Document, Row, Source};

pub(super) const TYPES: [&str; 18] = [
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
pub(super) const SHORTCUT_LABELS: [&str; 5] = [
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
    pub(super) fn matches(self, source: Source) -> bool {
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
    DiscardConfirmed,
    CloseEditorConfirmed,
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
    pub(super) original: (String, bool, String),
    pub(super) id: Option<String>,
    pub(super) raw: String,
    pub(super) text_mode: bool,
    pub(super) kind: String,
    pub(super) payload: String,
    pub(super) policy: String,
    pub(super) no_resolve: bool,
    pub(super) enabled: bool,
    pub(super) position: String,
    pub(super) error: String,
}
impl Editor {
    fn new(rule: Option<&CustomRule>, position: usize) -> Self {
        let raw = rule.map_or("DOMAIN,,DIRECT", |r| r.rule.as_str());
        let parts = rule_manager::parts(raw).ok();
        let kind = parts.as_ref().map_or("DOMAIN", |p| p.kind).to_owned();
        let mut editor = Self {
            original: Default::default(),
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
        };
        editor.original = (editor.content(), editor.enabled, editor.position.clone());
        editor
    }
    fn content(&self) -> String {
        if !self.text_mode {
            return self.form_raw();
        }
        match rule_manager::draft_parts(&self.raw) {
            Ok(p) if TYPES.contains(&p.kind) => {
                let raw = if p.kind == "MATCH" {
                    format!("MATCH,{}", p.policy)
                } else {
                    format!("{},{},{}", p.kind, p.payload, p.policy)
                };
                if p.no_resolve {
                    format!("{raw},no-resolve")
                } else {
                    raw
                }
            }
            _ => self.raw.trim().into(),
        }
    }
    fn modified(&self) -> bool {
        (self.content(), self.enabled, self.position.clone()) != self.original
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
        } else if self.raw.trim().is_empty() {
            self.payload.clear();
            self.no_resolve = false;
        } else {
            match rule_manager::draft_parts(&self.raw) {
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
#[derive(Debug, Default)]
pub struct State {
    pub document: Option<Arc<Document>>,
    pub draft: ProfileRules,
    pub(super) saved: Arc<ProfileRules>,
    pub(super) rows: Vec<Row>,
    pub(super) warnings: Vec<String>,
    pub(super) selected: Option<String>,
    pub(super) filter: SourceFilter,
    pub(super) expanded: bool,
    pub(super) editor: Option<Editor>,
    pub(super) move_open: bool,
    pub(super) move_position: String,
    pub(super) pending_profile: Option<ProfileChoice>,
}
impl State {
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }
    pub fn has_editor(&self) -> bool {
        self.editor.is_some()
    }
    pub fn dirty(&self) -> bool {
        &self.draft != self.saved.as_ref()
    }
    pub fn editor_modified(&self) -> bool {
        self.editor.as_ref().is_some_and(Editor::modified)
    }
    pub fn modal_open(&self) -> bool {
        self.editor.is_some() || self.move_open || self.pending_profile.is_some()
    }
    pub fn load(&mut self, document: Arc<Document>, rules: Arc<ProfileRules>) {
        self.document = Some(document);
        self.draft = rules.as_ref().clone();
        self.saved = rules;
        self.selected = None;
        self.editor = None;
        self.move_open = false;
        self.pending_profile = None;
        self.rebuild();
    }
    pub fn refresh_document(&mut self, document: Arc<Document>) {
        if self
            .document
            .as_ref()
            .is_some_and(|old| old.profile_id == document.profile_id)
        {
            self.document = Some(document);
            self.rebuild();
        }
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
    pub(super) fn selected_index(&self) -> Option<usize> {
        self.selected
            .as_ref()
            .and_then(|id| self.rows.iter().position(|r| &r.id == id))
    }
    pub(super) fn selected_custom(&self) -> Option<&CustomRule> {
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
        if self.working || self.exiting {
            return Task::none();
        }
        let focus_editor = matches!(&event, Event::Add | Event::Edit);
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
                if state.dirty() || state.editor_modified() {
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
                if state.dirty() || state.editor_modified() {
                    return self.ask(
                        "放弃规则修改？",
                        "当前未应用的规则顺序、快捷规则与自定义规则修改将被丢弃。".into(),
                        "放弃修改",
                        desktop_view::Pending::Rules(Event::DiscardConfirmed),
                    );
                }
            }
            Event::DiscardConfirmed => {
                state.draft = state.saved.as_ref().clone();
                state.editor = None;
                state.move_open = false;
                state.rebuild();
            }
            Event::Apply => {
                if state.dirty()
                    && state.editor.is_none()
                    && let Some(doc) = &state.document
                {
                    let action = Action::SaveRules(
                        doc.profile_id.clone(),
                        state.draft.clone(),
                        state.saved.clone(),
                    );
                    return self.dispatch_scope(action, Scope::Rules);
                }
            }
            Event::CloseEditor => {
                if state.pending_profile.is_some() {
                    state.pending_profile = None;
                } else if state.editor_modified() {
                    return self.ask(
                        "放弃规则编辑？",
                        "这条规则的编辑尚未加入草稿。关闭后将丢弃本次编辑，已有规则草稿仍会保留。"
                            .into(),
                        "放弃编辑",
                        desktop_view::Pending::Rules(Event::CloseEditorConfirmed),
                    );
                } else {
                    state.editor = None;
                    state.move_open = false;
                }
            }
            Event::CloseEditorConfirmed => {
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
        if focus_editor && let Some(editor) = &state.editor {
            desktop_runtime::widget::operation::focus(if editor.text_mode {
                "rule-raw"
            } else {
                "rule-payload"
            })
        } else {
            Task::none()
        }
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
        state.load(Arc::new(doc), Arc::new(ProfileRules::default()));
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
        state.draft = state.saved.as_ref().clone();
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
    #[test]
    fn unfinished_editor_switches_modes_without_bypassing_save_validation() {
        let mut state = state();
        state.editor = Some(Editor::new(None, 1));
        let editor = state.editor.as_mut().unwrap();
        editor.set_mode(true);
        editor.set_mode(false);
        assert!(!editor.text_mode);
        assert!(editor.error.is_empty());
        assert!(state.save_editor().is_err());
        assert!(!state.dirty());

        let editor = state.editor.as_mut().unwrap();
        editor.set_mode(true);
        editor.raw.clear();
        editor.set_mode(false);
        assert!(!editor.text_mode);
        assert!(editor.payload.is_empty());
        assert!(editor.error.is_empty());

        editor.set_mode(true);
        editor.raw = "DOMAIN,example.com,".into();
        editor.set_mode(false);
        assert!(!editor.text_mode);
        assert_eq!(editor.payload, "example.com");
        assert!(editor.policy.is_empty());
        assert!(state.save_editor().is_err());
        assert!(!state.dirty());
    }
    #[test]
    fn editor_mode_conversion_preserves_fields_and_complex_rules() {
        let mut editor = Editor::new(None, 2);
        editor.set_mode(true);
        editor.raw = "IP-CIDR,192.168.0.0/16,REJECT,no-resolve".into();
        editor.set_mode(false);
        assert!(!editor.text_mode);
        assert_eq!(editor.kind, "IP-CIDR");
        assert_eq!(editor.payload, "192.168.0.0/16");
        assert_eq!(editor.policy, "REJECT");
        assert!(editor.no_resolve);
        editor.set_mode(true);
        assert_eq!(editor.raw, "IP-CIDR,192.168.0.0/16,REJECT,no-resolve");

        let raw = "AND,((DOMAIN,a.test),(NETWORK,TCP)),REJECT";
        editor.raw = raw.into();
        editor.set_mode(false);
        assert!(editor.text_mode);
        assert_eq!(editor.raw, raw);
        assert!(!editor.error.is_empty());
    }
    #[test]
    fn refreshing_subscription_preserves_pending_editor_and_draft() {
        let mut state = state();
        state.editor = Some(Editor::new(None, 1));
        state.editor.as_mut().unwrap().payload = "b.test".into();
        state.save_editor().unwrap();
        let before = state.draft.clone();
        state.editor = Some(Editor::new(None, 2));
        let updated = Document::from_raw(
            "test",
            false,
            "proxies: []\nrules: [\"DOMAIN,new.test,DIRECT\", \"MATCH,DIRECT\"]",
        )
        .unwrap();
        state.refresh_document(Arc::new(updated));
        assert_eq!(state.draft, before);
        assert!(state.editor.is_some());
        assert!(
            state
                .rows
                .iter()
                .any(|r| r.raw.as_ref() == "DOMAIN,new.test,DIRECT")
        );
        assert!(
            !state
                .rows
                .iter()
                .any(|r| r.raw.as_ref() == "DOMAIN,a.test,DIRECT")
        );
    }
}
