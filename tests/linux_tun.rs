//! Run only through scripts/test-linux-tun.py, in a disposable network namespace.
#![cfg(target_os = "linux")]
use clash_of_rust::{
    api::Proxies,
    assets,
    config::{Settings, Store},
    engine::{Engine, ProxyMode, Scope},
};
use serde_json::Value;
use std::time::Duration;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn adapter_exists() -> bool {
    // Query the current network namespace; the host's sysfs mount may show
    // the host namespace even after unshare --net.
    let output = std::process::Command::new("ip")
        .args(["-json", "-details", "link", "show", "dev", "ClashRustTest"])
        .output()
        .unwrap();
    if !output.status.success() {
        return false;
    }
    let devices: Value = serde_json::from_slice(&output.stdout).unwrap();
    devices[0]["linkinfo"]["info_kind"] == "tun"
}

async fn assert_dns_configured(engine: &mut Engine) {
    // This bus has a real systemd-resolved and no PolicyKit service or agent.
    // All three asynchronous core DNS calls must work without a password.
    let mut diagnostics = Vec::new();
    for _ in 0..50 {
        let values: Vec<_> = ["dns", "domain", "default-route"]
            .into_iter()
            .map(|command| {
                let output = std::process::Command::new("/usr/bin/resolvectl")
                    .args([command, "ClashRustTest"])
                    .output()
                    .unwrap();
                (
                    output.status.success(),
                    String::from_utf8_lossy(&output.stdout).into_owned(),
                    String::from_utf8_lossy(&output.stderr).into_owned(),
                )
            })
            .collect();
        if values.iter().all(|value| value.0)
            && values[0].1.contains("198.18.0.2")
            && values[1].1.contains("~.")
            && values[2].1.contains("yes")
        {
            return;
        }
        diagnostics = values;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let logs = engine.poll(Scope::Logs).await.unwrap().logs;
    panic!(
        "TUN DNS setup did not succeed without PolicyKit authentication: {diagnostics:?}\nCore logs: {logs:?}"
    );
}

#[tokio::test]
#[ignore = "Requires installed/authorized core and an isolated network namespace"]
async fn ordinary_user_tun_switch_restart_and_cleanup() {
    assert_eq!(
        std::env::var("CLASH_TUN_TEST_NAMESPACE").as_deref(),
        Ok("1")
    );
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "The client must run as an ordinary user"
    );
    let resources = std::path::PathBuf::from("/opt/clash-of-rust/resources");
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
    let mut engine = Engine::with_resources(store.clone(), resources.clone()).unwrap();
    let profile = tmp.path().join("source.yaml");
    std::fs::write(&profile, "proxies: []\nproxy-groups:\n  - name: Example\n    type: select\n    proxies: [DIRECT, REJECT]\nrules:\n  - MATCH,Example\ntun:\n  device: ClashRustTest\n").unwrap();
    engine
        .import("TUN test".into(), profile.display().to_string(), None)
        .await
        .unwrap();
    engine
        .activate(engine.profiles.last().unwrap().id.clone())
        .await
        .unwrap();
    engine.start().await.unwrap();
    engine.api.select("Example", "REJECT").await.unwrap();
    engine.select_proxy_mode(ProxyMode::System).await.unwrap();
    engine.select_proxy_mode(ProxyMode::Tun).await.unwrap();
    assert!(adapter_exists());
    assert_dns_configured(&mut engine).await;
    let state = engine.poll(Scope::Home).await.unwrap();
    assert!(state.tun && !state.system_proxy);
    let routes = std::process::Command::new("ip")
        .args(["route", "show", "table", "all"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&routes.stdout).contains("ClashRustTest"));

    // No explicit proxy: only TUN can bring this isolated outbound attempt to mihomo.
    let direct = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    assert!(
        direct
            .get("http://203.0.113.2/tun-test")
            .send()
            .await
            .is_err()
    );
    for _ in 0..30 {
        let logs = engine.poll(Scope::Logs).await.unwrap().logs;
        if logs.iter().any(|line| line.contains("203.0.113.2")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        engine
            .poll(Scope::Logs)
            .await
            .unwrap()
            .logs
            .iter()
            .any(|line| line.contains("203.0.113.2"))
    );

    // Configuration and core restarts retain TUN and the selected node.
    engine
        .activate(engine.settings.active_profile.clone().unwrap())
        .await
        .unwrap();
    let mut changed = engine.settings.clone();
    changed.mixed_port = free_port();
    while changed.mixed_port == changed.controller_port {
        changed.mixed_port = free_port();
    }
    engine.save_settings(changed).await.unwrap();
    assert!(engine.poll(Scope::Home).await.unwrap().tun && adapter_exists());
    assert_dns_configured(&mut engine).await;
    let staged = tmp.path().join("staged");
    std::fs::create_dir(&staged).unwrap();
    for name in assets::GEO_FILES.into_iter().chain([assets::MANIFEST]) {
        std::fs::copy(resources.join(name), staged.join(name)).unwrap();
    }
    engine.install_geo(&staged).await.unwrap();
    assert!(engine.poll(Scope::Home).await.unwrap().tun && adapter_exists());
    assert_dns_configured(&mut engine).await;
    let proxies: Proxies = engine.api.get("proxies").await.unwrap();
    assert_eq!(proxies.proxies["Example"].now, "REJECT");
    engine.stop().await.unwrap();
    assert!(!adapter_exists());
    drop(engine);

    let mut engine = Engine::with_resources(store, resources).unwrap();
    engine.start().await.unwrap();
    assert!(
        !engine.restore_proxy_mode().await.unwrap(),
        "Linux must keep the GUI running"
    );
    assert!(engine.poll(Scope::Home).await.unwrap().tun && adapter_exists());
    assert_dns_configured(&mut engine).await;
    engine.select_proxy_mode(ProxyMode::System).await.unwrap();
    let state = engine.poll(Scope::Home).await.unwrap();
    assert!(!state.tun && state.system_proxy && !adapter_exists());
    for _ in 0..3 {
        engine.select_proxy_mode(ProxyMode::Tun).await.unwrap();
        assert!(adapter_exists());
        assert_dns_configured(&mut engine).await;
        engine.select_proxy_mode(ProxyMode::System).await.unwrap();
        assert!(!adapter_exists());
    }
    engine.select_proxy_mode(ProxyMode::Off).await.unwrap();
    engine.stop().await.unwrap();
    assert!(!adapter_exists());
}
