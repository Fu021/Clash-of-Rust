use super::*;

pub fn bounded(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(512)
        .collect()
}
pub fn proxy_bypass_display(value: &str) -> String {
    bounded(
        &value
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .take(24)
            .map(|part| {
                if part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b".-_:/[]*".contains(&c))
                {
                    bounded(part)
                } else {
                    "[无效或含敏感字符的条目未记录]".into()
                }
            })
            .collect::<Vec<_>>()
            .join(", "),
    )
}

pub fn redact_url(value: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(value) else {
        return "地址格式无效（原始内容未记录）".into();
    };
    if url.host_str().is_none() {
        return "地址格式无效（原始内容未记录）".into();
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_path("");
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

pub fn region_tag(language: &str) -> Option<String> {
    let language = language.split('.').next()?.replace('_', "-");
    language
        .split('-')
        .skip(1)
        .find(|part| part.len() == 2 && country_name(part).is_some())
        .map(str::to_ascii_uppercase)
}

pub fn timezone_country(zone: &str) -> Option<&'static str> {
    // A deliberately small map of unambiguous preference labels. UTC offsets
    // alone, ambiguous zones and language-only tags never identify a country.
    match zone {
        "Asia/Shanghai" | "Asia/Chongqing" | "Asia/Urumqi" | "China Standard Time" => Some("CN"),
        "Asia/Hong_Kong" => Some("HK"),
        "Asia/Taipei" | "Taipei Standard Time" => Some("TW"),
        "Asia/Tokyo" | "Tokyo Standard Time" => Some("JP"),
        "Asia/Singapore" => Some("SG"),
        "Asia/Seoul" | "Korea Standard Time" => Some("KR"),
        "Europe/London" => Some("GB"),
        "America/New_York"
        | "America/Los_Angeles"
        | "America/Chicago"
        | "America/Denver"
        | "Pacific/Honolulu" => Some("US"),
        "Europe/Berlin" => Some("DE"),
        "Europe/Paris" => Some("FR"),
        _ => None,
    }
}

pub fn preference(
    group: Group,
    title: &str,
    value: &str,
    source: &str,
    country: Option<&str>,
) -> Signal {
    let mut signal = Signal::new(
        group,
        title,
        bounded(value),
        source,
        Evidence::Measured,
        "这是当前环境的设置值，不代表物理所在地，也未确认 Claude 是否使用这一项进行地区判断。",
    );
    // The value is measured; attributing a region to it is an inference.
    if let Some(code) = country {
        signal.country = code.into();
        signal.evidence = Evidence::Inferred;
    }
    signal.advice =
        "如与预期环境不同，检查此环境的地区、语言或时区设置；无需仅因使用中文而调整。".into();
    signal
}
