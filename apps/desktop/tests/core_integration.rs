//! Explicit opt-in tests against an official mihomo binary.
//! OS proxy/TUN smoke tests require scripts/test-proxy-modes.py on disposable CI.
use clash_of_rust::{
    config::{Settings, Store},
    engine::{Engine, Scope},
};

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
                    "bundle/{}-{}/resources",
                    if cfg!(target_os = "macos") {
                        "macos"
                    } else {
                        "linux"
                    },
                    if cfg!(target_arch = "aarch64") {
                        "arm64"
                    } else {
                        "x64"
                    }
                )
            };
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(directory)
        })
}

fn free_port() -> u16 {
    use std::sync::atomic::{AtomicU32, Ordering};
    // mihomo's mixed/DNS listeners need the same port for TCP and UDP. Windows
    // can allocate an ephemeral UDP port from a range excluded for TCP (and
    // vice versa), so repeating either allocator can exhaust a short retry
    // budget without reaching a usable port. Probe the unprivileged range
    // directly; keep both sockets bound until their shared port is confirmed.
    // The process offset and cursor spread concurrent test processes/calls.
    const FIRST: u32 = 1024;
    const COUNT: u32 = u16::MAX as u32 + 1 - FIRST;
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let mut last_error = None;
    for _ in 0..COUNT {
        let offset =
            (std::process::id() % COUNT + NEXT.fetch_add(1, Ordering::Relaxed) % COUNT) % COUNT;
        let port = (FIRST + offset) as u16;
        let bindings = (|| -> std::io::Result<_> {
            let tcp = std::net::TcpListener::bind(("127.0.0.1", port))?;
            let udp = std::net::UdpSocket::bind(("127.0.0.1", port))?;
            Ok((tcp, udp))
        })();
        match bindings {
            Ok((_tcp, _udp)) => return port,
            Err(error) => last_error = Some(error),
        }
    }
    panic!("could not allocate a port available for both TCP and UDP: {last_error:?}")
}

#[tokio::test]
#[ignore = "Requires the disposable native proxy/TUN CI wrapper"]
async fn basic_system_proxy_and_tun() {
    use clash_of_rust::{engine::ProxyMode, platform};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    assert_eq!(std::env::var("CLASH_PROXY_SMOKE").as_deref(), Ok("1"));
    #[cfg(windows)]
    {
        assert_eq!(std::env::var("CLASH_EXPECT_UNELEVATED").as_deref(), Ok("1"));
        assert!(
            !platform::is_elevated(),
            "Windows TUN must work in an ordinary GUI token"
        );
        let directory = platform::windows_service::installed_directory().unwrap();
        assert!(
            std::fs::OpenOptions::new()
                .write(true)
                .open(directory.join("clash-tun-launcher.exe"))
                .is_err(),
            "ordinary users must not be able to replace the privileged helper"
        );
        assert!(
            !std::process::Command::new(directory.join("clash-tun-launcher.exe"))
                .arg("--windows-service-install")
                .status()
                .unwrap()
                .success(),
            "ordinary users must not install services without authorization"
        );
    }
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::at(temporary.path().join("data")).unwrap();
    let controller_port = free_port();
    let mut mixed_port = free_port();
    while mixed_port == controller_port {
        mixed_port = free_port();
    }
    store
        .save_settings(&Settings {
            controller_port,
            mixed_port,
            dns_override: false,
            ipv6: false,
            ..Default::default()
        })
        .unwrap();
    let mut engine = Engine::with_resources(store.clone(), test_resources()).unwrap();
    let profile = temporary.path().join("smoke.yaml");
    let device = if cfg!(target_os = "macos") {
        "utun233"
    } else {
        "ClashRustTest"
    };
    // Route only this documentation subnet. The runner's control traffic and
    // Internet default route remain outside TUN even if a test aborts.
    std::fs::write(&profile, format!("proxies: []\nrules: ['IP-CIDR,203.0.113.0/24,REJECT,no-resolve', 'MATCH,DIRECT']\nlog-level: debug\ndns: {{enable: false}}\ntun:\n  device: {device}\n  stack: system\n  route-address: [203.0.113.0/24]\n  dns-hijack: []\n")).unwrap();
    engine
        .import("Proxy smoke".into(), profile.display().to_string(), None)
        .await
        .unwrap();
    engine
        .activate(engine.profiles.last().unwrap().id.clone())
        .await
        .unwrap();
    engine.start().await.unwrap();
    engine.select_proxy_mode(ProxyMode::System).await.unwrap();
    assert!(platform::is_owned(&store.root.join("system-proxy.json")).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nproxy-ok")
            .await
            .unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::http(format!("http://127.0.0.1:{mixed_port}")).unwrap())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    assert_eq!(
        client
            .get(format!("http://{target}/"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "proxy-ok"
    );
    server.await.unwrap();
    engine.select_proxy_mode(ProxyMode::Tun).await.unwrap();
    let state = engine.poll(Scope::Home).await.unwrap();
    assert!(state.running && state.tun && !state.system_proxy);
    engine.poll(Scope::Logs).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let direct = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    assert!(
        direct
            .get("http://203.0.113.2/tun-smoke")
            .send()
            .await
            .is_err()
    );
    let mut captured = false;
    for _ in 0..50 {
        let logs = engine.poll(Scope::Logs).await.unwrap().logs;
        if logs
            .iter()
            .any(|line| line.contains("203.0.113.2") && line.contains("REJECT"))
        {
            captured = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        captured,
        "Unproxied traffic was not captured and routed by TUN"
    );
    #[cfg(windows)]
    {
        // Preserve TUN preference, but discard every in-memory Engine/session
        // flag. Reopening must recover TUN without relaunching the GUI as admin.
        engine.stop().await.unwrap();
        drop(engine);
        engine = Engine::with_resources(store.clone(), test_resources()).unwrap();
        engine.start().await.unwrap();
        engine.restore_proxy_mode().await.unwrap();
        let state = engine.poll(Scope::Home).await.unwrap();
        assert!(state.running && state.tun && !state.system_proxy);
        assert!(!platform::is_elevated());
        if let Some(path) = std::env::var_os("CLASH_SERVICE_CRASH_REPORT") {
            clash_of_rust::config::atomic_write(
                std::path::Path::new(&path),
                &serde_json::to_vec(&serde_json::json!({
                    "pid":std::process::id(), "controller":controller_port, "mixed":mixed_port,
                    "temporary":temporary.path()
                }))
                .unwrap(),
            )
            .unwrap();
            // The test driver kills this ordinary process, leaving the service
            // to notice its dead owner and reap the privileged core by itself.
            std::future::pending::<()>().await;
        }
    }
    engine.select_proxy_mode(ProxyMode::Off).await.unwrap();
    let state = engine.poll(Scope::Home).await.unwrap();
    assert!(!state.tun && !state.system_proxy);
    engine.stop().await.unwrap();
    assert!(!engine.running());
    assert!(!store.root.join("system-proxy.json").exists());
    println!("PASS: native system proxy, HTTP forwarding, TUN capture and cleanup");
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
