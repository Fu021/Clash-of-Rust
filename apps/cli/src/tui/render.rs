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

fn rate(value: u64) -> String {
    let (number, unit) = if value >= 1024 * 1024 {
        (value as f64 / 1048576.0, "MiB")
    } else {
        (value as f64 / 1024.0, "KiB")
    };
    format!("{number:.1} {unit}")
}

pub(super) fn frame(state: &State, columns: usize, rows: usize) -> Vec<Line> {
    let width = columns.clamp(1, 161) - 1;
    let height = rows.min(60);
    let mut lines = Vec::with_capacity(height);
    let line = |text: String, tone| Line {
        text: fit(&text, width),
        tone,
    };
    lines.push(line(
        format!(
            "  CLASH OF RUST   /   TERMINAL   {}",
            if state.demo {
                "演示 · 不连接后台"
            } else {
                "原型"
            }
        ),
        Tone::Accent,
    ));
    if columns < 60 || rows < 18 {
        lines.push(line(
            "  终端过小，请调整至至少 60 列 × 18 行；q 退出".into(),
            Tone::Warning,
        ));
        lines.truncate(height);
        return lines;
    }
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
        format!("  {}", "─".repeat(width.saturating_sub(4))),
        Tone::Muted,
    ));
    let body_height = height.saturating_sub(10);
    let body = match state.page {
        Page::Home => home(state),
        Page::Nodes => nodes(state, width.saturating_sub(6), body_height),
        Page::Connections => connections(state, width.saturating_sub(6), body_height),
        Page::Network => network(state),
    };
    for (text, tone) in body.into_iter().take(body_height) {
        lines.push(Line {
            text: fit(&format!("  {text}"), width),
            tone,
        });
    }
    while lines.len() < height - 6 {
        lines.push(Line {
            text: " ".repeat(width),
            tone: Tone::Plain,
        });
    }
    let (notice, tone) = if let Some(error) = &state.error {
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
                "正在检查网址…"
            } else {
                "正在提交节点切换…"
            }
            .into(),
            Tone::Accent,
        )
    } else if let Some(message) = &state.message {
        (message.clone(), Tone::Warning)
    } else {
        (
            "退出界面保留后台和代理；按 1/2/3/4 或 Tab 切换页面".into(),
            Tone::Muted,
        )
    };
    for (text, tone) in [
        (
            format!("  {}", "─".repeat(width.saturating_sub(4))),
            Tone::Muted,
        ),
        (format!("  {notice}"), tone),
        (
            if state.page == Page::Network {
                "  / 输入网址   Enter 检查   1–4 切页   q 退出".into()
            } else {
                "  ↑↓ 选择   Enter 打开/选择   / 搜索   Esc 返回   r 刷新   q 退出".into()
            },
            Tone::Muted,
        ),
        (
            if state.error.is_some() {
                "  诊断：cor diagnose --output cor-diagnose.json".into()
            } else {
                format!(
                    "  v{}   ·   本机后台   ·   每秒采样，变化时绘制",
                    clash_client_core::VERSION
                )
            },
            Tone::Muted,
        ),
    ] {
        lines.push(Line {
            text: fit(&text, width),
            tone,
        });
    }
    while lines.len() < height {
        lines.push(Line {
            text: " ".repeat(width),
            tone: Tone::Plain,
        });
    }
    lines
}

fn network(state: &State) -> Vec<(String, Tone)> {
    let mut lines = vec![
        ("网址检查".into(), Tone::Accent),
        (
            format!(
                "网址{}：{}",
                if state.searching {
                    "（输入中）"
                } else {
                    ""
                },
                state.query
            ),
            Tone::Plain,
        ),
        (
            "检查 DNS、HTTP / HTTPS 与实际线路；↑↓滚动结果。".into(),
            Tone::Muted,
        ),
        (String::new(), Tone::Plain),
    ];
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
                "界面不会自动启动内核。退出后使用 cor start，再重新打开。".into(),
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
    let available = height.saturating_sub(4).max(1);
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
            format!(
                "{} {}  {:>8}  {}",
                if selected { "›" } else { " " },
                fit(name, name_width),
                info,
                if current { "使用中" } else { "" }
            ),
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
    let available = height.saturating_sub(6).max(1);
    let offset = state.selected.saturating_sub(available - 1);
    for (index, id) in items.iter().enumerate().skip(offset).take(available) {
        let connection = poll
            .snapshot
            .connections
            .connections
            .iter()
            .find(|connection| connection.id.as_ref() == *id)
            .unwrap();
        let name = if connection.metadata.host.is_empty() {
            connection.metadata.destination_ip.as_ref()
        } else {
            connection.metadata.host.as_ref()
        };
        rows.push((
            format!(
                "{} {}  ↓ {}  ↑ {}",
                if index == state.selected { "›" } else { " " },
                fit(name, width.saturating_sub(32)),
                rate(connection.download),
                rate(connection.upload)
            ),
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
