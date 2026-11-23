use crate::messages;
use chrono::{DateTime, Local, Utc};
use clash_client_core::config::Language;
use serde_json::Value;
use std::borrow::Cow;

/// Escape terminal control characters without changing stored names or JSON.
pub(super) fn clean(text: &str) -> String {
    text.chars().fold(String::new(), |mut out, ch| {
        if ch.is_control() {
            out.extend(ch.escape_default());
        } else {
            out.push(ch);
        }
        out
    })
}

pub(super) fn bytes(value: u64) -> String {
    let mut number = value as f64;
    let mut unit = "B";
    for next in ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"] {
        if number < 1024.0 {
            break;
        }
        number /= 1024.0;
        unit = next;
    }
    if unit == "B" {
        format!("{value} B")
    } else {
        format!("{number:.1} {unit}")
    }
}

pub(super) fn text_in(row: &Value, key: &str, language: Language) -> String {
    scalar_in(key, &row[key], language)
}

pub(super) fn scalar_in(key: &str, value: &Value, language: Language) -> String {
    if key == "latency_ms" {
        return match value.as_u64() {
            Some(0) => messages::output_value_timed_out(language).into(),
            Some(latency) => format!("{latency} ms"),
            None => messages::output_value_not_tested(language).into(),
        };
    }
    match value {
        Value::Null if matches!(key, "upload_rate" | "download_rate") => {
            messages::output_value_not_sampled(language).into()
        }
        Value::Null => "—".into(),
        Value::Bool(value) => match (key, *value) {
            ("running", true) => messages::output_value_running(language),
            ("running", false) => messages::output_value_stopped(language),
            (
                "enabled" | "protected" | "dns_override" | "ipv6" | "udp" | "tray_speed" | "dark",
                true,
            ) => messages::output_value_on(language),
            (
                "enabled" | "protected" | "dns_override" | "ipv6" | "udp" | "tray_speed" | "dark",
                false,
            ) => messages::output_value_off(language),
            (_, true) => messages::output_value_yes(language),
            (_, false) => messages::output_value_no(language),
        }
        .into(),
        Value::Number(number) => {
            let Some(number_as_u64) = number.as_u64() else {
                return number.to_string();
            };
            match key {
                "upload" | "download" | "size" | "upload_total" | "download_total" => {
                    bytes(number_as_u64)
                }
                "upload_rate" | "download_rate" => format!("{}/s", bytes(number_as_u64)),
                "millis" => format!("{number_as_u64} ms"),
                "updated" | "installed_at" if number_as_u64 == 0 => {
                    messages::output_value_not_updated(language).into()
                }
                "updated" | "installed_at" => i64::try_from(number_as_u64)
                    .ok()
                    .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0))
                    .map(|time| {
                        time.with_timezone(&Local)
                            .format("%Y-%m-%d %H:%M:%S")
                            .to_string()
                    })
                    .unwrap_or_else(|| number.to_string()),
                "delay_interval_minutes"
                | "subscription_update_interval_minutes"
                | "update_check_interval_minutes" => match number_as_u64 {
                    0 => messages::output_value_off(language).into(),
                    minutes => messages::output_value_minutes(language, minutes),
                },
                "text_scale" => format!("{number_as_u64}%"),
                _ => number.to_string(),
            }
        }
        Value::String(value) if value.is_empty() => "—".into(),
        Value::String(value) => string_value(key, value, language),
        _ => clean(&value.to_string()),
    }
}

