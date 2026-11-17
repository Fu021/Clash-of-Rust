use super::{
    input::Key,
    model::{Action, State},
    render::{Tone, wrap},
};
use anyhow::{Context, Result, ensure};
use clash_client_core::rule_manager::{self, Document, ProfileRules, Row, Source};
use clash_client_runtime::control::{Command, RuleChange};
use std::sync::Arc;

pub(super) struct Browser {
    pub document: Arc<Document>,
    pub saved: Arc<ProfileRules>,
    pub rows: Vec<Row>,
    pub warnings: Vec<String>,
    pub menu: Option<(String, usize)>,
    pub input: Option<Input>,
    pub pending: Option<(Command, String)>,
}

pub(super) struct Input {
    pub id: Option<String>,
    pub raw: String,
    pub editing: bool,
}

#[derive(Clone, Copy)]
enum Operation {
    Up,
    Down,
    Toggle,
    Edit,
    Remove,
    Shortcuts,
}

impl Browser {
    pub fn new(document: Arc<Document>, saved: Arc<ProfileRules>) -> Self {
        let (rows, warnings) = document.rows(&saved);
        Self {
            document,
            saved,
            rows,
            warnings,
            menu: None,
            input: None,
            pending: None,
        }
    }

    pub fn busy(&self) -> bool {
        self.menu.is_some() || self.input.is_some() || self.pending.is_some()
    }

    pub fn items(&self, query: &str) -> Vec<&str> {
        ["shortcuts", "add"]
            .into_iter()
            .chain(
                self.rows
                    .iter()
                    .filter(|row| {
                        query.is_empty()
                            || row.raw.to_lowercase().contains(query)
                            || row.id.to_lowercase().contains(query)
                            || row.source.to_string().contains(query)
                    })
                    .map(|row| row.id.as_str()),
            )
            .collect()
    }

    fn current(&self, state: &State) -> Result<()> {
        let poll = state.poll.as_ref().context("后台未连接")?;
        ensure!(state.error.is_none(), "后台状态过期，请先刷新");
        ensure!(
            poll.meta.settings.claude_protection.is_none(),
            "Claude 保护期间不能修改规则"
        );
        ensure!(
            poll.meta.settings.active_profile.as_deref() == Some(self.document.profile_id.as_str()),
            "订阅已变化，请重新选择规则"
        );
        let current = ProfileRules::for_profile(
            &poll.meta.settings,
            &self.document.profile_id,
            self.document.builtin,
        );
        ensure!(
            *current == *self.saved,
            "规则已变化，输入保留；请返回列表重新选择"
        );
        let document = state
            .shortcuts
            .as_ref()
            .filter(|status| status.matches(poll) && status.error.is_none())
            .and_then(|status| status.document.as_ref())
            .context("规则状态尚未读取，请刷新")?;
        ensure!(
            self.document
                .original
                .iter()
                .map(|row| (&row.id, &row.raw))
                .eq(document.original.iter().map(|row| (&row.id, &row.raw)))
                && self.document.policies == document.policies
                && self.document.included == document.included,
            "订阅规则已更新，输入保留；请返回列表重新选择"
        );
        Ok(())
    }

    fn prepare(
        &self,
        state: &State,
        change: RuleChange,
        description: String,
    ) -> Result<(Command, String)> {
        self.current(state)?;
        let mut rules = (*self.saved).clone();
        change.apply(&self.document, &mut rules)?;
        Ok((
            Command::SaveRules {
                id: self.document.profile_id.clone(),
                rules,
                expected: Some(rule_manager::revision(&self.saved)?),
            },
            description,
        ))
    }

    fn operations(&self) -> Vec<(Operation, &'static str)> {
        let Some((id, _)) = &self.menu else {
            return Vec::new();
        };
        let Some(index) = self.rows.iter().position(|row| row.id == *id) else {
            return Vec::new();
        };
        let row = &self.rows[index];
        let mut actions = Vec::new();
        if index > 0 {
            actions.push((Operation::Up, "上移"));
        }
        if index + 1 < self.rows.len() {
            actions.push((Operation::Down, "下移"));
        }
        match row.source {
            Source::Custom => {
                actions.push((Operation::Toggle, if row.enabled { "停用" } else { "启用" }));
                actions.push((Operation::Edit, "编辑"));
                actions.push((Operation::Remove, "删除"));
            }
            Source::Shortcut => actions.push((Operation::Shortcuts, "管理快捷规则")),
            Source::Subscription => {}
        }
        actions
    }
}

