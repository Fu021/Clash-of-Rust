//! Exercises production DNS URLs over a real peer interface in an isolated VM.
use clash_of_rust::{
    config::Settings,
    engine::{Engine, ProxyMode, Scope},
};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn metrics() -> serde_json::Value {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
        .get("http://192.0.2.254:18080/metrics")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn query(host: &str) -> Vec<u8> {
    let mut packet = vec![0x49, 0x52, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in host.split('.') {
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.extend_from_slice(&[0, 0, 1, 0, 1]);
    packet
}

async fn hijacked_dns(host: &str, tcp: bool) -> IpAddr {
    // This IP belongs to the namespace upstream, whose plaintext DNS counts
    // leaks. TUN must answer the request without contacting that server.
    let packet = query(host);
    let reply = if tcp {
        let mut stream = tokio::net::TcpStream::connect("8.8.8.8:53").await.unwrap();
        stream.write_u16(packet.len() as u16).await.unwrap();
        stream.write_all(&packet).await.unwrap();
        let size = stream.read_u16().await.unwrap();
        let mut bytes = vec![0; usize::from(size)];
        stream.read_exact(&mut bytes).await.unwrap();
        bytes
    } else {
        let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await.unwrap();
        socket.send_to(&packet, "8.8.8.8:53").await.unwrap();
        let mut bytes = vec![0; 4096];
        let count = socket.recv(&mut bytes).await.unwrap();
        bytes.truncate(count);
        bytes
    };
    assert_eq!(&reply[..2], &packet[..2]);
    assert_eq!(reply[3] & 15, 0, "DNS response: {reply:?}");
    assert!(
        reply.len() >= packet.len() + 16,
        "Missing DNS answer: {reply:?}"
    );
    let address = &reply[reply.len() - 4..];
    std::net::Ipv4Addr::new(address[0], address[1], address[2], address[3]).into()
}

async fn protected_traffic(engine: &mut Engine) {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let addresses = engine
        .api
        .resolve_domain(&format!("{suffix}.fixture.test"))
        .await
        .unwrap();
    assert!(addresses.contains(&"203.0.113.20".parse().unwrap()));
    assert!(addresses.contains(&"2001:db8:2::20".parse().unwrap()));
    // Exercise both actual OS DNS transports, not just the controller API.
    for tcp in [false, true] {
        let ip = tokio::time::timeout(Duration::from_secs(8), hijacked_dns("claude.ai", tcp))
            .await
            .expect("TUN DNS hijack timed out");
        assert!(
            ip.to_string().starts_with("198.18."),
            "Expected fake IP: {ip}"
        );
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(8))
            .resolve("claude.ai", SocketAddr::new(ip, 80))
            .build()
            .unwrap();
        assert_eq!(
            client
                .get("http://claude.ai/tun-check")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "protected-ok"
        );
    }
    let actual: serde_json::Value = engine.api.get("configs").await.unwrap();
    assert_eq!(actual["tun"]["enable"], true);
    let protection = engine.settings.claude_protection.as_ref().unwrap();
    assert_eq!(actual["mode"], protection.run_mode());
    if protection.run_mode() == "global" {
        let global = engine.api.proxy_nodes("GLOBAL").await.unwrap();
        assert_eq!(global.now.as_str(), protection.node);
        assert_eq!(global.all.len(), 1);
    }
    let auto = engine.api.proxy_nodes("Auto").await.unwrap();
    assert_eq!(auto.kind.as_str(), "Selector");
    assert_eq!(
        auto.all.iter().map(|n| n.as_str()).collect::<Vec<_>>(),
        vec![protection.node.as_str()]
    );
    assert_eq!(auto.now.as_str(), protection.node);
    assert!(
        engine.select_node("Auto", "Other").await.is_err(),
        "Protected groups must reject node changes"
    );
    for mode in ["rule", "global", "direct"] {
        assert!(engine.select_mode(mode).await.is_err());
    }
}

pub async fn run(engine: &mut Engine, profile: &std::path::Path) {
    assert_eq!(
        std::env::var("CLASH_TUN_TEST_NAMESPACE").as_deref(),
        Ok("1")
    );
    engine.select_proxy_mode(ProxyMode::Off).await.unwrap();
    engine
        .save_settings(Settings {
            ipv6: true,
            ..engine.settings.clone()
        })
        .await
        .unwrap();
    for (node, fixed_blocked, explicit, resolver_hostname) in [
        ("node.fixture.test", false, false, false),
        ("node6.fixture.test", false, false, false),
        ("node.fixture.test", true, true, false),
        ("node6.fixture.test", true, false, false),
        ("node6.fixture.test", true, true, true),
    ] {
        let bootstrap = if resolver_hostname {
            "resolver.fixture.test"
        } else if fixed_blocked {
            "192.0.2.254"
        } else {
            "223.5.5.5"
        };
        let node_dns = if explicit {
            format!("  proxy-server-nameserver: ['https://{bootstrap}/dns-query#DIRECT']\n")
        } else {
            String::new()
        };
        let default_dns = if resolver_hostname {
            "  default-nameserver: ['udp://192.0.2.254:53']\n"
        } else {
            ""
        };
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!(
                "http://192.0.2.254:18080/block-fixed-bootstrap/{}",
                if fixed_blocked { "on" } else { "off" }
            ))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        std::fs::write(profile, format!("proxies: [{{name: fixture, type: socks5, server: {node}, port: 1080, udp: true}}, {{name: Other, type: socks5, server: {node}, port: 1081, udp: true}}]\nproxy-groups: [{{name: Main, type: select, proxies: [Auto]}}, {{name: Auto, type: url-test, proxies: [fixture, Other], url: 'http://claude.ai/auto-check', interval: 3600, lazy: true}}]\nrules: ['MATCH,Main']\ndns:\n  enable: true\n  enhanced-mode: fake-ip\n  nameserver: ['https://{bootstrap}/dns-query#DIRECT']\n{node_dns}{default_dns}  fake-ip-filter: ['+.fixture.test']\ntun: {{device: ClashRustTest, route-address: [203.0.113.0/24], dns-hijack: []}}\n")).unwrap();
        engine
            .import(
                format!("DNS uplink {node}"),
                profile.display().to_string(),
                None,
            )
            .await
            .unwrap();
        // Activate the fixture with the mode already saved, so the runtime
        // file baseline includes it (select_mode itself only patches the API).
        let mode = if explicit { "global" } else { "rule" };
        engine.select_mode(mode).await.unwrap();
        engine
            .activate(engine.profiles.last().unwrap().id.clone())
            .await
            .unwrap();
        engine.select_proxy_mode(ProxyMode::System).await.unwrap();
        if mode == "global" {
            engine.select_node("GLOBAL", "DIRECT").await.unwrap();
            let before = std::fs::read(engine.store.root.join("settings.json")).unwrap();
            let rejected = engine.claude_protection(true).await.unwrap_err();
            assert!(rejected.to_string().contains("DIRECT"));
            assert!(engine.settings.claude_protection.is_none());
            assert_eq!(
                before,
                std::fs::read(engine.store.root.join("settings.json")).unwrap()
            );
            engine.select_node("GLOBAL", "Main").await.unwrap();
        }
        // Real URLTest API: release a manual choice after bounded probes at
        // the subscription's own URL, rather than permanently PUT the winner.
        engine.api.select("Auto", "Other").await.unwrap();
        let controller = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(8))
            .build()
            .unwrap();
        let auto_url = format!(
            "http://127.0.0.1:{}/proxies/Auto",
            engine.settings.controller_port
        );
        let forced: serde_json::Value = controller
            .get(&auto_url)
            .bearer_auth(&engine.settings.secret)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(forced["fixed"], "Other");
        let measured = engine.api.delay_group("Auto").await.unwrap();
        let auto: serde_json::Value = controller
            .get(&auto_url)
            .bearer_auth(&engine.settings.secret)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(auto["fixed"], "");
        assert_eq!(auto["testUrl"], "http://claude.ai/auto-check");
        assert!(measured["fixture"] > 0 && measured["Other"] > 0);
        assert!(
            measured["fixture"] < measured["Other"],
            "Fixture must have a slower alternative"
        );
        assert_eq!(
            auto["now"], "fixture",
            "Manual group testing must restore automatic selection"
        );
        assert_eq!(measured["Auto"], measured[auto["now"].as_str().unwrap()]);
        let store = engine.store.clone();
        let settings = std::fs::read(store.root.join("settings.json")).unwrap();
        let runtime = std::fs::read(store.runtime().join("config.yaml")).unwrap();
        let before = metrics().await;
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(
                reqwest::Proxy::http(format!("http://127.0.0.1:{}", engine.settings.mixed_port))
                    .unwrap(),
            )
            .timeout(Duration::from_secs(8))
            .build()
            .unwrap();
        assert_eq!(
            client
                .get("http://claude.ai/before")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "protected-ok"
        );
        engine.claude_protection(true).await.unwrap();
        // No test-only DNS URL, hosts entry, restricted TUN route or API reload:
        // the exact production overlay and native engine transition are tested.
        let payload: serde_yaml::Value =
            serde_yaml::from_slice(&std::fs::read(store.runtime().join("config.yaml")).unwrap())
                .unwrap();
        assert!(payload["tun"]["route-address"].is_null());
        let result = tokio::time::timeout(Duration::from_secs(30), protected_traffic(engine)).await;
        if result.is_err() {
            eprintln!(
                "Protected DNS timeout; logs: {:?}",
                engine.poll(Scope::Logs).await.unwrap().logs
            );
        }
        result.expect("Protected DNS/HTTP must remain reachable");
        engine.stop().await.unwrap();
        engine.start().await.unwrap();
        engine.restore_proxy_mode().await.unwrap();
        tokio::time::timeout(Duration::from_secs(30), protected_traffic(engine))
            .await
            .expect("Cold-start protected DNS/HTTP must work");
        engine.claude_protection(false).await.unwrap();
        let auto: serde_json::Value = controller
            .get(&auto_url)
            .bearer_auth(&engine.settings.secret)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(auto["type"], "URLTest");
        assert_eq!(
            auto["fixed"], "",
            "Disabling protection must restore automatic selection after restart"
        );
        assert_eq!(
            settings,
            std::fs::read(store.root.join("settings.json")).unwrap()
        );
        assert_eq!(
            runtime,
            std::fs::read(store.runtime().join("config.yaml")).unwrap()
        );
        let state = engine.poll(Scope::Home).await.unwrap();
        assert!(state.running && state.system_proxy && !state.tun);
        assert_eq!(state.mode, mode);
        if mode == "global" {
            assert_eq!(
                engine.api.proxy_nodes("GLOBAL").await.unwrap().now.as_str(),
                "Main"
            );
        }
        let after = metrics().await;
        for key in ["bootstrap", "proxied_dns", "node_connections", "web"] {
            assert!(
                after[key].as_u64().unwrap() > before[key].as_u64().unwrap(),
                "Missing traffic for {key}: {after}"
            );
        }
        assert_eq!(
            after["direct_public_dns"], 0,
            "Public DNS bypassed the node: {after}"
        );
        assert_eq!(after["plaintext_dns"], 0, "Plaintext DNS leaked: {after}");
        assert_eq!(
            after["bootstrap_public_lookup"], 0,
            "Public name leaked to bootstrap: {after}"
        );
        if resolver_hostname {
            assert!(
                after["resolver_bootstrap"].as_u64().unwrap()
                    > before["resolver_bootstrap"].as_u64().unwrap()
            );
        }
        println!(
            "PASS: {node}, fixed DNS blocked={fixed_blocked}, explicit node DNS={explicit}, resolver hostname={resolver_hostname}: bootstrap DoH, pinned DoH A/AAAA, UDP/TCP TUN DNS, HTTP, cold restart and exact restore; {after}"
        );
        engine.select_proxy_mode(ProxyMode::Off).await.unwrap();
    }
    engine.select_proxy_mode(ProxyMode::System).await.unwrap();
    let settings = std::fs::read(engine.store.root.join("settings.json")).unwrap();
    let runtime = std::fs::read(engine.store.runtime().join("config.yaml")).unwrap();
    let control = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    control
        .get("http://192.0.2.254:18080/block-public-dns/on")
        .send()
        .await
        .unwrap();
    let result = engine.claude_protection(true).await;
    assert!(
        result.is_err(),
        "A broken DNS path must not commit protection as enabled"
    );
    assert!(format!("{:#}", result.unwrap_err()).contains("加密 DNS"));
    assert!(engine.settings.claude_protection.is_none());
    assert_eq!(
        settings,
        std::fs::read(engine.store.root.join("settings.json")).unwrap()
    );
    assert_eq!(
        runtime,
        std::fs::read(engine.store.runtime().join("config.yaml")).unwrap()
    );
    let state = engine.poll(Scope::Home).await.unwrap();
    assert!(state.running && state.system_proxy && !state.tun);
    control
        .get("http://192.0.2.254:18080/block-public-dns/off")
        .send()
        .await
        .unwrap();
    println!(
        "PASS: unreachable protected DoH rejects activation and restores system proxy and configuration"
    );
}
