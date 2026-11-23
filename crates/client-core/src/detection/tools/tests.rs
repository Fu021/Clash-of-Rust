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
    assert!(Tool::at(Tool::ALL.len() as i32).is_err());
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

#[test]
fn displayed_reports_distinguish_completion_failure_and_missing_evidence() {
    for (entries, expected, displayed) in [
        (
            vec![Entry::ok("HTTP", "HTTP 503；收到响应不代表服务正常")],
            "complete",
            "检查状态 · 完成",
        ),
        (
            vec![
                Entry::ok("系统 DNS", "192.0.2.1"),
                Entry::error("TLS", "证书过期", "tls_expired"),
            ],
            "partial",
            "检查状态 · 部分完成",
        ),
        (
            vec![Entry::error("代理", "连接被拒绝", "proxy_failed")],
            "failed",
            "检查状态 · 失败",
        ),
        (
            vec![Entry::unknown(
                "路由追踪",
                "未确认到达目标",
                "trace_not_reached",
            )],
            "unavailable",
            "检查状态 · 不可用或未确认",
        ),
    ] {
        let mut report = Report {
            schema: 1,
            tool: Tool::Http,
            target: "https://example.test".into(),
            elapsed_ms: 0,
            status: "unavailable",
            entries,
        };
        report.finish(Instant::now());
        assert_eq!(report.status, expected);
        let lines = report.lines();
        assert!(lines[0].starts_with(displayed));
        assert!(lines[0].ends_with(" ms"));
        assert_eq!(lines[1], "目标 · https://example.test");
        assert_eq!(
            &lines[2..],
            report
                .entries
                .iter()
                .map(|entry| format!("{} · {}", entry.label, entry.detail))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn english_network_display_keeps_serialized_evidence_and_external_values() {
    let report = Report {
        schema: 1,
        tool: Tool::Dns,
        target: "https://example.test".into(),
        elapsed_ms: 42,
        status: "partial",
        entries: vec![
            Entry::error("系统 DNS", "系统解析失败或超时", "dns_failed"),
            Entry::ok("记录", "TXT value: 保留原文"),
            Entry::unknown("第 2 跳", "*", "probe_timeout"),
        ],
    };
    let original = serde_json::to_value(&report).unwrap();
    let lines = report.lines_in(Language::English);
    assert_eq!(lines[0], "Status · Partial · 42 ms");
    assert_eq!(lines[1], "Target · https://example.test");
    assert_eq!(lines[2], "System DNS · System DNS failed or timed out");
    assert_eq!(lines[3], "Record · TXT value: 保留原文");
    assert_eq!(lines[4], "Hop 2 · *");
    assert_eq!(serde_json::to_value(&report).unwrap(), original);
    assert_eq!(
        entry_label_in("用户自定义名称", Language::English),
        "用户自定义名称"
    );
}

#[test]
fn english_dns_report_translates_generated_fields_without_changing_evidence() {
    let entries = http::dns_entries(
        &serde_json::json!({
            "Status": 0,
            "AA": true,
            "Answer": [
                {"type": 1, "TTL": 60, "data": "保留原文"},
                {"type": 16, "TTL": 0, "data": "hidden"}
            ]
        }),
        "A",
    )
    .unwrap();
    let report = Report {
        schema: 1,
        tool: Tool::Dns,
        target: "https://example.test".into(),
        elapsed_ms: 12,
        status: "complete",
        entries,
    };
    let original = serde_json::to_value(&report).unwrap();
    let lines = report.lines_in(Language::English);
    assert_eq!(
        lines[2],
        "DNS response · A · 2 answers · AA=yes · RA=unavailable"
    );
    assert_eq!(lines[3], "Record · Type 1 · TTL 60 s · 保留原文");
    assert_eq!(lines[4], "Record · Type 16 · TTL 0 s · Content hidden");
    assert_eq!(serde_json::to_value(&report).unwrap(), original);
    assert!(report.lines()[2].contains("2 条回答"));
}

#[test]
fn english_network_results_translate_generated_values_only() {
    for (label, detail, code, expected) in [
        (
            "请求结果",
            "收到响应 2 / 3 · 传输失败 1；HTTP 业务状态另列",
            "http_failed",
            "Responses 2 / 3 · Transport failures 1; HTTP status listed separately",
        ),
        (
            "响应头耗时",
            "最低 2 · 中位 3 · 最高 4 ms",
            "ok",
            "Min 2 · Median 3 · Max 4 ms",
        ),
        (
            "响应正文",
            "接收 4096 B（最多 4096 B），已丢弃",
            "byte_limit",
            "Received 4096 B (up to 4096 B); discarded",
        ),
        (
            "已接收",
            "8192 B / 16384 B 上限",
            "ok",
            "8192 B / 16384 B limit",
        ),
        (
            "平均速度",
            "12.5 KiB/s · 3.25 秒",
            "information",
            "12.5 KiB/s · 3.25 s",
        ),
        (
            "代理建立目标连接",
            "端口 443 · 25 ms（含代理侧 DNS、规则及出站处理）",
            "ok",
            "Port 443 · 25 ms (includes proxy DNS, rules and outbound processing)",
        ),
        (
            "证书验证",
            "域名、有效期和信任链验证通过 · 服务端提供 2 张证书",
            "ok",
            "Domain, validity and chain verified · 2 certificates",
        ),
        (
            "IPv6",
            "2001:db8::1 · 连接超时",
            "family_timeout",
            "2001:db8::1 · Connection timed out",
        ),
        (
            "DNS / UDP",
            "收到有效响应 · 11 ms",
            "ok",
            "Valid response · 11 ms",
        ),
        (
            "检查失败",
            "证书已过期，未绕过验证",
            "tls_certificate_expired",
            "Certificate expired",
        ),
    ] {
        assert_eq!(
            entry_detail_in(label, detail, code, Language::English),
            expected,
            "{label}"
        );
        assert_eq!(
            entry_detail_in(label, detail, code, Language::SimplifiedChinese),
            detail,
            "{label}"
        );
    }
    for (label, detail, code) in [
        ("记录", "类型 1 · TTL 60 s · 正文已隐藏", "ok"),
        ("记录", "TXT value: 保留原文", "ok"),
        ("平均速度", "源站 · 3.25 秒", "information"),
        ("DNS / UDP", "收到有效响应 · 11 ms", "process_output"),
        ("证书主体", "证书已过期，未绕过验证", "ok"),
    ] {
        let translated = entry_detail_in(label, detail, code, Language::English);
        if label == "记录" && detail.starts_with("类型 ") {
            assert_eq!(translated, "Type 1 · TTL 60 s · 正文已隐藏");
        } else {
            assert_eq!(translated, detail);
        }
    }
}

#[test]
fn route_explanations_keep_both_languages_and_older_reports_readable() {
    for (label, chinese, english) in [
        (
            "路径",
            "系统 ICMP / 逐跳探针，绕过系统代理；TUN 是否接管取决于系统路由。",
            "System ICMP or trace probe; bypasses the system proxy; TUN follows system routes",
        ),
        (
            "路径",
            "系统 ICMP / 逐跳探针；系统代理不承载此流量，TUN 是否接管取决于系统路由。这不是代理节点延迟",
            "System ICMP or trace probe; bypasses the system proxy; TUN follows system routes",
        ),
        (
            "检查路径",
            "目标 IP 经本机 SOCKS5，IP 与域名可能匹配不同规则；HTTPS 校验证书，HTTP 仅建立连接。",
            "Target IP via local SOCKS5; IP and domain rules may differ; HTTPS validates the domain certificate, HTTP only connects",
        ),
        (
            "检查路径",
            "强制目标 IP，经本机 SOCKS5 代理；HTTPS 验证原域名证书，HTTP 仅建立连接。IP 规则可能与域名请求不同；结果不代表本机或节点入口的地址族能力",
            "Target IP via local SOCKS5; IP and domain rules may differ; HTTPS validates the domain certificate, HTTP only connects",
        ),
    ] {
        assert_eq!(
            entry_detail_in(label, chinese, "information", Language::English),
            english
        );
        assert_eq!(
            entry_detail_in(label, chinese, "information", Language::SimplifiedChinese),
            chinese
        );
        let entry = Entry::unknown(label, chinese, "information");
        assert_eq!(serde_json::to_value(entry).unwrap()["detail"], chinese);
    }
}

#[test]
fn english_website_check_translates_only_known_probe_details() {
    for (label, code, detail, expected) in [
        (
            "HTTPS",
            "not_observed",
            "内核未运行，未发起请求",
            "Core is not running; request not sent",
        ),
        (
            "内核 DNS",
            "probe_failed",
            "未查到目标地址",
            "No addresses found for the target",
        ),
        (
            "内核 DNS",
            "probe_failed",
            "未查到目标地址：外部错误",
            "未查到目标地址：外部错误",
        ),
        (
            "系统 DNS",
            "ok",
            "192.0.2.1、2001:db8::1",
            "192.0.2.1, 2001:db8::1",
        ),
        ("内核 DNS", "ok", "地址、外部字段", "地址、外部字段"),
        (
            "实际线路",
            "ok",
            "東京节点 · 命中 DOMAIN (example.test)",
            "東京节点 · 命中 DOMAIN (example.test)",
        ),
    ] {
        assert_eq!(
            entry_detail_in(label, detail, code, Language::English),
            expected
        );
        assert_eq!(
            entry_detail_in(label, detail, code, Language::SimplifiedChinese),
            detail
        );
    }
    let addresses: Vec<_> = (1..=16).map(|index| format!("192.0.2.{index}")).collect();
    assert_eq!(
        entry_detail_in(
            "系统 DNS",
            &format!("{}（另有 2 项）", addresses.join("、")),
            "ok",
            Language::English,
        ),
        format!("{} (2 more)", addresses.join(", "))
    );
}

#[test]
fn english_route_uses_structured_translation_without_changing_exported_evidence() {
    let report = Report {
        schema: 1,
        tool: Tool::Website,
        target: "https://example.test".into(),
        elapsed_ms: 1,
        status: "complete",
        entries: vec![from_check(
            "实际线路",
            probe::DiagnosticCheck {
                success: Some(true),
                detail: "节点 · 命中 名称 · 命中 DomainSuffix (规则 · 命中 原文)".into(),
                detail_english: Some(
                    "节点 · 命中 名称 · Matched DomainSuffix (规则 · 命中 原文)".into(),
                ),
            },
        )],
    };
    let original = serde_json::to_value(&report).unwrap();
    assert_eq!(
        report.lines_in(Language::SimplifiedChinese)[2],
        "实际线路 · 节点 · 命中 名称 · 命中 DomainSuffix (规则 · 命中 原文)"
    );
    assert_eq!(
        report.lines_in(Language::English)[2],
        "Route · 节点 · 命中 名称 · Matched DomainSuffix (规则 · 命中 原文)"
    );
    assert_eq!(serde_json::to_value(&report).unwrap(), original);
    assert!(original["entries"][0].get("detail_english").is_none());

    let mut display = report.clone();
    display.localize_route_for_display(Language::English);
    assert_eq!(
        display.entries[0].detail,
        "节点 · 命中 名称 · Matched DomainSuffix (规则 · 命中 原文)"
    );
    assert_eq!(serde_json::to_value(&report).unwrap(), original);
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
fn certificate_name_placeholders_translate_only_in_human_output() {
    fn field(tag: u8, value: &[u8]) -> Vec<u8> {
        assert!(value.len() < 128);
        [&[tag, value.len() as u8], value].concat()
    }

    fn name(value: &[u8]) -> Vec<u8> {
        let attribute = [b"\x06\x03\x55\x04\x03".as_slice(), &field(0x0c, value)].concat();
        field(0x31, &field(0x30, &attribute))
    }

    fn report(subject: &[u8], issuer: &[u8]) -> Report {
        let body = [
            b"\x02\x01\x01\x30\x00".as_slice(),
            &field(0x30, issuer),
            &field(0x30, b"\x17\x0d250101000000Z\x17\x0d270101000000Z"),
            &field(0x30, subject),
        ]
        .concat();
        let mut entries = Vec::new();
        super::transport::certificate_entries(&field(0x30, &field(0x30, &body)), &mut entries)
            .unwrap();
        Report {
            schema: 1,
            tool: Tool::Tls,
            target: "https://example.test".into(),
            elapsed_ms: 1,
            status: "complete",
            entries,
        }
    }

    let missing = "证书未提供通用名称";
    let undecodable = "无法解码的名称";
    let placeholders = report(&[], &name(&[0xff]));
    let external = report(&name(missing.as_bytes()), &name(undecodable.as_bytes()));
    let original = serde_json::to_value(&placeholders).unwrap();
    let entries = original["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(
        entries[0],
        serde_json::json!({"label":"证书主体","detail":missing,"success":true,"code":"ok"})
    );
    assert_eq!(
        entries[1],
        serde_json::json!({"label":"签发者","detail":undecodable,"success":true,"code":"ok"})
    );
    let external_json = serde_json::to_value(&external).unwrap();
    assert_eq!(
        &entries[..2],
        &external_json["entries"].as_array().unwrap()[..2]
    );
    assert_eq!(
        placeholders.lines()[2..4],
        [
            format!("证书主体 · {missing}"),
            format!("签发者 · {undecodable}")
        ]
    );
    assert_eq!(
        placeholders.lines_in(Language::English)[2..4],
        [
            "Certificate subject · Certificate has no common name",
            "Issuer · Could not decode the name",
        ]
    );
    assert_eq!(
        external.lines_in(Language::English)[2..4],
        [
            format!("Certificate subject · {missing}"),
            format!("Issuer · {undecodable}"),
        ]
    );
    let mut display = placeholders.clone();
    display.localize_route_for_display(Language::English);
    assert_eq!(display.entries[0].detail, "Certificate has no common name");
    assert_eq!(display.entries[1].detail, "Could not decode the name");
    assert_eq!(serde_json::to_value(&placeholders).unwrap(), original);
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
        let received = socket.read(&mut request).await.unwrap();
        assert!(received > 0);
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
