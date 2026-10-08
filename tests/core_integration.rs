//! Explicit opt-in tests against an official mihomo binary. No OS proxy/TUN changes.
use clash_of_rust::{
    api::Proxies,
    config::{Settings, Store},
    engine::{Engine, Scope},
};
use serde_json::Value;

fn test_resources() -> std::path::PathBuf {
    std::env::var_os("MIHOMO_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let directory = if cfg!(windows) {
                if cfg!(target_arch = "aarch64") {
                    "bundle/windows-arm64/resources".to_owned()
                } else {
                    "bundle/resources".to_owned()
                }
            } else {
                format!(
                    "bundle/linux-{}/resources",
                    if cfg!(target_arch = "aarch64") {
                        "arm64"
                    } else {
                        "x64"
                    }
                )
            };
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(directory)
        })
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test]
#[ignore = "Requires bundled resources; verifies remembered modes without OS proxy changes"]
async fn remembered_modes_survive_core_and_client_restart() {
    use clash_of_rust::engine::ProxyMode;
    let resources = test_resources();
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::at(tmp.path().to_owned()).unwrap();
    let mut mixed = free_port();
    let controller = free_port();
    while mixed == controller {
        mixed = free_port();
    }
    store
        .save_settings(&Settings {
            controller_port: controller,
            mixed_port: mixed,
            ..Settings::default()
        })
        .unwrap();
    {
        let mut engine = Engine::with_resources(store.clone(), resources.clone()).unwrap();
        engine.start().await.unwrap();
        engine.select_mode("global").await.unwrap();
        engine.select_proxy_mode(ProxyMode::Off).await.unwrap();
        assert!(engine.select_mode("invalid").await.is_err());
        engine.stop().await.unwrap();
    }
    assert_eq!(store.load_settings().unwrap().run_mode, "global");
    assert_eq!(store.load_settings().unwrap().proxy_mode, ProxyMode::Off);
    let mut engine = Engine::with_resources(store, resources).unwrap();
    engine.start().await.unwrap();
    assert!(!engine.restore_proxy_mode().await.unwrap());
    assert_eq!(engine.poll(Scope::Home).await.unwrap().mode, "global");
    engine.stop().await.unwrap();
}

