use super::{
    form,
    model::State,
    render::{Tone, fit},
};

pub(super) fn settings(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let mut lines = vec![
        (
            "设置 · Enter 打开；直接选择开关、模式或编辑数值".into(),
            Tone::Accent,
        ),
        (format!("搜索：{}", state.query), Tone::Muted),
    ];
    let items = state.items();
    lines.truncate(height.saturating_sub(1));
    let available = height.saturating_sub(lines.len()).max(1);
    let offset = state.selected.saturating_sub(available - 1);
    for (index, key) in items.iter().enumerate().skip(offset).take(available) {
        let value = state.poll.as_ref().map_or_else(
            || "后台未连接".into(),
            |poll| super::settings::display(poll, key),
        );
        lines.push((
            if width < 50 {
                format!(
                    "{} {}：{value}",
                    if index == state.selected { "›" } else { " " },
                    super::settings::title(key)
                )
            } else {
                format!(
                    "{} {} {}",
                    if index == state.selected { "›" } else { " " },
                    fit(super::settings::title(key), width.saturating_sub(26)),
                    value
                )
            },
            if index == state.selected {
                Tone::Selected
            } else {
                Tone::Plain
            },
        ));
    }
    lines
}

pub(super) fn tools(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let mut lines = vec![
        (
            "管理操作 · 与 CLI 共用命令、授权和保护校验".into(),
            Tone::Accent,
        ),
        (format!("搜索命令或说明：{}", state.query), Tone::Muted),
    ];
    lines.truncate(height.saturating_sub(1));
    let available = height.saturating_sub(lines.len()).max(1);
    let offset = state.selected.saturating_sub(available - 1);
    for (index, path) in state
        .items()
        .iter()
        .enumerate()
        .skip(offset)
        .take(available)
    {
        let spec = form::commands().find(|spec| spec.path == *path).unwrap();
        lines.push((
            format!(
                "{} {}  · {}",
                if index == state.selected { "›" } else { " " },
                fit(spec.description, width.saturating_sub(18)),
                spec.path
            ),
            if index == state.selected {
                Tone::Selected
            } else {
                Tone::Plain
            },
        ));
    }
    lines
}

pub(super) fn editor(state: &State, height: usize) -> Vec<(String, Tone)> {
    let form = state.form.as_ref().unwrap();
    let mut lines = if let Some(key) = &form.setting {
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
    lines.truncate(height.saturating_sub(1));
    let available = height.saturating_sub(lines.len()).max(1);
    let indices = form
        .fields
        .iter()
        .enumerate()
        .filter(|(_, field)| !field.locked)
        .map(|(index, _)| index)
        .chain(std::iter::once(form.fields.len()))
        .collect::<Vec<_>>();
    let selected = indices
        .iter()
        .position(|index| *index == form.selected)
        .unwrap_or(0);
    let offset = selected.saturating_sub(available - 1);
    for index in indices.into_iter().skip(offset) {
        if lines.len() >= height {
            break;
        }
        let selected = index == form.selected;
        let text = if index == form.fields.len() {
            if form.setting.is_some() {
                "保存设置 →"
            } else {
                "检查并执行 →"
            }
            .into()
        } else {
            format!(
                "{}：{}{}",
                form.fields[index].label,
                form.visible_value(index),
                if form.editing && selected {
                    " ▏"
                } else if !form.fields[index].choices.is_empty() {
                    "  [← / →]"
                } else {
                    ""
                }
            )
        };
        lines.push((
            format!("{} {text}", if selected { "›" } else { " " }),
            if selected {
                Tone::Selected
            } else {
                Tone::Plain
            },
        ));
    }
    lines.push((
        if form.editing {
            "输入中 · Ctrl+U 清空 · Enter 保留草稿 · Esc 撤销本项"
        } else {
            "↑↓ 选项 · Enter 编辑/执行 · Esc 返回 · 所有写操作由后台最终校验"
        }
        .into(),
        Tone::Muted,
    ));
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
        .flat_map(|(label, value)| {
            super::render::wrap(
                &format!(
                    "{label}：{}",
                    if value.is_empty() {
                        "未提供"
                    } else {
                        &value
                    }
                ),
                width,
            )
            .into_iter()
            .map(|line| {
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
            "连接详情 · ↑↓ 滚动 · Esc 返回 · d 解析 · x 关闭".into(),
            Tone::Accent,
        ));
    }
    lines.extend(content.into_iter().skip(offset));
    lines
}
