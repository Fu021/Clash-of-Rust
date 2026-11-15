use super::{
    form,
    model::State,
    render::{Tone, fit, rate},
};

pub(super) fn settings(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let mut lines = vec![
        (
            "设置 · Enter 编辑；只读项目通过专用命令管理".into(),
            Tone::Accent,
        ),
        (format!("搜索：{}", state.query), Tone::Muted),
    ];
    let items = state.items();
    let available = height.saturating_sub(2).max(1);
    let offset = state.selected.saturating_sub(available - 1);
    for (index, key) in items.iter().enumerate().skip(offset).take(available) {
        let value = state
            .poll
            .as_ref()
            .map(|poll| form::setting_value(&poll.meta.settings, key))
            .map_or_else(
                || "后台未连接".into(),
                |value| match value.as_str() {
                    Some(text) => text.to_owned(),
                    None => value.to_string(),
                },
            );
        lines.push((
            format!(
                "{} {} {}{}",
                if index == state.selected { "›" } else { " " },
                fit(form::setting_title(key), width.saturating_sub(26)),
                if clash_client_core::config::MUTABLE_SETTINGS.contains(key) {
                    ""
                } else {
                    "[只读] "
                },
                value
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

pub(super) fn tools(state: &State, width: usize, height: usize) -> Vec<(String, Tone)> {
    let mut lines = vec![
        (
            "管理操作 · 与 CLI 共用命令、授权和保护校验".into(),
            Tone::Accent,
        ),
        (format!("搜索命令或说明：{}", state.query), Tone::Muted),
    ];
    let available = height.saturating_sub(2).max(1);
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
                "{} {}  {}",
                if index == state.selected { "›" } else { " " },
                fit(spec.path, width.min(26)),
                spec.description
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
    let mut lines = vec![
        (
            format!("cor {} · {}", form.spec.path, form.spec.description),
            Tone::Accent,
        ),
        (form.spec.usage.into(), Tone::Muted),
        (
            "每项单独填写，无需给空格或 emoji 加引号；数组使用 JSON。".into(),
            Tone::Muted,
        ),
    ];
    let available = height.saturating_sub(4).max(1);
    let offset = form.selected.saturating_sub(available - 1);
    for index in offset..=form.fields.len() {
        if lines.len() >= height.saturating_sub(1) {
            break;
        }
        let selected = index == form.selected;
        let text = if index == form.fields.len() {
            "检查并执行 →".into()
        } else {
            format!(
                "{}：{}{}",
                form.fields[index].label,
                form.visible_value(index),
                if form.editing && selected { " ▏" } else { "" }
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

pub(super) fn connection(state: &State) -> Vec<(String, Tone)> {
    let id = state.connection.as_deref().unwrap();
    let Some(connection) = state.poll.as_ref().and_then(|poll| {
        poll.snapshot
            .connections
            .connections
            .iter()
            .find(|connection| connection.id.as_ref() == id)
    }) else {
        return vec![
            (format!("连接 {id}"), Tone::Accent),
            (
                "连接已关闭或当前快照中不存在；Esc 返回列表".into(),
                Tone::Warning,
            ),
        ];
    };
    let metadata = &connection.metadata;
    let network = serde_json::to_value(&metadata.network).unwrap_or_default();
    let mut lines = vec![
        (
            "连接详情 · 每秒刷新 · Esc 返回 · d 补充解析详情 · x 关闭此连接".into(),
            Tone::Accent,
        ),
        (format!("ID        {id}"), Tone::Muted),
        (format!("域名      {}", metadata.host), Tone::Plain),
        (
            format!("嗅探域名  {}", metadata.sniff_host.as_str()),
            Tone::Plain,
        ),
        (
            format!("目标 IP   {}", metadata.destination_ip),
            Tone::Plain,
        ),
        (
            format!("目标端口  {}", metadata.destination_port.as_str()),
            Tone::Plain,
        ),
        (
            format!("协议      {}", network.as_str().unwrap_or("待确认")),
            Tone::Plain,
        ),
        (format!("开始时间  {}", connection.start), Tone::Plain),
        (
            format!(
                "命中规则  {} {}",
                connection.rule.as_str(),
                connection.rule_payload.as_str()
            ),
            Tone::Plain,
        ),
        (
            format!(
                "实际线路  {}",
                connection
                    .chains
                    .iter()
                    .rev()
                    .map(|part| part.as_str())
                    .collect::<Vec<_>>()
                    .join(" → ")
            ),
            Tone::Accent,
        ),
        (
            format!(
                "累计流量  ↓ {}  ↑ {}",
                rate(connection.download),
                rate(connection.upload)
            ),
            Tone::Plain,
        ),
    ];
    if let Some(rates) = connection.rates {
        lines.push((
            format!(
                "当前速率  ↓ {}/s  ↑ {}/s",
                rate(rates.download),
                rate(rates.upload)
            ),
            Tone::Plain,
        ));
    }
    let offset = state.detail_offset.min(lines.len().saturating_sub(1));
    lines.drain(..offset);
    lines
}
