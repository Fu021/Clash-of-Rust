use super::{form::Form, model::State, render, render::Tone, settings};
use anyhow::{Context, Result, ensure};
use clash_client_core::{
    config::{Language, Profile},
    subscription,
};
use clash_client_runtime::control::{Command, Poll, Remote};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

#[derive(Clone)]
pub(super) struct Shortcuts {
    pub profile: String,
    pub revision: u64,
    pub included: [bool; 5],
    pub document: Option<std::sync::Arc<clash_client_core::rule_manager::Document>>,
    pub error: Option<String>,
    pub sampled: Instant,
}

impl Shortcuts {
    pub fn matches(&self, poll: &Poll) -> bool {
        poll.meta.settings.active_profile.as_deref() == Some(self.profile.as_str())
            && poll.meta.view_revision == self.revision
    }
}

pub(super) async fn read_shortcuts(
    remote: &Remote,
    poll: &Poll,
    cached: Option<Shortcuts>,
    include_rules: bool,
    language: Language,
) -> Option<Shortcuts> {
    let profile = poll.meta.settings.active_profile.as_ref()?;
    if let Some(cached) = cached.filter(|cached| {
        cached.matches(poll)
            && cached.error.is_none()
            && (!include_rules || cached.document.is_some())
            && cached.sampled.elapsed() < Duration::from_secs(5)
    }) {
        return Some(cached);
    }
    let document = remote
        .request::<clash_client_core::rule_manager::Document>(Command::RuleDocument {
            id: profile.clone(),
        })
        .await;
    let (included, document, error) = match document {
        Ok(document) if document.profile_id == *profile => (
            document.included,
            include_rules.then(|| std::sync::Arc::new(document)),
            None,
        ),
        Ok(_) => (
            [false; 5],
            None,
            Some(
                language
                    .text(
                        "订阅已变化，请刷新后重试",
                        "The subscription changed; refresh and try again",
                    )
                    .into(),
            ),
        ),
        Err(error) => (
            [false; 5],
            None,
            Some(crate::output::localized_error(&error, language)),
        ),
    };
    Some(Shortcuts {
        profile: profile.clone(),
        revision: poll.meta.view_revision,
        included,
        document,
        error,
        sampled: Instant::now(),
    })
}

fn included(state: &State, key: &str) -> Result<bool> {
    let language = state.language;
    let poll = state
        .poll
        .as_ref()
        .context(language.text("后台未连接", "Backend disconnected"))?;
    let status = state
        .shortcuts
        .as_ref()
        .filter(|status| status.matches(poll))
        .context(language.text(
            "快捷规则状态尚未读取，请刷新",
            "Rule shortcut status is unavailable; refresh to retry",
        ))?;
    ensure!(
        status.error.is_none(),
        "{}{}",
        language.text("读取快捷规则失败：", "Failed to read rule shortcuts: "),
        status.error.as_deref().unwrap_or_default()
    );
    let index = settings::RULE_KEYS
        .iter()
        .position(|candidate| *candidate == key)
        .context(language.text("未知快捷规则", "Unknown rule shortcut"))?;
    Ok(status.included[index])
}

pub(super) fn open_shortcut(state: &State, key: &str) -> Result<Form> {
    ensure!(
        !included(state, key)?,
        "{}",
        state.language.text(
            "订阅已包含此规则，无需重复添加；原订阅保持不变",
            "The subscription already contains this rule; no duplicate is needed",
        )
    );
    settings::open(
        state
            .poll
            .as_ref()
            .context(state.language.text("后台未连接", "Backend disconnected"))?,
        key,
        state.language,
    )
}

