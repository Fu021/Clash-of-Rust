//! Compact subscription metadata shared by desktop and mobile cards.
use chrono::{Local, TimeZone};

fn amount(bytes: u64) -> (String, &'static str) {
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit + 1 < units.len() {
        value /= 1024.;
        unit += 1;
    }
    let mut text = format!("{value:.1}");
    if text.ends_with(".0") {
        text.truncate(text.len() - 2);
    }
    (text, units[unit])
}

fn traffic(bytes: u64) -> String {
    let (value, unit) = amount(bytes);
    format!("{value} {unit}")
}

fn date(seconds: u64, pattern: &str) -> Option<String> {
    Local
        .timestamp_opt(i64::try_from(seconds).ok()?, 0)
        .single()
        .map(|date| date.format(pattern).to_string())
}

/// Keep the used/quota pair and expiry date; omit redundant transfer breakdowns.
pub fn summary(value: Option<&str>, updated: u64) -> String {
    let (mut upload, mut download, mut total, mut expire) = (None, None, None, None);
    for field in value.unwrap_or_default().split(';') {
        let Some((name, value)) = field.trim().split_once('=') else {
            continue;
        };
        let Ok(value) = value.trim().parse::<u64>() else {
            continue;
        };
        match name.trim() {
            "upload" => upload = Some(value),
            "download" => download = Some(value),
            "total" => total = Some(value),
            "expire" => expire = Some(value),
            _ => {}
        }
    }
    let mut parts = Vec::new();
    let quota = total.filter(|value| *value > 0);
    if let (Some(upload), Some(download)) = (upload, download) {
        let used = upload.saturating_add(download);
        parts.push(if let Some(total) = quota {
            let (used_text, used_unit) = amount(used);
            let (total_text, total_unit) = amount(total);
            if used == 0 || used_unit == total_unit {
                format!("已用 {used_text}/{total_text} {total_unit}")
            } else {
                format!("已用 {used_text} {used_unit}/{total_text} {total_unit}")
            }
        } else {
            format!("已用 {}", traffic(used))
        });
    } else {
        if let Some(upload) = upload {
            parts.push(format!("上传 {}", traffic(upload)));
        }
        if let Some(download) = download {
            parts.push(format!("下载 {}", traffic(download)));
        }
        if let Some(total) = quota {
            parts.push(format!("总量 {}", traffic(total)));
        }
    }
    if let Some(expire) = expire {
        parts.push(if expire == 0 {
            "不限期".into()
        } else {
            format!(
                "到期 {}",
                date(expire, "%Y-%m-%d").unwrap_or_else(|| "未知".into())
            )
        });
    }
    if parts.is_empty() {
        parts.push("未提供用量与到期".into());
    }
    if updated != 0
        && let Some(updated) = date(updated, "%m-%d %H:%M")
    {
        parts.push(format!("更新 {updated}"));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_transfer_usage_without_hiding_small_or_partial_values() {
        assert_eq!(
            summary(
                Some("upload=1048576; download=2147483648; total=3221225472; expire=0"),
                0
            ),
            "已用 2/3 GiB · 不限期"
        );
        assert_eq!(
            summary(Some("upload=0; download=0; total=107374182400"), 0),
            "已用 0/100 GiB"
        );
        assert_eq!(
            summary(Some("upload=0; download=1048576; total=107374182400"), 0),
            "已用 1 MiB/100 GiB"
        );
        assert_eq!(
            summary(Some("upload=1048576; total=3221225472"), 0),
            "上传 1 MiB · 总量 3 GiB"
        );
        assert_eq!(
            summary(Some("upload=0; download=1048576; total=0"), 0),
            "已用 1 MiB"
        );
    }

    #[test]
    fn missing_invalid_and_overflowing_metadata_remains_readable() {
        assert_eq!(summary(None, 0), "未提供用量与到期");
        assert_eq!(
            summary(Some("broken=value; total=invalid"), 0),
            "未提供用量与到期"
        );
        assert_eq!(summary(Some("expire=18446744073709551615"), 0), "到期 未知");
        assert_eq!(
            summary(Some("upload=18446744073709551615; download=1"), 0),
            "已用 16 EiB"
        );
        let text = summary(Some("expire=1813147200"), 1813147200);
        assert!(text.starts_with("到期 2027-"));
        assert!(text.contains(" · 更新 "));
        assert_eq!(text.matches(':').count(), 1);
    }
}
