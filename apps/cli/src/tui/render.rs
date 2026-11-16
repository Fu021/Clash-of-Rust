use super::model::{Page, State};
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tone {
    Plain,
    Muted,
    Accent,
    Selected,
    Warning,
}

#[derive(PartialEq, Eq)]
pub(super) struct Line {
    pub text: String,
    pub tone: Tone,
}

pub(super) fn fit(text: &str, width: usize) -> String {
    let mut text: String = text
        .chars()
        .take(2048)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if text.width() > width {
        while text.width() > width.saturating_sub(1) {
            text.pop();
        }
        if width > 0 {
            text.push('…');
        }
    }
    text.push_str(&" ".repeat(width.saturating_sub(text.width())));
    text
}

pub(super) fn rate(value: u64) -> String {
    let (number, unit) = if value >= 1024 * 1024 {
        (value as f64 / 1048576.0, "MiB")
    } else {
        (value as f64 / 1024.0, "KiB")
    };
    format!("{number:.1} {unit}")
}

pub(super) fn content_width(columns: usize, rows: usize) -> usize {
    columns
        .min(161)
        .saturating_sub(if columns >= 80 && rows >= 24 { 3 } else { 1 })
}

pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut characters = text.chars();
    for character in characters.by_ref().take(2048) {
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        line.push(character);
        if line.width() > width {
            line.pop();
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            line.push(character);
            if line.width() > width {
                line = "…".into();
            }
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    if characters.next().is_some() {
        lines.push("…".into());
    }
    lines
}

pub(super) fn frame(state: &State, columns: usize, rows: usize) -> Vec<Line> {
    if columns == 0 || rows == 0 {
        return Vec::new();
    }
    let width = columns.clamp(1, 161) - 1;
    let height = rows.min(60);
    let mut lines = Vec::with_capacity(height);
    let line = |text: String, tone| Line {
        text: fit(&text, width),
        tone,
    };
    let spacious = columns >= 80 && rows >= 24;
    let page = match state.page {
        Page::Home => "首页",
        Page::Nodes => "节点",
        Page::Connections => "连接",
        Page::Network => "网络",
        Page::Settings => "设置",
        Page::Tools => "操作",
        Page::Profiles => "订阅",
        Page::Rules => "快捷规则",
    };
    let (notice, tone) = notice(state);
    if height <= 2 {
        let (text, tone) = focus_line(state, width);
        lines.push(line(text, tone));
        if height == 2 {
            lines.push(line("q 退出 · Esc 返回 · 1–8 切页".into(), Tone::Muted));
        }
        return lines;
    }
    if spacious {
        lines.push(line(
            format!(
                "  CLASH OF RUST   /   TERMINAL   {}",
                if state.demo {
                    "演示 · 不连接后台"
                } else {
                    "本机控制台"
                }
            ),
            Tone::Accent,
        ));
        lines.push(line(String::new(), Tone::Plain));
        lines.push(line(
            format!(
                "  {}     {}     {}     {}",
                if state.page == Page::Home {
                    "[1 首页]"
                } else {
                    " 1 首页 "
                },
                if state.page == Page::Nodes {
                    "[2 节点]"
                } else {
                    " 2 节点 "
                },
                if state.page == Page::Connections {
                    "[3 连接]"
                } else {
                    " 3 连接 "
                },
                if state.page == Page::Network {
                    "[4 网络]"
                } else {
                    " 4 网络 "
                }
            ),
            Tone::Accent,
        ));
        lines.push(line(
            format!(
                "  {}     {}     {}     {}",
                if state.page == Page::Settings {
                    "[5 设置]"
                } else {
                    " 5 设置 "
                },
                if state.page == Page::Tools {
                    "[6 操作]"
                } else {
                    " 6 操作 "
                },
                if state.page == Page::Profiles {
                    "[7 订阅]"
                } else {
                    " 7 订阅 "
                },
                if state.page == Page::Rules {
                    "[8 快捷规则]"
                } else {
                    " 8 快捷规则 "
                },
            ),
            Tone::Accent,
        ));
        lines.push(line(
            format!("  {}", "─".repeat(width.saturating_sub(4))),
            Tone::Muted,
        ));
    } else if height >= 3 {
        lines.push(line(
            format!("{page} · 1–8 切页 · Tab 下一页"),
            Tone::Accent,
        ));
    }
    let footer_height = if spacious {
        4
    } else if height >= 10 {
        2
    } else {
        usize::from(height >= 2)
    };
    let body_height = height.saturating_sub(lines.len() + footer_height);
    let body = if state.form.is_some() {
        super::panels::editor(state, body_height)
    } else if state.searching && body_height <= 2 {
        vec![(format!("输入中：{} ▏", state.query), Tone::Selected)]
    } else if state.connection.is_some() {
        super::panels::connection(state, content_width(columns, rows), body_height)
    } else {
        match state.page {
            Page::Home => home(state),
            Page::Nodes => nodes(state, width.saturating_sub(6), body_height),
            Page::Connections => connections(state, width.saturating_sub(6), body_height),
            Page::Network => network(state, body_height),
            Page::Settings => super::panels::settings(state, width.saturating_sub(6), body_height),
            Page::Tools => super::panels::tools(state, width.saturating_sub(6), body_height),
            Page::Profiles => super::catalog::profiles(state, width.saturating_sub(6), body_height),
            Page::Rules => super::catalog::rules(state, width.saturating_sub(6), body_height),
        }
    };
    for (text, tone) in body
        .into_iter()
        .filter(|(text, _)| spacious || !text.is_empty())
        .take(body_height)
    {
        lines.push(Line {
            text: fit(
                &format!("{}{text}", if spacious { "  " } else { "" }),
                width,
            ),
            tone,
        });
    }
    while lines.len() < height - footer_height {
        lines.push(Line {
            text: " ".repeat(width),
            tone: Tone::Plain,
        });
    }
    if spacious {
        lines.push(line("─".repeat(width), Tone::Muted));
    }
    if footer_height >= 2 {
        lines.push(line(notice, tone));
    } else if footer_height == 1
        && (state.execution.is_some()
            || state.confirmation.is_some()
            || state.error.is_some()
            || state.message.is_some()
            || state.saving)
    {
        lines.push(line(notice, tone));
        return lines;
    }
    if footer_height > 0 {
        let help = if state.form.is_some() {
            "↑↓ 选择 · Enter 编辑/确认 · Esc 返回 · q 退出"
        } else if state.connection.is_some() {
            "↑↓ 详情 · d 解析 · x 关闭 · Esc 返回 · q 退出"
        } else if state.page == Page::Network {
            "[ ] 工具 · / 输入 · Enter 检查 · Esc 取消 · q 退出"
        } else if state.page == Page::Nodes {
            "↑↓ 选择 · Enter 打开 · s 排序 · / 搜索 · Esc 返回 · q 退出"
        } else {
            "↑↓ 选择 · Enter 打开 · / 搜索 · Esc 返回 · q 退出"
        };
        lines.push(line(
            if spacious {
                help.into()
            } else {
                format!("q 退出 · Esc 返回 · {help}")
            },
            Tone::Muted,
        ));
    }
    if spacious {
        lines.push(line(
            if state.error.is_some() {
                "诊断：cor diagnose --output cor-diagnose.json".into()
            } else {
                format!(
                    "v{} · 本机后台 · 每秒采样，变化时绘制",
                    clash_client_core::VERSION
                )
            },
            Tone::Muted,
        ));
    }
    lines
}

fn focus_line(state: &State, width: usize) -> (String, Tone) {
    if state.execution.is_some()
        || state.confirmation.is_some()
        || state.error.is_some()
        || state.message.is_some()
        || state.saving
    {
        return notice(state);
    }
    let content = if state.form.is_some() {
        super::panels::editor(state, 1)
    } else if state.searching {
        vec![(format!("输入中：{} ▏", state.query), Tone::Selected)]
    } else if state.connection.is_some() {
        super::panels::connection(state, width, 1)
    } else {
        match state.page {
            Page::Home => home(state),
            Page::Nodes => nodes(state, width, 1),
            Page::Connections => connections(state, width, 1),
            Page::Settings => super::panels::settings(state, width, 1),
            Page::Tools => super::panels::tools(state, width, 1),
            Page::Profiles => super::catalog::profiles(state, width, 1),
            Page::Rules => super::catalog::rules(state, width, 1),
            Page::Network => vec![(
                state
                    .network_results
                    .get(state.selected)
                    .cloned()
                    .unwrap_or_else(|| {
                        format!(
                            "{} · {}",
                            clash_client_core::network_tools::Tool::ALL[state.network_tool].title(),
                            state.query
                        )
                    }),
                Tone::Plain,
            )],
        }
    };
    content
        .into_iter()
        .next()
        .unwrap_or_else(|| ("没有匹配项".into(), Tone::Muted))
}

fn notice(state: &State) -> (String, Tone) {
    if let Some(operation) = &state.execution {
        if let Some(form) = &state.form
            && let Some(key) = &form.setting
        {
            let field = form
                .fields
                .iter()
                .position(|field| !field.locked)
                .unwrap_or(0);
            return (
                format!(
                    "保存{}为「{}」？Enter 确认 · Esc 取消{}",
                    super::settings::title(key),
                    form.visible_value(field),
                    if super::settings::inline(operation) {
                        ""
                    } else {
                        "；普通终端完成授权"
                    }
                ),
                Tone::Warning,
            );
        }
        (
            format!(
                "执行 cor {}？Enter 确认 · Esc 取消；授权和结果将在普通终端显示",
                operation.path
            ),
            Tone::Warning,
        )
    } else if let Some(message) = state.message.as_ref().filter(|_| state.form.is_some()) {
        (message.clone(), Tone::Warning)
    } else if let Some(error) = &state.error {
        (
            format!("读取失败（旧数据仅供参考）：{error}"),
            Tone::Warning,
        )
    } else if let Some(target) = &state.confirmation {
        (
            format!(
                "切换 {} → {}？ Enter 确认 · Esc 取消",
                target.group, target.node
            ),
            Tone::Warning,
        )
    } else if state.saving {
        (
            if state.page == Page::Network {
                "正在检查… Esc 取消"
            } else {
                "正在保存操作…"
            }
            .into(),
            Tone::Accent,
        )
    } else if let Some(message) = &state.message {
        (message.clone(), Tone::Warning)
    } else {
        (
            "退出界面保留后台和代理；按 1–8 或 Tab 切换页面".into(),
            Tone::Muted,
        )
    }
}

fn network(state: &State, height: usize) -> Vec<(String, Tone)> {
    let tool = clash_client_core::network_tools::Tool::ALL[state.network_tool];
    let mut lines = vec![(
        format!(
            "{}  {}/{}",
            tool.title(),
            state.network_tool + 1,
            clash_client_core::network_tools::Tool::ALL.len()
        ),
        Tone::Accent,
    )];
    if tool.needs_target() {
        lines.push((
            format!(
                "目标{}：{}",
                if state.searching {
                    "（输入中）"
                } else {
                    ""
                },
                state.query
            ),
            Tone::Plain,
        ));
    }
    lines.extend([
        (tool.hint().into(), Tone::Muted),
        (String::new(), Tone::Plain),
    ]);
    if !state.network_results.is_empty() {
        lines.truncate(height.saturating_sub(1));
    }
    lines.extend(
        state
            .network_results
            .iter()
            .skip(state.selected)
            .cloned()
            .map(|line| (line, Tone::Plain)),
    );
    lines
}

fn home(state: &State) -> Vec<(String, Tone)> {
    let Some(poll) = &state.poll else {
        return vec![
            ("尚未连接后台".into(), Tone::Warning),
            (
                "按 6 进入「操作」，选择 start 启动；浏览界面不会自动启动内核。".into(),
                Tone::Muted,
            ),
        ];
    };
    let snapshot = &poll.snapshot;
    let settings = &poll.meta.settings;
    let owner = match poll.meta.owner {
        clash_client_runtime::control::Owner::Gui => "桌面后台",
        clash_client_runtime::control::Owner::Daemon => "CLI 后台",
    };
    let proxy = match snapshot.proxy_mode() {
        clash_client_runtime::engine::ProxyMode::Tun => "TUN",
        clash_client_runtime::engine::ProxyMode::System => "系统代理",
        clash_client_runtime::engine::ProxyMode::Off => "关闭",
    };
    let mode = match snapshot.mode.as_str() {
        "rule" => "规则",
        "global" => "全局",
        "direct" => "直连",
        _ => "待确认",
    };
    let profile = poll
        .meta
        .profiles
        .iter()
        .find(|profile| Some(&profile.id) == settings.active_profile.as_ref())
        .map(|profile| profile.name.as_str())
        .unwrap_or(if state.demo { "演示订阅" } else { "—" });
    let route = if snapshot.running {
        snapshot
            .active_route
            .as_ref()
            .map(|route| route.join(" → "))
            .or_else(|| snapshot.active_node.clone())
            .unwrap_or_else(|| "待确认".into())
    } else {
        "内核未运行".into()
    };
    vec![
        (
            format!(
                "{}   ·   {owner}   ·   {}",
                if snapshot.running {
                    "● 内核运行中"
                } else {
                    "○ 内核未运行"
                },
                poll.meta.version
            ),
            Tone::Accent,
        ),
        (String::new(), Tone::Plain),
        (format!("代理方式      {proxy} · {mode}模式"), Tone::Plain),
        (format!("当前订阅      {profile}"), Tone::Plain),
        (
            format!(
                "{}  {route}",
                if settings.claude_protection.is_some() {
                    "保护节点    "
                } else {
                    "参考线路    "
                }
            ),
            Tone::Accent,
        ),
        (
            format!(
                "Claude 保护   {}",
                if settings.claude_protection.is_some() {
                    "已开启 · 网络修改锁定"
                } else {
                    "已关闭"
                }
            ),
            Tone::Plain,
        ),
        (String::new(), Tone::Plain),
        (
            if snapshot.running {
                format!(
                    "↓ 下载  {}/s     ↑ 上传  {}/s     连接  {}",
                    rate(snapshot.download_rate),
                    rate(snapshot.upload_rate),
                    snapshot.connection_count
                )
            } else {
                "流量          内核未运行，暂无采样".into()
            },
            Tone::Plain,
        ),
        (String::new(), Tone::Plain),
        (
            "规则模式可有多个出口；连接页展示逐连接的实际规则与线路。".into(),
            Tone::Muted,
        ),
    ]
}

fn nodes(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let items = state.items();
    let Some(poll) = &state.poll else {
        return home(state);
    };
    let proxies = &poll.snapshot.proxies.proxies;
    let mut rows = vec![
        (
            format!(
                "{}   ·   {} 项",
                state.group.as_deref().unwrap_or("策略组"),
                items.len()
            ),
            Tone::Plain,
        ),
        (
            format!("[s 切换排序] 排序：{}", poll.meta.settings.tui_node_sort),
            Tone::Muted,
        ),
        (
            format!(
                "搜索{}：{}",
                if state.searching {
                    "（输入中）"
                } else {
                    " /"
                },
                state.query
            ),
            Tone::Muted,
        ),
        (String::new(), Tone::Plain),
    ];
    if state
        .group
        .as_deref()
        .and_then(|group| proxies.get(group))
        .is_some_and(|group| group.kind.as_str() != "Selector")
    {
        rows[3] = (
            "自动组 · 内核自行选择最优节点；在上一级手动组选择 AUTO 或具体节点".into(),
            Tone::Muted,
        );
    }
    rows.truncate(height.saturating_sub(1));
    let available = height.saturating_sub(rows.len()).max(1);
    let offset = state.selected.saturating_sub(available - 1);
    let name_width = width.saturating_sub(24);
    for (index, name) in items.iter().enumerate().skip(offset).take(available) {
        let proxy = proxies.get(*name);
        let info = proxy
            .map(|proxy| {
                if proxy.all.is_empty() {
                    proxy
                        .delay
                        .map(|delay| {
                            if delay == 0 {
                                "超时".into()
                            } else {
                                format!("{delay} ms")
                            }
                        })
                        .unwrap_or_else(|| "未测速".into())
                } else {
                    format!("{} 项", proxy.all.len())
                }
            })
            .unwrap_or_else(|| "未测速".into());
        let current = state
            .group
            .as_deref()
            .and_then(|group| proxies.get(group))
            .is_some_and(|group| group.now.as_str() == *name);
        let selected = index == state.selected;
        rows.push((
            if width < 50 {
                format!(
                    "{} {name} · {info}{}",
                    if selected { "›" } else { " " },
                    if current { " · 使用中" } else { "" }
                )
            } else {
                format!(
                    "{} {}  {}  {}",
                    if selected { "›" } else { " " },
                    fit(name, name_width),
                    fit(&info, 8),
                    if current { "使用中" } else { "" }
                )
            },
            if selected {
                Tone::Selected
            } else if current {
                Tone::Accent
            } else {
                Tone::Plain
            },
        ));
    }
    if items.is_empty() {
        rows.push(("没有匹配项".into(), Tone::Muted));
    }
    rows
}

fn connections(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let Some(poll) = &state.poll else {
        return home(state);
    };
    let items = state.items();
    let mut rows = vec![
        (
            format!(
                "活跃连接 {}   ·   匹配 {} 项",
                poll.snapshot.connection_count,
                items.len()
            ),
            Tone::Plain,
        ),
        (
            format!(
                "搜索{}：{}",
                if state.searching {
                    "（输入中）"
                } else {
                    " /"
                },
                state.query
            ),
            Tone::Muted,
        ),
    ];
    rows.truncate(height.saturating_sub(1));
    let available = height.saturating_sub(rows.len()).max(1);
    let offset = state.selected.saturating_sub(available - 1);
    for (index, id) in items.iter().enumerate().skip(offset).take(available) {
        let connection = poll
            .snapshot
            .connections
            .connections
            .iter()
            .find(|connection| connection.id.as_ref() == *id)
            .unwrap();
        let name = clash_client_core::connection::name(connection);
        let name = if name.is_empty() { "未提供" } else { name };
        rows.push((
            if width < 50 {
                format!("{} {name}", if index == state.selected { "›" } else { " " })
            } else {
                format!(
                    "{} {}  ↓ {}  ↑ {}",
                    if index == state.selected { "›" } else { " " },
                    fit(name, width.saturating_sub(32)),
                    rate(connection.download),
                    rate(connection.upload)
                )
            },
            if index == state.selected {
                Tone::Selected
            } else {
                Tone::Plain
            },
        ));
    }
    if let Some(id) = items.get(state.selected)
        && let Some(connection) = poll
            .snapshot
            .connections
            .connections
            .iter()
            .find(|connection| connection.id.as_ref() == *id)
    {
        rows.push((String::new(), Tone::Plain));
        rows.push((
            format!(
                "规则  {}  {}",
                connection.rule.as_str(),
                connection.rule_payload.as_str()
            ),
            Tone::Muted,
        ));
        rows.push((
            format!(
                "线路  {}",
                connection
                    .chains
                    .iter()
                    .rev()
                    .map(|name| name.as_str())
                    .collect::<Vec<_>>()
                    .join(" → ")
            ),
            Tone::Accent,
        ));
    }
    rows
}