pub(super) fn profiles(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let language = state.language;
    let header = vec![
        (
            language
                .text(
                    "订阅 · Enter 选择并确认切换 · / 搜索",
                    "Subscriptions · Enter select and confirm · / search",
                )
                .into(),
            Tone::Accent,
        ),
        (
            language
                .text(
                    "新增、编辑来源和更新可在操作页管理。",
                    "Add, edit, and update subscriptions under Actions.",
                )
                .into(),
            Tone::Muted,
        ),
    ];
    let Some(poll) = &state.poll else {
        return super::render::list(
            header,
            std::iter::once((
                language
                    .text(
                        "后台未连接；未读取订阅",
                        "Backend disconnected; subscriptions unavailable",
                    )
                    .into(),
                Tone::Warning,
            )),
            0,
            width,
            height,
        );
    };
    let items = state.items();
    if items.is_empty() {
        return super::render::list(
            header,
            std::iter::once((
                language
                    .text(
                        "没有匹配的订阅；清除搜索或在操作页添加。",
                        "No matching subscriptions; clear the search or add one under Actions.",
                    )
                    .into(),
                Tone::Muted,
            )),
            0,
            width,
            height,
        );
    }
    let row = |index, id: &str| {
        let profile = poll
            .meta
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .unwrap();
        let current = poll.meta.settings.active_profile.as_deref() == Some(id);
        let selected = index == state.selected;
        (
            profile_lines(profile, current, selected, width, height, language),
            if selected {
                Tone::Selected
            } else {
                Tone::Plain
            },
        )
    };
    let reserved_rows = items
        .get(state.selected)
        .map_or(1, |id| row(state.selected, id).0.len());
    let rows = items.iter().enumerate().map(|(index, id)| row(index, id));
    render::list_rows(header, rows, state.selected, width, height, reserved_rows)
}

fn profile_lines(
    profile: &Profile,
    current: bool,
    selected: bool,
    width: usize,
    height: usize,
    language: Language,
) -> Vec<String> {
    let name = crate::output::profile_name(profile, language).replace(char::is_control, " ");
    let marker = if selected { "›" } else { " " };
    let current = if current {
        language.text("[当前] ", "[Current] ")
    } else {
        ""
    };
    let min_name_width = name.width().min(3);
    let current = if width >= marker.width() + 1 + current.width() + min_name_width {
        current
    } else {
        ""
    };
    let local = profile.is_local();
    let source = if profile.builtin {
        language.text("内置", "Built-in")
    } else if local {
        language.text("本地", "Local")
    } else {
        language.text("远程", "Remote")
    };
    let title = format!("{marker} {current}{name} · {source}");
    if profile.builtin || local {
        return render::wrap(&title, width);
    }

    let summary = subscription::summary(profile.usage.as_deref(), profile.updated, language);
    let inline = format!("{title} · {summary}");
    if inline.width() <= width {
        return vec![inline];
    }

    let name_width =
        width.saturating_sub(marker.width() + 1 + current.width() + 3 + source.width());
    let mut lines = vec![if title.width() <= width {
        title
    } else if name_width < min_name_width {
        render::fit(&format!("{marker} {current}{name}"), width)
    } else {
        let name = render::fit(&name, name_width);
        format!("{marker} {current}{} · {source}", name.trim_end())
    }];
    lines.extend(profile_summary_lines(
        &summary,
        width,
        height.saturating_sub(1),
        language,
    ));
    lines
}

fn profile_summary_lines(
    summary: &str,
    width: usize,
    height: usize,
    language: Language,
) -> Vec<String> {
    if height == 0 {
        return Vec::new();
    }
    let mut parts = summary.split(" · ").collect::<Vec<_>>();
    let mut lines = pack_summary(&parts, width);
    if lines.len() > height.min(2)
        && parts
            .last()
            .is_some_and(|part| part.starts_with(language.text("更新 ", "Updated ")))
    {
        parts.pop();
        lines = pack_summary(&parts, width);
    }
    if lines.len() > height
        && let Some(expiry) = parts.last().filter(|part| {
            **part == language.text("不限期", "No expiry")
                || part.starts_with(language.text("到期 ", "Expires "))
        })
    {
        let mut expiry_lines = render::wrap(expiry, width);
        if expiry_lines.len() > height
            && let Some(date) = expiry.strip_prefix(language.text("到期 ", "Expires "))
            && date.starts_with(|character: char| character.is_ascii_digit())
            && date.width() <= width
        {
            expiry_lines = vec![date.into()];
        }
        lines = pack_summary(&parts[..parts.len() - 1], width);
        lines.truncate(height.saturating_sub(expiry_lines.len()));
        lines.extend(expiry_lines);
    }
    lines.truncate(height);
    lines
}

