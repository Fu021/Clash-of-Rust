use super::{
    field_in,
    fields::{clean, scalar_in, text_in},
    rows, table,
};
use crate::args::Args;
use crate::messages;
use anyhow::Result;
use clash_client_core::{
    config::Language,
    ip_report::{self, Counts, Status},
    subscription,
};
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
        "Selector" => messages::output_group_selector(language),
        "URLTest" => messages::output_group_url_test(language),
        "Fallback" => messages::output_group_fallback(language),
        "LoadBalance" => messages::output_group_load_balance(language),
        "Relay" => messages::output_group_relay(language),
        other => other,
    })
}

pub(super) fn ip_results(out: &mut dyn Write, value: &Value, language: Language) -> Result<()> {
    let results = rows(&value["results"]);
    let mut counts = Counts::default();
    for row in results {
        let status = match row["state"].as_str() {
            Some("Confirmed") => Status::Available,
            Some("Partial") => Status::Partial,
            Some("Reachable") => Status::Reachable,
            Some("Identified") => Status::Identified,
            Some("Restricted") => Status::Restricted,
            Some("Failed") => Status::Failed,
            _ => Status::Unknown,
        };
        counts.0[status as usize] += 1;
    }
    let heading = messages::output_ip_check_complete(language, counts.total());
    let summary = counts.summary_in(language);
    if summary.is_empty() {
        writeln!(out, "{heading}")?;
    } else {
        writeln!(out, "{heading} · {summary}")?;
    }
    let cells = results
        .iter()
        .map(|row| {
            let skipped = row["authentication_check_skipped"].as_bool() == Some(true);
            let summary = if skipped {
                ip_report::authentication_check_skipped_in(language).into()
            } else if row["state"] == "Failed"
                && row["summary"].as_str().is_none_or(str::is_empty)
                && let Some(error) = row["detail"].as_str()
            {
                clean(&clash_client_core::error_message::localize(error, language))
            } else if row["summary"].as_str().is_none_or(str::is_empty) {
                text_in(row, "detail", language)
            } else {
                text_in(row, "summary", language)
            };
            let duration = if skipped {
                ip_report::no_response_measurement_in(language).into()
            } else {
                text_in(row, "millis", language)
            };
            let country = text_in(row, "country", language);
            vec![
                text_in(row, "name", language),
                text_in(row, "state", language),
                summary,
                ip_report::country_in(&country, skipped, language).into(),
                duration,
            ]
        })
        .collect::<Vec<_>>();
    table::write_in(
        out,
        &[
            messages::output_column_item(language),
            messages::output_column_status(language),
            messages::output_column_result(language),
            messages::output_column_region(language),
            messages::output_column_duration(language),
        ],
        &cells,
        table::width(),
        language,
    )?;
    let exits = results
        .iter()
        .filter(|row| row.get("target_exit").is_some())
        .collect::<Vec<_>>();
    if !exits.is_empty() {
        writeln!(out, "\n{}", messages::output_ip_service_exits(language))?;
        for row in exits {
            let exit = &row["target_exit"];
            let address = exit["ip"]
                .as_str()
                .unwrap_or_else(|| messages::output_unconfirmed(language));
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
        writeln!(out, "{}\n", messages::output_nodes_offline(language))?;
    }
    let entries = rows(&value["items"]);
    if let Some(name) = args.positional.first() {
        field_in(
            out,
            messages::output_node_group(language),
            &clean(name),
            language,
        )?;
        writeln!(out)?;
    }
    if entries.is_empty() {
        writeln!(out, "{}", messages::output_nodes_no_match(language))?;
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
                        "DIRECT" => messages::output_builtin_direct(language),
                        "REJECT" => messages::output_builtin_reject(language),
                        "REJECT-DROP" => messages::output_builtin_drop(language),
                        _ => messages::output_builtin_action(language),
                    }
                    .into(),
                ),
            }
            if selected_column {
                line.push(
                    if row["selected"] == true {
                        messages::output_current(language)
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
            "groups" => messages::output_node_groups(language),
            "nodes" => messages::output_nodes_heading(language),
            _ => messages::output_builtin_routes(language),
        };
        writeln!(
            out,
            "{}",
            messages::output_with_detail(language, cells.len(), heading)
        )?;
        let mut headers = match section {
            "groups" => vec![
                messages::output_column_name(language),
                messages::output_column_type(language),
                messages::output_column_selected(language),
                messages::output_column_members(language),
            ],
            "nodes" => vec![
                messages::output_column_name(language),
                messages::output_column_protocol(language),
                messages::output_column_latency(language),
            ],
            _ => vec![
                messages::output_column_name(language),
                messages::output_column_action(language),
            ],
        };
        if selected_column {
            headers.push(messages::output_column_status(language));
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
                messages::output_column_node_or_group(language),
                messages::output_column_latency(language),
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
        writeln!(
            out,
            "{}",
            messages::output_delay_summary(language, available, failed)
        )?;
        if (args.flag("all") || args.get("group").is_some())
            && !map.is_empty()
            && map.values().all(|delay| delay.as_u64() == Some(0))
        {
            writeln!(out, "{}", messages::output_delay_all_failed(language))?;
            writeln!(out, "{}", messages::output_status_and_proxy_hint(language))?;
            writeln!(out, "{}", messages::output_export_diagnosis_hint(language))?;
        }
    }
    Ok(())
}

pub(super) fn profiles(out: &mut dyn Write, value: &Value, language: Language) -> Result<()> {
    let entries = rows(value);
    if entries.is_empty() {
        writeln!(out, "{}", messages::output_profiles_no_match(language))?;
    }
    for (index, row) in entries.iter().enumerate() {
        if index > 0 {
            writeln!(out)?;
        }
        writeln!(
            out,
            "{}{}",
            if row["active"] == true {
                messages::output_profile_current_prefix(language)
            } else {
                ""
            },
            text_in(row, "name", language)
        )?;
        if let Some(owner) = row["owner"].as_str() {
            let owner = clean(owner);
            let uid = text_in(row, "uid", language);
            writeln!(
                out,
                "  {}",
                messages::output_profile_owner(language, owner, uid)
            )?;
            field_in(
                out,
                &format!("  {}", messages::output_profile_data_directory(language)),
                &text_in(row, "data_dir", language),
                language,
            )?;
        }
        for (label, key) in [
            ("ID", "id"),
            (messages::output_column_source(language), "source"),
            (messages::output_profile_updated(language), "updated"),
        ] {
            field_in(
                out,
                &format!("  {label}"),
                &text_in(row, key, language),
                language,
            )?;
        }
        if row["show_usage"] == true {
            writeln!(
                out,
                "  {}",
                subscription::summary(row["usage"].as_str(), 0, language)
            )?;
        }
    }
    writeln!(
        out,
        "{}",
        messages::output_profile_total(language, entries.len())
    )?;
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
                    messages::output_rule_enabled(language)
                } else {
                    messages::output_rule_disabled(language)
                }
                .into(),
                if row["duplicate"] == true {
                    messages::output_rule_duplicate_source(
                        language,
                        text_in(row, "source", language),
                    )
                } else {
                    text_in(row, "source", language)
                },
                text_in(row, "rule", language),
            ]
        })
        .collect();
    table::write_in(
        out,
        &[
            messages::output_column_position(language),
            messages::output_column_status(language),
            messages::output_column_source(language),
            messages::output_column_rule(language),
        ],
        &cells,
        table::width(),
        language,
    )?;
    table::page_in(out, value, language)
}

