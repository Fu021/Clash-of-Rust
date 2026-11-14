use super::*;

fn environment() -> Environment {
    Environment {
        port: None,
        api: None,
        protected: true,
        udp_enabled: true,
    }
}

#[test]
fn tool_identifiers_and_parameter_bounds_reject_invalid_requests() {
    for (index, tool) in Tool::ALL.into_iter().enumerate() {
        assert_eq!(Tool::at(index as i32).unwrap(), tool);
        assert_eq!(Tool::parse(tool.id()).unwrap(), tool);
    }
    assert!(Tool::at(-1).is_err());
    assert!(Tool::at(11).is_err());
    assert!(Tool::parse("unknown").is_err());
    for options in [
        Options {
            count: 0,
            ..Options::default()
        },
        Options {
            count: 21,
            ..Options::default()
        },
        Options {
            seconds: 0,
            ..Options::default()
        },
        Options {
            seconds: 31,
            ..Options::default()
        },
        Options {
            max_bytes: 0,
            ..Options::default()
        },
        Options {
            max_bytes: 17 * 1024 * 1024,
            ..Options::default()
        },
        Options {
            interval_ms: 0,
            ..Options::default()
        },
        Options {
            interval_ms: 5001,
            ..Options::default()
        },
        Options {
            max_hops: 0,
            ..Options::default()
        },
        Options {
            max_hops: 17,
            ..Options::default()
        },
    ] {
        assert!(options.validate().is_err());
    }
}

#[test]
fn form_parameters_preserve_defaults_and_reject_overflow_or_malformed_values() {
    assert_eq!(Options::from_fields("").count, 5);
    let values = Options::from_fields("20|5000|30|16|16");
    values.validate().unwrap();
    assert_eq!(values.max_bytes, 16 * 1024 * 1024);
    for fields in [
        "1|100|1|1|1|extra",
        "1|100",
        "-1|100|1|1|1",
        "1|100|1|18446744073709551615|1",
        "1|100|1|0|1",
        "0|100|1|1|1",
        "1|99|1|1|1",
        "a|100|1|1|1",
    ] {
        assert!(Options::from_fields(fields).validate().is_err(), "{fields}");
    }
}

#[test]
fn successful_output_and_bounded_completion_do_not_become_partial_reports() {
    for code in [
        "process_output",
        "output_truncated",
        "byte_limit",
        "time_limit",
        "completed",
    ] {
        let mut report = Report {
            schema: 1,
            tool: Tool::Ping,
            target: "example.test".into(),
            elapsed_ms: 0,
            status: "unavailable",
            entries: vec![
                Entry::ok("探针", "成功"),
                Entry::unknown("说明", "已完成", code),
            ],
        };
        report.finish(Instant::now());
        assert_eq!(report.status, "complete", "{code}");
    }
}

#[tokio::test]
async fn protection_rejects_raw_probes_before_opening_any_socket() {
    for tool in [Tool::Udp, Tool::Ping, Tool::Trace] {
        let report = run(tool, "127.0.0.1", environment(), Options::default())
            .await
            .unwrap();
        assert_eq!(report.status, "unavailable");
        assert_eq!(report.entries[0].code, "protection_restricted");
        assert!(report.entries.iter().all(|entry| entry.success.is_none()));
    }
}

#[tokio::test]
async fn network_tools_do_not_start_a_core_or_use_a_direct_fallback() {
    for tool in [
        Tool::Download,
        Tool::Phase,
        Tool::Tls,
        Tool::Family,
        Tool::Stability,
    ] {
        let report = run(
            tool,
            "http://127.0.0.1:9/private?token=hidden",
            environment(),
            Options::default(),
        )
        .await
        .unwrap();
        assert_eq!(report.status, "failed");
        let encoded = serde_json::to_string(&report).unwrap();
        assert!(!encoded.contains("hidden"));
        assert!(!encoded.contains("/private"));
    }
}

#[test]
fn certificate_display_parser_is_bounded_and_rejects_truncation() {
    let root = webpki_root_certs::TLS_SERVER_ROOT_CERTS.first().unwrap();
    let mut entries = Vec::new();
    super::transport::certificate_entries(root.as_ref(), &mut entries).unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| entry.label == "证书 SHA256" && entry.detail.len() == 64)
    );
    assert!(
        super::transport::certificate_entries(
            &[0x30, 0x84, 0xff, 0xff, 0xff, 0xff],
            &mut Vec::new()
        )
        .is_err()
    );
    assert!(super::transport::certificate_entries(&root.as_ref()[..20], &mut Vec::new()).is_err());
}

