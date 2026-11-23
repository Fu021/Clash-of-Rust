use crate::{config::Language, messages, network};
use std::borrow::Cow;

type TranslateReason = fn(&str, Language) -> Cow<'_, str>;

pub enum CheckErrorText<'a> {
    RetrySummary(Cow<'a, str>),
    Detail(Cow<'a, str>),
}

impl<'a> CheckErrorText<'a> {
    pub fn into_text(self) -> Cow<'a, str> {
        match self {
            Self::RetrySummary(text) | Self::Detail(text) => text,
        }
    }
}

pub fn localize_check_error(error: &str, language: Language) -> Cow<'_, str> {
    localize_check_error_with(error, language, crate::error_message::localize).into_text()
}

pub fn localize_check_error_with(
    error: &str,
    language: Language,
    translate_reason: TranslateReason,
) -> CheckErrorText<'_> {
    for (prefix, timed_out) in [
        ("检查更新失败（已尝试 ", false),
        ("检查更新超时（已尝试 ", true),
    ] {
        let Some(rest) = error.strip_prefix(prefix) else {
            continue;
        };
        let Some((attempts, detail)) = rest.split_once(" 次）") else {
            return CheckErrorText::Detail(error.into());
        };
        let Ok(count) = attempts.parse::<usize>() else {
            return CheckErrorText::Detail(error.into());
        };
        let detail = if detail.is_empty() {
            None
        } else if let Some(detail) = detail.strip_prefix('：') {
            Some(detail)
        } else {
            return CheckErrorText::Detail(error.into());
        };
        if language == Language::SimplifiedChinese {
            return CheckErrorText::RetrySummary(error.into());
        }
        let summary = match (timed_out, count) {
            (false, 1) => messages::update_check_failed_once(language).into(),
            (false, _) => messages::update_check_failed_after(language, count),
            (true, 1) => messages::update_check_timed_out_once(language).into(),
            (true, _) => messages::update_check_timed_out_after(language, count),
        };
        let text = match detail {
            Some(detail) => messages::update_check_with_detail(
                language,
                check_detail(detail, language, translate_reason),
                summary,
            ),
            None => summary,
        };
        return CheckErrorText::RetrySummary(text.into());
    }
    CheckErrorText::Detail(if language == Language::SimplifiedChinese {
        error.into()
    } else {
        check_detail(error, language, translate_reason)
    })
}

fn check_detail(
    error: &str,
    language: Language,
    translate_reason: TranslateReason,
) -> Cow<'_, str> {
    if let Some(body) = error.strip_suffix("（已尝试可用代理和直连）") {
        let prerelease = body.strip_prefix("暂时无法获取预发布版本列表：");
        let mut details = String::new();
        for (index, detail) in prerelease.unwrap_or(body).split('；').enumerate() {
            let translated = check_route(detail, language, translate_reason);
            if index > 0 {
                details.push_str(if translated == detail {
                    "；"
                } else {
                    messages::download_route_error_separator(language)
                });
            }
            details.push_str(&translated);
        }
        let details = messages::download_route_error_tried_all(language, details);
        return if prerelease.is_some() {
            messages::update_check_prerelease_failed(language, details).into()
        } else {
            details.into()
        };
    }
    check_route(error, language, translate_reason)
}

fn check_route(error: &str, language: Language, translate_reason: TranslateReason) -> Cow<'_, str> {
    if let Some((route, detail)) = error.split_once('：')
        && let Some(label) = network::localized_route_label(route, language)
    {
        return messages::download_route_error(
            language,
            check_reason(detail, language, translate_reason),
            label,
        )
        .into();
    }
    check_reason(error, language, translate_reason)
}

fn check_reason(
    error: &str,
    language: Language,
    translate_reason: TranslateReason,
) -> Cow<'_, str> {
    for (prefix, label) in [
        ("GitHub API", "GitHub API"),
        (
            "Release 订阅",
            messages::update_check_release_feed(language),
        ),
        ("安装包信息", messages::update_check_package_info(language)),
        ("下载", messages::update_check_download(language)),
    ] {
        if let Some(reason) = error.strip_prefix(prefix) {
            let reason = match reason {
                "连接超时" => Some(messages::update_check_connection_timeout(language)),
                "证书校验失败，请检查系统时间或代理证书" => {
                    Some(messages::update_check_certificate_failed(language))
                }
                "域名解析失败" => Some(messages::update_check_dns_failed(language)),
                "代理连接失败" => Some(messages::update_check_proxy_failed(language)),
                "网络连接失败" => Some(messages::update_check_network_failed(language)),
                _ => None,
            };
            if let Some(reason) = reason {
                return messages::update_check_target_reason(language, reason, label).into();
            }
        }
    }
    match error {
        "GitHub 暂时限制请求，请稍后再试" => {
            return messages::update_check_rate_limited(language).into();
        }
        "Release 信息格式无效" => {
            return messages::update_check_invalid_release_information(language).into();
        }
        "Release 订阅格式无效" => {
            return messages::update_check_invalid_release_feed(language).into();
        }
        _ => {}
    }
    if let Some(status) = error.strip_prefix("GitHub 返回 HTTP ") {
        return messages::update_check_github_status(language, status).into();
    }
    translate_reason(error, language)
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
