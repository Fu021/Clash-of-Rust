#[allow(dead_code)]
#[path = "../src/application_policies.rs"]
mod application_policies;
#[allow(dead_code)]
#[path = "../src/application_routing.rs"]
mod application_routing;
#[path = "../src/application_routing_mode.rs"]
mod application_routing_mode;

use anyhow::Result;
use clash_mobile_runtime::{NativeHost, Session, core::config::Language};
use serde_json::{Value, json};
use slint::{
    ComponentHandle,
    platform::{Platform, PlatformError, WindowAdapter, software_renderer::MinimalSoftwareWindow},
};
use std::{
    rc::Rc,
    sync::{Arc, Mutex},
};

slint::include_modules!();

struct Headless(Rc<MinimalSoftwareWindow>);

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

struct Host;

impl NativeHost for Host {
    fn request(&self, value: Value) -> Result<Value> {
        match value["op"].as_str() {
            Some("applications-list") => Ok(json!({
                "applications": [{
                    "name": "Example App",
                    "package": "example.app",
                    "uid": 10000,
                    "launchable": true
                }],
                "mode": "all",
                "selected": []
            })),
            Some("application-policy-capability") => Ok(json!({"supported": true})),
            operation => anyhow::bail!("unexpected operation: {operation:?}"),
        }
    }
}

#[tokio::test]
async fn application_rules_localize_views_and_keep_action_values() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = Session::open(directory.path().to_owned(), Arc::new(Host)).unwrap();
    let mut editor = application_policies::Editor::default();
    editor
        .request(&mut session, "open", "", Language::English)
        .await
        .unwrap();

    let english = editor.view(&session, Language::English);
    let chinese = editor.view(&session, Language::SimplifiedChinese);
    assert!(english.status.contains("VPN apps"));
    assert!(chinese.status.contains("进入 VPN"));
    assert_eq!(english.targets, chinese.targets);
    assert!(english.targets.iter().any(|target| target == "直连"));

    editor
        .request(&mut session, "new", "", Language::English)
        .await
        .unwrap();
    let error = editor
        .request(&mut session, "new", "", Language::English)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Save or undo the current rule first");
    assert!(editor.view(&session, Language::English).editing);
}

#[test]
fn protected_application_routing_reports_the_selected_language() {
    let adapter = MinimalSoftwareWindow::new(
        slint::platform::software_renderer::RepaintBufferType::NewBuffer,
    );
    slint::platform::set_platform(Box::new(Headless(adapter))).unwrap();
    let window = AndroidWindow::new().unwrap();
    window.global::<MobileData>().set_protected(true);
    let editor = Arc::new(Mutex::new(application_routing::Editor::default()));
    for (language, expected) in [
        (
            Language::English,
            "Disable Claude protection before changing application routing",
        ),
        (
            Language::SimplifiedChinese,
            "请先关闭 Claude 保护，再修改应用分流",
        ),
    ] {
        let error = application_routing::request(
            &editor,
            Arc::new(Host),
            &window,
            AndroidIntent::ApplicationMode,
            "1",
            language,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), expected);
    }
}
