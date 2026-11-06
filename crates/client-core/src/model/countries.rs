use std::{collections::BTreeMap, sync::OnceLock};

fn countries() -> &'static BTreeMap<String, String> {
    static NAMES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../resources/flags/countries.json"
        )))
        .unwrap()
    })
}

pub fn country_code(name: &str) -> Option<&'static str> {
    if let Some((code, _)) = countries().get_key_value(name) {
        return Some(code);
    }
    countries().iter().find_map(|(code, english)| {
        (english.eq_ignore_ascii_case(name)
            || crate::probe::country_name(code).as_deref() == Some(name))
        .then_some(code.as_str())
    })
}

pub fn country_text(country: &str) -> &str {
    let mut chars = country.chars();
    if let (Some(first), Some(second)) = (chars.next(), chars.next())
        && (0x1f1e6..=0x1f1ff).contains(&u32::from(first))
        && (0x1f1e6..=0x1f1ff).contains(&u32::from(second))
    {
        return country[first.len_utf8() + second.len_utf8()..].trim_start();
    }
    country
}

pub fn flag_code(country: &str) -> Option<&'static str> {
    let code = emoji_code(country).or_else(|| country_code(country))?;
    Some(if uses_china_flag(code) { "CN" } else { code })
}

fn uses_china_flag(code: &str) -> bool {
    matches!(code, "HK" | "MO" | "TW")
}

fn emoji_code(value: &str) -> Option<&'static str> {
    let mut chars = value.chars();
    let first = chars.next()?;
    let second = chars.next()?;
    if ![first, second]
        .iter()
        .all(|character| (0x1f1e6..=0x1f1ff).contains(&u32::from(*character)))
    {
        return None;
    }
    let code = [first, second]
        .into_iter()
        .filter_map(|character| char::from_u32(u32::from(character) - 0x1f1e6 + u32::from(b'A')))
        .collect::<String>();
    country_code(&code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_codes_accept_emoji_names_and_codes() {
        assert_eq!(flag_code("🇩🇪"), Some("DE"));
        assert_eq!(flag_code("德国"), Some("DE"));
        assert_eq!(flag_code("DE"), Some("DE"));
        assert_eq!(flag_code("🇺🇸 美国"), Some("US"));
        assert_eq!(flag_code("日本"), Some("JP"));
        assert_eq!(flag_code("🇯🇵"), Some("JP"));
        assert_eq!(flag_code("🇺🇸 德国"), Some("US"));
        assert_eq!(flag_code("未识别"), None);
    }

    #[test]
    fn hong_kong_macao_and_taiwan_use_the_china_flag() {
        for (code, emoji, name) in [
            ("HK", "🇭🇰", "香港"),
            ("MO", "🇲🇴", "澳门"),
            ("TW", "🇹🇼", "台湾"),
        ] {
            assert_eq!(country_code(name), Some(code));
            for country in [code, emoji, name] {
                assert_eq!(flag_code(country), Some("CN"), "{country}");
            }
        }
    }

    #[test]
    fn report_country_text_keeps_the_region_name_and_rejects_unknown_flags() {
        for (country, expected) in [
            ("🇺🇸 美国", "美国"),
            ("🇭🇰 香港", "香港"),
            ("🇹🇼 台湾", "台湾"),
            ("🇲🇴 澳门", "澳门"),
            ("德国", "德国"),
            ("未知", "未知"),
        ] {
            assert_eq!(country_text(country), expected);
        }
        for country in ["", "🇽🇽 未知", "🇩", "🚀 🇺🇸 美国"] {
            assert_eq!(flag_code(country), None, "{country}");
        }
    }
}
