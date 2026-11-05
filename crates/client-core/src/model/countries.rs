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
    emoji_prefix(country)
        .map(|(code, _)| code)
        .or_else(|| country_code(country))
        .filter(|code| !matches!(*code, "HK" | "MO" | "TW"))
}

fn emoji_prefix(value: &str) -> Option<(&'static str, &str)> {
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
    Some((
        country_code(&code)?,
        value[first.len_utf8() + second.len_utf8()..].trim_start(),
    ))
}

fn node_name_code(name: &str) -> Option<&'static str> {
    static LABELS: OnceLock<Vec<(String, &'static str)>> = OnceLock::new();
    let labels = LABELS.get_or_init(|| {
        let mut labels = Vec::new();
        for (code, english) in countries() {
            labels.push((english.clone(), code.as_str()));
            if let Some(localized) = crate::probe::country_name(code)
                && localized != *code
            {
                labels.push((localized, code.as_str()));
            }
        }
        labels.sort_by_key(|(label, _)| std::cmp::Reverse(label.len()));
        labels
    });
    labels.iter().find_map(|(label, code)| {
        let prefix = name.get(..label.len())?;
        let remaining = &name[label.len()..];
        (prefix.eq_ignore_ascii_case(label)
            && remaining
                .chars()
                .next()
                .is_none_or(|character| character.is_ascii_digit() || node_separator(character)))
        .then_some(*code)
    })
}

fn node_separator(character: char) -> bool {
    character.is_whitespace() || "-_|·".contains(character)
}

pub fn node_label(name: &str) -> (&str, Option<&'static str>) {
    let trimmed = name.trim_start();
    let (code, remaining) = if let Some((code, remaining)) = emoji_prefix(trimmed) {
        (Some(code), remaining)
    } else if let Some(prefix) = trimmed.get(..2)
        && prefix.bytes().all(|byte| byte.is_ascii_uppercase())
        && trimmed[2..].chars().next().is_none_or(node_separator)
    {
        (
            country_code(prefix),
            trimmed[2..].trim_start_matches(node_separator),
        )
    } else {
        return (name, node_name_code(trimmed).and_then(flag_code));
    };
    let Some(code) = code else {
        return (name, None);
    };
    let flag = flag_code(code);
    let label = if remaining.is_empty() && flag.is_none() {
        code
    } else {
        remaining
    };
    (label, flag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_flags_do_not_depend_on_regional_indicator_fonts() {
        for name in ["🇩🇪 德国 01", "DE 德国 01", "DE-德国 01", "DE | 德国 01"] {
            assert_eq!(node_label(name), ("德国 01", Some("DE")), "{name}");
        }
        assert_eq!(node_label("🇯🇵東京"), ("東京", Some("JP")));
        assert_eq!(node_label("🇩🇪"), ("", Some("DE")));
        assert_eq!(node_label("DE"), ("", Some("DE")));
        for (name, label, code) in [
            ("🇺🇸 美国 01", "美国 01", "US"),
            ("US-美国 01", "美国 01", "US"),
            ("🇯🇵 日本 01", "日本 01", "JP"),
            ("JP | 日本 01", "日本 01", "JP"),
            ("美国 01", "美国 01", "US"),
            ("美国01", "美国01", "US"),
            ("日本 01", "日本 01", "JP"),
            ("日本-01", "日本-01", "JP"),
            ("新加坡 01", "新加坡 01", "SG"),
            ("德国 01", "德国 01", "DE"),
            ("United States 01", "United States 01", "US"),
            ("Japan 01", "Japan 01", "JP"),
        ] {
            assert_eq!(node_label(name), (label, Some(code)), "{name}");
        }
    }

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
    fn hong_kong_macao_and_taiwan_remain_text_only() {
        for (code, emoji, name) in [
            ("HK", "🇭🇰", "香港"),
            ("MO", "🇲🇴", "澳门"),
            ("TW", "🇹🇼", "台湾"),
        ] {
            assert_eq!(country_code(name), Some(code));
            for country in [code, emoji, name] {
                assert_eq!(flag_code(country), None, "{country}");
            }
            let prefixed = format!("{emoji} {name} 01");
            let label = format!("{name} 01");
            assert_eq!(node_label(&prefixed), (label.as_str(), None));
            assert_eq!(node_label(&label), (label.as_str(), None));
            assert_eq!(node_label(emoji), (code, None));
            assert_eq!(node_label(code), (code, None));
        }
    }

    #[test]
    fn node_labels_preserve_names_without_an_explicit_country_prefix() {
        for name in [
            "",
            "DIRECT",
            "DEFAULT",
            "🇽🇽 未知",
            "XX-01",
            "de 01",
            "Japanify 01",
            "United Stateside",
            "🇩 node",
            "  node-a",
            "普通 🇩🇪 节点",
            "DE.example.com",
        ] {
            assert_eq!(node_label(name), (name, None), "{name}");
        }
    }
}
