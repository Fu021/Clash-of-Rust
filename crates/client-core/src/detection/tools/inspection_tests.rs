use super::*;
use serde_json::json;

fn offline() -> Environment {
    Environment {
        port: None,
        api: None,
        protected: false,
        udp_enabled: true,
        data_dir: None,
    }
}

#[test]
fn static_rules_stop_at_unknown_conditions_and_respect_modes_and_boundaries() {
    for (rule, host, expected) in [
        (
            "DOMAIN-SUFFIX,example.test,Proxy",
            "sub.example.test",
            "Proxy",
        ),
        (
            "DOMAIN-SUFFIX,example.test,Proxy",
            "badexample.test",
            "DIRECT",
        ),
        ("DOMAIN-KEYWORD,EXAMPLE,Proxy", "Example.Test", "Proxy"),
        ("IP-CIDR,192.0.2.0/24,Proxy", "192.0.2.5", "Proxy"),
        ("IP-CIDR,0.0.0.0/0,Proxy", "198.51.100.5", "Proxy"),
        ("IP-CIDR6,2001:db8::/32,Proxy", "2001:db8::1", "Proxy"),
        ("DST-PORT,443,Proxy", "example.test", "Proxy"),
        ("NETWORK,TCP,Proxy", "example.test", "Proxy"),
    ] {
        let raw = format!("mode: rule\nrules: ['{rule}', 'MATCH,DIRECT']");
        let entries = rule_preview::preview(&raw, host, 443, false).unwrap();
        assert!(
            entries.last().unwrap().detail.ends_with(expected),
            "{entries:?}"
        );
    }
    for rule in [
        "RULE-SET,private,DIRECT",
        "GEOIP,CN,DIRECT",
        "PROCESS-NAME,app,DIRECT",
        "IP-CIDR,192.0.2.0/24,DIRECT",
    ] {
        let entries = rule_preview::preview(
            &format!("rules: ['{rule}', 'MATCH,Proxy']"),
            "example.test",
            443,
            false,
        )
        .unwrap();
        assert_eq!(entries.last().unwrap().code, "rule_unknown");
        assert!(!entries.last().unwrap().detail.contains("private"));
    }
    for (mode, policy) in [("direct", "DIRECT"), ("global", "GLOBAL")] {
        let entries = rule_preview::preview(
            &format!("mode: {mode}\nrules: ['MATCH,Proxy']"),
            "example.test",
            443,
            false,
        )
        .unwrap();
        assert!(entries.last().unwrap().detail.contains(policy));
    }
    let protected =
        rule_preview::preview("rules: ['MATCH,DIRECT']", "claude.ai", 443, true).unwrap();
    assert_eq!(protected.last().unwrap().code, "rule_unknown");
}

#[test]
fn config_diff_never_emits_values_or_dynamic_field_names() {
    let original = "secret: private-token\nproxies: [{name: secret-name, server: secret-host, password: secret-password}]\ndns: {nameserver: ['https://secret-doh/?key=secret-key']}\nsecret-field: secret-value";
    let effective = "secret: other-token\nproxies: []\ndns: {enable: true, enhanced-mode: fake-ip}\ntun: {enable: true}\nrules: ['MATCH,DIRECT']";
    let entries = config_diff::compare(original, effective).unwrap();
    let encoded = serde_json::to_string(&entries).unwrap();
    for hidden in [
        "private-token",
        "secret-",
        "other-token",
        "MATCH",
        "https://",
    ] {
        assert!(!encoded.contains(hidden), "{encoded}");
    }
    assert!(
        entries
            .iter()
            .any(|entry| entry.label == "proxies" && entry.detail.contains("1 项 → 列表 0 项"))
    );
    assert!(config_diff::compare("bad: [", effective).is_err());
}

