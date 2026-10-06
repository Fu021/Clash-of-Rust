//! Segoe UI for Latin text, with an embedded Source Han Sans fallback
//! for CJK runs. This also covers mixed-language editable text fields.
use cosmic_text::{Attrs, Buffer, Fallback, Family, Metrics, PlatformFallback, Shaping};
use iced::Font;
use std::sync::{Arc, OnceLock};
use unicode_script::Script;

pub const ENGLISH_FONT: Font = Font {
    family: iced::font::Family::Name(if cfg!(windows) {
        "Segoe UI"
    } else {
        "Noto Sans"
    }),
    ..Font::DEFAULT
};

/// Fill the 114 px left after the sidebar's padding, 24 px icon and 6 px gap.
pub fn brand_size() -> f32 {
    static SIZE: OnceLock<f32> = OnceLock::new();
    *SIZE.get_or_init(|| {
        let mut system = iced_graphics::text::font_system().write().unwrap();
        let raw = system.raw();
        let family = if cfg!(windows) {
            "Segoe UI"
        } else {
            "Noto Sans"
        };
        let mut buffer = Buffer::new(raw, Metrics::new(20.0, 24.0));
        buffer.set_text(
            raw,
            "Clash of Rust",
            &Attrs::new().family(Family::Name(family)),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(raw, false);
        let width = buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max);
        if width > 0.0 {
            20.0 * 112.0 / width
        } else {
            20.0
        }
    })
}

struct ChineseFallback;
impl Fallback for ChineseFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &[
            "Segoe UI",
            "Source Han Sans CN",
            "Source Han Sans SC",
            "Noto Sans CJK SC",
            "Noto Sans",
            "Segoe UI Emoji",
            "Segoe UI Symbol",
        ]
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn script_fallback(&self, script: Script, locale: &str) -> &[&'static str] {
        if script == Script::Han {
            &[
                "Source Han Sans CN",
                "Source Han Sans SC",
                "Noto Sans CJK SC",
                "WenQuanYi Micro Hei",
                "PingFang SC",
            ]
        } else {
            PlatformFallback.script_fallback(script, locale)
        }
    }
}

pub fn initialize() {
    let mut system = iced_graphics::text::font_system()
        .write()
        .expect("font system lock must be available during startup");
    // Keep a reference to the executable's embedded bytes rather than copying
    // the complete CJK font into another heap allocation.
    system
        .raw()
        .db_mut()
        .load_font_source(cosmic_text::fontdb::Source::Binary(Arc::new(
            include_bytes!("../resources/fonts/SourceHanSansCN-Regular.otf").as_slice(),
        )));
    let database = system.raw().db_mut().clone();
    *system.raw() = cosmic_text::FontSystem::new_with_locale_and_db_and_fallback(
        "zh-CN".into(),
        database,
        ChineseFallback,
    );
}
