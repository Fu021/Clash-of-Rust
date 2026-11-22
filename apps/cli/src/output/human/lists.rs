use super::{
    field_in,
    fields::{clean, scalar_in, text_in},
    rows, table,
};
use crate::args::Args;
use anyhow::Result;
use clash_client_core::config::Language;
use serde_json::Value;
use std::io::Write;

fn kind(row: &Value) -> &str {
    row["type"].as_str().unwrap_or("")
}
fn group(row: &Value) -> bool {
    row["members"].as_u64().unwrap_or(0) > 0
        || matches!(
            kind(row),
            "Selector" | "URLTest" | "Fallback" | "LoadBalance" | "Relay"
        )
}
fn builtin(row: &Value) -> bool {
    matches!(
        row["name"].as_str().unwrap_or(""),
        "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS" | "PASS-RULE" | "COMPATIBLE"
    ) && matches!(
        kind(row),
        "Direct" | "Reject" | "RejectDrop" | "Pass" | "PassRule" | "Compatible"
    )
}
fn group_kind(row: &Value, language: Language) -> String {
    clean(match kind(row) {
        "Selector" => language.text("手动选择", "Manual selection"),
        "URLTest" => language.text("自动测速", "Automatic testing"),
        "Fallback" => language.text("故障转移", "Fallback"),
        "LoadBalance" => language.text("负载均衡", "Load balancing"),
        "Relay" => language.text("链式代理", "Relay"),
        other => other,
    })
}

pub(super) fn ip_results(out: &mut dyn Write, value: &Value, language: Language) -> Result<()> {
    let cells = rows(&value["results"])
        .iter()
        .map(|row| {
            let summary = if row["summary"].as_str().is_none_or(str::is_empty) {
                text_in(row, "detail", language)
            } else {
                text_in(row, "summary", language)
            };
            vec![
                text_in(row, "name", language),
                text_in(row, "state", language),
                summary,
                text_in(row, "country", language),
                text_in(row, "millis", language),
            ]
        })
        .collect::<Vec<_>>();
    table::write_in(
        out,
        &[
            language.text("项目", "Item"),
            language.text("状态", "Status"),
            language.text("结果", "Result"),
            language.text("地区", "Region"),
            language.text("耗时", "Duration"),
        ],
        &cells,
        table::width(),
        language,
    )?;
    let exits = rows(&value["results"])
        .iter()
        .filter(|row| row.get("target_exit").is_some())
        .collect::<Vec<_>>();
    if !exits.is_empty() {
        writeln!(
            out,
            "\n{}",
            language.text(
                "分流出口 · 同域探针（本应用内核的本次请求）",
                "Service exits · Same-domain probes"
            )
        )?;
        for row in exits {
            let exit = &row["target_exit"];
            let address = exit["ip"]
                .as_str()
                .unwrap_or_else(|| language.text("未确认", "Unconfirmed"));
            field_in(
                out,
                &text_in(row, "name", language),
                &clean(address),
                language,
            )?;
        }
    }
    Ok(())
}

