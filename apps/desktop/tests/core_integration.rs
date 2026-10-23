//! Explicit opt-in tests against an official mihomo binary.
//! OS proxy/TUN smoke tests require scripts/test-proxy-modes.py on disposable CI.
use clash_of_rust::{
    config::{Settings, Store},
    engine::{Engine, Scope},
};

#[cfg(target_os = "linux")]
#[path = "support/protected_dns.rs"]
mod protected_dns;

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

fn assert_system_proxy_ports(store: &Store, port: u16) {
    use clash_of_rust::platform::{self, ProxyState, ProxyValue};
    let path = store.root.join("system-proxy.json");
    assert!(
        platform::is_owned(&path).unwrap(),
        "native settings must match the lease"
    );
    let journal: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let owned: ProxyState = serde_json::from_value(journal["owned"].clone()).unwrap();
    #[cfg(target_os = "linux")]
    for protocol in ["http", "https", "socks"] {
        assert_eq!(
            owned[&format!("org.gnome.system.proxy.{protocol}|host")],
            Some(ProxyValue::Text("'127.0.0.1'".into()))
        );
        assert_eq!(
            owned[&format!("org.gnome.system.proxy.{protocol}|port")],
            Some(ProxyValue::Text(port.to_string()))
        );
    }
    #[cfg(windows)]
    assert_eq!(
        owned["ProxyServer"],
        Some(ProxyValue::Text(format!("127.0.0.1:{port}")))
    );
    #[cfg(target_os = "macos")]
    for service in owned.values() {
        let Some(ProxyValue::Text(text)) = service else {
            panic!("invalid service proxy")
        };
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        for protocol in ["HTTP", "HTTPS", "SOCKS"] {
            assert_eq!(value[format!("{protocol}Enable")], 1);
            assert_eq!(value[format!("{protocol}Proxy")], "127.0.0.1");
            assert_eq!(value[format!("{protocol}Port")], port);
        }
    }
}

