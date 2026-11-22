use super::{
    form::{self, Form},
    input::Key,
    model::{Action, State},
    render::{Tone, rate, wrap},
    settings,
};
use anyhow::{Context, Result, ensure};
use clash_client_core::{config::Language, network::DownloadPorts, update::schedule::Schedule};
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
    let Some(poll) = &state.poll else {
        return vec!["core"];
    };
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
        && state.error.is_none()
        && poll.meta.settings.claude_protection.is_none()
    {
        items.push("update_install");
    }
    items
}

fn mutable(state: &State) -> Result<()> {
    ensure!(
        state.error.is_none(),
        "{}",
        state.language.text(
            "后台状态过期，请先刷新",
            "Backend state is stale; refresh first"
        )
    );
    state
        .poll
        .as_ref()
        .context(
            state
                .language
                .text("后台尚未连接", "Backend is not connected"),
        )?
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
        state.message = Some(
            state
                .language
                .text(
                    "更新通道已变化，请重新检查版本",
                    "Update channel changed; check again",
                )
                .into(),
        );
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
    ensure!(
        !state.demo,
        "{}",
        state.language.text(
            "演示模式不访问更新服务",
            "Demo mode does not check for updates"
        )
    );
    ensure!(
        !state.updates.checking,
        "{}",
        state
            .language
            .text("正在检查更新，请稍候", "An update check is already running")
    );
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
                state.message = Some(
                    state
                        .language
                        .text("演示模式：未安装更新", "Demo mode: update not installed")
                        .into(),
                );
            } else if let Err(error) = mutable(state) {
                state.message = Some(crate::output::localized_error(&error, state.language));
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
            state.message = Some(
                state
                    .language
                    .text(
                        "后台状态过期，请先刷新",
                        "Backend state is stale; refresh first",
                    )
                    .into(),
            );
            return Action::None;
        }
        if let Some(poll) = &state.poll
            && poll.snapshot.running
            && let Err(error) = poll.meta.settings.require_mutable()
        {
            state.message = Some(crate::output::localized_error(&error, state.language));
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
        let mut form = Form::new_in(
            form::commands().find(|spec| spec.path == path).unwrap(),
            state.language,
        );
        form.selected = form.fields.len();
        state.form = Some(form);
        return Action::None;
    }
    if key == "update_check" {
        if state.demo {
            state.message = Some(
                state
                    .language
                    .text(
                        "演示模式：未发起更新检查",
                        "Demo mode: update check not started",
                    )
                    .into(),
            );
            return Action::None;
        }
        return if state.updates.checking {
            Action::None
        } else {
            Action::CheckUpdate
        };
    }
    if key == "update_install" {
        if state.updates.checking {
            return Action::None;
        }
        let result = (|| {
            mutable(state)?;
            let available = state.updates.available().context(
                state
                    .language
                    .text("请先检查更新", "Check for updates first"),
            )?;
            ensure!(
                available.package.is_some(),
                "{}",
                state.language.text(
                    "当前平台没有可自动安装的包",
                    "No automatic installer is available for this platform"
                )
            );
            Ok::<_, anyhow::Error>(available.clone())
        })();
        match result {
            Ok(available) => state.updates.pending = Some(available),
            Err(error) => {
                state.message = Some(crate::output::localized_error(&error, state.language))
            }
        }
        return Action::None;
    }
    let result = (|| {
        ensure!(
            state.error.is_none(),
            "{}",
            state.language.text(
                "后台状态过期，请先刷新",
                "Backend state is stale; refresh first"
            )
        );
        let poll = state.poll.as_ref().context(
            state
                .language
                .text("后台尚未连接", "Backend is not connected"),
        )?;
        if key != "claude_protection" {
            poll.meta.settings.require_mutable()?;
        }
        settings::open(poll, key, state.language)
    })();
    match result {
        Ok(form) => state.form = Some(form),
        Err(error) => state.message = Some(crate::output::localized_error(&error, state.language)),
    }
    Action::None
}

