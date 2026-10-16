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
        if let Some((tag, _)) = line.strip_prefix('[').and_then(|line| line.split_once(']'))
            && let Some(level) = Self::parse(tag)
        {
            return level;
        }
        let header = line.split_once(" msg=").map_or(line, |(header, _)| header);
        if let Some(level) = header.split_ascii_whitespace().find_map(|token| {
            token
                .strip_prefix("level=")
                .and_then(|level| Self::parse(level.trim_matches('"')))
        }) {
            return level;
        }
        if line.starts_with("[客户端] 操作失败：") {
            Self::Error
        } else {
            Self::Info
        }
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
            ("[warning] dial failed: error", Level::Warn),
            ("[info] error counter reset", Level::Info),
            ("[debug] test", Level::Debug),
            (
                "[内核] time=2026-10-16 level=warning msg=\"error: timeout\"",
                Level::Warn,
            ),
            (
                "[内核] level=info msg=\"user text level=error\"",
                Level::Info,
            ),
            ("[客户端] 操作失败：timeout", Level::Error),
        ] {
            assert_eq!(Level::of(line), level, "{line}");
        }
    }
}