pub(super) fn sync(state: &mut State) {
    if state.page != super::model::Page::Rules {
        return;
    }
    if state.shortcuts_open {
        state.rules = None;
        if let Some(status) = &mut state.shortcuts {
            status.document = None;
        }
        return;
    }
    if state.rules.as_ref().is_some_and(Browser::busy) {
        return;
    }
    let document = state
        .shortcuts
        .as_ref()
        .filter(|status| state.poll.as_ref().is_some_and(|poll| status.matches(poll)))
        .and_then(|status| status.document.clone());
    let Some(document) = document else {
        if state.demo && state.rules.is_none() {
            let document = Document::from_raw(
                "demo",
                false,
                "proxies: []\nrules:\n  - DOMAIN,github.com,DIRECT\n  - MATCH,DIRECT\n",
            )
            .unwrap();
            state.rules = Some(Browser::new(
                Arc::new(document),
                Arc::new(ProfileRules::default()),
            ));
        }
        return;
    };
    let saved = ProfileRules::for_profile(
        &state.poll.as_ref().unwrap().meta.settings,
        &document.profile_id,
        document.builtin,
    );
    if state.rules.as_ref().is_some_and(|browser| {
        Arc::ptr_eq(&browser.document, &document) && *browser.saved == *saved
    }) {
        return;
    }
    let selected = state.items().get(state.selected).map(|id| (*id).to_owned());
    state.rules = Some(Browser::new(document, saved));
    let items = state.items();
    state.selected = selected
        .and_then(|id| items.iter().position(|candidate| *candidate == id))
        .unwrap_or(state.selected.min(items.len().saturating_sub(1)));
}

pub(super) fn open(state: &mut State) {
    let selected = state.items().get(state.selected).map(|id| (*id).to_owned());
    let Some(selected) = selected else {
        return;
    };
    state.message = None;
    if selected == "shortcuts" {
        shortcuts(state);
    } else if let Some(browser) = &mut state.rules {
        if selected == "add" {
            browser.input = Some(Input {
                id: None,
                raw: String::new(),
                editing: true,
            });
        } else {
            browser.menu = Some((selected, 0));
        }
    }
}

fn shortcuts(state: &mut State) {
    state.shortcuts_open = true;
    state.query.clear();
    state.selected = 0;
    sync(state);
}

pub(super) fn key(state: &mut State, key: Key) -> Option<Action> {
    let browser = state.rules.as_mut()?;
    if let Some(input) = browser.input.as_mut().filter(|input| input.editing) {
        match key {
            Key::Char(character) => append(&mut input.raw, &character.to_string()),
            Key::Paste(text) => append(&mut input.raw, &text),
            Key::Clear => input.raw.clear(),
            Key::Back => {
                input.raw.pop();
            }
            Key::Escape => input.editing = false,
            Key::Enter => {
                input.editing = false;
                review_input(state);
            }
            _ => {}
        }
        return Some(Action::None);
    }
    if browser.pending.is_some() {
        match key {
            Key::Escape | Key::Back => browser.pending = None,
            Key::Enter if state.demo => {
                browser.pending = None;
                state.message = Some("演示模式：未保存规则".into());
            }
            Key::Enter => match state.rules.as_ref().unwrap().current(state) {
                Ok(_) => {
                    let (command, _) = state.rules.as_mut().unwrap().pending.take().unwrap();
                    return Some(Action::SaveRules(Box::new(command)));
                }
                Err(error) => {
                    state.rules.as_mut().unwrap().pending = None;
                    state.message = Some(format!("{error:#}"));
                }
            },
            Key::Char('r') => return Some(Action::Refresh),
            _ => {}
        }
        return Some(Action::None);
    }
    if let Some(input) = &mut browser.input {
        match key {
            Key::Enter => input.editing = true,
            Key::Escape | Key::Back => browser.input = None,
            Key::Char('r') => return Some(Action::Refresh),
            _ => {}
        }
        return Some(Action::None);
    }
    if browser.menu.is_some() {
        let count = browser.operations().len();
        match key {
            Key::Up => {
                let selected = &mut browser.menu.as_mut().unwrap().1;
                *selected = selected.saturating_sub(1);
            }
            Key::Down | Key::Tab => {
                let selected = &mut browser.menu.as_mut().unwrap().1;
                *selected = (*selected + 1).min(count.saturating_sub(1));
            }
            Key::Escape | Key::Back => browser.menu = None,
            Key::Enter => review_operation(state),
            Key::Char('r') => return Some(Action::Refresh),
            _ => {}
        }
        return Some(Action::None);
    }
    None
}

