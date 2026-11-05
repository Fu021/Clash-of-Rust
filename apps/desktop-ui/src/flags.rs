//! Offline bitmap flags avoid Windows font-dependent regional indicator rendering.
pub use clash_client_core::countries::{country_code, country_text, flag_code};
use slint::{Image, SharedPixelBuffer};
use std::sync::OnceLock;
use std::{cell::RefCell, collections::VecDeque};

const SIZE: u32 = 72;
const BYTES: usize = (SIZE * SIZE * 4) as usize;
const CACHE_LIMIT: usize = 32;
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

thread_local! {
    static HANDLES: RefCell<VecDeque<(usize, Image)>> = const { RefCell::new(VecDeque::new()) };
}

#[doc(hidden)]
pub fn release_cache() {
    HANDLES.with(|handles| *handles.borrow_mut() = VecDeque::new());
}

pub fn label(country: &str) -> (&str, Option<Image>) {
    let text = country_text(country);
    let Some(code) = flag_code(country) else {
        return (text, None);
    };
    let handle = codes().iter().position(|item| item == code).map(|index| {
        HANDLES.with(|handles| {
            let mut handles = handles.borrow_mut();
            let image = if let Some(position) = handles.iter().position(|(key, _)| *key == index) {
                handles.remove(position).unwrap().1
            } else {
                Image::from_rgba8(SharedPixelBuffer::clone_from_slice(
                    &PIXELS[index * BYTES..(index + 1) * BYTES],
                    SIZE,
                    SIZE,
                ))
            };
            handles.push_front((index, image.clone()));
            handles.truncate(CACHE_LIMIT);
            image
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
        for code in codes() {
            assert!(label(code).1.is_some(), "{code}");
        }
        assert_eq!(country_code("United States"), Some("US"));
        assert_eq!(country_code("No"), None);
        assert_eq!(label("🇺🇸 美国").0, "美国");
        assert!(label("🇺🇸 美国").1.is_some());
        for country in [
            "🇩🇪 德国",
            "德国",
            "DE",
            "Germany",
            "🇺🇸 美国",
            "美国",
            "US",
            "United States",
            "🇯🇵 日本",
            "日本",
            "JP",
            "Japan",
        ] {
            assert!(label(country).1.is_some(), "{country}");
        }
        assert!(label("未识别").1.is_none());
        for country in [
            "香港",
            "澳门",
            "台湾",
            "HK",
            "MO",
            "TW",
            "🇭🇰 香港",
            "🇲🇴 澳门",
            "🇹🇼 台湾",
        ] {
            assert!(label(country).1.is_some(), "{country}");
        }
    }

    #[test]
    fn bitmap_cache_is_bounded_and_can_release_all_its_handles() {
        release_cache();
        for code in codes() {
            let _ = label(code);
        }
        HANDLES.with(|handles| assert_eq!(handles.borrow().len(), CACHE_LIMIT));
        let retained = label("DE").1.unwrap();
        release_cache();
        HANDLES.with(|handles| assert!(handles.borrow().is_empty()));
        assert_eq!(retained.size().width, SIZE);
        assert_eq!(retained.size().height, SIZE);
        assert!(label("DE").1.is_some());
        release_cache();
    }
}
