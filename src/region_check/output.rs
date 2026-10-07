// SPDX-License-Identifier: GPL-3.0-only
use super::*;
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
    let (state, summary) = if crate::flags::country_code(value.trim()).is_some() {
        (State::Confirmed, "已识别地区")
    } else if lower.starts_with("failed") || lower.contains("unexpected") || lower.is_empty() {
        (State::Unknown, "检测失败或结果未确认")
    } else if lower.starts_with("web reachable") {
        (State::Reachable, "网页可达")
    } else if lower.starts_with("web only") {
        (State::Reachable, "仅网页可用")
    } else if lower.starts_with("originals only") || lower.starts_with("original only") {
        (State::Reachable, "仅自制内容")
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
            crate::flags::country_code(name)
                .or_else(|| crate::flags::country_code(&name.to_ascii_uppercase()))
        })
        .or_else(|| crate::flags::country_code(value.trim()))
        .and_then(country_name)
        .map(decorate_country)
        .unwrap_or_else(|| "未提供".into());
    CheckResult {
        state,
        summary: summary.into(),
        country,
        millis: 0,
        detail: format!("原生地区检测结果：{value}"),
    }
}