pub(super) fn nodes(out: &mut dyn Write, args: &Args, value: &Value) -> Result<()> {
    let language = args.language();
    if value["offline"] == true {
        writeln!(
            out,
            "{}\n",
            language.text(
                "内核未运行；显示已保存订阅，未测速。远程节点提供者及动态筛选成员未展开。",
                "Core stopped; showing saved subscriptions without latency tests. Remote providers and dynamic members are not expanded."
            )
        )?;
    }
    let entries = rows(&value["items"]);
    if let Some(name) = args.positional.first() {
        field_in(
            out,
            language.text("策略组", "Group"),
            &clean(name),
            language,
        )?;
        writeln!(out)?;
    }
    if entries.is_empty() {
        writeln!(
            out,
            "{}",
            language.text("未找到匹配的节点或策略组", "No matching nodes or groups")
        )?;
    }
    let selected_column = !args.positional.is_empty();
    for section in ["groups", "nodes", "builtins"] {
        let mut cells = Vec::new();
        for row in entries {
            let is_group = group(row);
            let is_builtin = builtin(row);
            if match section {
                "groups" => !is_group,
                "nodes" => is_group || is_builtin,
                _ => !is_builtin,
            } {
                continue;
            }
            let mut line = vec![text_in(row, "name", language)];
            match section {
                "groups" => line.extend([
                    group_kind(row, language),
                    text_in(row, "now", language),
                    text_in(row, "members", language),
                ]),
                "nodes" => line.extend([
                    text_in(row, "type", language),
                    scalar_in("latency_ms", &row["latency_ms"], language),
                ]),
                _ => line.push(
                    match row["name"].as_str().unwrap_or("") {
                        "DIRECT" => language.text("直连", "Direct"),
                        "REJECT" => language.text("拦截连接", "Reject connections"),
                        "REJECT-DROP" => language.text("丢弃连接", "Drop connections"),
                        _ => language.text("内核内置动作", "Built-in action"),
                    }
                    .into(),
                ),
            }
            if selected_column {
                line.push(
                    if row["selected"] == true {
                        language.text("当前", "Current")
                    } else {
                        "—"
                    }
                    .into(),
                );
            }
            cells.push(line);
        }
        if cells.is_empty() {
            continue;
        }
        let heading = match section {
            "groups" => language.text("策略组", "Groups"),
            "nodes" => language.text("节点", "Nodes"),
            _ => language.text("内置出站", "Built-in routes"),
        };
        match language {
            Language::SimplifiedChinese => writeln!(out, "{heading}（{}）", cells.len())?,
            Language::English => writeln!(out, "{heading} ({})", cells.len())?,
        }
        let mut headers = match section {
            "groups" => vec![
                language.text("名称", "Name"),
                language.text("类型", "Type"),
                language.text("当前选择", "Selected"),
                language.text("成员数", "Members"),
            ],
            "nodes" => vec![
                language.text("名称", "Name"),
                language.text("协议", "Protocol"),
                language.text("延迟", "Latency"),
            ],
            _ => vec![
                language.text("名称", "Name"),
                language.text("作用", "Action"),
            ],
        };
        if selected_column {
            headers.push(language.text("状态", "Status"));
        }
        table::write_in(out, &headers, &cells, table::width(), language)?;
        writeln!(out)?;
    }
    table::page_in(out, value, language)
}

pub(super) fn delays(out: &mut dyn Write, args: &Args, value: &Value) -> Result<()> {
    let language = args.language();
    if let Some(map) = value.as_object() {
        let cells: Vec<_> = map
            .iter()
            .map(|(name, delay)| vec![clean(name), scalar_in("latency_ms", delay, language)])
            .collect();
        table::write_in(
            out,
            &[
                language.text("节点 / 策略组", "Node / group"),
                language.text("延迟", "Latency"),
            ],
            &cells,
            table::width(),
            language,
        )?;
        let available = map
            .values()
            .filter(|v| v.as_u64().is_some_and(|n| n > 0))
            .count();
        let failed = map.len() - available;
        match language {
            Language::SimplifiedChinese => {
                writeln!(out, "测速完成：{available} 项可达，{failed} 项超时或不可达")?
            }
            Language::English => writeln!(
                out,
                "Speed test complete: {available} reachable, {failed} timed out or unreachable"
            )?,
        }
        if (args.flag("all") || args.get("group").is_some())
            && !map.is_empty()
            && map.values().all(|delay| delay.as_u64() == Some(0))
        {
            writeln!(
                out,
                "{}",
                language.text(
                    "全部节点不可达。请更换节点；更新失败后会自动尝试直连。",
                    "All nodes are unreachable. Choose another node; updates retry directly after a failure."
                )
            )?;
            writeln!(
                out,
                "{}",
                language.text(
                    "接管状态：cor status · 关闭代理：cor proxy off",
                    "Status: cor status · Turn off proxy: cor proxy off"
                )
            )?;
            writeln!(
                out,
                "{}",
                language.text(
                    "导出诊断：cor diagnose --output cor-diagnose.json",
                    "Export diagnostics: cor diagnose --output cor-diagnose.json"
                )
            )?;
        }
    }
    Ok(())
}

pub(super) fn profiles(out: &mut dyn Write, value: &Value, language: Language) -> Result<()> {
    let entries = rows(value);
    if entries.is_empty() {
        writeln!(
            out,
            "{}",
            language.text("未找到匹配的订阅", "No matching subscriptions")
        )?;
    }
    for (index, row) in entries.iter().enumerate() {
        if index > 0 {
            writeln!(out)?;
        }
        writeln!(
            out,
            "{}{}",
            if row["active"] == true {
                language.text("[当前] ", "[Current] ")
            } else {
                ""
            },
            text_in(row, "name", language)
        )?;
        if let Some(owner) = row["owner"].as_str() {
            let owner = clean(owner);
            let uid = text_in(row, "uid", language);
            match language {
                Language::SimplifiedChinese => writeln!(out, "  所属用户：{owner}（UID {uid}）")?,
                Language::English => writeln!(out, "  Owner: {owner} (UID {uid})")?,
            }
            field_in(
                out,
                language.text("  数据目录", "  Data directory"),
                &text_in(row, "data_dir", language),
                language,
            )?;
        }
        for (label, key) in [
            ("ID", "id"),
            (language.text("来源", "Source"), "source"),
            (language.text("更新", "Updated"), "updated"),
        ] {
            field_in(
                out,
                &format!("  {label}"),
                &text_in(row, key, language),
                language,
            )?;
        }
        if !row["usage"].is_null() {
            field_in(
                out,
                language.text("  用量", "  Usage"),
                &text_in(row, "usage", language),
                language,
            )?;
        }
    }
    match language {
        Language::SimplifiedChinese => writeln!(out, "共 {} 个订阅", entries.len())?,
        Language::English => writeln!(out, "{} subscriptions", entries.len())?,
    }
    Ok(())
}

