//! Country tokens; raster images belong exclusively to the renderer process.
pub use clash_client_core::countries::{country_code, country_text};
pub fn label(country: &str) -> (&str, Option<String>) {
    let text = country_text(country);
    let mut chars = country.chars();
    let flag = match (chars.next(), chars.next()) {
        (Some(a), Some(b))
            if (0x1f1e6..=0x1f1ff).contains(&u32::from(a))
                && (0x1f1e6..=0x1f1ff).contains(&u32::from(b)) =>
        {
            Some(
                [a, b]
                    .into_iter()
                    .map(|c| char::from_u32(u32::from(c) - 0x1f1e6 + 65).unwrap())
                    .collect::<String>(),
            )
        }
        _ => None,
    };
    let code = flag.or_else(|| country_code(text).map(str::to_owned));
    (
        text,
        code.filter(|v| country_code(v).is_some() && !matches!(v.as_str(), "HK" | "MO" | "TW")),
    )
}
