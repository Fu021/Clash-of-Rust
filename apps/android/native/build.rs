fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android") {
        return;
    }
    let config = slint_build::CompilerConfiguration::new()
        .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config(
        "../../../crates/client-ui/ui/mobile/android/window.slint",
        config,
    )
    .expect("Compile Android Slint UI");
}
