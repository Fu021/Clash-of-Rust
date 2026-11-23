use std::path::{Path, PathBuf};

#[path = "../../../translations/compile_catalog.rs"]
mod compile_catalog;

fn main() {
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    compile_catalog::compile(
        Path::new("../../../translations"),
        "android/rust",
        &output.join("messages.rs"),
    );
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android")
        && std::env::var_os("CARGO_FEATURE_MOBILE_UI_TESTS").is_none()
    {
        return;
    }
    let translations = "../../../translations/android/slint";
    println!("cargo:rerun-if-changed={translations}");
    let config = slint_build::CompilerConfiguration::new()
        .with_edition((1, 19))
        .with_bundled_translations(translations)
        .with_default_translation_context(slint_build::DefaultTranslationContext::None)
        .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config(
        "../../../crates/client-ui/ui/mobile/android/window.slint",
        config,
    )
    .expect("Compile Android Slint UI");
}
