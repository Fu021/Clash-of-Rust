use super::*;
use crate::claude_check::protection::{Previous, Protection};
use crate::config::Store;

#[tokio::test]
async fn protection_transition_cannot_overlap_a_login_service_change() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::at(root.path().into()).unwrap();
    let mut engine = crate::engine::test_support::fixture(store.clone());
    let service = store.protection_transition_lock().unwrap();
    let error = engine.claude_protection(true).await.unwrap_err();
    assert!(error.to_string().contains("登录自启动"));
    assert!(engine.settings.claude_protection.is_none());
    drop(service);
    assert!(store.protection_transition_lock().is_ok());
}

#[tokio::test]
async fn protected_measurement_cannot_start_a_stopped_core() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = crate::engine::test_support::fixture(Store::at(root.path().into()).unwrap());
    engine.settings.claude_protection = Some(Arc::new(Protection::new(
        uuid::Uuid::new_v4().to_string(),
        "fixture".into(),
        Previous {
            mode: "rule".into(),
            proxy_mode: ProxyMode::Off,
            selectors: Default::default(),
        },
    )));
    engine.settings.active_profile = engine
        .settings
        .claude_protection
        .as_ref()
        .map(|protection| protection.profile_id.clone());
    let settings = serde_json::to_value(&engine.settings).unwrap();
    let shared = Arc::new(Mutex::new(engine));
    for command in [
        Command::Delay {
            name: None,
            group: false,
        },
        Command::Delay {
            name: Some("fixture".into()),
            group: true,
        },
        Command::Delay {
            name: Some("fixture".into()),
            group: false,
        },
    ] {
        let error = execute(&shared, command, Owner::Daemon, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Claude 保护"), "{error:#}");
        let engine = shared.lock().await;
        assert!(!engine.running());
        assert_eq!(serde_json::to_value(&engine.settings).unwrap(), settings);
        assert!(!engine.store.runtime().join("config.yaml").exists());
        assert_eq!(engine.view_revision(), 0);
    }
    assert!(matches!(
        execute(&shared, Command::RecoverUnreachable, Owner::Daemon, None)
            .await
            .unwrap(),
        Payload::Json(Value::Null)
    ));
    let Payload::Json(meta) = execute(&shared, Command::Meta, Owner::Daemon, None)
        .await
        .unwrap()
    else {
        panic!("metadata command must return a JSON payload");
    };
    assert!(meta["settings"]["claude_protection"].is_object());
}

#[tokio::test]
async fn protected_display_patch_persists_without_starting_or_changing_protection() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = crate::engine::test_support::fixture(Store::at(root.path().into()).unwrap());
    let profile = uuid::Uuid::new_v4().to_string();
    engine.settings.active_profile = Some(profile.clone());
    engine.settings.claude_protection = Some(Arc::new(Protection::new(
        profile,
        "fixture".into(),
        Previous {
            mode: "rule".into(),
            proxy_mode: ProxyMode::Off,
            selectors: Default::default(),
        },
    )));
    let protection = engine.settings.claude_protection.clone();
    let shared = Arc::new(Mutex::new(engine));
    execute(
        &shared,
        Command::ConfigSet {
            key: "accent_color".into(),
            value: json!("blue"),
        },
        Owner::Daemon,
        None,
    )
    .await
    .unwrap();
    execute(
        &shared,
        Command::ConfigSet {
            key: "tui_node_sort".into(),
            value: json!("name"),
        },
        Owner::Daemon,
        None,
    )
    .await
    .unwrap();
    execute(
        &shared,
        Command::ConfigSet {
            key: "proxy_groups_paginated".into(),
            value: json!(true),
        },
        Owner::Daemon,
        None,
    )
    .await
    .unwrap();
    let settings = {
        let engine = shared.lock().await;
        assert!(!engine.running());
        assert!(engine.store.load_settings().unwrap().proxy_groups_paginated);
        assert_eq!(engine.settings.claude_protection, protection);
        assert_eq!(
            engine.store.load_settings().unwrap().accent_color,
            crate::config::AccentColor::Blue
        );
        assert_eq!(
            engine.store.load_settings().unwrap().tui_node_sort,
            clash_client_core::proxy_order::NodeSort::Name
        );
        assert!(!engine.store.runtime().join("config.yaml").exists());
        engine.settings.clone()
    };
    let mut mixed = settings.clone();
    mixed.text_scale = 125;
    mixed.ipv6 = !mixed.ipv6;
    let error = execute(
        &shared,
        Command::Settings {
            settings: mixed,
            expected: None,
        },
        Owner::Daemon,
        None,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("Claude 保护"));
    let engine = shared.lock().await;
    assert_eq!(engine.settings, settings);
    assert_eq!(engine.store.load_settings().unwrap(), settings);
}
