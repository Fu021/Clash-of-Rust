use super::model::{Page, State};
use clash_client_core::config::Language;
use unicode_width::UnicodeWidthStr;

mod footer;

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

pub(super) fn list(
    header: Vec<(String, Tone)>,
    items: impl Iterator<Item = (String, Tone)>,
    selected: usize,
    width: usize,
    height: usize,
) -> Vec<(String, Tone)> {
    if height == 0 {
        return Vec::new();
    }
    let mut lines: Vec<_> = header
        .into_iter()
        .flat_map(|(text, tone)| wrap(&text, width).into_iter().map(move |text| (text, tone)))
        .take(height - 1)
        .collect();
    let available = height - lines.len();
    let mut rows = Vec::new();
    let mut selected_range = (0, 0);
    for (index, (text, tone)) in items
        .enumerate()
        .skip(selected.saturating_sub(available - 1))
        .take(available)
    {
        let start = rows.len();
        rows.extend(wrap(&text, width).into_iter().map(|text| (text, tone)));
        if index == selected {
            selected_range = (start, rows.len());
        }
    }
    let offset = if selected_range.1 - selected_range.0 >= available {
        selected_range.0
    } else {
        selected_range.1.saturating_sub(available)
    };
    lines.extend(rows.into_iter().skip(offset).take(available));
    lines
}

