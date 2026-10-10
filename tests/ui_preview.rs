//! Optional main-thread screenshot harness; absent from distributed binaries.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "../src/main.rs"]
mod app;

fn main() {
    if std::env::var_os("CLASH_UI_PREVIEW_SCENE").is_none() {
        return;
    }
    #[cfg(target_os = "linux")]
    app::ui_preview::run().expect("UI preview must render successfully");
}