#[test]
fn traffic_handles_missing_metadata_saturates_counters_and_limits_output() {
    let mut connections = Vec::new();
    for index in 0..200 {
        connections.push(json!({"metadata":{"host":format!("host-{index}.test"),"process":"app"},"chains":["node","Proxy"],"rule":"Domain","upload":u64::MAX,"download":2}));
    }
    connections.push(json!({}));
    let snapshot = serde_json::from_value(json!({"connections":connections})).unwrap();
    let mut report = Report {
        schema: 1,
        tool: Tool::Traffic,
        target: "当前实例".into(),
        elapsed_ms: 0,
        status: "unavailable",
        entries: Vec::new(),
    };
    inspection::summarize(snapshot, &mut report);
    assert!(report.entries.len() <= 34);
    assert_eq!(
        report
            .entries
            .iter()
            .filter(|entry| entry.label == "主机")
            .count(),
        8
    );
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.detail.contains("未提供"))
    );
    let empty = serde_json::from_value(json!({"connections":[]})).unwrap();
    report.entries.clear();
    inspection::summarize(empty, &mut report);
    assert!(report.entries[0].detail.starts_with("0 个"));
}

#[test]
fn node_information_keeps_emoji_and_reports_missing_or_cyclic_targets() {
    let proxies = serde_json::from_value(json!({"proxies":{
        "Proxy":{"type":"Selector","all":["AUTO"],"now":"AUTO"},
        "AUTO":{"type":"URLTest","all":["🇰🇷 韩国 01"],"now":"🇰🇷 韩国 01"},
        "🇰🇷 韩国 01":{"type":"Trojan","history":[{"delay":123}]},
        "Loop":{"type":"Selector","all":["Loop"],"now":"Loop"}
    }}))
    .unwrap();
    let mut report = Report {
        schema: 1,
        tool: Tool::Node,
        target: "AUTO".into(),
        elapsed_ms: 0,
        status: "unavailable",
        entries: Vec::new(),
    };
    inspection::node("AUTO", &proxies, &mut report);
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.detail.contains("AUTO → 🇰🇷 韩国 01"))
    );
    report.entries.clear();
    inspection::node("Loop", &proxies, &mut report);
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "node_path_unknown")
    );
    report.entries.clear();
    inspection::node("Missing", &proxies, &mut report);
    assert_eq!(report.entries[0].code, "node_not_found");
}

#[test]
fn dns_details_keep_empty_answers_and_limits_explicit() {
    let records = http::dns_entries(
        &json!({"Status":0, "Answer":[{"type":16,"TTL":30,"data":"private-token"}]}),
        "TXT",
    )
    .unwrap();
    assert!(
        !serde_json::to_string(&records)
            .unwrap()
            .contains("private-token")
    );
    let empty = http::dns_entries(&json!({"Status":0, "Answer":[]}), "MX").unwrap();
    assert_eq!(empty.last().unwrap().code, "dns_no_record");
    assert_eq!(
        http::dns_entries(&json!({"Status":3}), "A").unwrap()[0].code,
        "dns_failed"
    );
    assert!(http::dns_entries(&json!({}), "A").is_err());
    let mixed = http::dns_entries(
        &json!({"Status":0, "Answer":[{"type":16,"TTL":30,"data":"private-token"},{"type":6,"TTL":30,"data":"private-contact"}]}),
        "A",
    )
    .unwrap();
    let encoded = serde_json::to_string(&mixed).unwrap();
    assert!(!encoded.contains("private-"), "{encoded}");
    for fields in ["5|1000|10|8|8|AAAA|GET", "5|1000|10|8|8|TXT|OPTIONS"] {
        Options::from_fields(fields).validate().unwrap();
    }
    for fields in ["5|1000|10|8|8|ANY|HEAD", "5|1000|10|8|8|A|POST"] {
        assert!(Options::from_fields(fields).validate().is_err());
    }
}

#[tokio::test]
async fn invalid_tcp_ports_are_rejected_before_opening_the_proxy() {
    for target in [
        "example.test:",
        "example.test:0",
        "example.test:65536",
        "example.test:abc",
        "[2001:db8::1]:",
        "example.test/path",
    ] {
        let report = run(
            Tool::Tcp,
            target,
            Environment {
                port: Some(1),
                ..offline()
            },
            Options::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            report.entries[0].code, "invalid_target",
            "{target}: {report:?}"
        );
    }
}

