//! Human-readable values shared by the native UI.
use chrono::{DateTime, Local};

pub fn date_text(value: &str) -> String {
    DateTime::parse_from_rfc3339(value)
        .map(|date| {
            date.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates_remain_readable() {
        assert!(!date_text("2026-10-04T01:48:01Z").contains('T'));
    }
}
