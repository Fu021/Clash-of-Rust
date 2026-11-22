fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android")
        && std::env::var_os("CARGO_FEATURE_MOBILE_UI_TESTS").is_none()
    {
        return;
    }
    let config = slint_build::CompilerConfiguration::new()
        .with_edition((1, 19))
        .with_bundled_translations("translations")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None)
        .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config(
        "../../../crates/client-ui/ui/mobile/android/window.slint",
        config,
    )
    .expect("Compile Android Slint UI");
}
