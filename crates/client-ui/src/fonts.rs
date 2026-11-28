use slint::fontique_012::{fontique, shared_collection};
use std::sync::Arc;

pub fn editor_font() -> slint::SharedString {
    let mut collection = shared_collection();
    let family = collection
        .generic_families(fontique::GenericFamily::Monospace)
        .next();
    family
        .and_then(|family| collection.family_name(family))
        .unwrap_or("Source Han Sans CN")
        .into()
}

pub fn register_emoji() -> Result<(), slint::PlatformError> {
    let mut collection = shared_collection();
    let fonts = collection.register_fonts(
        fontique::Blob::new(Arc::new(include_bytes!(
            "../../../resources/fonts/NotoColorEmoji-Regular.ttf"
        ) as &'static [u8])),
        None,
    );
    if fonts.is_empty() {
        return Err("无法注册内置 emoji 字体".into());
    }
    for generic in [
        fontique::GenericFamily::Emoji,
        fontique::GenericFamily::SansSerif,
        fontique::GenericFamily::SystemUi,
    ] {
        let previous = collection.generic_families(generic).collect::<Vec<_>>();
        collection.set_generic_families(
            generic,
            fonts.iter().map(|(family, _)| *family).chain(
                previous
                    .into_iter()
                    .filter(|family| !fonts.iter().any(|(registered, _)| registered == family)),
            ),
        );
    }
    Ok(())
}