fn string_value(key: &str, value: &str, language: Language) -> String {
    if key == "country" {
        let name = if language == Language::SimplifiedChinese {
            clash_client_core::probe::country_name(value).map(Cow::Owned)
        } else {
            clash_client_core::countries::english_name(value).map(Cow::Borrowed)
        };
        if let Some(name) = name {
            return clean(&messages::output_annotated_value(language, name, value));
        }
    }
    if matches!(key, "start" | "generated_at")
        && let Ok(time) = DateTime::parse_from_rfc3339(value)
    {
        return time
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
    }
    let translated = match (key, value) {
        ("mode" | "run_mode", "rule") => messages::output_column_rule(language),
        ("mode" | "run_mode", "global") => messages::output_value_mode_global(language),
        ("mode" | "run_mode", "direct") => messages::output_builtin_direct(language),
        ("proxy" | "proxy_mode", "off") => messages::output_value_proxy_off(language),
        ("proxy" | "proxy_mode", "system") => messages::output_value_proxy_system(language),
        ("proxy" | "proxy_mode", "tun") => "TUN",
        ("owner", "gui") => messages::output_value_owner_gui(language),
        ("owner", "daemon") => messages::output_value_owner_daemon(language),
        ("source", "Custom") => messages::output_value_source_custom(language),
        ("source", "Shortcut") => messages::output_value_source_shortcut(language),
        ("source", "Subscription") => messages::output_label_profile(language),
        ("source", "system") => messages::output_system_dns(language),
        ("source", "core/proxy") => messages::output_label_core_dns(language),
        ("ip_source", "resolver") => messages::output_value_ip_source_resolver(language),
        ("theme_mode" | "language", "system") => messages::output_value_theme_mode_system(language),
        ("theme_mode", "dark") => messages::output_value_theme_mode_dark(language),
        ("theme_mode", "light") => messages::output_value_theme_mode_light(language),
        ("language", "zh-CN") => messages::output_value_language_zh_cn(language),
        ("language", tag) => Language::from_tag(tag).map_or(tag, |language| language.native_name()),
        ("accent_color", "teal") => messages::output_value_accent_color_teal(language),
        ("accent_color", "blue") => messages::output_value_accent_color_blue(language),
        ("accent_color", "purple") => messages::output_value_accent_color_purple(language),
        ("accent_color", "rose") => messages::output_value_accent_color_rose(language),
        ("accent_color", "amber") => messages::output_value_accent_color_amber(language),
        ("accent_color", "slate") => messages::output_value_accent_color_slate(language),
        ("node_sort" | "tui_node_sort", "name") => messages::output_value_node_sort_name(language),
        ("node_sort" | "tui_node_sort", "latency_ascending") => {
            messages::output_value_node_sort_latency_ascending(language)
        }
        ("node_sort" | "tui_node_sort", "latency_descending") => {
            messages::output_value_node_sort_latency_descending(language)
        }
        ("evidence", "measured") => messages::output_value_evidence_measured(language),
        ("evidence", "inferred") => messages::output_value_evidence_inferred(language),
        ("evidence", "unknown") => messages::output_value_evidence_unknown(language),
        ("environment", "core") => messages::output_value_environment_core(language),
        ("environment", "web") => messages::output_value_environment_web(language),
        ("environment", "desktop") => messages::output_value_environment_desktop(language),
        ("environment" | "group", "code") => messages::output_value_environment_code(language),
        ("group", "exit") => messages::output_value_group_exit(language),
        ("group", "browser") => messages::output_value_group_browser(language),
        ("group", "web-rtc") => messages::output_value_group_web_rtc(language),
        ("group", "dns") => messages::output_value_group_dns(language),
        ("group", "desktop") => messages::output_value_group_desktop(language),
        ("group", "quality") => messages::output_value_group_quality(language),
        ("group", "cloudflare") => messages::output_value_group_cloudflare(language),
        ("group", "connectivity") => messages::output_value_group_connectivity(language),
        (
            "state",
            "Confirmed" | "Partial" | "Reachable" | "Identified" | "Restricted" | "Unknown"
            | "Failed",
        ) => label_in(value, language),
        ("state", "Off") => messages::output_value_state_off(language),
        ("state", "Active") => messages::output_value_state_active(language),
        ("state", "Checking") => messages::output_value_state_checking(language),
        ("state", "Error") => messages::output_value_state_error(language),
        ("level", "issue") => messages::output_value_level_issue(language),
        ("level", "review") => messages::output_value_level_review(language),
        ("level", "incomplete") => messages::output_value_level_incomplete(language),
        ("scope", "current-user-logon") => {
            messages::output_value_scope_current_user_logon(language)
        }
        ("effective", "next-login") => messages::output_value_effective_next_login(language),
        _ => value,
    };
    if matches!(
        key,
        "theme_mode" | "language" | "accent_color" | "node_sort" | "tui_node_sort"
    ) && translated != value
    {
        return clean(&messages::output_annotated_value(
            language, translated, value,
        ));
    }
    clean(translated)
}

