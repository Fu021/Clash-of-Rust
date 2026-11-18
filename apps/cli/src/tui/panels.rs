use super::{form, model::State, render::Tone};

pub(super) fn settings(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let mut lines = vec![
        (
            "设置 · Enter 打开；直接选择开关、模式或编辑数值".into(),
            Tone::Accent,
        ),
        (format!("搜索：{}", state.query), Tone::Muted),
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
        let next_category = super::settings::category(key);
        if next_category != category && available >= 3 {
            rows.push((format!("── {next_category} ──"), Tone::Accent));
            category = next_category;
        }
        let selected = index == state.selected;
        let start = rows.len();
        let value = state.poll.as_ref().map_or_else(
            || "后台未连接".into(),
            |poll| super::settings::display(poll, key),
        );
        let text = format!(
            "{} {} {value}",
            if selected { "›" } else { " " },
            super::settings::title(key)
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
            "管理操作 · 与 CLI 共用命令、授权和保护校验".into(),
            Tone::Accent,
        ),
        (format!("搜索命令或说明：{}", state.query), Tone::Muted),
    ];
    let items = state.items();
    let rows = items.iter().enumerate().map(|(index, path)| {
        let spec = form::commands().find(|spec| spec.path == *path).unwrap();
        (
            format!(
                "{} {}  · {}",
                if index == state.selected { "›" } else { " " },
                spec.description,
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
    let mut lines = if form.network == Some(super::network::Editor::Export) {
        vec![
            ("工具 · 导出本次结果".into(), Tone::Accent),
            (
                "保存现有证据，不重新运行检查；Esc 返回。".into(),
                Tone::Muted,
            ),
        ]
    } else if let Some(super::network::Editor::Parameters(tool)) = form.network {
        vec![
            (format!("{} · 参数", tool.title()), Tone::Accent),
            (
                "本页生效；切换工具恢复默认；Esc 丢弃草稿。".into(),
                Tone::Muted,
            ),
        ]
    } else if let Some(key) = &form.setting {
        vec![
            (
                format!("设置 · {}", super::settings::title(key)),
                Tone::Accent,
            ),
            (super::settings::hint(key).into(), Tone::Muted),
            ("草稿不会自动保存；选择保存后确认。".into(), Tone::Muted),
        ]
    } else {
        vec![
            (
                format!("cor {} · {}", form.spec.path, form.spec.description),
                Tone::Accent,
            ),
            (form.spec.usage.into(), Tone::Muted),
            (
                "每项单独填写，无需给空格或 emoji 加引号；数组使用 JSON。".into(),
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
                "导出 JSON →"
            } else if form.network.is_some() {
                "应用参数 →"
            } else if form.setting.is_some() {
                "保存设置 →"
            } else {
                "检查并执行 →"
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
                "{}：{}{}",
                form.fields[index].label,
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
    let Some(connection) = state.poll.as_ref().and_then(|poll| {
        poll.snapshot
            .connections
            .connections
            .iter()
            .find(|connection| connection.id.as_ref() == id)
    }) else {
        return super::render::wrap(
            &format!("连接 {id} 已关闭或当前快照中不存在；Esc 返回列表"),
            width,
        )
        .into_iter()
        .map(|line| (line, Tone::Warning))
        .collect();
    };
    clash_client_core::connection::detail_values(connection, "")
        .into_iter()
        .filter(|(_, value)| !value.trim().is_empty() && value != "—")
        .flat_map(|(label, value)| {
            super::render::wrap(&format!("{label}：{value}"), width)
                .into_iter()
                .map(move |line| {
                    (
                        line,
                        if label == "代理链" {
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
            "连接详情 · 空字段已隐藏 · ↑↓ 滚动 · Esc 返回 · d 解析 · x 关闭".into(),
            Tone::Accent,
        ));
    }
    lines.extend(content.into_iter().skip(offset));
    lines
}