fn pack_summary(parts: &[&str], width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for part in parts {
        if part.width() > width {
            lines.extend(render::wrap(part, width));
        } else if let Some(line) = lines.last_mut()
            && line.width() + 3 + part.width() <= width
        {
            line.push_str(" · ");
            line.push_str(part);
        } else {
            lines.push((*part).into());
        }
    }
    lines
}

pub(super) fn rules(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let language = state.language;
    let profile_name = state
        .poll
        .as_ref()
        .and_then(|poll| {
            poll.meta
                .profiles
                .iter()
                .find(|profile| Some(&profile.id) == poll.meta.settings.active_profile.as_ref())
        })
        .map(|profile| crate::output::profile_name(profile, language))
        .unwrap_or(language.text("未选择", "None selected"));
    let header = vec![
        (
            language
                .text(
                    "快捷规则 · 当前订阅 · Enter 编辑 · / 搜索",
                    "Rule shortcuts · Current subscription · Enter edit · / search",
                )
                .into(),
            Tone::Accent,
        ),
        (
            format!(
                "{profile_name} · {}",
                language.text(
                    "已有规则自动复用，不重复插入。",
                    "Existing rules are reused without duplicates.",
                )
            ),
            Tone::Muted,
        ),
    ];
    let problem = match &state.poll {
        None => Some(
            language
                .text(
                    "后台未连接；未读取快捷规则",
                    "Backend disconnected; rule shortcuts unavailable",
                )
                .into(),
        ),
        Some(poll) => match poll
            .meta
            .profiles
            .iter()
            .find(|profile| Some(&profile.id) == poll.meta.settings.active_profile.as_ref())
        {
            None => Some(
                language
                    .text(
                        "尚未选择订阅；按 3 打开订阅页",
                        "Select a subscription; press 3 to open subscriptions",
                    )
                    .into(),
            ),
            Some(profile) if profile.builtin => Some(
                language
                    .text(
                        "内置订阅不使用快捷规则；按 3 选择其他订阅",
                        "Built-in profiles do not use shortcuts; press 3 to select another",
                    )
                    .into(),
            ),
            Some(_) => state
                .shortcuts
                .as_ref()
                .filter(|status| status.matches(poll))
                .map_or_else(
                    || {
                        Some(
                            language
                                .text(
                                    "快捷规则状态尚未读取，请刷新",
                                    "Rule shortcut status is unavailable; refresh to retry",
                                )
                                .into(),
                        )
                    },
                    |status| {
                        status.error.as_ref().map(|error| {
                            format!(
                                "{}{error}",
                                language
                                    .text("读取快捷规则失败：", "Failed to read rule shortcuts: ",)
                            )
                        })
                    },
                ),
        },
    };
    if let Some(problem) = problem {
        return super::render::list(
            header,
            std::iter::once((problem, Tone::Warning)),
            0,
            width,
            height,
        );
    }
    let items = state.items();
    if items.is_empty() {
        return super::render::list(
            header,
            std::iter::once((
                language
                    .text("没有匹配的快捷规则", "No matching rule shortcuts")
                    .into(),
                Tone::Muted,
            )),
            0,
            width,
            height,
        );
    }
    let rows = items.iter().enumerate().map(|(index, key)| {
        let (value, warning) = match included(state, key) {
            Ok(true) => (
                language
                    .text("订阅已包含", "Included in subscription")
                    .into(),
                false,
            ),
            Ok(false) => (
                settings::display(state.poll.as_ref().unwrap(), key, state.language),
                false,
            ),
            Err(error) => (format!("{error:#}"), true),
        };
        let text = format!(
            "{} {}：{value}",
            if index == state.selected { "›" } else { " " },
            settings::title(key, state.language)
        );
        (
            text,
            if index == state.selected {
                Tone::Selected
            } else if warning {
                Tone::Warning
            } else {
                Tone::Plain
            },
        )
    });
    super::render::list(header, rows, state.selected, width, height)
}
