//! Optional main-thread screenshot harness; absent from distributed binaries.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(dead_code)]
#[path = "../src/main.rs"]
mod app;

fn main() {
    #[cfg(target_os = "macos")]
    if clash_of_rust::update::helper_main() {
        return;
    }
    #[cfg(target_os = "macos")]
    if std::env::var_os("CLASH_CI_FLOW").is_some() {
        app::ui_preview::run_macos_flow().expect("native macOS flow must run successfully");
        return;
    }
    if std::env::var_os("CLASH_UI_PREVIEW_SCENE").is_none() {
        return;
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    app::ui_preview::run().expect("UI preview must render successfully");
}
