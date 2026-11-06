pub use clash_client_core::countries::{country_code, country_text, flag_emoji};
pub fn label(country: &str) -> (&str, Option<String>) {
    let text = country_text(country);
    (text, flag_emoji(country))
}
