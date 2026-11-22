use super::{form, model::State, render::Tone};

pub(super) fn settings(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let language = state.language;
    let mut lines = vec![
        (
            language
                .text(
                    "设置 · Enter 打开；直接选择开关、模式或编辑数值",
                    "Press Enter to edit a setting",
                )
                .into(),
            Tone::Accent,
        ),
        (
            format!("{}{}", language.text("搜索：", "Search: "), state.query),
            Tone::Muted,
        ),
    ];
    let items = state.items();
    lines = lines
        .into_iter()
        .flat_map(|(text, tone)| {
            super::render::wrap(&text, width)
                .into_iter()
                .map(move |text| (text, tone))
        })
        .take(height.saturating_sub(1))
        .collect();
    let available = height.saturating_sub(lines.len()).max(1);
    let mut rows = Vec::new();
    let mut selected_range = (0, 0);
    let mut category = "";
    for (index, key) in items.iter().enumerate() {
        let next_category = super::settings::category(key, language);
        if next_category != category && available >= 3 {
            rows.push((format!("── {next_category} ──"), Tone::Accent));
            category = next_category;
        }
        let selected = index == state.selected;
        let start = rows.len();
        let value = state.poll.as_ref().map_or_else(
            || language.text("后台未连接", "Backend disconnected").into(),
            |poll| super::settings::display(poll, key, language),
        );
        let text = format!(
            "{} {} {value}",
            if selected { "›" } else { " " },
            super::settings::title(key, language)
        );
        let tone = if selected {
            Tone::Selected
        } else {
            Tone::Plain
        };
        rows.extend(
            super::render::wrap(&text, width)
                .into_iter()
                .map(|text| (text, tone)),
        );
        if selected {
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

pub(super) fn tools(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let header = vec![
        (
            state
                .language
                .text(
                    "管理操作 · 与 CLI 共用命令、授权和保护校验",
                    "Commands · Same permissions and protection as the CLI",
                )
                .into(),
            Tone::Accent,
        ),
        (
            format!(
                "{}{}",
                state.language.text("搜索命令或说明：", "Search commands: "),
                state.query
            ),
            Tone::Muted,
        ),
    ];
    let items = state.items();
    let rows = items.iter().enumerate().map(|(index, path)| {
        let spec = form::commands().find(|spec| spec.path == *path).unwrap();
        (
            format!(
                "{} {}  · {}",
                if index == state.selected { "›" } else { " " },
                spec.localized_description(state.language),
                spec.path
            ),
            if index == state.selected {
                Tone::Selected
            } else {
                Tone::Plain
            },
        )
    });
    super::render::list(header, rows, state.selected, width, height)
}

pub(super) fn editor(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let form = state.form.as_ref().unwrap();
    let language = state.language;
    let mut lines = if form.network == Some(super::network::Editor::Export) {
        vec![
            (
                language
                    .text("工具 · 导出本次结果", "Tools · Export result")
                    .into(),
                Tone::Accent,
            ),
            (
                language
                    .text(
                        "保存现有证据，不重新运行检查；Esc 返回。",
                        "Save the current result; Esc returns.",
                    )
                    .into(),
                Tone::Muted,
            ),
        ]
    } else if let Some(super::network::Editor::Parameters(tool)) = form.network {
        vec![
            (
                format!(
                    "{} · {}",
                    super::network::title(tool, language),
                    language.text("参数", "Parameters")
                ),
                Tone::Accent,
            ),
            (
                language
                    .text(
                        "本页生效；切换工具恢复默认；Esc 丢弃草稿。",
                        "Applies here; switching tools resets defaults; Esc discards.",
                    )
                    .into(),
                Tone::Muted,
            ),
        ]
    } else if let Some(key) = &form.setting {
        vec![
            (
                format!(
                    "{} · {}",
                    language.text("设置", "Settings"),
                    super::settings::title(key, language),
                ),
                Tone::Accent,
            ),
            (super::settings::hint(key, language).into(), Tone::Muted),
            (
                language
                    .text(
                        "草稿不会自动保存；选择保存后确认。",
                        "Select Save and confirm to apply.",
                    )
                    .into(),
                Tone::Muted,
            ),
        ]
    } else {
        vec![
            (
                format!(
                    "cor {} · {}",
                    form.spec.path,
                    form.spec.localized_description(state.language)
                ),
                Tone::Accent,
            ),
            (form.spec.usage.into(), Tone::Muted),
            (
                state
                    .language
                    .text(
                        "每项单独填写，无需给空格或 emoji 加引号；数组使用 JSON。",
                        "Enter values as shown. Use JSON for arrays.",
                    )
                    .into(),
                Tone::Muted,
            ),
        ]
    };
    lines = lines
        .into_iter()
        .flat_map(|(text, tone)| {
            super::render::wrap(&text, width)
                .into_iter()
                .map(move |line| (line, tone))
        })
        .collect();
    lines.truncate(height / 3);
    let available = height.saturating_sub(lines.len()).max(1);
    let indices = form
        .fields
        .iter()
        .enumerate()
        .filter(|(_, field)| !field.locked)
        .map(|(index, _)| index)
        .chain(std::iter::once(form.fields.len()))
        .collect::<Vec<_>>();
    let mut fields = Vec::new();
    let mut selected_end = 0;
    for index in indices {
        let selected = index == form.selected;
        let text = if index == form.fields.len() {
            if form.network == Some(super::network::Editor::Export) {
                language.text("导出 JSON →", "Export JSON →")
            } else if form.network.is_some() {
                language.text("应用参数 →", "Apply parameters →")
            } else if form.setting.is_some() {
                language.text("保存设置 →", "Save setting →")
            } else {
                language.text("检查并执行 →", "Review and run →")
            }
            .into()
        } else {
            let mut value = form.visible_value(index);
            if selected && form.editing && value.chars().count() > 1024 {
                value = format!(
                    "…{}",
                    value
                        .chars()
                        .skip(value.chars().count() - 1024)
                        .collect::<String>()
                );
            }
            format!(
                "{}{}{}{}",
                form.fields[index].label,
                language.text("：", ": "),
                value,
                if form.editing && selected {
                    " ▏"
                } else if !form.fields[index].choices.is_empty() {
                    "  [← / →]"
                } else {
                    ""
                }
            )
        };
        fields.extend(
            super::render::wrap(
                &format!("{} {text}", if selected { "›" } else { " " }),
                width,
            )
            .into_iter()
            .map(|line| {
                (
                    line,
                    if selected {
                        Tone::Selected
                    } else {
                        Tone::Plain
                    },
                )
            }),
        );
        if selected {
            selected_end = fields.len();
        }
    }
    lines.extend(
        fields
            .into_iter()
            .skip(selected_end.saturating_sub(available))
            .take(available),
    );
    lines
}

pub(super) fn connection_rows(state: &State, width: usize) -> Vec<(String, Tone)> {
    let id = state.connection.as_deref().unwrap();
    let language = state.language;
    let Some(connection) = state.poll.as_ref().and_then(|poll| {
        poll.snapshot
            .connections
            .connections
            .iter()
            .find(|connection| connection.id.as_ref() == id)
    }) else {
        let message = match language {
            clash_client_core::config::Language::SimplifiedChinese => {
                format!("连接 {id} 已关闭或当前快照中不存在；Esc 返回列表")
            }
            clash_client_core::config::Language::English => {
                format!(
                    "Connection {id} has closed or is absent from the current snapshot; Esc returns to the list"
                )
            }
        };
        return super::render::wrap(&message, width)
            .into_iter()
            .map(|line| (line, Tone::Warning))
            .collect();
    };
    clash_client_core::connection::detail_values_in(connection, "", language)
        .into_iter()
        .filter(|(_, value)| !value.trim().is_empty() && value != "—")
        .flat_map(|(label, value)| {
            super::render::wrap(
                &format!("{label}{}{value}", language.text("：", ": ")),
                width,
            )
            .into_iter()
            .map(move |line| {
                (
                    line,
                    if label == language.text("代理链", "Proxy chain") {
                        Tone::Accent
                    } else {
                        Tone::Plain
                    },
                )
            })
        })
        .collect()
}

pub(super) fn connection(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let content = connection_rows(state, width);
    let offset = state.detail_offset.min(content.len().saturating_sub(1));
    let mut lines = Vec::new();
    if height >= 3 {
        lines.push((
            state
                .language
                .text(
                    "连接详情 · 空字段已隐藏 · ↑↓ 滚动 · Esc 返回 · d 解析 · x 关闭",
                    "Connection details · ↑↓ scroll · Esc back · d resolve · x close",
                )
                .into(),
            Tone::Accent,
        ));
    }
    lines.extend(content.into_iter().skip(offset));
    lines
}
