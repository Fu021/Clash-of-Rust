//! Offline bitmap flags avoid Windows font-dependent regional indicator rendering.
pub use clash_client_core::countries::{country_code, country_text};
use slint::{Image, SharedPixelBuffer};
use std::cell::OnceCell;
use std::sync::OnceLock;

const SIZE: u32 = 72;
const BYTES: usize = (SIZE * SIZE * 4) as usize;
const PIXELS: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/flags/flags.rgba"
));

fn codes() -> &'static Vec<String> {
    static CODES: OnceLock<Vec<String>> = OnceLock::new();
    CODES.get_or_init(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../resources/flags/codes.json"
        )))
        .unwrap()
    })
}

pub fn label(country: &str) -> (&str, Option<Image>) {
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
                    .filter_map(|ch| char::from_u32(u32::from(ch) - 0x1f1e6 + u32::from(b'A')))
                    .collect::<String>(),
            )
        }
        _ => None,
    };
    let code = flag.or_else(|| country_code(text).map(str::to_owned));
    let Some(code) = code else {
        return (text, None);
    };
    if matches!(code.as_str(), "HK" | "MO" | "TW") {
        return (text, None);
    }
    thread_local! {
        static HANDLES: Vec<OnceCell<Image>> = codes().iter().map(|_| OnceCell::new()).collect();
    }
    let handle = codes().iter().position(|item| item == &code).map(|index| {
        HANDLES.with(|handles| {
            handles[index]
                .get_or_init(|| {
                    Image::from_rgba8(SharedPixelBuffer::clone_from_slice(
                        &PIXELS[index * BYTES..(index + 1) * BYTES],
                        SIZE,
                        SIZE,
                    ))
                })
                .clone()
        })
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
        for country in ["🇩🇪 德国", "德国", "DE", "Germany"] {
            assert!(label(country).1.is_some(), "{country}");
        }
        assert!(label("未识别").1.is_none());
        for country in ["香港", "澳门", "台湾", "🇭🇰 香港", "🇲🇴 澳门", "🇹🇼 台湾"]
        {
            assert!(label(country).1.is_none());
        }
    }
}