#[test]
fn unavailable_results_are_not_reported_as_success_or_failure() {
    let mut report = Report {
        schema: 1,
        tool: Tool::Udp,
        target: "example.com".into(),
        elapsed_ms: 0,
        status: "complete",
        entries: vec![Entry::unknown("UDP", "未响应", "udp_no_response")],
    };
    report.finish(Instant::now());
    assert_eq!(report.status, "unavailable");
    report.entries.push(Entry::ok("TCP", "可连接"));
    report
        .entries
        .push(Entry::error("TLS", "验证失败", "tls_failed"));
    report.finish(Instant::now());
    assert_eq!(report.status, "partial");
}

#[tokio::test]
async fn invalid_targets_and_options_return_reports_without_secrets() {
    for target in [
        "",
        "ftp://example.test/",
        "https://user:private@example.test/",
        "https://example.test/\n",
        "https://example.test/\r\nHeader: secret",
        "https://example.test/\0",
    ] {
        let report = run(Tool::Tls, target, environment(), Options::default())
            .await
            .unwrap();
        assert_eq!(report.status, "failed");
        assert_eq!(report.entries[0].code, "invalid_target");
        assert!(!serde_json::to_string(&report).unwrap().contains("private"));
    }
    let report = run(
        Tool::Download,
        "https://example.test",
        environment(),
        Options {
            max_bytes: 0,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(report.entries[0].code, "invalid_options");
    let report = run(
        Tool::Udp,
        "1.1.1.1",
        Environment {
            protected: false,
            udp_enabled: false,
            ..environment()
        },
        Options::default(),
    )
    .await
    .unwrap();
    assert_eq!(report.entries[0].code, "udp_disabled");
    for target in [
        "1.1.1.1:0",
        "https://1.1.1.1/query",
        "https://1.1.1.1/?token=secret",
    ] {
        let report = run(
            Tool::Udp,
            target,
            Environment {
                protected: false,
                ..environment()
            },
            Options::default(),
        )
        .await
        .unwrap();
        assert_eq!(report.entries[0].code, "invalid_target", "{report:?}");
        assert!(!serde_json::to_string(&report).unwrap().contains("secret"));
    }
}

async fn http_fixture(response: Vec<u8>) -> (Environment, tokio::task::JoinHandle<()>) {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut byte = [0];
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
            assert!(request.len() < 4096);
        }
        let _ = socket.write_all(&response).await;
    });
    (
        Environment {
            port: Some(port),
            protected: false,
            ..environment()
        },
        task,
    )
}

#[tokio::test]
async fn download_rejects_redirect_empty_and_truncated_bodies_and_honors_byte_limit() {
    for (response, code, expected) in [
        (
            b"HTTP/1.1 302 Found\r\nLocation: http://private.invalid/\r\nContent-Length: 0\r\n\r\n"
                .to_vec(),
            "download_http_status",
            "failed",
        ),
        (
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
            "empty_response",
            "failed",
        ),
        (
            b"HTTP/1.1 200 OK\r\nContent-Length: 9999\r\n\r\nx".to_vec(),
            "http_body_failed",
            "failed",
        ),
    ] {
        let (context, task) = http_fixture(response).await;
        let report = run(
            Tool::Download,
            "http://example.invalid/private?secret=hidden",
            context,
            Options::default(),
        )
        .await
        .unwrap();
        task.await.unwrap();
        assert_eq!(report.status, expected, "{report:?}");
        assert!(
            report.entries.iter().any(|entry| entry.code == code),
            "{report:?}"
        );
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("hidden"));
        assert!(!json.contains("private.invalid"));
    }
    let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 8192\r\n\r\n".to_vec();
    response.extend(vec![b'x'; 8192]);
    let (context, task) = http_fixture(response).await;
    let report = run(
        Tool::Download,
        "http://example.invalid/",
        context,
        Options {
            max_bytes: 1024,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    task.await.unwrap();
    assert_eq!(report.entries[0].detail, "1024 B / 1024 B 上限");
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "byte_limit")
    );
}

#[tokio::test]
async fn socks_authentication_refusal_never_becomes_a_direct_request() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut greeting = [0; 3];
        socket.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 1, 0]);
        socket.write_all(&[5, 0xff]).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let report = run(
        Tool::Phase,
        "http://127.0.0.1/private?secret=hidden",
        Environment {
            port: Some(port),
            ..environment()
        },
        Options::default(),
    )
    .await
    .unwrap();
    server.await.unwrap();
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "proxy_authentication")
    );
    assert!(!serde_json::to_string(&report).unwrap().contains("hidden"));
}

