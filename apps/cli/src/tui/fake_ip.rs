use super::{
    input::Key,
    model::{Action, State},
    render::{Tone, wrap},
};
use anyhow::{Context, Result, ensure};
use clash_client_core::config::{DEFAULT_FAKE_IP_EXCLUSIONS, Settings, parse_fake_ip_exclusions};
use clash_client_runtime::control::Command;

pub(super) struct Entry {
    pub domain: String,
    pub custom: bool,
    pub default: bool,
    pub enabled: bool,
}

pub(super) struct Editor {
    pub selected: usize,
    pub draft: String,
    pub editing: bool,
    pub entries: Vec<Entry>,
    pub disabled_defaults: usize,
    pub pending: Option<(Command, String)>,
    pub saving_addition: bool,
}

impl Editor {
    pub fn new(settings: &Settings) -> Self {
        let mut editor = Self {
            selected: 0,
            draft: String::new(),
            editing: false,
            entries: Vec::new(),
            disabled_defaults: 0,
            pending: None,
            saving_addition: false,
        };
        editor.sync(settings);
        editor
    }

    pub fn sync(&mut self, settings: &Settings) {
        let selected = self
            .selected
            .checked_sub(2)
            .and_then(|index| self.entries.get(index))
            .map(|entry| entry.domain.clone());
        self.entries = settings
            .fake_ip_exclusion_entries()
            .into_iter()
            .map(|entry| Entry {
                domain: entry.domain.into(),
                custom: entry.custom,
                default: entry.dns_override,
                enabled: true,
            })
            .collect();
        for domain in DEFAULT_FAKE_IP_EXCLUSIONS {
            if !self
                .entries
                .iter()
                .any(|entry| entry.domain.eq_ignore_ascii_case(domain))
            {
                self.entries.push(Entry {
                    domain: (*domain).into(),
                    custom: false,
                    default: true,
                    enabled: false,
                });
            }
        }
        self.disabled_defaults = settings.fake_ip_disabled_defaults.len();
        self.selected = selected
            .and_then(|domain| self.entries.iter().position(|entry| entry.domain == domain))
            .map_or(self.selected.min(self.entries.len() + 1), |index| index + 2);
    }

    fn prepare(&self, settings: &Settings) -> Result<(Command, String)> {
        let mut next = settings.clone();
        let description = match self.selected {
            0 => {
                let values = parse_fake_ip_exclusions(&self.draft)?;
                ensure!(!values.is_empty(), "请输入域名后再保存");
                next.add_fake_ip_exclusions(&values)?;
                format!("新增排除项：{}", values.join(", "))
            }
            1 => {
                ensure!(self.disabled_defaults > 0, "默认排除项均已启用");
                next.restore_fake_ip_defaults()?;
                "恢复全部默认排除项，保留自定义项".into()
            }
            index => {
                let entry = self.entries.get(index - 2).context("请重新选择排除项")?;
                if entry.enabled {
                    next.remove_fake_ip_exclusion(&entry.domain)?;
                    format!(
                        "{}：{}",
                        if entry.default {
                            "停用默认排除项"
                        } else {
                            "删除自定义排除项"
                        },
                        entry.domain
                    )
                } else {
                    ensure!(next.dns_override, "请先开启 DNS 覆盖");
                    next.fake_ip_disabled_defaults
                        .retain(|domain| !domain.eq_ignore_ascii_case(&entry.domain));
                    format!("恢复默认排除项：{}", entry.domain)
                }
            }
        };
        settings.require_settings_change(&next)?;
        next.validate()?;
        Ok((
            Command::Settings {
                settings: next,
                expected: Some(serde_json::to_value(settings)?),
            },
            description,
        ))
    }
}

pub(super) fn key(state: &mut State, key: Key) -> Action {
    let editor = state.fake_ip.as_mut().unwrap();
    if editor.editing {
        match key {
            Key::Escape => editor.editing = false,
            Key::Back => {
                editor.draft.pop();
            }
            Key::Clear => editor.draft.clear(),
            Key::Char(character) => append(&mut editor.draft, &character.to_string()),
            Key::Paste(text) => append(&mut editor.draft, &text),
            Key::Enter => {
                editor.editing = false;
                return review(state);
            }
            _ => {}
        }
        return Action::None;
    }
    if key == Key::Char('r') {
        return Action::Refresh;
    }
    if editor.pending.is_some() {
        match key {
            Key::Escape | Key::Back => editor.pending = None,
            Key::Enter if state.demo => {
                editor.pending = None;
                state.message = Some("演示模式：未保存 Fake-IP 排除项".into());
            }
            Key::Enter if state.error.is_some() => {
                state.message = Some("后台状态过期，输入保留；等待读取恢复后再保存".into());
            }
            Key::Enter => {
                let unchanged = state.poll.as_ref().is_some_and(|poll| {
                    let Command::Settings { expected, .. } = &editor.pending.as_ref().unwrap().0
                    else {
                        return false;
                    };
                    serde_json::to_value(&poll.meta.settings)
                        .is_ok_and(|current| expected.as_ref() == Some(&current))
                });
                if unchanged {
                    let (command, _) = editor.pending.take().unwrap();
                    editor.saving_addition = editor.selected == 0;
                    return Action::SaveFakeIp(Box::new(command));
                }
                editor.pending = None;
                state.message = Some("设置已变化，输入保留；请重新确认操作".into());
            }
            _ => {}
        }
        return Action::None;
    }
    match key {
        Key::Up => editor.selected = editor.selected.saturating_sub(1),
        Key::Down | Key::Tab => {
            editor.selected = (editor.selected + 1).min(editor.entries.len() + 1);
        }
        Key::Escape | Key::Back => {
            state.fake_ip = None;
            state.message = None;
        }
        Key::Enter if editor.selected == 0 => editor.editing = true,
        Key::Enter => return review(state),
        _ => {}
    }
    Action::None
}