pub(super) fn label_in(key: &str, language: Language) -> &str {
    match key {
        "id" => "ID",
        "name" => messages::output_column_name(language),
        "type" => messages::output_column_type(language),
        "running" => messages::output_core(language),
        "owner" => messages::output_label_owner(language),
        "version" => messages::output_label_version(language),
        "profile" => messages::output_label_profile(language),
        "node" => messages::output_label_node(language),
        "mode" | "run_mode" => messages::output_label_mode(language),
        "proxy" | "proxy_mode" => messages::output_label_proxy(language),
        "protected" | "claude_protection" | "protection" => {
            messages::output_claude_protection(language)
        }
        "connections" => messages::output_label_connections(language),
        "upload_rate" => messages::output_label_upload_rate(language),
        "download_rate" => messages::output_label_download_rate(language),
        "upload" => messages::output_label_upload(language),
        "download" => messages::output_label_download(language),
        "source" => messages::output_column_source(language),
        "updated" => messages::output_label_updated(language),
        "active" => messages::output_label_active(language),
        "usage" => messages::output_label_usage(language),
        "position" => messages::output_column_position(language),
        "rule" => messages::output_label_rule(language),
        "rule_payload" => messages::output_label_rule_payload(language),
        "enabled" => messages::output_label_enabled(language),
        "duplicate" => messages::output_label_duplicate(language),
        "already_in_subscription" => messages::output_label_already_in_subscription(language),
        "host" => messages::output_label_host(language),
        "sniff_host" => messages::output_label_sniff_host(language),
        "ip" => messages::output_label_ip(language),
        "port" => messages::output_label_port(language),
        "network" => messages::output_label_network(language),
        "chain" => messages::output_label_chain(language),
        "start" => messages::output_label_start(language),
        "ip_source" => messages::output_label_ip_source(language),
        "resolution_error" => messages::output_label_resolution_error(language),
        "controller_port" => messages::output_label_controller_port(language),
        "mixed_port" => messages::output_label_mixed_port(language),
        "secret" => messages::output_label_secret(language),
        "dark" => messages::output_label_dark(language),
        "theme_mode" => messages::output_label_theme_mode(language),
        "language" => messages::output_label_language(language),
        "accent_color" => messages::output_label_accent_color(language),
        "text_scale" => messages::output_label_text_scale(language),
        "tray_speed" => messages::output_label_tray_speed(language),
        "dns_override" => messages::output_label_dns_override(language),
        "ipv6" => "IPv6",
        "udp" => "UDP",
        "delay_interval_minutes" => messages::output_label_delay_interval_minutes(language),
        "subscription_update_interval_minutes" => {
            messages::output_label_subscription_update_interval_minutes(language)
        }
        "update_check_interval_minutes" => {
            messages::output_label_update_check_interval_minutes(language)
        }
        "active_profile" => messages::output_label_active_profile(language),
        "node_sort" => messages::output_label_node_sort(language),
        "tui_node_sort" => messages::output_label_tui_node_sort(language),
        "rule_overrides" => messages::output_label_rule_overrides(language),
        "rule_profiles" => messages::output_label_rule_profiles(language),
        "application_rules" => messages::output_label_application_rules(language),
        "path" => messages::output_label_path(language),
        "url" => messages::output_label_url(language),
        "size" => messages::output_label_size(language),
        "sha256" => "SHA256",
        "available" => messages::output_label_available(language),
        "package" => messages::output_label_package(language),
        "installed" => messages::output_label_installed(language),
        "matches_current" => messages::output_label_matches_current(language),
        "scope" => messages::output_label_scope(language),
        "effective" => messages::output_label_effective(language),
        "source_profile_id" | "profile_id" => messages::output_label_source_profile_id(language),
        "source_group" => messages::output_label_source_group(language),
        "target" => messages::output_label_target(language),
        "system_dns" => messages::output_system_dns(language),
        "core_dns" => messages::output_label_core_dns(language),
        "answers" => messages::output_label_answers(language),
        "http" => messages::output_label_http(language),
        "ipv4" => messages::output_label_ipv4(language),
        "error" => messages::output_label_error(language),
        "status" => messages::output_label_status(language),
        "millis" => messages::output_column_duration(language),
        "detail" => messages::output_label_detail(language),
        "category" | "group" => messages::output_label_category(language),
        "state" => messages::output_label_state(language),
        "summary" => messages::output_label_summary(language),
        "country" => messages::output_column_region(language),
        "total" => messages::output_label_total(language),
        "completed" => messages::output_label_completed(language),
        "confirmed_available" | "Confirmed" => messages::output_label_confirmed_available(language),
        "counts" => messages::output_label_counts(language),
        "categories" => messages::output_label_categories(language),
        "results" => messages::output_label_results(language),
        "normal" => messages::output_label_normal(language),
        "risk" => messages::output_label_risk(language),
        "challenged" => messages::output_label_challenged(language),
        "clues" => messages::output_label_clues(language),
        "report" => messages::output_label_report(language),
        "terminal" => messages::output_label_terminal(language),
        "projects" => messages::output_label_projects(language),
        "environment" => messages::output_label_environment(language),
        "generated_at" => messages::output_label_generated_at(language),
        "context" => messages::output_label_context(language),
        "signals" => messages::output_label_signals(language),
        "routes" => messages::output_label_routes(language),
        "title" => messages::output_column_item(language),
        "value" => messages::output_column_result(language),
        "evidence" => messages::output_label_evidence(language),
        "advice" => messages::output_label_advice(language),
        "time" => messages::output_label_time(language),
        "outcome" => messages::output_label_outcome(language),
        "level" => messages::output_label_level(language),
        "field" => messages::output_label_field(language),
        "reason" => messages::output_label_reason(language),
        "files" => messages::output_label_files(language),
        "missing_projects" => messages::output_label_missing_projects(language),
        "omitted" => messages::output_label_omitted(language),
        "incomplete" => messages::output_label_incomplete(language),
        "findings" => messages::output_label_findings(language),
        "clean_count" => messages::output_label_clean_count(language),
        "clean_files" => messages::output_label_clean_files(language),
        "issues" => messages::output_label_issues(language),
        "reviews" => messages::output_label_reviews(language),
        "unreadable" => messages::output_label_unreadable(language),
        "guard" => messages::output_label_guard(language),
        "protection_status" => messages::output_label_protection_status(language),
        "observed" => messages::output_label_observed(language),
        "connected" => messages::output_label_connected(language),
        "failures" => messages::output_label_failures(language),
        "normal_closed" => messages::output_label_normal_closed(language),
        "notifications" => messages::output_label_notifications(language),
        "last_error" => messages::output_label_last_error(language),
        "Ok" => messages::output_label_ok(language),
        "Err" => messages::output_label_err(language),
        "Partial" => messages::output_label_partial(language),
        "Reachable" => messages::output_label_reachable(language),
        "Identified" => messages::output_label_identified(language),
        "Restricted" => messages::output_label_restricted(language),
        "Unknown" => messages::output_label_unknown(language),
        "Failed" => messages::output_label_failed(language),
        _ => key,
    }
}