#[tokio::test]
async fn tcp_probe_sends_no_application_bytes_and_cancellation_closes_the_socket() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    for cancel in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (ready, received) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut greeting = [0; 3];
            socket.read_exact(&mut greeting).await.unwrap();
            socket.write_all(&[5, 0]).await.unwrap();
            let mut request = [0; 10];
            socket.read_exact(&mut request).await.unwrap();
            assert_eq!(request, [5, 1, 0, 1, 192, 0, 2, 1, 0, 22]);
            if !cancel {
                socket
                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                    .await
                    .unwrap();
            }
            let _ = ready.send(());
            let mut data = [0];
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), socket.read(&mut data))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
        });
        let task = tokio::spawn(run(
            Tool::Tcp,
            "192.0.2.1:22",
            Environment {
                port: Some(port),
                protected: true,
                ..offline()
            },
            Options::default(),
        ));
        received.await.unwrap();
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            let report = task.await.unwrap().unwrap();
            assert_eq!(report.status, "complete", "{report:?}");
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn no_target_tools_ignore_stale_input_and_never_start_or_write_data() {
    let temporary = tempfile::tempdir().unwrap();
    let missing = temporary.path().join("missing");
    for tool in [Tool::ConfigDiff, Tool::Traffic, Tool::Performance] {
        let report = run(
            tool,
            "\nold url",
            Environment {
                data_dir: Some(missing.clone()),
                ..offline()
            },
            Options::default(),
        )
        .await
        .unwrap();
        assert_eq!(report.target, "当前实例");
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.success == Some(false))
        );
        assert!(!missing.exists());
    }
    let report = run(Tool::Node, "", offline(), Options::default())
        .await
        .unwrap();
    assert_eq!(report.entries[0].code, "invalid_target");
}

#[tokio::test]
async fn http_details_do_not_follow_redirects_or_save_private_headers_and_bodies() {
    for (method, status) in [("HEAD", 302), ("GET", 403), ("OPTIONS", 500)] {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let expected = method.to_owned();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
                assert!(request.len() < 4096);
            }
            assert!(String::from_utf8_lossy(&request).starts_with(&expected));
            let body = if expected == "HEAD" {
                ""
            } else {
                "private-body"
            };
            let response = format!(
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nLocation: http://private.invalid/?secret=hidden\r\nSet-Cookie: private-cookie\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
        });
        let report = run(
            Tool::Http,
            "http://example.test/path?secret=hidden",
            Environment {
                port: Some(port),
                protected: true,
                ..offline()
            },
            Options {
                method: method.into(),
                ..Options::default()
            },
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert!(
            report.entries.iter().any(
                |entry| entry.label == "网站响应" && entry.detail.contains(&status.to_string())
            )
        );
        let encoded = serde_json::to_string(&report).unwrap();
        for hidden in [
            "private-body",
            "private-cookie",
            "private.invalid",
            "hidden",
            "/path",
        ] {
            assert!(!encoded.contains(hidden), "{encoded}");
        }
    }
}

#[tokio::test]
async fn performance_reads_one_stream_sample_without_waiting_for_eof() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = Api::new(&crate::config::Settings {
        controller_port: listener.local_addr().unwrap().port(),
        ..Default::default()
    })
    .unwrap();
    let server = tokio::spawn(async move {
        for body in [
            r#"{"version":"fixture"}"#,
            r#"{"connections":[]}"#,
            r#"{"inuse":1048576}"#,
        ] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
            }
            if body.contains("inuse") {
                stream
                    .write_all(
                        format!("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{body}\n").as_bytes(),
                    )
                    .await
                    .unwrap();
                let mut closed = [0];
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(2), stream.read(&mut closed))
                        .await
                        .unwrap()
                        .unwrap(),
                    0
                );
            } else {
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        }
    });
    let report = run(
        Tool::Performance,
        "",
        Environment {
            api: Some(api),
            ..offline()
        },
        Options::default(),
    )
    .await
    .unwrap();
    server.await.unwrap();
    assert_eq!(report.status, "complete", "{report:?}");
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.detail.contains("1048576 B"))
    );
}
