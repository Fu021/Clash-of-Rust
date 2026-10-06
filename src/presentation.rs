//! Human-readable values shared by the native UI.
use chrono::{DateTime, Local, TimeZone};

pub fn timestamp(seconds: u64) -> String {
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| Local.timestamp_opt(seconds, 0).single())
        .map(|date| date.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "未知时间".into())
}

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

pub fn traffic(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.2} GiB", bytes as f64 / 1_073_741_824.0)
    } else {
        format!("{:.2} MiB", bytes as f64 / 1_048_576.0)
    }
}

pub fn subscription_usage(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "订阅未提供流量/到期信息".into();
    };
    let fields: std::collections::BTreeMap<_, _> = value
        .split(';')
        .filter_map(|field| {
            let (name, value) = field.trim().split_once('=')?;
            Some((name.trim(), value.trim().parse::<u64>().ok()?))
        })
        .collect();
    let mut result = Vec::new();
    if let Some(upload) = fields.get("upload") {
        result.push(format!("上传 {}", traffic(*upload)));
    }
    if let Some(download) = fields.get("download") {
        result.push(format!("下载 {}", traffic(*download)));
    }
    if let Some(total) = fields.get("total") {
        result.push(format!("总流量 {}", traffic(*total)));
        if let (Some(upload), Some(download)) = (fields.get("upload"), fields.get("download")) {
            result.push(format!(
                "剩余 {}",
                traffic(total.saturating_sub(upload.saturating_add(*download)))
            ));
        }
    }
    if let Some(expire) = fields.get("expire") {
        result.push(if *expire == 0 {
            "到期时间：不限".into()
        } else {
            format!("到期 {}", timestamp(*expire))
        });
    }
    if result.is_empty() {
        "订阅未提供有效流量/到期信息".into()
    } else {
        result.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usage_handles_units_expiration_and_missing_fields() {
        let result = subscription_usage(Some(
            "upload=1048576; download=2147483648; total=3221225472; expire=0",
        ));
        assert!(result.contains("上传 1.00 MiB"));
        assert!(result.contains("下载 2.00 GiB"));
        assert!(result.contains("剩余 1023.00 MiB"));
        assert!(result.contains("不限"));
        assert!(!result.contains("upload="));
        assert_eq!(
            subscription_usage(Some("broken=value")),
            "订阅未提供有效流量/到期信息"
        );
        assert_eq!(traffic(1_073_741_823), "1024.00 MiB");
        assert_eq!(traffic(1_073_741_824), "1.00 GiB");
        assert!(!date_text("2026-10-04T01:48:01Z").contains('T'));
        assert!(!geo_version("latest · 2026-10-04T01:48:01Z").contains('Z'));
        assert!(!geo_version("latest 路 2026-10-04T01:48:01Z").contains('T'));
        assert!(!geo_version("latest 路 2026-10-04T01:48:01Z").contains('路'));
    }
}
