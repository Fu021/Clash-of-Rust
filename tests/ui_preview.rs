//! Optional main-thread screenshot harness; absent from distributed binaries.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(dead_code)]
#[path = "../src/main.rs"]
mod app;

fn main() {
    if std::env::var_os("CLASH_UI_PREVIEW_SCENE").is_none() {
        return;
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    app::ui_preview::run().expect("UI preview must render successfully");
}
