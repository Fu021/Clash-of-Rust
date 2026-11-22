//! Read the actual source level, not words such as "error" inside a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
impl Level {
    pub(super) fn parse(value: &str) -> Option<Self> {
        [
            Self::Trace,
            Self::Debug,
            Self::Info,
            Self::Warn,
            Self::Error,
        ]
        .into_iter()
        .find(|level| value.eq_ignore_ascii_case(level.label()))
        .or_else(|| {
            if value.eq_ignore_ascii_case("warning") {
                Some(Self::Warn)
            } else if value.eq_ignore_ascii_case("fatal") || value.eq_ignore_ascii_case("panic") {
                Some(Self::Error)
            } else {
                None
            }
        })
    }
    pub(super) fn of(line: &str) -> Self {
        if line.starts_with("[客户端] 操作失败：") {
            return Self::Error;
        }
        let source = line.strip_prefix("[内核] ").unwrap_or(line);
        if let Some((tag, _)) = source
            .strip_prefix('[')
            .and_then(|body| body.split_once(']'))
        {
            return Self::parse(tag).unwrap_or(Self::Info);
        }
        if source.starts_with("time=") || source.starts_with("level=") {
            return source
                .split_ascii_whitespace()
                .take_while(|field| !field.starts_with("msg="))
                .filter_map(|field| field.strip_prefix("level="))
                .find_map(|level| Self::parse(level.trim_matches('"')))
                .unwrap_or(Self::Info);
        }
        Self::Info
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
    pub(super) fn tone(self) -> i32 {
        match self {
            Self::Error => 3,
            Self::Warn => 2,
            _ => 4,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_level_is_authoritative_even_when_payload_contains_other_levels() {
        for (line, level) in [
            ("[warning] dial failed: level=error", Level::Warn),
            ("[info] error counter reset", Level::Info),
            ("[debug] test", Level::Debug),
            ("[内核] [warn] 连接失败 level=error", Level::Warn),
            ("[info] [客户端] 操作失败：level=error", Level::Info),
        ] {
            assert_eq!(Level::of(line), level, "{line}");
        }
    }

    #[test]
    fn console_log_level_comes_from_the_header_with_or_without_a_core_prefix() {
        for (line, level) in [
            (
                r#"time="2026-10-16T00:00:00Z" level=warning msg="连接失败 level=error""#,
                Level::Warn,
            ),
            (r#"level="INFO" msg="user text level=error""#, Level::Info),
            (
                "time=2026-10-16\tmsg=\"user text level=error\"",
                Level::Info,
            ),
        ] {
            assert_eq!(Level::of(line), level, "{line}");
            assert_eq!(Level::of(&format!("[内核] {line}")), level, "{line}");
        }
    }

    #[test]
    fn guarded_client_prefix_and_unstructured_payload_do_not_supply_a_level() {
        for line in [
            "[内核] [客户端] 任意正文 level=error",
            "[内核] [客户端] 操作失败： level=error",
            "[vendor] time=2026-10-16 level=error",
            "external text level=error",
        ] {
            assert_eq!(Level::of(line), Level::Info, "{line}");
        }
    }

    #[test]
    fn client_operation_failure_keeps_error_level_regardless_of_details() {
        for line in [
            "[客户端] 操作失败：timeout",
            "[客户端] 操作失败：上游回复 level=warning",
        ] {
            assert_eq!(Level::of(line), Level::Error, "{line}");
        }
    }
}
