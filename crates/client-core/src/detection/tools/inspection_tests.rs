use super::*;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn offline() -> Environment {
    Environment {
        port: None,
        api: None,
        protected: false,
        udp_enabled: true,
    }
}

#[test]
fn tls_certificate_failures_keep_specific_causes_without_bypassing_verification() {
    use tokio_rustls::rustls::{CertificateError, Error};
    for (cause, code) in [
        (CertificateError::Expired, "tls_certificate_expired"),
        (
            CertificateError::NotValidYet,
            "tls_certificate_not_yet_valid",
        ),
        (CertificateError::NotValidForName, "tls_certificate_name"),
        (CertificateError::UnknownIssuer, "tls_certificate_issuer"),
        (CertificateError::BadSignature, "tls_certificate_invalid"),
    ] {
        let failure = transport::tls_error(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            Error::InvalidCertificate(cause),
        ));
        assert_eq!(error::entry("TLS", &failure).code, code);
    }
    let failure = transport::tls_error(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
    assert_eq!(error::entry("TLS", &failure).code, "tls_failed");
}

#[test]
fn dns_details_keep_empty_answers_and_limits_explicit() {
    for kind in ["TXT", "A"] {
        let records = http::dns_entries(
            &json!({"Status":0, "Answer":[{"type":16,"TTL":30,"data":"private-token"},{"type":6,"TTL":30,"data":"private-contact"}]}),
            kind,
        )
        .unwrap();
        assert!(
            !serde_json::to_string(&records)
                .unwrap()
                .contains("private-")
        );
    }
    let empty = http::dns_entries(&json!({"Status":0, "Answer":[]}), "MX").unwrap();
    assert_eq!(empty.last().unwrap().code, "dns_no_record");
    assert_eq!(
        http::dns_entries(&json!({"Status":3}), "A").unwrap()[0].code,
        "dns_failed"
    );
    assert!(http::dns_entries(&json!({}), "A").is_err());
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
            assert_eq!(task.await.unwrap().unwrap().status, "complete");
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn tcp_timeout_covers_an_unresponsive_socks_handshake() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut greeting = [0; 3];
        socket.read_exact(&mut greeting).await.unwrap();
        let mut data = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), socket.read(&mut data))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    });
    let report = run(
        Tool::Tcp,
        "example.test:443",
        Environment {
            port: Some(port),
            ..offline()
        },
        Options {
            seconds: 1,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    server.await.unwrap();
    assert_eq!(report.status, "failed");
    assert_eq!(report.entries[0].code, "connection_timeout");
}

#[tokio::test]
async fn http_details_do_not_follow_redirects_or_save_private_headers_and_bodies() {
    for (method, status) in [("HEAD", 302), ("GET", 403), ("OPTIONS", 500)] {
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
        assert_eq!(report.status, "complete", "{report:?}");
        assert!(
            report.entries.iter().any(
                |entry| entry.label == "网站响应" && entry.detail.contains(&status.to_string())
            )
        );
        assert!(!report.entries.iter().any(|entry| entry.label == "实际线路"));
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
async fn stability_distinguishes_http_errors_from_transport_failures() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server =
        tokio::spawn(async move {
            for status in [200, 403, 500] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await.unwrap());
                    assert!(request.len() < 4096);
                }
                stream.write_all(format!(
                "HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            ).as_bytes()).await.unwrap();
            }
        });
    let report = run(
        Tool::Stability,
        "http://example.test/",
        Environment {
            port: Some(port),
            ..offline()
        },
        Options {
            count: 3,
            interval_ms: 100,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    server.await.unwrap();
    assert_eq!(report.status, "complete", "{report:?}");
    assert!(report.entries[0].detail.contains("传输失败 0"));
    let statuses = report
        .entries
        .iter()
        .find(|entry| entry.label == "HTTP 状态分布")
        .unwrap();
    for status in ["200 × 1", "403 × 1", "500 × 1"] {
        assert!(statuses.detail.contains(status));
    }
}
