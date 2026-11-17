use super::{
    form::{self, Form},
    input::Key,
    model::{Action, State},
    render::{Tone, rate, wrap},
    settings,
};
use anyhow::{Context, Result, ensure};
use clash_client_core::{network::DownloadPorts, update::schedule::Schedule};
use clash_client_runtime::update::Available;
use std::time::Instant;

#[derive(Default)]
pub(super) struct Updates {
    pub checking: bool,
    pub result: Option<Result<Option<Available>, String>>,
    pub pending: Option<Available>,
    pub channel: Option<bool>,
    pub schedule: Schedule,
}

impl Updates {
    pub fn available(&self) -> Option<&Available> {
        self.result.as_ref()?.as_ref().ok()?.as_ref()
    }
}

pub(super) fn items(state: &State) -> Vec<&'static str> {
    if state.poll.is_none() {
        return vec!["core"];
    }
    let mut items = vec![
        "core",
        "proxy_mode",
        "run_mode",
        "active_profile",
        "claude_protection",
        "update_check",
    ];
    if state
        .updates
        .available()
        .is_some_and(|update| update.package.is_some())
    {
        items.push("update_install");
    }
    items
}

fn mutable(state: &State) -> Result<()> {
    ensure!(state.error.is_none(), "后台状态过期，请先刷新");
    state
        .poll
        .as_ref()
        .context("后台尚未连接")?
        .meta
        .settings
        .require_mutable()
}

pub(super) fn sync(state: &mut State) -> bool {
    let Some(poll) = &state.poll else {
        let cancelled = state.updates.checking || state.updates.pending.is_some();
        state.updates.checking = false;
        state.updates.pending = None;
        return cancelled;
    };
    let channel = poll.meta.settings.update_prereleases;
    let changed = state
        .updates
        .channel
        .is_some_and(|previous| previous != channel);
    state.updates.channel = Some(channel);
    let blocked = poll.meta.settings.claude_protection.is_some() || state.error.is_some();
    if changed {
        state.updates.result = None;
        state.updates.schedule = Schedule::default();
        state.message = Some("更新通道已变化，请重新检查版本".into());
    }
    if changed || blocked {
        let cancelled = state.updates.checking || state.updates.pending.is_some();
        state.updates.checking = false;
        state.updates.pending = None;
        if state.page == super::model::Page::Home {
            state.selected = state.selected.min(items(state).len().saturating_sub(1));
        }
        return changed || cancelled;
    }
    false
}

pub(super) fn due(state: &mut State, now: Instant) -> bool {
    if state.demo
        || state.updates.checking
        || state.updates.pending.is_some()
        || state.saving
        || state.form.is_some()
        || state.fake_ip.is_some()
        || state
            .rules
            .as_ref()
            .is_some_and(super::rules::Browser::busy)
        || mutable(state).is_err()
        || state
            .poll
            .as_ref()
            .is_some_and(|poll| poll.meta.owner == clash_client_runtime::control::Owner::Gui)
    {
        return false;
    }
    state.updates.schedule.due(
        state
            .poll
            .as_ref()
            .unwrap()
            .meta
            .settings
            .update_check_interval_minutes,
        now,
    )
}

pub(super) fn begin_check(
    state: &mut State,
    now: Instant,
) -> Result<(Option<DownloadPorts>, bool)> {
    mutable(state)?;
    ensure!(!state.demo, "演示模式不访问更新服务");
    ensure!(!state.updates.checking, "正在检查更新，请稍候");
    let poll = state.poll.as_ref().unwrap();
    let channel = poll.meta.settings.update_prereleases;
    let ports = poll
        .snapshot
        .running
        .then_some(poll.meta.settings.download_ports());
    state.updates.schedule.checked(now);
    state.updates.channel = Some(channel);
    state.updates.checking = true;
    state.updates.pending = None;
    state.message = None;
    Ok((ports, channel))
}

pub(super) fn accept(state: &mut State, result: Result<Option<Available>, String>) {
    state.updates.checking = false;
    state.updates.result = Some(result);
    if state.page == super::model::Page::Home {
        state.selected = state.selected.min(items(state).len().saturating_sub(1));
    }
}

pub(super) fn confirm(state: &mut State, key: Key) -> Action {
    match key {
        Key::Escape | Key::Back => state.updates.pending = None,
        Key::Enter => {
            if state.demo {
                state.updates.pending = None;
                state.message = Some("演示模式：未安装更新".into());
            } else if let Err(error) = mutable(state) {
                state.message = Some(format!("{error:#}"));
            } else if let Some(available) = state.updates.pending.take() {
                return Action::InstallUpdate(Box::new(available));
            }
        }
        _ => {}
    }
    Action::None
}

