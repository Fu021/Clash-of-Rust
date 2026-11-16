use super::{form::Form, model::State, render::Tone, settings};
use anyhow::{Context, Result, ensure};
use clash_client_runtime::control::{Command, Poll, Remote};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub(super) struct Shortcuts {
    pub profile: String,
    pub revision: u64,
    pub included: [bool; 5],
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
) -> Option<Shortcuts> {
    let profile = poll.meta.settings.active_profile.as_ref()?;
    if let Some(cached) = cached.filter(|cached| {
        cached.matches(poll)
            && cached.error.is_none()
            && cached.sampled.elapsed() < Duration::from_secs(5)
    }) {
        return Some(cached);
    }
    let document = remote
        .request::<clash_client_core::rule_manager::Document>(Command::RuleDocument {
            id: profile.clone(),
        })
        .await;
    let (included, error) = match document {
        Ok(document) if document.profile_id == *profile => (document.included, None),
        Ok(_) => ([false; 5], Some("订阅已变化，请刷新后重试".into())),
        Err(error) => ([false; 5], Some(format!("{error:#}"))),
    };
    Some(Shortcuts {
        profile: profile.clone(),
        revision: poll.meta.view_revision,
        included,
        error,
        sampled: Instant::now(),
    })
}

fn included(state: &State, key: &str) -> Result<bool> {
    let poll = state.poll.as_ref().context("后台未连接")?;
    let status = state
        .shortcuts
        .as_ref()
        .filter(|status| status.matches(poll))
        .context("快捷规则状态尚未读取，请刷新")?;
    ensure!(
        status.error.is_none(),
        "读取快捷规则失败：{}",
        status.error.as_deref().unwrap_or_default()
    );
    let index = settings::RULE_KEYS
        .iter()
        .position(|candidate| *candidate == key)
        .context("未知快捷规则")?;
    Ok(status.included[index])
}

pub(super) fn open_shortcut(state: &State, key: &str) -> Result<Form> {
    ensure!(
        !included(state, key)?,
        "订阅已包含此规则，无需重复添加；原订阅保持不变"
    );
    settings::open(state.poll.as_ref().context("后台未连接")?, key)
}

pub(super) fn profiles(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let mut lines = vec![
        ("订阅 · Enter 选择并确认切换 · / 搜索".into(), Tone::Accent),
        ("新增、编辑来源和更新可在操作页管理。".into(), Tone::Muted),
    ];
    lines.truncate(height.saturating_sub(1));
    let available = height.saturating_sub(lines.len()).max(1);
    let Some(poll) = &state.poll else {
        lines.push(("后台未连接；未读取订阅".into(), Tone::Warning));
        return lines;
    };
    let items = state.items();
    if items.is_empty() {
        lines.push((
            "没有匹配的订阅；清除搜索或在操作页添加。".into(),
            Tone::Muted,
        ));
    }
    let offset = state.selected.saturating_sub(available - 1);
    for (index, id) in items.iter().enumerate().skip(offset).take(available) {
        let profile = poll
            .meta
            .profiles
            .iter()
            .find(|profile| profile.id == *id)
            .unwrap();
        let current = poll.meta.settings.active_profile.as_deref() == Some(*id);
        let text = format!(
            "{} {}{} · {}",
            if index == state.selected { "›" } else { " " },
            if current { "[当前] " } else { "" },
            profile.name,
            if profile.builtin {
                "内置"
            } else if profile.is_local() {
                "本地"
            } else {
                "远程"
            },
        );
        lines.push((
            super::render::fit(&text, width),
            if index == state.selected {
                Tone::Selected
            } else {
                Tone::Plain
            },
        ));
    }
    lines
}

pub(super) fn rules(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let profile_name = state
        .poll
        .as_ref()
        .and_then(|poll| {
            poll.meta
                .profiles
                .iter()
                .find(|profile| Some(&profile.id) == poll.meta.settings.active_profile.as_ref())
        })
        .map(|profile| profile.name.as_str())
        .unwrap_or("未选择");
    let mut lines = vec![
        (
            "快捷规则 · 当前订阅 · Enter 编辑 · / 搜索".into(),
            Tone::Accent,
        ),
        (
            format!("{profile_name} · 已有规则自动复用，不重复插入。"),
            Tone::Muted,
        ),
    ];
    lines.truncate(height.saturating_sub(1));
    let available = height.saturating_sub(lines.len()).max(1);
    let problem = match &state.poll {
        None => Some("后台未连接；未读取快捷规则".into()),
        Some(poll) => match poll
            .meta
            .profiles
            .iter()
            .find(|profile| Some(&profile.id) == poll.meta.settings.active_profile.as_ref())
        {
            None => Some("尚未选择订阅；按 7 打开订阅页".into()),
            Some(profile) if profile.builtin => {
                Some("内置订阅不使用快捷规则；按 7 选择其他订阅".into())
            }
            Some(_) => state
                .shortcuts
                .as_ref()
                .filter(|status| status.matches(poll))
                .map_or_else(
                    || Some("快捷规则状态尚未读取，请刷新".into()),
                    |status| {
                        status
                            .error
                            .as_ref()
                            .map(|error| format!("读取快捷规则失败：{error}"))
                    },
                ),
        },
    };
    if let Some(problem) = problem {
        lines.push((super::render::fit(&problem, width), Tone::Warning));
        return lines;
    }
    let offset = state.selected.saturating_sub(available - 1);
    let items = state.items();
    if items.is_empty() {
        lines.push(("没有匹配的快捷规则".into(), Tone::Muted));
    }
    for (index, key) in items.iter().enumerate().skip(offset).take(available) {
        let (value, warning) = match included(state, key) {
            Ok(true) => ("订阅已包含".into(), false),
            Ok(false) => (settings::display(state.poll.as_ref().unwrap(), key), false),
            Err(error) => (format!("{error:#}"), true),
        };
        let text = format!(
            "{} {}：{value}",
            if index == state.selected { "›" } else { " " },
            settings::title(key)
        );
        lines.push((
            super::render::fit(&text, width),
            if index == state.selected {
                Tone::Selected
            } else if warning {
                Tone::Warning
            } else {
                Tone::Plain
            },
        ));
    }
    lines
}