fn append(raw: &mut String, text: &str) {
    for character in text.chars().filter(|character| !character.is_control()) {
        if raw.len() + character.len_utf8() > 8192 {
            break;
        }
        raw.push(character);
    }
}

fn review_input(state: &mut State) {
    let browser = state.rules.as_ref().unwrap();
    let input = browser.input.as_ref().unwrap();
    let change = if let Some(id) = &input.id {
        RuleChange::Edit {
            rule: id.clone(),
            raw: input.raw.clone(),
        }
    } else {
        RuleChange::Add {
            raw: input.raw.clone(),
            position: 1,
            enabled: true,
        }
    };
    review(state, change, format!("保存规则：{}", input.raw));
}

fn review_operation(state: &mut State) {
    let browser = state.rules.as_ref().unwrap();
    let (id, selected) = browser.menu.as_ref().unwrap();
    let Some((operation, label)) = browser.operations().get(*selected).copied() else {
        return;
    };
    let index = browser.rows.iter().position(|row| row.id == *id).unwrap();
    let row = &browser.rows[index];
    let change = match operation {
        Operation::Up => RuleChange::Move {
            rule: id.clone(),
            position: index,
        },
        Operation::Down => RuleChange::Move {
            rule: id.clone(),
            position: index + 2,
        },
        Operation::Toggle => RuleChange::Enable {
            rule: id.clone(),
            enabled: !row.enabled,
        },
        Operation::Remove => RuleChange::Remove { rule: id.clone() },
        Operation::Edit => {
            let input = Input {
                id: Some(id.clone()),
                raw: row.raw.to_string(),
                editing: true,
            };
            state.rules.as_mut().unwrap().input = Some(input);
            return;
        }
        Operation::Shortcuts => {
            shortcuts(state);
            return;
        }
    };
    let description = format!("{label}规则：{}", row.raw);
    review(state, change, description);
}

fn review(state: &mut State, change: RuleChange, description: String) {
    let browser = state.rules.as_ref().unwrap();
    if state.demo {
        state.message = Some("演示模式：未保存规则".into());
        return;
    }
    match browser.prepare(state, change, description) {
        Ok(pending) => {
            state.rules.as_mut().unwrap().pending = Some(pending);
            state.message = None;
        }
        Err(error) => state.message = Some(format!("{error:#}")),
    }
}

