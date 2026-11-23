use crate::backend::{RUNNING_BACKEND_UNREACHABLE, RUNNING_BACKEND_UNREACHABLE_EN};
use clash_client_core::{config::Language, error_message::localize_diagnostic};
use std::borrow::Cow;

pub(super) fn error_in<'a>(field: &str, error: &'a str, language: Language) -> Cow<'a, str> {
    if language != Language::English {
        return error.into();
    }
    match field {
        "config_error" => match error.strip_prefix("无法读取 CLI 登录自启配置：") {
            Some(detail) => {
                format!("Could not read the CLI login startup configuration: {detail}").into()
            }
            None => error.into(),
        },
        "backend_error" if error == "查询后台超过 3 秒" => {
            "Backend query timed out after 3 seconds".into()
        }
        "backend_error" => {
            if let Some((RUNNING_BACKEND_UNREACHABLE, detail)) = error.split_once(": ") {
                return format!("{RUNNING_BACKEND_UNREACHABLE_EN}: {detail}").into();
            }
            localize_diagnostic(error, language)
        }
        "service_error" => {
            if let Some(cause) = error.strip_prefix("无法确认 launchd 服务是否已加载: ")
            {
                let cause = service_query_error(cause);
                return format!("Could not confirm whether the launchd service is loaded: {cause}")
                    .into();
            }
            service_query_error(error)
        }
        _ => error.into(),
    }
}

fn service_query_error(error: &str) -> Cow<'_, str> {
    if let Some(stderr) = error.strip_prefix("登录服务查询失败：") {
        return format!("Login service query failed: {stderr}").into();
    }
    let (summary, detail) = error
        .split_once(": ")
        .map_or((error, None), |(summary, detail)| (summary, Some(detail)));
    let translated = match summary {
        "查询登录服务超过 5 秒" => "Login service query timed out after 5 seconds",
        "无法查询当前用户的登录服务" => {
            "Could not query the current user's login service"
        }
        "登录服务查询缺少状态字段" => {
            "Login service query did not report a required status field"
        }
        "服务进程号无效" => "Invalid service process ID",
        "launchd 查询缺少运行状态" => "launchd query did not report a runtime state",
        "launchd 进程号无效" => "Invalid launchd process ID",
        _ => return error.into(),
    };
    match detail {
        Some(detail) => format!("{translated}: {detail}").into(),
        None => translated.into(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    fn render(value: &Value, options: &[&str]) -> String {
        let args = crate::args::parse(
            ["service", "status"]
                .into_iter()
                .chain(options.iter().copied())
                .map(str::to_owned),
        )
        .unwrap();
        let mut output = Vec::new();
        crate::output::write_output(&mut output, &args, value).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn service_timeouts_translate_only_the_human_output() {
        let value = json!({
            "service_error": "查询登录服务超过 5 秒: deadline has elapsed",
            "backend_error": "查询后台超过 3 秒",
            "config_error": "外部错误：保持原文: 查询登录服务超过 5 秒"
        });
        let english = render(&value, &["--lang=en"]);
        for expected in [
            "Query failed: Login service query timed out after 5 seconds: deadline has elapsed",
            "Query failed: Backend query timed out after 3 seconds",
            "Query failed: 外部错误：保持原文: 查询登录服务超过 5 秒",
        ] {
            assert!(english.contains(expected), "{english}");
        }
        let chinese = render(&value, &["--lang=zh-CN"]);
        for field in ["service_error", "backend_error", "config_error"] {
            assert!(
                chinese.contains(value[field].as_str().unwrap()),
                "{chinese}"
            );
        }
        for flag in ["--json", "--jsonl"] {
            for language in ["--lang=en", "--lang=zh-CN"] {
                let machine: Value =
                    serde_json::from_str(&render(&value, &[flag, language])).unwrap();
                assert_eq!(machine["command"], "service status");
                assert_eq!(machine["data"], value);
            }
        }
    }

    #[test]
    fn launchd_timeout_keeps_the_context_and_does_not_translate_the_cause() {
        let value = json!({
            "service_error": "无法确认 launchd 服务是否已加载: 查询登录服务超过 5 秒: deadline has elapsed",
            "backend_error": "第三方错误: 查询后台超过 3 秒",
        });
        let english = render(&value, &["--lang=en"]);
        assert!(
            english.contains(
                "Could not confirm whether the launchd service is loaded: Login service query timed out after 5 seconds: deadline has elapsed"
            ),
            "{english}"
        );
        assert!(
            english.contains("第三方错误: 查询后台超过 3 秒"),
            "{english}"
        );
    }

    #[test]
    fn service_query_failures_translate_known_contexts_and_keep_native_details() {
        for (error, expected) in [
            (
                "无法查询当前用户的登录服务: No such file or directory",
                "Could not query the current user's login service: No such file or directory",
            ),
            (
                "登录服务查询缺少状态字段",
                "Login service query did not report a required status field",
            ),
            (
                "服务进程号无效: invalid digit",
                "Invalid service process ID: invalid digit",
            ),
            (
                "launchd 查询缺少运行状态",
                "launchd query did not report a runtime state",
            ),
            (
                "launchd 进程号无效: invalid digit",
                "Invalid launchd process ID: invalid digit",
            ),
            (
                "无法确认 launchd 服务是否已加载: 登录服务查询失败：查询登录服务超过 5 秒: 系统原文",
                "Could not confirm whether the launchd service is loaded: Login service query failed: 查询登录服务超过 5 秒: 系统原文",
            ),
            (
                "无法确认 launchd 服务是否已加载: unrecognized native error",
                "Could not confirm whether the launchd service is loaded: unrecognized native error",
            ),
        ] {
            let value = json!({"service_error": error});
            let english = render(&value, &["--lang=en"]);
            assert!(english.contains(expected), "{english}");
        }
    }
}