#[tokio::test]
#[ignore = "Requires bundled resources; starts a real core on dynamic loopback ports"]
async fn real_core_lifecycle_and_configuration() {
    let resources = test_resources();
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::at(tmp.path().to_owned()).unwrap();
    let port = free_port();
    let mut mixed = free_port();
    while mixed == port {
        mixed = free_port();
    }
    store
        .save_settings(&Settings {
            controller_port: port,
            mixed_port: mixed,
            ..Settings::default()
        })
        .unwrap();
    let mut engine = Engine::with_resources(store.clone(), resources).unwrap();
    // A fresh install must start immediately with the bundled direct profile.
    assert_eq!(engine.profiles.len(), 1);
    engine.start().await.unwrap();
    assert!(engine.poll(Scope::Connections).await.unwrap().running);
    assert!(!engine.poll(Scope::Home).await.unwrap().system_proxy);
    engine.stop().await.unwrap();
    let source = tmp.path().join("source.yaml");
    std::fs::write(&source, include_str!("../examples/direct.yaml")).unwrap();
    engine
        .import("Integration".into(), source.display().to_string(), None)
        .await
        .unwrap();
    assert_eq!(engine.profiles.len(), 2);
    let id = engine
        .profiles
        .iter()
        .find(|p| p.name == "Integration")
        .unwrap()
        .id
        .clone();
    engine.activate(id.clone()).await.unwrap();
    engine.start().await.unwrap();
    let snapshot = engine.poll(Scope::Home).await.unwrap();
    assert!(snapshot.running);
    assert!(!snapshot.version.is_empty());
    assert!(!snapshot.tun);
    assert!(!snapshot.system_proxy);
    assert_eq!(snapshot.mode, "rule");
    for mode in ["global", "direct", "rule"] {
        engine.mode(mode).await.unwrap();
        let config: Value = engine.api.get("configs").await.unwrap();
        assert_eq!(config["mode"], mode);
    }
    engine.api.select("Example", "REJECT").await.unwrap();
    let proxies: Proxies = engine.api.get("proxies").await.unwrap();
    assert_eq!(proxies.proxies["Example"].now, "REJECT");
    engine.api.select("Example", "DIRECT").await.unwrap();
    assert_eq!(
        engine.poll(Scope::Rules).await.unwrap().rules.rules.len(),
        2
    );

    // Invalid updates cannot replace the last usable raw or running config.
    let original = std::fs::read_to_string(store.profile_path(&id).unwrap()).unwrap();
    std::fs::write(&source, "proxies: [invalid-node]\nrules: [MATCH,DIRECT]").unwrap();
    assert!(
        engine
            .import(
                "Broken".into(),
                source.display().to_string(),
                Some(id.clone())
            )
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(store.profile_path(&id).unwrap()).unwrap(),
        original
    );
    assert!(engine.poll(Scope::Home).await.unwrap().running);

    // Valid updates retain the mode selected in the currently running core.
    engine.mode("global").await.unwrap();
    std::fs::write(
        &source,
        original.replace("DOMAIN-SUFFIX,example.com", "DOMAIN-SUFFIX,example.org"),
    )
    .unwrap();
    engine
        .import(
            "Updated".into(),
            source.display().to_string(),
            Some(id.clone()),
        )
        .await
        .unwrap();
    let snapshot = engine.poll(Scope::Rules).await.unwrap();
    assert_eq!(snapshot.mode, "global");
    assert_eq!(snapshot.rules.rules[0].payload, "example.org");
    assert_eq!(engine.profiles.len(), 2);
    engine.api.select("Example", "REJECT").await.unwrap();
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let blocked = Settings {
        mixed_port: occupied.local_addr().unwrap().port(),
        ..engine.settings.clone()
    };
    assert!(engine.save_settings(blocked).await.is_err());
    assert_eq!(engine.settings.mixed_port, mixed);
    assert!(engine.running());
    let old_api = engine.api.clone();
    let new_controller = free_port();
    let mut new_mixed = free_port();
    while new_mixed == new_controller {
        new_mixed = free_port();
    }
    engine
        .save_settings(Settings {
            controller_port: new_controller,
            mixed_port: new_mixed,
            ..engine.settings.clone()
        })
        .await
        .unwrap();
    let snapshot = engine.poll(Scope::Proxies).await.unwrap();
    assert!(snapshot.running);
    assert_eq!(snapshot.mode, "global");
    assert_eq!(snapshot.proxies.proxies["Example"].now, "REJECT");
    assert_eq!(store.load_settings().unwrap().mixed_port, new_mixed);
    assert!(old_api.get::<Value>("version").await.is_err());
    engine
        .save_settings(Settings {
            dark: false,
            ..engine.settings.clone()
        })
        .await
        .unwrap();
    assert!(!store.load_settings().unwrap().dark);
    assert_eq!(engine.poll(Scope::Home).await.unwrap().mode, "global");
    engine.stop().await.unwrap();
    assert!(!engine.poll(Scope::Home).await.unwrap().running);
    assert!(engine.api.get::<Value>("version").await.is_err());
    // Stop is idempotent and does not leave a child behind.
    engine.stop().await.unwrap();
}

#[tokio::test]
#[ignore = "Requires bundled resources; verifies active subscription deletion without OS proxy changes"]
async fn delete_active_profile_keeps_core_running() {
    let resources = test_resources();
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::at(tmp.path().join("data")).unwrap();
    let mut settings = Settings {
        controller_port: free_port(),
        mixed_port: free_port(),
        ..Settings::default()
    };
    while settings.controller_port == settings.mixed_port {
        settings.mixed_port = free_port();
    }
    store.save_settings(&settings).unwrap();
    let mut engine = Engine::with_resources(store.clone(), resources).unwrap();
    let default = engine.profiles[0].id.clone();
    assert!(engine.delete_profile(&default).await.is_err());
    let source = tmp.path().join("subscription.yaml");
    std::fs::write(&source, include_str!("../examples/direct.yaml")).unwrap();
    engine
        .import("Delete me".into(), source.display().to_string(), None)
        .await
        .unwrap();
    let id = engine.profiles.last().unwrap().id.clone();
    engine.activate(id.clone()).await.unwrap();
    engine.start().await.unwrap();
    engine.mode("global").await.unwrap();
    engine.delete_profile(&id).await.unwrap();
    assert_eq!(
        engine.settings.active_profile.as_deref(),
        Some(default.as_str())
    );
    assert_eq!(engine.profiles.len(), 1);
    assert!(!store.profile_path(&id).unwrap().exists());
    assert_eq!(store.profiles().unwrap().len(), 1);
    let snapshot = engine.poll(Scope::Home).await.unwrap();
    assert!(snapshot.running);
    assert_eq!(snapshot.mode, "global");
    assert!(engine.delete_profile(&default).await.is_err());
    engine.stop().await.unwrap();
}

