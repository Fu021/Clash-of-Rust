#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod application;

fn main() -> anyhow::Result<()> {
    #[cfg(feature = "ui-preview")]
    if std::env::var_os("CLASH_UI_PREVIEW_SCENE").is_some() {
        return application::ui_preview::run();
    }
    application::run()
}
