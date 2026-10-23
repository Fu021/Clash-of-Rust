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

pub fn geo_version(value: &str) -> String {
    if let Some(date) = value.split_whitespace().last()
        && DateTime::parse_from_rfc3339(date).is_ok()
    {
        let tag = value
            .strip_suffix(date)
            .unwrap_or("")
            .trim_end_matches(|ch: char| {
                ch.is_whitespace() || matches!(ch, '·' | '路' | '|' | 'Â')
            });
        if !tag.is_empty() {
            return format!("{tag} · {}", date_text(date));
        }
    }
    date_text(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geo_dates_remain_readable() {
        assert!(!date_text("2026-10-04T01:48:01Z").contains('T'));
        assert!(!geo_version("latest · 2026-10-04T01:48:01Z").contains('Z'));
        assert!(!geo_version("latest 路 2026-10-04T01:48:01Z").contains('T'));
        assert!(!geo_version("latest 路 2026-10-04T01:48:01Z").contains('路'));
    }
}