fn update_status(state: &State) -> String {
    let language = state.language;
    if state.updates.checking {
        return language.text("检查中…", "Checking…").into();
    }
    if state
        .poll
        .as_ref()
        .is_some_and(|poll| poll.meta.settings.claude_protection.is_some())
    {
        return language
            .text(
                "Claude 保护中，检查已暂停",
                "Update checks paused during Claude protection",
            )
            .into();
    }
    match &state.updates.result {
        Some(Ok(Some(update))) => {
            let release = if update.preview {
                language.text("预发布版", "pre-release")
            } else {
                language.text("正式版", "release")
            };
            let offer = match language {
                Language::SimplifiedChinese => format!("发现 {release} {}", update.version),
                Language::English => format!("Found {release} {}", update.version),
            };
            match update.package_manager.map(|manager| {
                clash_client_runtime::update::package_manager_update_hint_in(manager, language)
            }) {
                Some(hint) => format!("{offer} · {hint}"),
                None if update.package.is_none() => format!(
                    "{offer} · {}",
                    language.text(
                        "当前平台无自动安装包",
                        "No automatic installer for this platform"
                    )
                ),
                None => offer,
            }
        }
        Some(Ok(None)) => language
            .text("当前通道已是最新版本", "Up to date on this channel")
            .into(),
        Some(Err(error)) => format!("{}{error}", language.text("检查失败：", "Check failed: ")),
        None => {
            if state
                .poll
                .as_ref()
                .is_some_and(|poll| poll.meta.settings.update_check_interval_minutes == 0)
            {
                language
                    .text(
                        "未检查 · 自动检查已关闭",
                        "Not checked · automatic checks disabled",
                    )
                    .into()
            } else {
                language.text("未检查", "Not checked").into()
            }
        }
    }
}

pub(super) fn panel(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let language = state.language;
    let mut blocks = vec![(
        language
            .text(
                "首页 · Enter 操作，修改前确认",
                "Home · Enter to act, then confirm changes",
            )
            .into(),
        Tone::Accent,
        false,
    )];
    if let Some(poll) = &state.poll {
        let owner = match poll.meta.owner {
            clash_client_runtime::control::Owner::Gui => {
                language.text("桌面后台", "Desktop backend")
            }
            clash_client_runtime::control::Owner::Daemon => {
                language.text("CLI 后台", "CLI backend")
            }
        };
        blocks.push((
            format!(
                "{} · {owner} · {}",
                if poll.snapshot.running {
                    language.text("内核运行中", "Core running")
                } else {
                    language.text("内核未运行", "Core stopped")
                },
                poll.meta.version
            ),
            Tone::Accent,
            false,
        ));
        let traffic = if poll.snapshot.running {
            match language {
                Language::SimplifiedChinese => format!(
                    "↓ 下载 {}/s · ↑ 上传 {}/s · 连接 {}",
                    rate(poll.snapshot.download_rate),
                    rate(poll.snapshot.upload_rate),
                    poll.snapshot.connection_count
                ),
                Language::English => format!(
                    "↓ Download {}/s · ↑ Upload {}/s · Connections {}",
                    rate(poll.snapshot.download_rate),
                    rate(poll.snapshot.upload_rate),
                    poll.snapshot.connection_count
                ),
            }
        } else {
            language
                .text(
                    "流量：内核未运行，暂无采样",
                    "Traffic: core stopped; no samples",
                )
                .into()
        };
        blocks.push((traffic, Tone::Plain, false));
        let route = if poll.snapshot.running {
            poll.snapshot
                .active_route
                .as_ref()
                .map(|route| route.join(" → "))
                .or_else(|| poll.snapshot.active_node.clone())
                .unwrap_or_else(|| language.text("待确认", "Unconfirmed").into())
        } else {
            language.text("内核未运行", "Core stopped").into()
        };
        blocks.push((
            format!(
                "{}{}{route}",
                if poll.meta.settings.claude_protection.is_some() {
                    language.text("保护节点", "Protected node")
                } else {
                    language.text("参考线路", "Reference route")
                },
                language.text("：", ": ")
            ),
            Tone::Muted,
            false,
        ));
    } else {
        blocks.push((
            language
                .text(
                    "尚未连接后台；选择下方项目启动。",
                    "Backend disconnected; select an action below to start it.",
                )
                .into(),
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
                    language.text("停止内核", "Stop core").into()
                } else {
                    language
                        .text("启动后台与内核", "Start backend and core")
                        .into()
                }
            }
            "update_check" => format!(
                "{} · {}",
                language.text("检查更新", "Check for updates"),
                update_status(state)
            ),
            "update_install" => format!(
                "{} · {}{}",
                language.text("安装更新", "Install update"),
                state.updates.available().unwrap().version,
                if state.updates.checking {
                    language.text("（检查中，暂不可安装）", " (unavailable while checking)")
                } else {
                    ""
                }
            ),
            key => format!(
                "{} · {}",
                settings::title(key, state.language),
                settings::display(state.poll.as_ref().unwrap(), key, state.language)
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
