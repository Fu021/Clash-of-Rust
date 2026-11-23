use crate::config::Language;
use std::borrow::Cow;

type TranslateReason = fn(&str) -> Cow<'_, str>;

pub fn localize_check_error(error: &str, language: Language) -> Cow<'_, str> {
    localize_check_error_with(error, language, |reason| {
        crate::error_message::localize(reason, Language::English)
    })
}

pub fn localize_check_error_with(
    error: &str,
    language: Language,
    translate_reason: TranslateReason,
) -> Cow<'_, str> {
    if language == Language::SimplifiedChinese {
        return error.into();
    }
    for (prefix, summary) in [
        ("检查更新失败（已尝试 ", "Update check failed"),
        ("检查更新超时（已尝试 ", "Update check timed out"),
    ] {
        let Some(rest) = error.strip_prefix(prefix) else {
            continue;
        };
        let Some((attempts, detail)) = rest.split_once(" 次）") else {
            return error.into();
        };
        let Ok(count) = attempts.parse::<usize>() else {
            return error.into();
        };
        let detail = if detail.is_empty() {
            String::new()
        } else if let Some(detail) = detail.strip_prefix('：') {
            format!(": {}", check_detail(detail, translate_reason))
        } else {
            return error.into();
        };
        let unit = if count == 1 { "attempt" } else { "attempts" };
        return format!("{summary} after {count} {unit}{detail}").into();
    }
    check_detail(error, translate_reason)
}

fn check_detail(error: &str, translate_reason: TranslateReason) -> Cow<'_, str> {
    if let Some(body) = error.strip_suffix("（已尝试可用代理和直连）") {
        let (prefix, body) = match body.strip_prefix("暂时无法获取预发布版本列表：") {
            Some(body) => ("Could not get the pre-release list: ", body),
            None => ("", body),
        };
        let mut details = String::new();
        for (index, detail) in body.split('；').enumerate() {
            let translated = check_route(detail, translate_reason);
            if index > 0 {
                details.push_str(if translated == detail { "；" } else { "; " });
            }
            details.push_str(&translated);
        }
        return format!("{prefix}{details} (tried available proxies and direct connections)")
            .into();
    }
    check_route(error, translate_reason)
}

fn check_route(error: &str, translate_reason: TranslateReason) -> Cow<'_, str> {
    for (prefix, label) in [
        ("内核代理：", "Core proxy"),
        ("系统代理：", "System proxy"),
        ("内核直连：", "Core direct"),
        ("系统网络：", "System network"),
        ("直连：", "Direct"),
    ] {
        if let Some(detail) = error.strip_prefix(prefix) {
            return format!("{label}: {}", check_reason(detail, translate_reason)).into();
        }
    }
    check_reason(error, translate_reason)
}

fn check_reason(error: &str, translate_reason: TranslateReason) -> Cow<'_, str> {
    for (prefix, label) in [
        ("GitHub API", "GitHub API"),
        ("Release 订阅", "Release feed"),
        ("安装包信息", "Package information"),
        ("下载", "Download"),
    ] {
        if let Some(reason) = error.strip_prefix(prefix) {
            let reason = match reason {
                "连接超时" => Some("connection timed out"),
                "证书校验失败，请检查系统时间或代理证书" => Some(
                    "certificate verification failed; check the system clock or proxy certificate",
                ),
                "域名解析失败" => Some("DNS resolution failed"),
                "代理连接失败" => Some("proxy connection failed"),
                "网络连接失败" => Some("network connection failed"),
                _ => None,
            };
            if let Some(reason) = reason {
                return format!("{label}: {reason}").into();
            }
        }
    }
    match error {
        "GitHub 暂时限制请求，请稍后再试" => {
            return "GitHub is temporarily rate limiting requests; retry later".into();
        }
        "Release 信息格式无效" => return "Release information is invalid".into(),
        "Release 订阅格式无效" => return "Release feed is invalid".into(),
        _ => {}
    }
    if let Some(status) = error.strip_prefix("GitHub 返回 HTTP ") {
        return format!("GitHub returned HTTP {status}").into();
    }
    translate_reason(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_update_checks_translate_retries_and_keep_provider_details() {
        let reason = "检查更新失败（已尝试 4 次）：系统网络：更新校验文件格式无效: 提供方原文";
        assert_eq!(
            localize_check_error(reason, Language::English),
            "Update check failed after 4 attempts: System network: The update checksum file is invalid: 提供方原文"
        );
        assert_eq!(
            localize_check_error(reason, Language::SimplifiedChinese),
            reason
        );
        assert_eq!(
            localize_check_error("检查更新超时（已尝试 1 次）", Language::English),
            "Update check timed out after 1 attempt"
        );
    }
}