fn append(draft: &mut String, text: &str) {
    for character in text.chars() {
        let character = match character {
            '\n' | '\r' => ',',
            character if character.is_control() => continue,
            character => character,
        };
        if draft.len() + character.len_utf8() > 8192 {
            break;
        }
        draft.push(character);
    }
}

fn review(state: &mut State) -> Action {
    if state.error.is_some() {
        state.message = Some("后台状态过期，输入保留；等待读取恢复后再保存".into());
    } else if let Some(poll) = &state.poll {
        let editor = state.fake_ip.as_mut().unwrap();
        match editor.prepare(&poll.meta.settings) {
            Ok(pending) => {
                editor.pending = Some(pending);
                state.message = None;
            }
            Err(error) => state.message = Some(format!("{error:#}")),
        }
    } else {
        state.message = Some("后台未运行，输入保留".into());
    }
    Action::None
}

pub(super) fn panel(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let editor = state.fake_ip.as_ref().unwrap();
    let mut heading: Vec<_> = wrap("设置 · Fake-IP 排除项", width)
        .into_iter()
        .take(height.saturating_sub(1))
        .map(|line| (line, Tone::Accent))
        .collect();
    let available = height.saturating_sub(heading.len());
    let mut lines = Vec::new();
    let mut selected_range = (0, 0);
    let mut push = |text: String, tone| {
        lines.extend(wrap(&text, width).into_iter().map(|line| (line, tone)));
    };
    if let Some((_, description)) = &editor.pending {
        push(description.clone(), Tone::Selected);
        push("Enter 确认保存 · Esc 取消".into(), Tone::Warning);
    } else {
        if let Some(poll) = &state.poll {
            if !poll.meta.settings.dns_override {
                push(
                    "DNS 覆盖已关闭 · 排除项未应用，只可查看。".into(),
                    Tone::Warning,
                );
            } else if poll.meta.settings.claude_protection.is_some() {
                push("Claude 保护中 · 排除项只可查看。".into(), Tone::Warning);
            } else if !poll.meta.settings.fake_ip_enabled() {
                push(
                    "Fake-IP 已关闭 · 列表保留，启用后生效。".into(),
                    Tone::Muted,
                );
            }
        }
        push(
            "新增支持逗号或换行；订阅原有过滤独立合并。".into(),
            Tone::Muted,
        );
        for index in 0..editor.entries.len() + 2 {
            let selected = editor.selected == index;
            let text = match index {
                0 => format!(
                    "新增：{}{}",
                    editor.draft,
                    if editor.editing {
                        " ▏"
                    } else {
                        "（Enter 编辑）"
                    }
                ),
                1 => format!("恢复默认 · {} 项已停用", editor.disabled_defaults),
                index => {
                    let entry = &editor.entries[index - 2];
                    let source = match (entry.custom, entry.default) {
                        (true, true) => "用户自定义、DNS 覆盖",
                        (true, false) => "用户自定义",
                        _ => "DNS 覆盖默认",
                    };
                    format!(
                        "{} · {} · {source}",
                        entry.domain,
                        if entry.enabled { "启用" } else { "已停用" }
                    )
                }
            };
            if selected {
                selected_range.0 = lines.len();
            }
            let tone = if selected {
                Tone::Selected
            } else {
                Tone::Plain
            };
            for text in wrap(
                &format!("{} {text}", if selected { "›" } else { " " }),
                width,
            ) {
                lines.push((text, tone));
            }
            if selected {
                selected_range.1 = lines.len();
            }
        }
    }
    let offset = if selected_range.1 - selected_range.0 >= available {
        selected_range.0
    } else {
        selected_range.1.saturating_sub(available)
    };
    heading.extend(lines.into_iter().skip(offset).take(available));
    heading
}
