//! Optional main-thread screenshot harness; absent from distributed binaries.
#[cfg(any(target_os = "linux", target_os = "macos"))]
// harness=false imports test modules without running their #[test] functions.
#[allow(dead_code, unused_imports)]
#[path = "../src/main.rs"]
mod app;

fn main() {
    #[cfg(target_os = "macos")]
    if let Some(journal) = std::env::var_os("CLASH_CI_RESTORE_PROXY") {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
        clash_of_rust::platform::restore(std::path::Path::new(&journal))
            .expect("native flow cleanup must restore its system proxy");
        return;
    }
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
        #[cfg(target_os = "macos")]
        if option_env!("CLASH_MAC_UPDATE_TEST") == Some("1") {
            app::ui_preview::run_diagnostic_app().expect("diagnostic Mac app must run");
        }
        return;
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    app::ui_preview::run().expect("UI preview must render successfully");
}