fn navigation(state: &State) -> [String; 2] {
    let items = Page::ALL.map(|target| {
        let number = Page::ALL.iter().position(|page| *page == target).unwrap() + 1;
        (target, format!("{number} {}", target.title(state.language)))
    });
    let mut rows = [String::new(), String::new()];
    for (row, items) in rows.iter_mut().zip(items.chunks(4)) {
        row.push_str("  ");
        for (index, (target, label)) in items.iter().enumerate() {
            if index > 0 {
                row.push_str("     ");
            }
            row.push(if state.page == *target { '[' } else { ' ' });
            row.push_str(label);
            row.push(if state.page == *target { ']' } else { ' ' });
        }
    }
    rows
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
        Page::Connections if state.connection.is_some() => {
            state.language.text("连接详情", "Connection details")
        }
        Page::Settings if state.fake_ip.is_some() => {
            state.language.text("Fake-IP 排除项", "Fake-IP exclusions")
        }
        Page::Rules if state.shortcuts_open => state.language.text("快捷规则", "Rule shortcuts"),
        page => page.title(state.language),
    };
    if height <= 2 {
        let (text, tone) = focus_line(state, width);
        lines.push(line(text, tone));
        if height == 2 {
            lines.push(line(footer::compact_help(state).into(), Tone::Muted));
        }
        return lines;
    }
    let navigation = navigation(state);
    let expanded_navigation = rows >= 24 && navigation.iter().all(|row| row.width() <= width);
    if expanded_navigation {
        let status = if state.demo {
            state.language.text("演示 · 不连接后台", "Demo · Offline")
        } else {
            state.language.text("本机控制台", "Local console")
        };
        let title = format!("  CLASH OF RUST   /   TERMINAL   {status}");
        lines.push(line(
            if title.width() > width {
                format!("  CLASH OF RUST · {status}")
            } else {
                title
            },
            Tone::Accent,
        ));
        lines.push(line(String::new(), Tone::Plain));
        lines.extend(navigation.into_iter().map(|text| line(text, Tone::Accent)));
        lines.push(line(
            format!("  {}", "─".repeat(width.saturating_sub(4))),
            Tone::Muted,
        ));
    } else if height >= 3 {
        let title = format!("[{page}]");
        let padding = width.saturating_sub(title.width()) / 2;
        lines.push(line(
            format!("{}{title}", " ".repeat(padding)),
            Tone::Accent,
        ));
        if height >= 10 {
            lines.push(line(
                state
                    .language
                    .text("1–8 切页 · Tab 下一页", "1–8 Switch page · Tab Next")
                    .into(),
                Tone::Muted,
            ));
        }
    }
    let footer = footer::lines(state, width, height, lines.len(), spacious);
    let footer_height = footer.len();
    let body_height = height.saturating_sub(lines.len() + footer_height);
    let body = if state.fake_ip.is_some() {
        super::fake_ip::panel(state, content_width(columns, rows), body_height)
    } else if state.form.is_some() {
        super::panels::editor(state, content_width(columns, rows), body_height)
    } else if state.searching && body_height <= 2 {
        vec![(
            format!(
                "{}{} ▏",
                state.language.text("输入中：", "Typing: "),
                state.query
            ),
            Tone::Selected,
        )]
    } else if state.connection.is_some() {
        super::panels::connection(state, content_width(columns, rows), body_height)
    } else {
        match state.page {
            Page::Home => super::home::panel(state, content_width(columns, rows), body_height),
            Page::Nodes => nodes(state, content_width(columns, rows), body_height, spacious),
            Page::Connections => connections(state, content_width(columns, rows), body_height),
            Page::Network => network(state, content_width(columns, rows), body_height),
            Page::Settings => {
                super::panels::settings(state, content_width(columns, rows), body_height)
            }
            Page::Tools => super::panels::tools(state, content_width(columns, rows), body_height),
            Page::Profiles => {
                super::catalog::profiles(state, content_width(columns, rows), body_height)
            }
            Page::Rules if state.shortcuts_open => {
                super::catalog::rules(state, content_width(columns, rows), body_height)
            }
            Page::Rules => super::rules::panel(state, content_width(columns, rows), body_height),
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
    lines.extend(footer);
    lines
}

fn focus_line(state: &State, width: usize) -> (String, Tone) {
    if footer::has_notice(state) {
        return notice(state);
    }
    let content = if state.fake_ip.is_some() {
        super::fake_ip::panel(state, width, 1)
    } else if state.form.is_some() {
        super::panels::editor(state, width, 1)
    } else if state.searching {
        vec![(
            format!(
                "{}{} ▏",
                state.language.text("输入中：", "Typing: "),
                state.query
            ),
            Tone::Selected,
        )]
    } else if state.connection.is_some() {
        super::panels::connection(state, width, 1)
    } else {
        match state.page {
            Page::Home => super::home::panel(state, width, 1),
            Page::Nodes => nodes(state, width, 1, false),
            Page::Connections => connections(state, width, 1),
            Page::Settings => super::panels::settings(state, width, 1),
            Page::Tools => super::panels::tools(state, width, 1),
            Page::Profiles => super::catalog::profiles(state, width, 1),
            Page::Rules if state.shortcuts_open => super::catalog::rules(state, width, 1),
            Page::Rules => super::rules::panel(state, width, 1),
            Page::Network => vec![(
                super::network::rows(state, width)
                    .get(state.selected)
                    .map(|row| row.text.clone())
                    .unwrap_or_else(|| {
                        format!(
                            "{} · {}",
                            super::network::title(
                                clash_client_core::network_tools::Tool::ALL[state.network_tool],
                                state.language
                            ),
                            state.query
                        )
                    }),
                Tone::Plain,
            )],
        }
    };
    content.into_iter().next().unwrap_or_else(|| {
        (
            state.language.text("没有匹配项", "No matches").into(),
            Tone::Muted,
        )
    })
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
            let title = super::settings::title(key, state.language);
            let value = form.visible_value(field);
            let authorization = if super::settings::inline(operation) {
                ""
            } else {
                state
                    .language
                    .text("；普通终端完成授权", " · Authorize in the terminal")
            };
            let text = match state.language {
                Language::SimplifiedChinese => {
                    format!("保存{title}为「{value}」？Enter 确认 · Esc 取消{authorization}")
                }
                Language::English => {
                    format!("Save {title} as {value}? Enter confirm · Esc cancel{authorization}")
                }
            };
            return (text, Tone::Warning);
        }
        let text = match state.language {
            Language::SimplifiedChinese => format!(
                "执行 cor {}？Enter 确认 · Esc 取消；授权和结果将在普通终端显示",
                operation.path
            ),
            Language::English => format!(
                "Run cor {}? Enter confirm · Esc cancel; authorization and results appear in the terminal",
                operation.path
            ),
        };
        (text, Tone::Warning)
    } else if let Some(message) = state.message.as_ref().filter(|_| state.form.is_some()) {
        (message.clone(), Tone::Warning)
    } else if let Some(error) = &state.error {
        let prefix = state.language.text(
            "读取失败（旧数据仅供参考）：",
            "Read failed (previous data shown for reference): ",
        );
        (format!("{prefix}{error}"), Tone::Warning)
    } else if let Some((_, description)) = state
        .rules
        .as_ref()
        .and_then(|rules| rules.pending.as_ref())
    {
        (
            format!(
                "{description}{}",
                state
                    .language
                    .text("？Enter 确认 · Esc 取消", "? Enter confirm · Esc cancel")
            ),
            Tone::Warning,
        )
    } else if let Some(available) = &state.updates.pending {
        let text = match state.language {
            Language::SimplifiedChinese => format!(
                "{}安装{} {}？继续后在普通终端确认与授权。",
                if available.preview {
                    clash_client_core::update::channel::PRERELEASE_WARNING
                } else {
                    ""
                },
                if available.preview {
                    "预发布版"
                } else {
                    "正式版"
                },
                available.version,
            ),
            Language::English => format!(
                "{}Install {} {}? Confirm and authorize in the terminal.",
                if available.preview {
                    "Pre-releases may cause crashes, network problems or configuration incompatibilities. Back up your configuration before installing. "
                } else {
                    ""
                },
                if available.preview {
                    "pre-release"
                } else {
                    "release"
                },
                available.version,
            ),
        };
        (text, Tone::Warning)
    } else if let Some(target) = &state.confirmation {
        let text = match state.language {
            Language::SimplifiedChinese => format!(
                "切换 {} → {}？ Enter 确认 · Esc 取消",
                target.group, target.node
            ),
            Language::English => format!(
                "Switch {} → {}? Enter confirm · Esc cancel",
                target.group, target.node
            ),
        };
        (text, Tone::Warning)
    } else if state.saving {
        (
            if state.form.as_ref().and_then(|form| form.network)
                == Some(super::network::Editor::Export)
            {
                state
                    .language
                    .text("正在导出本次结果…", "Exporting results…")
            } else if state.page == Page::Network {
                state
                    .language
                    .text("正在检查… Esc 取消", "Checking… Esc cancel")
            } else {
                state.language.text("正在保存操作…", "Saving…")
            }
            .into(),
            Tone::Accent,
        )
    } else if let Some(message) = &state.message {
        (message.clone(), Tone::Warning)
    } else if let Some(message) = state
        .fake_ip
        .as_ref()
        .and_then(|editor| editor.review_message(state.language))
    {
        (message, Tone::Warning)
    } else {
        (
            state
                .language
                .text(
                    "退出界面保留后台和代理；按 1–8 或 Tab 切换页面",
                    "Quitting keeps the background and proxy running; use 1–8 or Tab to switch pages",
                )
                .into(),
            Tone::Muted,
        )
    }
}

