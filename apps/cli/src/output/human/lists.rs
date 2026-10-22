use super::{
    field,
    fields::{clean, latency, text},
    rows, table,
};
use crate::args::Args;
use anyhow::Result;
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
fn group_kind(row: &Value) -> String {
    clean(match kind(row) {
        "Selector" => "手动选择",
        "URLTest" => "自动测速",
        "Fallback" => "故障转移",
        "LoadBalance" => "负载均衡",
        "Relay" => "链式代理",
        other => other,
    })
}

pub(super) fn ip_results(out: &mut dyn Write, value: &Value) -> Result<()> {
    let cells = rows(&value["results"])
        .iter()
        .map(|row| {
            let summary = if row["summary"].as_str().is_none_or(str::is_empty) {
                text(row, "detail")
            } else {
                text(row, "summary")
            };
            vec![
                text(row, "name"),
                text(row, "state"),
                summary,
                text(row, "country"),
                text(row, "millis"),
            ]
        })
        .collect::<Vec<_>>();
    table::write(
        out,
        &["项目", "状态", "结果", "地区", "耗时"],
        &cells,
        table::width(),
    )
}

pub(super) fn nodes(out: &mut dyn Write, args: &Args, value: &Value) -> Result<()> {
    let entries = rows(&value["items"]);
    if let Some(name) = args.positional.first() {
        writeln!(out, "策略组：{}\n", clean(name))?;
    }
    if entries.is_empty() {
        writeln!(out, "未找到匹配的节点或策略组")?;
    }
    let selected_column = !args.positional.is_empty();
    for section in ["策略组", "节点", "内置出站"] {
        let mut cells = Vec::new();
        for row in entries {
            let is_group = group(row);
            let is_builtin = builtin(row);
            if match section {
                "策略组" => !is_group,
                "节点" => is_group || is_builtin,
                _ => !is_builtin,
            } {
                continue;
            }
            let mut line = vec![text(row, "name")];
            match section {
                "策略组" => {
                    line.extend([group_kind(row), text(row, "now"), text(row, "members")])
                }
                "节点" => line.extend([text(row, "type"), latency(&row["latency_ms"])]),
                _ => line.push(
                    match row["name"].as_str().unwrap_or("") {
                        "DIRECT" => "直连",
                        "REJECT" => "拦截连接",
                        "REJECT-DROP" => "丢弃连接",
                        _ => "内核内置动作",
                    }
                    .into(),
                ),
            }
            if selected_column {
                line.push(
                    if row["selected"] == true {
                        "当前"
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
        writeln!(out, "{section}（{}）", cells.len())?;
        let mut headers = match section {
            "策略组" => vec!["名称", "类型", "当前选择", "成员数"],
            "节点" => vec!["名称", "协议", "延迟"],
            _ => vec!["名称", "作用"],
        };
        if selected_column {
            headers.push("状态");
        }
        table::write(out, &headers, &cells, table::width())?;
        writeln!(out)?;
    }
    table::page(out, value)
}

pub(super) fn delays(out: &mut dyn Write, value: &Value) -> Result<()> {
    if let Some(map) = value.as_object() {
        let cells: Vec<_> = map
            .iter()
            .map(|(name, delay)| vec![clean(name), latency(delay)])
            .collect();
        table::write(out, &["节点 / 策略组", "延迟"], &cells, table::width())?;
        let available = map
            .values()
            .filter(|v| v.as_u64().is_some_and(|n| n > 0))
            .count();
        writeln!(
            out,
            "测速完成：{available} 项可达，{} 项超时或不可达",
            map.len() - available
        )?;
    }
    Ok(())
}

pub(super) fn profiles(out: &mut dyn Write, value: &Value) -> Result<()> {
    let entries = rows(value);
    if entries.is_empty() {
        writeln!(out, "未找到匹配的订阅")?;
    }
    for (index, row) in entries.iter().enumerate() {
        if index > 0 {
            writeln!(out)?;
        }
        writeln!(
            out,
            "{}{}",
            if row["active"] == true {
                "[当前] "
            } else {
                ""
            },
            text(row, "name")
        )?;
        for (label, key) in [("ID", "id"), ("来源", "source"), ("更新", "updated")] {
            writeln!(out, "  {label}：{}", text(row, key))?;
        }
        if !row["usage"].is_null() {
            writeln!(out, "  用量：{}", text(row, "usage"))?;
        }
    }
    writeln!(out, "共 {} 个订阅", entries.len())?;
    Ok(())
}

pub(super) fn rules(out: &mut dyn Write, value: &Value) -> Result<()> {
    let entries = rows(&value["items"]);
    let cells: Vec<_> = entries
        .iter()
        .map(|row| {
            vec![
                text(row, "position"),
                if row["enabled"] == true {
                    "启用"
                } else {
                    "停用"
                }
                .into(),
                format!(
                    "{}{}",
                    text(row, "source"),
                    if row["duplicate"] == true {
                        "（重复）"
                    } else {
                        ""
                    }
                ),
                text(row, "rule"),
            ]
        })
        .collect();
    table::write(
        out,
        &["序号", "状态", "来源", "规则"],
        &cells,
        table::width(),
    )?;
    table::page(out, value)
}

pub(super) fn connections(out: &mut dyn Write, value: &Value) -> Result<()> {
    let entries: Vec<_> = if value.get("items").is_some() {
        rows(&value["items"]).iter().collect()
    } else {
        vec![value]
    };
    if entries.is_empty() {
        writeln!(out, "未找到匹配的连接")?;
    }
    for (index, row) in entries.iter().enumerate() {
        if index > 0 {
            writeln!(out)?;
        }
        writeln!(out, "{}（{}）", text(row, "name"), text(row, "network"))?;
        field(out, "  ID", &text(row, "id"))?;
        field(
            out,
            "  规则",
            &format!(
                "{}{}",
                text(row, "rule"),
                row["rule_payload"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("（{}）", clean(s)))
                    .unwrap_or_default()
            ),
        )?;
        let chain = rows(&row["chain"])
            .iter()
            .rev()
            .filter_map(Value::as_str)
            .map(clean)
            .collect::<Vec<_>>()
            .join(" → ");
        field(out, "  线路", if chain.is_empty() { "—" } else { &chain })?;
        writeln!(
            out,
            "  速度：上传 {} / 下载 {}",
            text(row, "upload_rate"),
            text(row, "download_rate")
        )?;
        writeln!(
            out,
            "  流量：上传 {} / 下载 {}",
            text(row, "upload"),
            text(row, "download")
        )?;
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
                    field(out, &format!("  {}", super::label(key)), &text(row, key))?;
                }
            }
        }
    }
    if value.get("items").is_some() {
        table::page(out, value)?;
    }
    Ok(())
}
