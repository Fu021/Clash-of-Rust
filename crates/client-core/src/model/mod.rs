#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    #[default]
    Off,
    System,
    Tun,
}

pub fn contains_query(value: &str, lower_query: &str) -> bool {
    if lower_query.is_empty() {
        return true;
    }
    if value.is_ascii() && lower_query.is_ascii() {
        value
            .as_bytes()
            .windows(lower_query.len())
            .any(|part| part.eq_ignore_ascii_case(lower_query.as_bytes()))
    } else {
        value.to_lowercase().contains(lower_query)
    }
}
