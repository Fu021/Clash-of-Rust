// SPDX-License-Identifier: GPL-3.0-only
use super::*;
use crate::ip_report::NATIVE_REGION_DETAIL_PREFIX;
fn strip_ansi(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut output = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for part in chars.by_ref() {
                if ('@'..='~').contains(&part) {
                    break;
                }
            }
        } else if ch == '\r' || ch == '\t' {
            output.push(' ');
        } else if !ch.is_control() || ch == '\n' {
            output.push(ch);
        }
    }
    output
}

pub(super) fn parse_output(text: &str) -> CheckResult {
    let text = strip_ansi(text);
    let line = text
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("");
    // Platform labels can contain a colon (J:COM). Separator is followed by whitespace.
    let value = line
        .char_indices()
        .find_map(|(at, ch)| {
            (ch == ':' && line[at + 1..].starts_with(char::is_whitespace))
                .then(|| line[at + 1..].trim())
        })
        .unwrap_or(line);
    let lower = value.to_ascii_lowercase();
    let (state, summary) = if crate::countries::country_code(value.trim()).is_some() {
        (State::Identified, "已识别地区")
    } else if lower.starts_with("failed") {
        (State::Failed, "检测失败")
    } else if lower.contains("unexpected") || lower.is_empty() {
        (State::Unknown, "检测失败或结果未确认")
    } else if lower.starts_with("web reachable") {
        (State::Reachable, "网页可达")
    } else if lower.starts_with("web only") {
        (State::Partial, "仅网页可用")
    } else if lower.starts_with("originals only") || lower.starts_with("original only") {
        (State::Partial, "仅自制内容")
    } else if lower.starts_with("yes") {
        (State::Confirmed, "可用")
    } else if lower.starts_with("no") && lower.contains("unsupported region") {
        (State::Restricted, "地区不支持")
    } else if lower.starts_with("no") && lower.contains("disallowed isp") {
        (State::Restricted, "出口网络受限")
    } else if lower.starts_with("no") && lower.contains("blocked") {
        (State::Unknown, "请求被拦截，未确认")
    } else if lower.starts_with("no") || lower.starts_with("blocked") {
        (State::Restricted, "不可用或受限")
    } else {
        (State::Unknown, "已返回检测信息")
    };
    let country = value
        .split_once("Region:")
        .and_then(|(_, tail)| tail.split(')').next().map(str::trim))
        .and_then(|name| {
            crate::countries::country_code(name)
                .or_else(|| crate::countries::country_code(&name.to_ascii_uppercase()))
        })
        .or_else(|| crate::countries::country_code(value.trim()))
        .and_then(country_name)
        .map(decorate_country)
        .unwrap_or_else(|| "未提供".into());
    CheckResult {
        state,
        summary: summary.into(),
        country,
        millis: None,
        authentication_check_skipped: false,
        detail: format!("{NATIVE_REGION_DETAIL_PREFIX}{value}"),
        target_exit: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_semantics_keep_region_partial_access_and_failures_distinct() {
        use crate::{
            config::Language,
            ip_report::{self, Status},
        };

        let service = Service {
            id: "fixture".into(),
            name: "Fixture".into(),
            group: "Fixture".into(),
            url: "https://fixture.example".into(),
        };
        for (evidence, state, english_summary) in [
            ("JP", State::Identified, "Region identified"),
            ("Yes (Region: JP)", State::Confirmed, "Available"),
            ("Originals Only", State::Partial, "Originals only"),
            ("Web Only", State::Partial, "Website only"),
            ("Web Reachable", State::Reachable, "Website reachable"),
            (
                "No (Unsupported Region)",
                State::Restricted,
                "Unsupported region",
            ),
            (
                "No (Disallowed ISP)",
                State::Restricted,
                "Exit network restricted",
            ),
            (
                "No (Blocked)",
                State::Unknown,
                "Request blocked · Unconfirmed",
            ),
            ("No", State::Restricted, "Unavailable or restricted"),
            ("Failed (Network Connection)", State::Failed, "Check failed"),
            (
                "Unexpected response",
                State::Unknown,
                "Check failed or result unconfirmed",
            ),
            ("上游原始结果", State::Unknown, "Check information returned"),
        ] {
            let result = parse_output(&format!("Platform: {evidence}"));
            assert_eq!(result.state, state, "{evidence}");
            let outcome = Ok(result);
            let result = outcome.as_ref().unwrap();
            let status = Status::of(Some(&outcome));
            assert_eq!(
                status.summary_in(&result.summary, Language::English),
                english_summary
            );
            assert_eq!(
                status.summary_in(&result.summary, Language::SimplifiedChinese),
                result.summary
            );
            let raw_detail = format!("原生地区检测结果：{evidence}");
            assert_eq!(
                ip_report::detail_in(&service, &outcome, Language::English),
                format!("Native region check result: {evidence}")
            );
            assert_eq!(
                ip_report::detail_in(&service, &outcome, Language::SimplifiedChinese),
                raw_detail
            );
            let json = crate::support::ip_result(result);
            assert_eq!(json["summary"], result.summary);
            assert_eq!(json["detail"], raw_detail);
        }
    }
}