fn network(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let tool = clash_client_core::network_tools::Tool::ALL[state.network_tool];
    let mut lines = vec![(
        format!(
            "{}  {}/{}",
            super::network::title(tool, state.language),
            state.network_tool + 1,
            clash_client_core::network_tools::Tool::ALL.len()
        ),
        Tone::Accent,
    )];
    if tool.needs_target() {
        lines.push((
            format!(
                "{}{}{}{}",
                state.language.text("目标", "Target"),
                if state.searching {
                    state.language.text("（输入中）", " (editing)")
                } else {
                    ""
                },
                state.language.text("：", ": "),
                state.query
            ),
            Tone::Plain,
        ));
    }
    lines.extend([
        (
            format!(
                "{}{}{}",
                state.language.text("参数", "Parameters"),
                state.language.text("：", ": "),
                super::network::summary(tool, &state.network_options, state.language)
            ),
            Tone::Muted,
        ),
        (
            super::network::hint(tool, state.language).into(),
            Tone::Muted,
        ),
        (String::new(), Tone::Plain),
    ]);
    let mut lines = lines
        .into_iter()
        .flat_map(|(text, tone)| wrap(&text, width).into_iter().map(move |line| (line, tone)))
        .collect::<Vec<_>>();
    if !state.network_results.is_empty() {
        lines.truncate(height / 2);
    }
    lines.extend(
        super::network::rows(state, width)
            .into_iter()
            .skip(state.selected)
            .map(|row| (row.text, Tone::Plain)),
    );
    lines
}

