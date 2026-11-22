use clash_mobile_runtime::core::{config::Language, error_message};

pub(crate) fn failure(error: &str, detail: Option<&str>, language: Language) -> String {
    let error = error_message::localize(error, language);
    let mut message = format!(
        "{}{error}\n{}",
        language.text("应用初始化失败：", "App initialization failed: "),
        language.text("请检查问题后重试。", "Check the problem and retry.")
    );
    if let Some(detail) = detail {
        message.push_str("\n\n");
        message.push_str(&error_message::localize(detail, language));
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_errors_follow_language_and_preserve_external_details() {
        let error = "设置文件格式错误：/private/settings.json: unexpected token";
        let english = failure(error, Some("provider rejected 東京"), Language::English);
        assert!(english.starts_with("App initialization failed: Invalid settings file: "));
        assert!(english.contains("/private/settings.json: unexpected token"));
        assert!(english.ends_with("provider rejected 東京"));
        let chinese = failure(error, None, Language::SimplifiedChinese);
        assert!(chinese.starts_with("应用初始化失败：设置文件格式错误："));
        assert!(chinese.ends_with("请检查问题后重试。"));
    }
}