#[tokio::test]
#[ignore = "Requires the disposable native proxy/TUN CI wrapper"]
async fn basic_system_proxy_and_tun() {
    use clash_of_rust::engine::ProxyMode;
    #[cfg(windows)]
    use clash_of_rust::platform;
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
    let dns = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let dns_address = dns.local_addr().unwrap();
    let dns_server = tokio::spawn(async move {
        for _ in 0..2 {
            let mut packet = [0; 4096];
            let (count, peer) = tokio::time::timeout(
                std::time::Duration::from_secs(20),
                dns.recv_from(&mut packet),
            )
            .await
            .unwrap()
            .unwrap();
            let mut end = 12;
            while end < count && packet[end] != 0 {
                end += usize::from(packet[end]) + 1;
            }
            end += 5;
            assert!(end <= count);
            let kind = u16::from_be_bytes([packet[end - 4], packet[end - 3]]);
            let mut response = packet[..end].to_vec();
            response[2..12].copy_from_slice(&[0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0]);
            response.extend_from_slice(&[0xc0, 0x0c]);
            response.extend_from_slice(&kind.to_be_bytes());
            response.extend_from_slice(&[0, 1, 0, 0, 0, 0]);
            let address: std::net::IpAddr = if kind == 1 {
                "203.0.113.7"
            } else {
                "2001:db8::7"
            }
            .parse()
            .unwrap();
            let bytes = match address {
                std::net::IpAddr::V4(ip) => ip.octets().to_vec(),
                std::net::IpAddr::V6(ip) => ip.octets().to_vec(),
            };
            response.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            response.extend_from_slice(&bytes);
            dns.send_to(&response, peer).await.unwrap();
        }
    });
    // Route only this documentation subnet. The runner's control traffic and
    // Internet default route remain outside TUN even if a test aborts.
    std::fs::write(&profile, format!("proxies: []\nrules: ['IP-CIDR,203.0.113.0/24,REJECT,no-resolve', 'MATCH,DIRECT']\nlog-level: debug\ndns: {{enable: true, enhanced-mode: redir-host, nameserver: ['{dns_address}']}}\ntun:\n  device: {device}\n  stack: system\n  route-address: [203.0.113.0/24]\n  dns-hijack: []\n")).unwrap();
    engine
        .import("Proxy smoke".into(), profile.display().to_string(), None)
        .await
        .unwrap();
    engine
        .activate(engine.profiles.last().unwrap().id.clone())
        .await
        .unwrap();
    let conflicting_controller =
        std::net::TcpListener::bind(("127.0.0.1", controller_port)).unwrap();
    engine.start().await.unwrap();
    assert_ne!(engine.settings.controller_port, controller_port);
    assert_eq!(
        store.load_settings().unwrap().controller_port,
        engine.settings.controller_port
    );
    assert!(std::net::TcpListener::bind(("127.0.0.1", controller_port)).is_err());
    drop(conflicting_controller);
    let addresses = engine
        .api
        .resolve_domain("connection.example.test")
        .await
        .unwrap();
    assert!(addresses.contains(&"203.0.113.7".parse().unwrap()));
    assert!(addresses.contains(&"2001:db8::7".parse().unwrap()));
    dns_server.await.unwrap();
    println!("PASS: real mihomo DNS API returns IPv4 and IPv6 for connection details");
    engine.select_proxy_mode(ProxyMode::System).await.unwrap();
    assert_system_proxy_ports(&store, mixed_port);
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
    println!("PASS: native system proxy and HTTP forwarding");
    mixed_port = free_port();
    while mixed_port == engine.settings.controller_port {
        mixed_port = free_port();
    }
    engine.save_ports(mixed_port).await.unwrap();
    assert_system_proxy_ports(&store, mixed_port);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nsocks-ok")
            .await
            .unwrap();
    });
    let socks = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(format!("socks5h://127.0.0.1:{mixed_port}")).unwrap())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    assert_eq!(
        socks
            .get(format!("http://{target}/"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "socks-ok"
    );
    server.await.unwrap();
    println!("PASS: changing mixed port updates native proxy endpoints and SOCKS forwards traffic");
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
    println!("PASS: unproxied traffic captured by TUN");
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
                    "pid":std::process::id(), "controller":engine.settings.controller_port, "mixed":mixed_port,
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
    #[cfg(target_os = "linux")]
    {
        use futures_util::FutureExt;
        if let Err(panic) = std::panic::AssertUnwindSafe(protected_dns::run(&mut engine, &profile))
            .catch_unwind()
            .await
        {
            for line in engine.poll(Scope::Logs).await.unwrap().logs {
                eprintln!("{line}");
            }
            std::panic::resume_unwind(panic);
        }
    }
    engine.select_proxy_mode(ProxyMode::Off).await.unwrap();
    let state = engine.poll(Scope::Home).await.unwrap();
    assert!(!state.tun && !state.system_proxy);
    engine.stop().await.unwrap();
    assert!(!engine.running());
    assert!(!store.root.join("system-proxy.json").exists());
    println!("CLI smoke: creating the shared control endpoint");
    // Exercise the same native proxy/TUN lifecycle through the authenticated
    // console transport, including a GUI facade attaching without a second core.
    let engine = std::sync::Arc::new(tokio::sync::Mutex::new(engine));
    clash_of_rust::control::serve(&engine, clash_of_rust::control::Owner::Daemon)
        .await
        .unwrap();
    let remote = clash_of_rust::control::Remote::discover(&store, Duration::from_secs(60))
        .unwrap()
        .unwrap();
    use clash_of_rust::control::{Command as Control, Meta};
    let _: serde_json::Value = remote.request(Control::Start).await.unwrap();
    println!("CLI smoke: shared owner started; attaching GUI facade");
    let attach_store = store.clone();
    let mut facade =
        tokio::task::spawn_blocking(move || Engine::with_resources(attach_store, test_resources()))
            .await
            .unwrap()
            .unwrap();
    assert!(facade.is_remote() && facade.running());
    let original_revision = facade.view_revision();
    let cli =
        std::env::var_os("MIHOMO_TEST_CLI").expect("native wrapper must provide the bundled CLI");
    async fn console(
        executable: &std::ffi::OsStr,
        store: &Store,
        args: &[&str],
    ) -> serde_json::Value {
        println!("CLI smoke: {args:?}");
        let output = tokio::time::timeout(
            Duration::from_secs(45),
            tokio::process::Command::new(executable)
                .kill_on_drop(true)
                .arg("--data-dir")
                .arg(&store.root)
                .args(["--json", "--yes", "--timeout", "30"])
                .args(args)
                .output(),
        )
        .await
        .expect("console did not exit within 45 seconds")
        .unwrap();
        assert!(
            output.status.success(),
            "CLI {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["data"].take()
    }
    let status = console(&cli, &store, &["status"]).await;
    assert_eq!(status["running"], true);
    console(&cli, &store, &["config", "set", "dark", "true"]).await;
    facade.poll(Scope::Other).await.unwrap();
    assert!(facade.settings.dark);
    assert_ne!(facade.view_revision(), original_revision);
    assert_eq!(
        console(&cli, &store, &["config", "get", "dark"]).await["dark"],
        true
    );
    let secret = console(&cli, &store, &["config", "get", "secret"]).await;
    assert_eq!(secret["secret"], "[已隐藏]");
    let meta: Meta = remote.request(Control::Meta).await.unwrap();
    let active = meta.settings.active_profile.clone().unwrap();
    console(
        &cli,
        &store,
        &["profile", "edit", &active, "--name", "CLI renamed"],
    )
    .await;
    facade.poll(Scope::Other).await.unwrap();
    assert!(
        facade
            .profiles
            .iter()
            .any(|profile| profile.id == active && profile.name == "CLI renamed")
    );
    let builtin = meta
        .profiles
        .iter()
        .find(|p| p.id == active)
        .unwrap()
        .is_default();
    let previous =
        clash_of_rust::rule_manager::ProfileRules::for_profile(&meta.settings, &active, builtin);
    let revision = clash_of_rust::rule_manager::revision(&previous).unwrap();
    console(&cli, &store, &["rule", "add", "DOMAIN,cli.test,REJECT"]).await;
    facade.poll(Scope::Other).await.unwrap();
    assert!(
        clash_of_rust::rule_manager::ProfileRules::for_profile(&facade.settings, &active, builtin)
            .custom
            .iter()
            .any(|rule| rule.rule == "DOMAIN,cli.test,REJECT")
    );
    let conflict = engine
        .lock()
        .await
        .save_profile_rules_checked(&active, previous.as_ref().clone(), Some(revision))
        .await;
    assert!(
        conflict.is_err(),
        "a stale GUI draft must not replace a console edit"
    );
    let rules = console(&cli, &store, &["rule", "list"]).await;
    let rule = rules["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["rule"] == "DOMAIN,cli.test,REJECT")
        .unwrap();
    console(
        &cli,
        &store,
        &["rule", "remove", rule["id"].as_str().unwrap()],
    )
    .await;
    for mode in [ProxyMode::System, ProxyMode::Tun, ProxyMode::Off] {
        let _: serde_json::Value = remote.request(Control::Proxy { mode }).await.unwrap();
        let meta: Meta = remote.request(Control::Meta).await.unwrap();
        assert!(meta.running);
        assert_eq!(meta.settings.proxy_mode, mode);
    }
    drop(facade); // Releasing an attached GUI must not stop its owner.
    assert!(engine.lock().await.running());
    println!(
        "CLI smoke: GUI settings, profiles and rules synchronized; facade released without stopping the owner"
    );
    let _: serde_json::Value = remote.request(Control::Shutdown).await.unwrap();
    assert!(!engine.lock().await.running());
    assert!(!store.root.join("system-proxy.json").exists());
    let attach_store = store.clone();
    let mut orphaned =
        tokio::task::spawn_blocking(move || Engine::with_resources(attach_store, test_resources()))
            .await
            .unwrap()
            .unwrap();
    drop(engine);
    assert!(orphaned.poll(Scope::Other).await.is_err());
    assert!(
        !orphaned.running(),
        "a dead owner must not leave an attached GUI marked running"
    );
    drop(orphaned);
    // With the GUI owner gone, the same binary must create and clean up its own
    // headless owner rather than relying on the test process's runtime.
    let resources = test_resources();
    println!("CLI smoke: starting a standalone owner");
    #[cfg(not(windows))]
    let output = tokio::process::Command::new(&cli)
        .arg("--data-dir")
        .arg(&store.root)
        .arg("--resources")
        .arg(resources)
        .args(["--json", "start"])
        .output()
        .await
        .unwrap();
    #[cfg(windows)]
    let output = tokio::process::Command::new(
        std::path::Path::new(&cli).with_file_name("clash-of-rust.exe"),
    )
    .arg("--cli-background")
    .arg("--data-dir")
    .arg(&store.root)
    .arg("--resources")
    .arg(resources)
    .output()
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut started = false;
    for _ in 0..100 {
        if console(&cli, &store, &["status"]).await["running"] == true {
            started = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        started,
        "the CLI login/start entry did not start its own core"
    );
    let gui = clash_of_rust::control::GuiSession::register(store.clone()).unwrap();
    console(&cli, &store, &["stop"]).await;
    assert!(
        store.root.join("control.json").exists(),
        "an attached GUI must retain a usable backend"
    );
    assert_eq!(console(&cli, &store, &["status"]).await["running"], false);
    drop(gui);
    console(&cli, &store, &["stop"]).await;
    assert!(
        !store.root.join("control.json").exists(),
        "cor stop returned before its independent owner finished cleanup"
    );
    println!("PASS: native system proxy, HTTP forwarding, TUN capture and cleanup");
}