pub(super) fn rules(out: &mut dyn Write, value: &Value, language: Language) -> Result<()> {
    let entries = rows(&value["items"]);
    let cells: Vec<_> = entries
        .iter()
        .map(|row| {
            vec![
                text_in(row, "position", language),
                if row["enabled"] == true {
                    language.text("启用", "Enabled")
                } else {
                    language.text("停用", "Disabled")
                }
                .into(),
                format!(
                    "{}{}",
                    text_in(row, "source", language),
                    if row["duplicate"] == true {
                        language.text("（重复）", " (duplicate)")
                    } else {
                        ""
                    }
                ),
                text_in(row, "rule", language),
            ]
        })
        .collect();
    table::write_in(
        out,
        &[
            language.text("序号", "Position"),
            language.text("状态", "Status"),
            language.text("来源", "Source"),
            language.text("规则", "Rule"),
        ],
        &cells,
        table::width(),
        language,
    )?;
    table::page_in(out, value, language)
}

pub(super) fn connections(out: &mut dyn Write, value: &Value, language: Language) -> Result<()> {
    if value["offline"] == true {
        writeln!(
            out,
            "{}",
            language.text(
                "内核未运行，没有活动连接；查询未启动后台或内核。",
                "Core stopped; no active connections. This query did not start the core."
            )
        )?;
        return table::page_in(out, value, language);
    }
    let entries: Vec<_> = if value.get("items").is_some() {
        rows(&value["items"]).iter().collect()
    } else {
        vec![value]
    };
    if entries.is_empty() {
        writeln!(
            out,
            "{}",
            language.text("未找到匹配的连接", "No matching connections")
        )?;
    }
    for (index, row) in entries.iter().enumerate() {
        if index > 0 {
            writeln!(out)?;
        }
        match language {
            Language::SimplifiedChinese => writeln!(
                out,
                "{}（{}）",
                text_in(row, "name", language),
                text_in(row, "network", language)
            )?,
            Language::English => writeln!(
                out,
                "{} ({})",
                text_in(row, "name", language),
                text_in(row, "network", language)
            )?,
        }
        field_in(out, "  ID", &text_in(row, "id", language), language)?;
        field_in(
            out,
            language.text("  规则", "  Rule"),
            &format!(
                "{}{}",
                text_in(row, "rule", language),
                row["rule_payload"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| match language {
                        Language::SimplifiedChinese => format!("（{}）", clean(s)),
                        Language::English => format!(" ({})", clean(s)),
                    })
                    .unwrap_or_default()
            ),
            language,
        )?;
        let chain = rows(&row["chain"])
            .iter()
            .rev()
            .filter_map(Value::as_str)
            .map(clean)
            .collect::<Vec<_>>()
            .join(" → ");
        field_in(
            out,
            language.text("  线路", "  Route"),
            if chain.is_empty() { "—" } else { &chain },
            language,
        )?;
        let upload_rate = text_in(row, "upload_rate", language);
        let download_rate = text_in(row, "download_rate", language);
        let upload = text_in(row, "upload", language);
        let download = text_in(row, "download", language);
        match language {
            Language::SimplifiedChinese => {
                writeln!(out, "  速度：上传 {upload_rate} / 下载 {download_rate}")?;
                writeln!(out, "  流量：上传 {upload} / 下载 {download}")?;
            }
            Language::English => {
                writeln!(
                    out,
                    "  Speed: Upload {upload_rate} / Download {download_rate}"
                )?;
                writeln!(out, "  Traffic: Upload {upload} / Download {download}")?;
            }
        }
        if value.get("items").is_none() {
            for key in [
                "host",
                "sniff_host",
                "ip",
                "port",
                "start",
                "ip_source",
                "resolution_error",
            ] {
                if !row[key].is_null() {
                    field_in(
                        out,
                        &format!("  {}", super::label_in(key, language)),
                        &text_in(row, key, language),
                        language,
                    )?;
                }
            }
        }
    }
    if value.get("items").is_some() {
        table::page_in(out, value, language)?;
    }
    Ok(())
}