pub(super) fn panel(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let Some(browser) = &state.rules else {
        let message = match &state.poll {
            None => "规则 · 后台未连接，按 r 重试",
            Some(poll) if poll.meta.settings.active_profile.is_none() => {
                "规则 · 尚未选择订阅，按 3 打开订阅页"
            }
            Some(_) => "规则 · 状态尚未读取，按 r 刷新",
        };
        let mut lines = wrapped(vec![(message.into(), Tone::Accent)], width);
        if let Some(error) = state
            .shortcuts
            .as_ref()
            .and_then(|status| status.error.as_ref())
        {
            lines.extend(wrapped(
                vec![(format!("读取规则失败：{error}"), Tone::Warning)],
                width,
            ));
        }
        lines.truncate(height);
        return lines;
    };
    let mut lines = vec![(
        "规则 · Enter 打开操作 · / 搜索原文或来源".into(),
        Tone::Accent,
    )];
    if let Some(error) = state
        .shortcuts
        .as_ref()
        .and_then(|status| status.error.as_ref())
    {
        lines.push((
            format!("读取规则失败，旧列表仅供参考：{error}"),
            Tone::Warning,
        ));
    }
    if let Some(input) = &browser.input {
        lines.push((
            format!(
                "{}规则 · 单行文本",
                if input.id.is_some() {
                    "编辑"
                } else {
                    "新增"
                }
            ),
            Tone::Accent,
        ));
        lines.push((
            "示例：DOMAIN-SUFFIX,example.test,DIRECT".into(),
            Tone::Muted,
        ));
        lines.push((
            format!("策略：{}", browser.document.policies.join(" / ")),
            Tone::Muted,
        ));
        let tail = input.raw.chars().count().saturating_sub(1024);
        lines.push((
            format!(
                "{}{}",
                input.raw.chars().skip(tail).collect::<String>(),
                if input.editing { " ▏" } else { "" }
            ),
            Tone::Selected,
        ));
    } else if let Some((id, selected)) = &browser.menu {
        let row = browser.rows.iter().find(|row| row.id == *id).unwrap();
        lines.push((
            format!(
                "{} · {} · {}",
                row.source,
                if row.enabled { "启用" } else { "停用" },
                row.raw
            ),
            Tone::Plain,
        ));
        lines.push((format!("稳定 ID：{}", row.id), Tone::Muted));
        lines.push((
            "订阅原规则只调整顺序；快捷规则在独立入口管理。".into(),
            Tone::Muted,
        ));
        let mut header = wrapped(lines, width);
        header.truncate(height.saturating_sub(3));
        let actions = browser.operations();
        let available = height.saturating_sub(header.len());
        for (index, (_, label)) in actions
            .iter()
            .enumerate()
            .skip(selected.saturating_sub(available.saturating_sub(1)))
        {
            header.extend(wrapped(
                vec![(
                    format!("{} {label}", if index == *selected { "›" } else { " " }),
                    if index == *selected {
                        Tone::Selected
                    } else {
                        Tone::Plain
                    },
                )],
                width,
            ));
        }
        header.truncate(height);
        return header;
    } else {
        lines.push((
            format!("{} 条 · 来源：订阅 / 自定义 / 快捷规则", browser.rows.len()),
            Tone::Muted,
        ));
        if let Some(warning) = browser.warnings.first() {
            lines.push((warning.clone(), Tone::Warning));
        }
        let mut header = wrapped(lines, width);
        header.truncate(height.saturating_sub(1));
        let available = height.saturating_sub(header.len()).max(1);
        let items = state.items();
        let start = state.selected.saturating_sub(available.saturating_sub(1));
        let mut content = Vec::new();
        let mut selected_end = 0;
        for (index, id) in items.iter().enumerate().skip(start).take(available * 2) {
            let label = match *id {
                "shortcuts" => "快捷规则 →".into(),
                "add" => "新增自定义规则 →".into(),
                id => {
                    let row = browser.rows.iter().find(|row| row.id == id).unwrap();
                    format!(
                        "{} · {}{} · {}",
                        row.source,
                        if row.enabled { "启用" } else { "停用" },
                        if row.duplicate { " · 重复" } else { "" },
                        row.raw
                    )
                }
            };
            let selected = index == state.selected;
            content.extend(wrapped(
                vec![(
                    format!("{} {label}", if selected { "›" } else { " " }),
                    if selected {
                        Tone::Selected
                    } else {
                        Tone::Plain
                    },
                )],
                width,
            ));
            if selected {
                selected_end = content.len();
            }
        }
        header.extend(
            content
                .into_iter()
                .skip(selected_end.saturating_sub(available))
                .take(available),
        );
        return header;
    }
    let mut lines = wrapped(lines, width);
    if lines.len() > height {
        let title = lines.remove(0);
        let offset = lines.len().saturating_sub(height.saturating_sub(1));
        lines = std::iter::once(title)
            .chain(lines.into_iter().skip(offset))
            .collect();
    }
    lines
}

fn wrapped(lines: Vec<(String, Tone)>, width: usize) -> Vec<(String, Tone)> {
    lines
        .into_iter()
        .flat_map(|(text, tone)| wrap(&text, width).into_iter().map(move |text| (text, tone)))
        .collect()
}
