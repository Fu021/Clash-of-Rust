fn main() {
    let translations = "../../translations/desktop/slint";
    println!("cargo:rerun-if-changed={translations}");
    let config = slint_build::CompilerConfiguration::new()
        .with_edition((1, 19))
        .with_bundled_translations(translations)
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).expect("Compile the desktop UI");
}
