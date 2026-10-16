use std::{collections::BTreeMap, sync::OnceLock};

pub fn country_code(name: &str) -> Option<&'static str> {
    static NAMES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    let names = NAMES.get_or_init(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../resources/flags/countries.json"
        )))
        .unwrap()
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
