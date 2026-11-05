//! Country tokens; raster images belong exclusively to the renderer process.
pub use clash_client_core::countries::{country_code, country_text, flag_code};
pub fn label(country: &str) -> (&str, Option<String>) {
    let text = country_text(country);
    (text, flag_code(country).map(str::to_owned))
}