#[tokio::test]
async fn socks_rejections_and_malformed_responses_keep_distinct_error_codes() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    for (reply, code) in [
        ([5, 2, 0, 1], "proxy_rule_denied"),
        ([5, 3, 0, 1], "proxy_target_unreachable"),
        ([5, 5, 0, 1], "proxy_target_refused"),
        ([5, 7, 0, 1], "proxy_command_unsupported"),
        ([5, 8, 0, 1], "proxy_family_unsupported"),
        ([4, 0, 0, 1], "proxy_response_invalid"),
        ([5, 0, 1, 1], "proxy_response_invalid"),
        ([5, 0, 0, 9], "proxy_response_invalid"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut greeting = [0; 3];
            socket.read_exact(&mut greeting).await.unwrap();
            socket.write_all(&[5, 0]).await.unwrap();
            let mut request = [0; 10];
            socket.read_exact(&mut request).await.unwrap();
            socket.write_all(&reply).await.unwrap();
        });
        let report = run(
            Tool::Phase,
            "http://127.0.0.1/private?secret=hidden",
            Environment {
                port: Some(port),
                ..environment()
            },
            Options::default(),
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert!(
            report.entries.iter().any(|entry| entry.code == code),
            "{report:?}"
        );
        assert!(!serde_json::to_string(&report).unwrap().contains("hidden"));
    }
}

#[tokio::test]
async fn download_stops_at_time_limit_without_waiting_for_remote_eof() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        socket.read(&mut request).await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 999999\r\n\r\nx")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
    });
    let start = Instant::now();
    let report = run(
        Tool::Download,
        "http://example.invalid/",
        Environment {
            port: Some(port),
            ..environment()
        },
        Options {
            seconds: 1,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    server.abort();
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "time_limit"),
        "{report:?}"
    );
}

#[test]
fn error_classification_withholds_arbitrary_network_messages() {
    for (kind, code) in [
        (std::io::ErrorKind::PermissionDenied, "permission_denied"),
        (std::io::ErrorKind::NotFound, "component_unavailable"),
        (std::io::ErrorKind::ConnectionRefused, "connection_refused"),
        (std::io::ErrorKind::TimedOut, "connection_timeout"),
        (std::io::ErrorKind::UnexpectedEof, "connection_closed"),
    ] {
        let failure: anyhow::Error = std::io::Error::new(kind, "secret-url?token=hidden").into();
        let entry = super::error::entry("探针", &failure);
        assert_eq!(entry.code, code);
        assert!(!entry.detail.contains("hidden"));
    }
}

async fn socks_fixture(response: &'static [u8]) -> (u16, tokio::task::JoinHandle<()>) {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut greeting = [0; 3];
        socket.read_exact(&mut greeting).await.unwrap();
        socket.write_all(&[5, 0]).await.unwrap();
        let mut request = [0; 10];
        socket.read_exact(&mut request).await.unwrap();
        assert_eq!(request[3], 1);
        socket
            .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
            .await
            .unwrap();
        let mut received = [0; 1024];
        let _ = socket.read(&mut received).await;
        let _ = socket.write_all(response).await;
    });
    (port, task)
}

#[tokio::test]
async fn invalid_tls_is_not_silently_downgraded() {
    let (port, task) = socks_fixture(b"HTTP/1.1 200 Not-TLS\r\n\r\n").await;
    let report = run(
        Tool::Tls,
        "https://127.0.0.1/",
        Environment {
            port: Some(port),
            ..environment()
        },
        Options::default(),
    )
    .await
    .unwrap();
    task.await.unwrap();
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "tls_failed")
    );
    assert_eq!(report.status, "partial");
    assert!(!report.entries.iter().any(|entry| entry.label == "证书验证"));
}

#[tokio::test]
async fn ipv4_only_target_is_partial_without_inventing_an_ipv6_failure() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let settings = crate::config::Settings {
        controller_port: listener.local_addr().unwrap().port(),
        ..Default::default()
    };
    let controller = Api::new(&settings).unwrap();
    let dns = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                bytes.push(byte[0]);
                if bytes.ends_with(b"\r\n\r\n") {
                    break;
                }
                assert!(bytes.len() < 4096);
            }
            let body = if String::from_utf8_lossy(&bytes).contains("type=AAAA") {
                r#"{"Status":0,"Answer":[]}"#
            } else {
                r#"{"Status":0,"Answer":[{"type":1,"data":"127.0.0.1"}]}"#
            };
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    let (port, proxy) = socks_fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
    let report = run(
        Tool::Family,
        "http://example.test/",
        Environment {
            port: Some(port),
            api: Some(controller),
            ..environment()
        },
        Options::default(),
    )
    .await
    .unwrap();
    dns.await.unwrap();
    proxy.await.unwrap();
    assert_eq!(report.status, "partial");
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.label == "IPv4" && entry.success == Some(true))
    );
    assert!(report.entries.iter().any(|entry| entry.label == "IPv6"
        && entry.code == "family_no_address"
        && entry.success.is_none()));
    assert!(
        !report
            .entries
            .iter()
            .any(|entry| entry.success == Some(false))
    );
}
