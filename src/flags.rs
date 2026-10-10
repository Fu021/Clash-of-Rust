//! Offline bitmap flags avoid Windows font-dependent regional indicator rendering.
use iced::widget::image::Handle;
use std::{collections::BTreeMap, sync::OnceLock};

const SIZE: u32 = 72;
const BYTES: usize = (SIZE * SIZE * 4) as usize;
const PIXELS: &[u8] = include_bytes!("../resources/flags/flags.rgba");

fn codes() -> &'static Vec<String> {
    static CODES: OnceLock<Vec<String>> = OnceLock::new();
    CODES.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/flags/codes.json")).unwrap()
    })
}

pub(crate) fn country_code(name: &str) -> Option<&'static str> {
    static NAMES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    let names = NAMES.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/flags/countries.json")).unwrap()
    });
    names.iter().find_map(|(code, english)| {
        (english.eq_ignore_ascii_case(name)
            || (code == name && name == name.to_ascii_uppercase())
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

pub fn label(country: &str) -> (&str, Option<Handle>) {
    let mut chars = country.char_indices();
    let Some((_, first)) = chars.next() else {
        return (country, None);
    };
    let Some((_, second)) = chars.next() else {
        return (country, None);
    };
    if !(0x1f1e6..=0x1f1ff).contains(&u32::from(first))
        || !(0x1f1e6..=0x1f1ff).contains(&u32::from(second))
    {
        return (country, None);
    }
    let code: String = [first, second]
        .into_iter()
        .filter_map(|ch| char::from_u32(u32::from(ch) - 0x1f1e6 + u32::from(b'A')))
        .collect();
    let text = country_text(country);
    if matches!(code.as_str(), "HK" | "MO" | "TW") {
        return (text, None);
    }
    static HANDLES: OnceLock<Vec<OnceLock<Handle>>> = OnceLock::new();
    let handles = HANDLES.get_or_init(|| codes().iter().map(|_| OnceLock::new()).collect());
    let handle = codes().iter().position(|item| item == &code).map(|index| {
        handles[index]
            .get_or_init(|| {
                Handle::from_rgba(SIZE, SIZE, &PIXELS[index * BYTES..(index + 1) * BYTES])
            })
            .clone()
    });
    (text, handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn countries_and_pixels_are_consistent() {
        assert_eq!(PIXELS.len(), codes().len() * BYTES);
        assert_eq!(country_code("United States"), Some("US"));
        assert_eq!(country_code("No"), None);
        assert_eq!(label("🇺🇸 美国").0, "美国");
        assert!(label("🇺🇸 美国").1.is_some());
        for country in ["香港", "澳门", "台湾", "🇭🇰 香港", "🇲🇴 澳门", "🇹🇼 台湾"]
        {
            assert!(label(country).1.is_none());
        }
    }
}