fn disconnected(width: usize, height: usize, language: Language) -> Vec<(String, Tone)> {
    wrap(
        language.text(
            "尚未连接后台；按 1 回首页启动后台与内核。",
            "Backend disconnected; press 1 to start the background and core from Home.",
        ),
        width,
    )
    .into_iter()
    .take(height)
    .map(|text| (text, Tone::Warning))
    .collect()
}

fn nodes(state: &State, width: usize, height: usize, spacious: bool) -> Vec<(String, Tone)> {
    let items = state.items();
    let Some(poll) = &state.poll else {
        return disconnected(width, height, state.language);
    };
    let proxies = &poll.snapshot.proxies.proxies;
    let language = state.language;
    let group = state
        .group
        .as_deref()
        .unwrap_or(language.text("策略组", "Group"));
    let sort = match poll.meta.settings.tui_node_sort {
        clash_client_core::proxy_order::NodeSort::LatencyAscending => {
            language.text("延迟从低到高", "Lowest latency first")
        }
        clash_client_core::proxy_order::NodeSort::LatencyDescending => {
            language.text("延迟从高到低", "Highest latency first")
        }
        clash_client_core::proxy_order::NodeSort::Name => language.text("名称排序", "Name"),
    };
    let item_count = items.len();
    let mut rows = vec![
        (
            match language {
                Language::SimplifiedChinese => format!("{group}   ·   {item_count} 项"),
                Language::English => format!("{group}   ·   {item_count} items"),
            },
            Tone::Plain,
        ),
        (
            match language {
                Language::SimplifiedChinese => format!("[s 切换排序] 排序：{sort}"),
                Language::English => format!("[s change sort] Sort: {sort}"),
            },
            Tone::Muted,
        ),
        (
            format!(
                "{}{}{}{}",
                language.text("搜索", "Search"),
                if state.searching {
                    language.text("（输入中）", " (editing)")
                } else {
                    " /"
                },
                language.text("：", ": "),
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
            language.text(
                "自动组 · 内核自行选择最优节点；在上一级手动组选择 AUTO 或具体节点",
                "Automatic group · The core selects a node; choose AUTO or a node in the parent manual group",
            ).into(),
            Tone::Muted,
        );
    }
    if !spacious {
        rows.retain(|(text, _)| !text.is_empty());
    }
    if items.is_empty() {
        return list(
            rows,
            std::iter::once((
                language.text("没有匹配项", "No matches").into(),
                Tone::Muted,
            )),
            0,
            width,
            height,
        );
    }
    let name_width = width.saturating_sub(24);
    let entries = items.iter().enumerate().map(|(index, name)| {
        let proxy = proxies.get(*name);
        let info = proxy
            .map(|proxy| {
                if proxy.all.is_empty() {
                    proxy
                        .delay
                        .map(|delay| {
                            if delay == 0 {
                                language.text("超时", "Timed out").into()
                            } else {
                                format!("{delay} ms")
                            }
                        })
                        .unwrap_or_else(|| language.text("未测速", "Not tested").into())
                } else {
                    match language {
                        Language::SimplifiedChinese => format!("{} 项", proxy.all.len()),
                        Language::English => format!("{} members", proxy.all.len()),
                    }
                }
            })
            .unwrap_or_else(|| language.text("未测速", "Not tested").into());
        let current = state
            .group
            .as_deref()
            .and_then(|group| proxies.get(group))
            .is_some_and(|group| group.now.as_str() == *name);
        let selected = index == state.selected;
        (
            if width < 50 || name.width() > name_width || info.width() > 8 {
                format!(
                    "{} {name} · {info}{}",
                    if selected { "›" } else { " " },
                    if current {
                        language.text(" · 使用中", " · Current")
                    } else {
                        ""
                    }
                )
            } else {
                format!(
                    "{} {}  {}  {}",
                    if selected { "›" } else { " " },
                    fit(name, name_width),
                    fit(&info, 8),
                    if current {
                        language.text("使用中", "Current")
                    } else {
                        ""
                    }
                )
            },
            if selected {
                Tone::Selected
            } else if current {
                Tone::Accent
            } else {
                Tone::Plain
            },
        )
    });
    list(rows, entries, state.selected, width, height)
}

fn connections(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let Some(poll) = &state.poll else {
        return disconnected(width, height, state.language);
    };
    let items = state.items();
    let language = state.language;
    let header = vec![
        (
            match language {
                Language::SimplifiedChinese => format!(
                    "活跃连接 {}   ·   匹配 {} 项",
                    poll.snapshot.connection_count,
                    items.len()
                ),
                Language::English => format!(
                    "Active connections {}   ·   {} matches",
                    poll.snapshot.connection_count,
                    items.len()
                ),
            },
            Tone::Plain,
        ),
        (
            format!(
                "{}{}{}{}",
                language.text("搜索", "Search"),
                if state.searching {
                    language.text("（输入中）", " (editing)")
                } else {
                    " /"
                },
                language.text("：", ": "),
                state.query
            ),
            Tone::Muted,
        ),
    ];
    let entries = items.iter().enumerate().map(|(index, id)| {
        let connection = poll
            .snapshot
            .connections
            .connections
            .iter()
            .find(|connection| connection.id.as_ref() == *id)
            .unwrap();
        let name = clash_client_core::connection::name(connection);
        let name = if name.is_empty() {
            language.text("未提供", "Not provided")
        } else {
            name
        };
        (
            if width < 50 {
                format!("{} {name}", if index == state.selected { "›" } else { " " })
            } else {
                format!(
                    "{} {}  ↓ {}  ↑ {}",
                    if index == state.selected { "›" } else { " " },
                    name,
                    rate(connection.download),
                    rate(connection.upload)
                )
            },
            if index == state.selected {
                Tone::Selected
            } else {
                Tone::Plain
            },
        )
    });
    let mut rows = list(header, entries, state.selected, width, height);
    if let Some(id) = items.get(state.selected)
        && let Some(connection) = poll
            .snapshot
            .connections
            .connections
            .iter()
            .find(|connection| connection.id.as_ref() == *id)
    {
        let detail = [
            (String::new(), Tone::Plain),
            (
                format!(
                    "{}  {}  {}",
                    language.text("规则", "Rule"),
                    connection.rule.as_str(),
                    connection.rule_payload.as_str()
                ),
                Tone::Muted,
            ),
            (
                format!(
                    "{}  {}",
                    language.text("线路", "Route"),
                    connection
                        .chains
                        .iter()
                        .rev()
                        .map(|name| name.as_str())
                        .collect::<Vec<_>>()
                        .join(" → ")
                ),
                Tone::Accent,
            ),
        ];
        rows.extend(
            detail
                .into_iter()
                .flat_map(|(text, tone)| {
                    wrap(&text, width).into_iter().map(move |text| (text, tone))
                })
                .take(height.saturating_sub(rows.len())),
        );
    }
    rows
}