pub(super) fn connections(out: &mut dyn Write, value: &Value, language: Language) -> Result<()> {
    if value["offline"] == true {
        writeln!(out, "{}", messages::output_connections_offline(language))?;
        return table::page_in(out, value, language);
    }
    let entries: Vec<_> = if value.get("items").is_some() {
        rows(&value["items"]).iter().collect()
    } else {
        vec![value]
    };
    if entries.is_empty() {
        writeln!(out, "{}", messages::output_connections_no_match(language))?;
    }
    for (index, row) in entries.iter().enumerate() {
        if index > 0 {
            writeln!(out)?;
        }
        writeln!(
            out,
            "{}",
            messages::output_with_detail(
                language,
                text_in(row, "network", language),
                text_in(row, "name", language)
            )
        )?;
        field_in(out, "  ID", &text_in(row, "id", language), language)?;
        let rule = text_in(row, "rule", language);
        let rule = if let Some(payload) = row["rule_payload"]
            .as_str()
            .filter(|payload| !payload.is_empty())
        {
            messages::output_with_detail(language, clean(payload), rule)
        } else {
            rule
        };
        field_in(
            out,
            &format!("  {}", messages::output_column_rule(language)),
            &rule,
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
            &format!("  {}", messages::output_connection_route(language)),
            if chain.is_empty() { "—" } else { &chain },
            language,
        )?;
        let upload_rate = text_in(row, "upload_rate", language);
        let download_rate = text_in(row, "download_rate", language);
        let upload = text_in(row, "upload", language);
        let download = text_in(row, "download", language);
        writeln!(
            out,
            "  {}",
            messages::output_connection_speed(language, download_rate, upload_rate)
        )?;
        writeln!(
            out,
            "  {}",
            messages::output_connection_traffic(language, download, upload)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ip_results_summarize_each_status_independently() {
        let report = serde_json::json!({"total": 8, "confirmed_available": 1, "non_red": 6, "results": [
            {"state":"Confirmed"},
            {"state":"Partial"},
            {"state":"Reachable"},
            {"state":"Identified"},
            {"state":"Restricted"},
            {"state":"Unknown"},
            {"state":"Unknown","authentication_check_skipped":true},
            {"state":"Failed"}
        ]});
        for (language, expected) in [
            (
                Language::SimplifiedChinese,
                "IP 检测完成 · 共 8 项 · 确认可用 1 · 部分可用 1 · 仅网页可达 1 · 信息已识别 1 · 不可用或受限 1 · 结果未确认 2 · 超时或失败 1",
            ),
            (
                Language::English,
                "IP check complete · Total 8 · Available 1 · Partially available 1 · Website reachable 1 · Identified 1 · Unavailable or restricted 1 · Unconfirmed 2 · Timed out or failed 1",
            ),
        ] {
            let mut output = Vec::new();
            ip_results(&mut output, &report, language).unwrap();
            let output = String::from_utf8(output).unwrap();
            assert_eq!(output.lines().next(), Some(expected));
        }
    }

    #[test]
    fn ip_results_distinguish_skipped_authentication_from_legacy_durations() {
        let report = serde_json::json!({"total": 3, "confirmed_available": 0, "non_red": 2, "results": [
            {"name":"Skipped","state":"Unknown","summary":"opaque summary",
             "country":"未提供","detail":"opaque detail","millis":null,"authentication_check_skipped":true},
            {"name":"Legacy zero","state":"Unknown","summary":"未执行认证检测","country":"未提供","millis":0},
            {"name":"Legacy failure","state":"Failed","detail":"fixture error","millis":null}
        ]});
        for (language, duration, reason, summary) in [
            (
                Language::SimplifiedChinese,
                "无响应测量",
                "未执行认证检测",
                "IP 检测完成 · 共 3 项 · 结果未确认 2 · 超时或失败 1",
            ),
            (
                Language::English,
                "No response measurement",
                "Authentication check was not run",
                "IP check complete · Total 3 · Unconfirmed 2 · Timed out or failed 1",
            ),
        ] {
            let mut output = Vec::new();
            ip_results(&mut output, &report, language).unwrap();
            let output = String::from_utf8(output).unwrap();
            assert_eq!(output.lines().next(), Some(summary));
            assert_eq!(output.matches(duration).count(), 1, "{output}");
            assert!(output.contains(reason), "{output}");
            assert_eq!(output.matches("0 ms").count(), 1, "{output}");
            assert!(output.contains("fixture error"), "{output}");
            assert!(!output.contains("opaque summary"), "{output}");
            if language == Language::English {
                assert_eq!(output.matches("Not provided").count(), 1, "{output}");
                assert_eq!(output.matches("未提供").count(), 1, "{output}");
            } else {
                assert_eq!(output.matches("未提供").count(), 2, "{output}");
            }
        }
        assert_eq!(report["non_red"], 2);
    }

    #[test]
    fn ip_results_translate_exit_failures_and_keep_raw_evidence() {
        let original = "出口 IP 查询未返回有效结果（HTTP 503）";
        let report = serde_json::json!({"results": [
            {"id":"exit-ip","name":"Exit IP","state":"Failed","detail":original},
            {"id":"example","name":"Example","state":"Failed","detail":"提供方原文"}
        ]});
        for (language, expected) in [
            (Language::SimplifiedChinese, original),
            (
                Language::English,
                "Exit IP lookup returned no valid result (HTTP 503)",
            ),
        ] {
            let mut output = Vec::new();
            ip_results(&mut output, &report, language).unwrap();
            let output = String::from_utf8(output).unwrap();
            assert!(output.contains(expected), "{output}");
            assert!(output.contains("提供方原文"), "{output}");
        }
        assert_eq!(report["results"][0]["detail"], original);
    }
}
