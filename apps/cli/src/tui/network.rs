use super::{
    form::{self, Form},
    model::{Page, State},
    render,
};
use anyhow::{Context, Result, ensure};
use clash_client_core::network_tools::{Options, Report, Tool};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Editor {
    Parameters(Tool),
    Export,
}

pub(super) struct Export {
    path: std::path::PathBuf,
    report: Report,
}

impl Export {
    pub fn save(self) -> Result<String> {
        let command = format!("network {}", self.report.tool.id());
        let value = crate::output::redact(serde_json::to_value(self.report)?);
        crate::output::save(&self.path, &command, &value)?;
        Ok(self.path.display().to_string())
    }
}

pub(super) fn editor(tool: Tool, target: &str, options: &Options) -> Form {
    let path = format!("network {}", tool.id());
    let mut editor = Form::new(form::commands().find(|spec| spec.path == path).unwrap());
    editor.network = Some(Editor::Parameters(tool));
    editor.fields.retain(|field| field.option != Some("output"));
    for field in &mut editor.fields {
        let (label, value) = match field.option {
            None => (
                match tool {
                    Tool::Dns => "域名",
                    Tool::Udp => "DNS 服务器",
                    Tool::Tcp => "主机与端口",
                    _ => "目标",
                },
                target.to_owned(),
            ),
            Some("count") => ("次数（1～20）", options.count.to_string()),
            Some("interval-ms") => ("间隔（毫秒）", options.interval_ms.to_string()),
            Some("seconds") => ("时限（秒）", options.seconds.to_string()),
            Some("max-bytes") => ("接收上限（KiB）", (options.max_bytes / 1024).to_string()),
            Some("max-hops") => ("最大跳数", options.max_hops.to_string()),
            Some("type") => ("记录类型", options.record_type.clone()),
            Some("method") => ("HTTP 方法", options.method.clone()),
            _ => unreachable!(),
        };
        field.label = label.into();
        field.value = value;
        let choices: &[&str] = match field.option {
            Some("type") => &[
                "ADDR", "A", "AAAA", "CNAME", "MX", "TXT", "NS", "SOA", "PTR", "CAA",
            ],
            Some("method") => &["HEAD", "GET", "OPTIONS"],
            _ => &[],
        };
        field.choices = choices
            .iter()
            .map(|value| ((*value).into(), (*value).into()))
            .collect();
    }
    editor
}

pub(super) fn export_editor() -> Form {
    let mut editor = Form::new(
        form::commands()
            .find(|spec| spec.path == "network dns")
            .unwrap(),
    );
    editor.network = Some(Editor::Export);
    editor.fields.retain(|field| field.option == Some("output"));
    editor.fields[0].label = "JSON 文件".into();
    editor.fields[0].value = "cor-network-report.json".into();
    editor
}

pub(super) fn prepare_export(state: &State) -> Result<Export> {
    ensure!(!state.demo, "演示模式不写入文件");
    let editor = state.form.as_ref().context("没有正在编辑的导出路径")?;
    ensure!(
        state.page == Page::Network && editor.network == Some(Editor::Export),
        "请从网络页导出当前结果"
    );
    let path = editor.fields[0].value.trim();
    ensure!(!path.is_empty(), "请输入 JSON 文件路径");
    Ok(Export {
        path: crate::backend::absolute(path)?,
        report: state
            .network_report
            .clone()
            .context("没有可导出的检查报告")?,
    })
}

pub(super) fn apply(state: &mut State) -> Result<()> {
    let editor = state.form.as_ref().context("没有正在编辑的工具参数")?;
    ensure!(
        state.page == Page::Network
            && editor.network == Some(Editor::Parameters(Tool::ALL[state.network_tool])),
        "工具已变化，请重新编辑参数"
    );
    let mut options = Options::default();
    let mut target = String::new();
    for field in &editor.fields {
        let value = field.value.trim();
        let parse_error = || format!("{}必须填写有效整数", field.label);
        match field.option {
            None => target = value.to_owned(),
            Some("count") => options.count = value.parse().with_context(parse_error)?,
            Some("interval-ms") => options.interval_ms = value.parse().with_context(parse_error)?,
            Some("seconds") => options.seconds = value.parse().with_context(parse_error)?,
            Some("max-bytes") => {
                options.max_bytes = value
                    .parse::<u64>()
                    .with_context(parse_error)?
                    .checked_mul(1024)
                    .context("接收上限过大")?;
            }
            Some("max-hops") => options.max_hops = value.parse().with_context(parse_error)?,
            Some("type") => options.record_type = value.to_ascii_uppercase(),
            Some("method") => options.method = value.to_ascii_uppercase(),
            _ => unreachable!(),
        }
    }
    ensure!(
        !target.is_empty() && target.len() <= 256,
        "目标不能为空且最多 256 字节"
    );
    options.validate()?;
    state.network_options = options;
    state.query = target;
    state.selected = 0;
    state.network_results.clear();
    state.network_report = None;
    state.form = None;
    state.message = Some("参数仅本页生效；Enter 开始检查".into());
    Ok(())
}

pub(super) fn summary(tool: Tool, options: &Options) -> String {
    match tool {
        Tool::Website => "默认检查".into(),
        Tool::Dns => format!("记录 {}", options.record_type),
        Tool::Stability => format!(
            "{} 次 · 间隔 {}ms · 时限 {}s",
            options.count, options.interval_ms, options.seconds
        ),
        Tool::Ping => format!("{} 次", options.count),
        Tool::Trace => format!("最多 {} 跳", options.max_hops),
        Tool::Download => format!(
            "{} KiB · 时限 {}s",
            options.max_bytes / 1024,
            options.seconds
        ),
        Tool::Http => format!("{} · 时限 {}s", options.method, options.seconds),
        _ => format!("时限 {}s", options.seconds),
    }
}

pub(super) struct Row {
    pub text: String,
    entry: usize,
    offset: usize,
}

pub(super) fn rows(state: &State, width: usize) -> Vec<Row> {
    state
        .network_results
        .iter()
        .enumerate()
        .flat_map(|(entry, text)| {
            let mut offset = 0;
            render::wrap(text, width).into_iter().map(move |text| {
                let row = Row {
                    offset,
                    entry,
                    text,
                };
                offset += row.text.chars().count();
                row
            })
        })
        .collect()
}

pub(super) fn scroll(state: &mut State, width: usize, down: bool) {
    let limit = rows(state, width).len().saturating_sub(1);
    state.selected = if down {
        state.selected.saturating_add(1).min(limit)
    } else {
        state.selected.min(limit).saturating_sub(1)
    };
}

pub(super) fn resize(state: &mut State, old_width: usize, new_width: usize) {
    let old = rows(state, old_width);
    let Some(anchor) = old.get(state.selected) else {
        state.selected = 0;
        return;
    };
    state.selected = rows(state, new_width)
        .iter()
        .rposition(|row| {
            row.entry < anchor.entry || row.entry == anchor.entry && row.offset <= anchor.offset
        })
        .unwrap_or(0);
}