pub(super) fn open(state: &mut State) -> Action {
    let Some(key) = items(state).get(state.selected).copied() else {
        return Action::None;
    };
    state.message = None;
    if key == "core" {
        if state.error.is_some() {
            state.message = Some("后台状态过期，请先刷新".into());
            return Action::None;
        }
        if let Some(poll) = &state.poll
            && let Err(error) = poll.meta.settings.require_mutable()
        {
            state.message = Some(format!("{error:#}"));
            return Action::None;
        }
        let path = if state
            .poll
            .as_ref()
            .is_some_and(|poll| poll.snapshot.running)
        {
            "stop"
        } else {
            "start"
        };
        let mut form = Form::new(form::commands().find(|spec| spec.path == path).unwrap());
        form.selected = form.fields.len();
        state.form = Some(form);
        return Action::None;
    }
    if key == "update_check" {
        if state.demo {
            state.message = Some("演示模式：未发起更新检查".into());
            return Action::None;
        }
        return Action::CheckUpdate;
    }
    if key == "update_install" {
        let result = (|| {
            mutable(state)?;
            ensure!(!state.updates.checking, "请等本次更新检查结束后再安装");
            let available = state.updates.available().context("请先检查更新")?;
            ensure!(available.package.is_some(), "当前平台没有可自动安装的包");
            Ok::<_, anyhow::Error>(available.clone())
        })();
        match result {
            Ok(available) => state.updates.pending = Some(available),
            Err(error) => state.message = Some(format!("{error:#}")),
        }
        return Action::None;
    }
    let result = (|| {
        ensure!(state.error.is_none(), "后台状态过期，请先刷新");
        let poll = state.poll.as_ref().context("后台尚未连接")?;
        if key != "claude_protection" {
            poll.meta.settings.require_mutable()?;
        }
        settings::open(poll, key)
    })();
    match result {
        Ok(form) => state.form = Some(form),
        Err(error) => state.message = Some(format!("{error:#}")),
    }
    Action::None
}

fn update_status(state: &State) -> String {
    if state.updates.checking {
        return "检查中…".into();
    }
    if state
        .poll
        .as_ref()
        .is_some_and(|poll| poll.meta.settings.claude_protection.is_some())
    {
        return "Claude 保护中，检查已暂停".into();
    }
    match &state.updates.result {
        Some(Ok(Some(update))) => format!(
            "发现 {} {}{}",
            if update.preview {
                "预发布版"
            } else {
                "正式版"
            },
            update.version,
            if update.package.is_none() {
                " · 当前平台无自动安装包"
            } else {
                ""
            },
        ),
        Some(Ok(None)) => "当前通道已是最新版本".into(),
        Some(Err(error)) => format!("检查失败：{error}"),
        None => {
            if state
                .poll
                .as_ref()
                .is_some_and(|poll| poll.meta.settings.update_check_interval_minutes == 0)
            {
                "未检查 · 自动检查已关闭".into()
            } else {
                "未检查".into()
            }
        }
    }
}

pub(super) fn panel(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let mut blocks = vec![("首页 · Enter 操作，修改前确认".into(), Tone::Accent, false)];
    if let Some(poll) = &state.poll {
        let owner = match poll.meta.owner {
            clash_client_runtime::control::Owner::Gui => "桌面后台",
            clash_client_runtime::control::Owner::Daemon => "CLI 后台",
        };
        blocks.push((
            format!(
                "{} · {owner} · {}",
                if poll.snapshot.running {
                    "内核运行中"
                } else {
                    "内核未运行"
                },
                poll.meta.version
            ),
            Tone::Accent,
            false,
        ));
        let traffic = if poll.snapshot.running {
            format!(
                "↓ 下载 {}/s · ↑ 上传 {}/s · 连接 {}",
                rate(poll.snapshot.download_rate),
                rate(poll.snapshot.upload_rate),
                poll.snapshot.connection_count
            )
        } else {
            "流量：内核未运行，暂无采样".into()
        };
        blocks.push((traffic, Tone::Plain, false));
        let route = if poll.snapshot.running {
            poll.snapshot
                .active_route
                .as_ref()
                .map(|route| route.join(" → "))
                .or_else(|| poll.snapshot.active_node.clone())
                .unwrap_or_else(|| "待确认".into())
        } else {
            "内核未运行".into()
        };
        blocks.push((
            format!(
                "{}：{route}",
                if poll.meta.settings.claude_protection.is_some() {
                    "保护节点"
                } else {
                    "参考线路"
                }
            ),
            Tone::Muted,
            false,
        ));
    } else {
        blocks.push((
            "尚未连接后台；选择下方项目启动。".into(),
            Tone::Muted,
            false,
        ));
    }
    for (index, key) in items(state).iter().enumerate() {
        let selected = index == state.selected;
        let label = match *key {
            "core" => {
                if state
                    .poll
                    .as_ref()
                    .is_some_and(|poll| poll.snapshot.running)
                {
                    "停止内核".into()
                } else {
                    "启动后台与内核".into()
                }
            }
            "update_check" => format!("检查更新 · {}", update_status(state)),
            "update_install" => {
                format!("安装更新 · {}", state.updates.available().unwrap().version)
            }
            key => format!(
                "{} · {}",
                settings::title(key),
                settings::display(state.poll.as_ref().unwrap(), key)
            ),
        };
        blocks.push((
            format!("{} {label}", if selected { "›" } else { " " }),
            if selected {
                Tone::Selected
            } else {
                Tone::Plain
            },
            selected,
        ));
    }
    let mut lines = Vec::new();
    let mut selected_end = 0;
    let mut selected_start = 0;
    for (text, tone, selected) in blocks {
        let start = lines.len();
        lines.extend(wrap(&text, width).into_iter().map(|text| (text, tone)));
        if selected {
            selected_start = start;
            selected_end = lines.len();
        }
    }
    let offset = if selected_end.saturating_sub(selected_start) >= height {
        selected_start
    } else {
        selected_end.saturating_sub(height)
    };
    lines.into_iter().skip(offset).take(height).collect()
}
