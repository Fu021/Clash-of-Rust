//! Optional main-thread screenshot harness; absent from distributed binaries.
// harness=false imports test modules without running their #[test] functions.
#[allow(dead_code, unused_imports)]
#[path = "../src/application/mod.rs"]
mod app;

fn main() {
    if std::env::var_os("CLASH_UI_PREVIEW_SCENE").is_none() {
        return;
    }
    app::ui_preview::run().expect("UI preview must render successfully");
}