#[test]
fn data_directory_is_exclusive() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::at(tmp.path().to_owned()).unwrap();
    let first = store.instance_lock().unwrap();
    assert!(store.instance_lock().is_err());
    drop(first);
    assert!(store.instance_lock().is_ok());
}

#[tokio::test]
#[ignore = "Requires bundled Geo files and mihomo; validates offline DAT/MMDB/ASN startup"]
async fn bundled_geo_rules_start_without_downloads() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::at(tmp.path().to_owned()).unwrap();
    let resources = test_resources();
    let mut settings = Settings {
        controller_port: free_port(),
        mixed_port: free_port(),
        ..Settings::default()
    };
    while settings.mixed_port == settings.controller_port {
        settings.mixed_port = free_port();
    }
    store.save_settings(&settings).unwrap();
    let mut engine = Engine::with_resources(store.clone(), resources).unwrap();
    for mode in [true, false] {
        let source = tmp.path().join("geo.yaml");
        std::fs::write(&source, format!("proxies: []\ngeodata-mode: {mode}\nrules:\n  - GEOSITE,cn,DIRECT\n  - GEOIP,CN,DIRECT\n  - IP-ASN,13335,DIRECT\n  - MATCH,DIRECT\n")).unwrap();
        engine
            .import("Geo".into(), source.display().to_string(), None)
            .await
            .unwrap();
        let id = engine.profiles.last().unwrap().id.clone();
        engine.activate(id).await.unwrap();
        engine.start().await.unwrap();
        assert_eq!(
            engine.poll(Scope::Rules).await.unwrap().rules.rules.len(),
            4
        );
        let snapshot = engine.poll(Scope::Logs).await.unwrap();
        assert!(
            !snapshot
                .logs
                .iter()
                .any(|l| l.to_ascii_lowercase().contains("download")),
            "Unexpected startup download: {:?}",
            snapshot.logs
        );
        engine.stop().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "Requires bundled resources; exercises Geo replacement with active MMDB mappings"]
async fn geo_update_restarts_core_and_restores_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let resources = test_resources();
    let store = Store::at(tmp.path().join("data")).unwrap();
    let mut settings = Settings {
        controller_port: free_port(),
        mixed_port: free_port(),
        ..Settings::default()
    };
    while settings.controller_port == settings.mixed_port {
        settings.mixed_port = free_port();
    }
    store.save_settings(&settings).unwrap();
    let mut engine = Engine::with_resources(store.clone(), resources.clone()).unwrap();
    let source = tmp.path().join("source.yaml");
    std::fs::write(&source, "proxies: []\nproxy-groups:\n  - name: Example\n    type: select\n    proxies: [DIRECT, REJECT]\nrules:\n  - GEOSITE,cn,Example\n  - GEOIP,CN,DIRECT\n  - IP-ASN,13335,DIRECT\n  - MATCH,DIRECT\n").unwrap();
    engine
        .import("Mapped".into(), source.display().to_string(), None)
        .await
        .unwrap();
    engine
        .activate(engine.profiles.last().unwrap().id.clone())
        .await
        .unwrap();
    engine.start().await.unwrap();
    engine.mode("global").await.unwrap();
    engine.api.select("Example", "REJECT").await.unwrap();
    let staged = tmp.path().join("staged");
    std::fs::create_dir(&staged).unwrap();
    for name in clash_of_rust::assets::GEO_FILES
        .into_iter()
        .chain([clash_of_rust::assets::MANIFEST])
    {
        std::fs::copy(resources.join(name), staged.join(name)).unwrap();
    }
    let mut manifest = clash_of_rust::assets::verify_geo(&staged).unwrap();
    manifest.version = "test-update".into();
    std::fs::write(
        staged.join(clash_of_rust::assets::MANIFEST),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    engine.install_geo(&staged).await.unwrap();
    assert_eq!(engine.poll(Scope::Home).await.unwrap().mode, "global");
    let proxies: Proxies = engine.api.get("proxies").await.unwrap();
    assert_eq!(proxies.proxies["Example"].now, "REJECT");
    assert_eq!(engine.geo_manifest.version, "test-update");
    engine.stop().await.unwrap();
    assert_eq!(
        clash_of_rust::assets::verify_geo(&store.runtime())
            .unwrap()
            .version,
        "test-update"
    );
}
