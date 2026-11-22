use super::{
    form::{self, Form},
    model::{Page, State},
    render,
};
use anyhow::{Context, Result, ensure};
use clash_client_core::config::Language;
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

pub(super) fn title(tool: Tool, language: Language) -> &'static str {
    language.text(
        tool.title(),
        match tool {
            Tool::Website => "Website check",
            Tool::Dns => "DNS lookup",
            Tool::Phase => "Connection timing",
            Tool::Family => "IPv4 / IPv6",
            Tool::Tls => "TLS certificate",
            Tool::Stability => "Short-term stability",
            Tool::Udp => "UDP connectivity",
            Tool::Ping => "Ping",
            Tool::Trace => "Traceroute",
            Tool::Download => "Limited download",
            Tool::Tcp => "TCP port",
            Tool::Http => "HTTP check",
        },
    )
}

pub(super) fn hint(tool: Tool, language: Language) -> &'static str {
    language.text(
        tool.hint(),
        match tool {
            Tool::Website => "Check resolution, response, and matching rules.",
            Tool::Dns => "Compare system and core DNS records, TTLs, and response codes.",
            Tool::Phase => "Measure proxy connection, TLS, and first-byte timing separately.",
            Tool::Family => "Connect to each target address family through the proxy.",
            Tool::Tls => "Check the TLS handshake and certificate without an HTTP request.",
            Tool::Stability => "Measure transfer failures, HTTP status, and response times.",
            Tool::Udp => "Send a probe to the DNS server through the core's UDP relay.",
            Tool::Ping => "Measure ICMP latency through the system network.",
            Tool::Trace => "Trace hops through the system network; some may not respond.",
            Tool::Download => "Measure throughput with time and size limits.",
            Tool::Tcp => "Connect through the current proxy without sending application data.",
            Tool::Http => "Inspect method, status, and security headers; credentials are hidden.",
        },
    )
}

pub(super) fn editor(tool: Tool, target: &str, options: &Options, language: Language) -> Form {
    let path = format!("network {}", tool.id());
    let mut editor = Form::new_in(
        form::commands().find(|spec| spec.path == path).unwrap(),
        language,
    );
    editor.network = Some(Editor::Parameters(tool));
    editor.fields.retain(|field| field.option != Some("output"));
    for field in &mut editor.fields {
        let (label, value) = match field.option {
            None => (
                match tool {
                    Tool::Dns => language.text("域名", "Domain"),
                    Tool::Udp => language.text("DNS 服务器", "DNS server"),
                    Tool::Tcp => language.text("主机与端口", "Host and port"),
                    _ => language.text("目标", "Target"),
                },
                target.to_owned(),
            ),
            Some("count") => (
                language.text("次数（1～20）", "Count (1–20)"),
                options.count.to_string(),
            ),
            Some("interval-ms") => (
                language.text("间隔（毫秒）", "Interval (ms)"),
                options.interval_ms.to_string(),
            ),
            Some("seconds") => (
                language.text("时限（秒）", "Timeout (seconds)"),
                options.seconds.to_string(),
            ),
            Some("max-bytes") => (
                language.text("接收上限（KiB）", "Size limit (KiB)"),
                (options.max_bytes / 1024).to_string(),
            ),
            Some("max-hops") => (
                language.text("最大跳数", "Maximum hops"),
                options.max_hops.to_string(),
            ),
            Some("type") => (
                language.text("记录类型", "Record type"),
                options.record_type.clone(),
            ),
            Some("method") => (
                language.text("HTTP 方法", "HTTP method"),
                options.method.clone(),
            ),
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

pub(super) fn export_editor(language: Language) -> Form {
    let mut editor = Form::new_in(
        form::commands()
            .find(|spec| spec.path == "network dns")
            .unwrap(),
        language,
    );
    editor.network = Some(Editor::Export);
    editor.fields.retain(|field| field.option == Some("output"));
    editor.fields[0].label = language.text("JSON 文件", "JSON file").into();
    editor.fields[0].value = "cor-network-report.json".into();
    editor
}

pub(super) fn prepare_export(state: &State) -> Result<Export> {
    let language = state.language;
    ensure!(
        !state.demo,
        "{}",
        language.text("演示模式不写入文件", "Demo mode does not write files")
    );
    let editor = state
        .form
        .as_ref()
        .context(language.text("没有正在编辑的导出路径", "No export path is being edited"))?;
    ensure!(
        state.page == Page::Network && editor.network == Some(Editor::Export),
        "{}",
        language.text(
            "请从网络页导出当前结果",
            "Export the current result from the Network page"
        )
    );
    let path = editor.fields[0].value.trim();
    ensure!(
        !path.is_empty(),
        "{}",
        language.text("请输入 JSON 文件路径", "Enter a JSON file path")
    );
    Ok(Export {
        path: crate::backend::absolute(path)?,
        report: state
            .network_report
            .clone()
            .context(language.text("没有可导出的检查报告", "No report is available to export"))?,
    })
}

pub(super) fn apply(state: &mut State) -> Result<()> {
    let language = state.language;
    let editor = state.form.as_ref().context(language.text(
        "没有正在编辑的工具参数",
        "No tool parameters are being edited",
    ))?;
    ensure!(
        state.page == Page::Network
            && editor.network == Some(Editor::Parameters(Tool::ALL[state.network_tool])),
        "{}",
        language.text(
            "工具已变化，请重新编辑参数",
            "The tool changed; edit its parameters again"
        )
    );
    let mut options = Options::default();
    let mut target = String::new();
    for field in &editor.fields {
        let value = field.value.trim();
        let parse_error = || {
            format!(
                "{}{}",
                field.label,
                language.text("必须填写有效整数", " must be a valid integer")
            )
        };
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
                    .context(language.text("接收上限过大", "Size limit is too large"))?;
            }
            Some("max-hops") => options.max_hops = value.parse().with_context(parse_error)?,
            Some("type") => options.record_type = value.to_ascii_uppercase(),
            Some("method") => options.method = value.to_ascii_uppercase(),
            _ => unreachable!(),
        }
    }
    ensure!(
        !target.is_empty() && target.len() <= 256,
        "{}",
        language.text(
            "目标不能为空且最多 256 字节",
            "Target must contain 1–256 bytes"
        )
    );
    options.validate_for_language(language)?;
    state.network_options = options;
    state.query = target;
    state.selected = 0;
    state.network_results.clear();
    state.network_report = None;
    state.form = None;
    state.message = Some(
        language
            .text(
                "参数仅本页生效；Enter 开始检查",
                "Parameters apply here; press Enter to run",
            )
            .into(),
    );
    Ok(())
}

pub(super) fn summary(tool: Tool, options: &Options, language: Language) -> String {
    if language == Language::English {
        return match tool {
            Tool::Website => "Default check".into(),
            Tool::Dns => format!("Record {}", options.record_type),
            Tool::Stability => format!(
                "{} requests · {}ms interval · {}s timeout",
                options.count, options.interval_ms, options.seconds
            ),
            Tool::Ping => format!("{} requests", options.count),
            Tool::Trace => format!("Up to {} hops", options.max_hops),
            Tool::Download => format!(
                "{} KiB · {}s timeout",
                options.max_bytes / 1024,
                options.seconds
            ),
            Tool::Http => format!("{} · {}s timeout", options.method, options.seconds),
            _ => format!("{}s timeout", options.seconds),
        };
    }
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
