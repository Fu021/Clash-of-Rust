use crate::args;
use serde_json::{Value, json};

fn render(language: &str, flags: &[&str], report: &Value) -> String {
    let args = args::parse(
        ["diagnose".into(), format!("--lang={language}")]
            .into_iter()
            .chain(flags.iter().map(|flag| (*flag).into())),
    )
    .unwrap();
    let mut out = Vec::new();
    super::super::write_output(&mut out, &args, report).unwrap();
    String::from_utf8(out).unwrap()
}

fn report() -> Value {
    json!({
        "dns_host": "example.test",
        "system_dns": {"Err": "DNS 查询请输入域名，例如 github.com"},
        "core_dns": {"Err": "未查到目标地址"},
        "system_route_http": {
            "url": "https://example.test/原始路径",
            "status": 200,
            "millis": 12,
            "detail": "HTTP 200 · 响应头耗时，不是 ICMP 延迟"
        },
        "http": {"error": "直连：外部服务：東京；请输入 HTTP/HTTPS 地址"},
        "ipv4": {"error": "IP 查询返回 HTTP 503，未获得有效出口地址"},
        "ipv6": {"error": "IP 查询响应不是有效 UTF-8 文本: 直连：外部详情：東京；原文"},
        "core_logs": {"error": "读取内核日志超过 5 秒"},
        "backend_error": "当前用户后台未运行"
    })
}

#[test]
fn diagnose_localizes_known_probe_messages_and_preserves_external_details() {
    let report = report();
    for (language, expected) in [
        (
            "en",
            [
                "System DNS: Failed: Enter a domain name for DNS lookup, for example github.com",
                "Current core DNS: Failed: No addresses found for the target",
                "System route HTTP (without proxy environment variables): https://example.test/原始路径 · HTTP 200 · Response header time, not ICMP latency · 12 ms",
                "Local proxy HTTP: Failed: 直连：外部服务：東京；请输入 HTTP/HTTPS 地址",
                "Proxy IPv4 exit: Failed: IP lookup returned HTTP 503 without a valid exit address",
                "Proxy IPv6 exit: Failed: IP lookup response is not valid UTF-8 text: 直连：外部详情：東京；原文",
            ],
        ),
        (
            "zh-CN",
            [
                "系统 DNS：失败：DNS 查询请输入域名，例如 github.com",
                "当前内核 DNS：失败：未查到目标地址",
                "系统路由 HTTP（不使用代理环境变量）：https://example.test/原始路径 · HTTP 200 · 响应头耗时，不是 ICMP 延迟 · 12 ms",
                "本地代理 HTTP：失败：直连：外部服务：東京；请输入 HTTP/HTTPS 地址",
                "代理 IPv4 出口：失败：IP 查询返回 HTTP 503，未获得有效出口地址",
                "代理 IPv6 出口：失败：IP 查询响应不是有效 UTF-8 文本: 直连：外部详情：東京；原文",
            ],
        ),
    ] {
        let output = render(language, &[], &report);
        for line in expected {
            assert!(output.lines().any(|actual| actual == line), "{output}");
        }
    }
}

#[test]
fn diagnose_localizes_backend_and_probe_timeouts() {
    let mut report = report();
    report["system_dns"]["Err"] = json!("单项检测超过 15 秒: deadline has elapsed");
    report["system_route_http"] = json!({"error": "请输入 HTTP/HTTPS 地址"});
    report["ipv4"]["error"] = json!("IP 查询返回 HTTP 東京，未获得有效出口地址");
    for (raw, english) in [
        (
            "当前用户后台未运行",
            "The current user's backend is not running",
        ),
        (
            "当前用户内核未运行",
            "The current user's core is not running",
        ),
        (
            "连接当前用户后台超过 5 秒；可用 --offline 保存本地现场: deadline has elapsed",
            "Connecting to the current user's backend timed out after 5 seconds (use --offline to save local diagnostics): deadline has elapsed",
        ),
        ("外部后台：東京；原文", "外部后台：東京；原文"),
    ] {
        report["backend_error"] = json!(raw);
        let output = render("en", &[], &report);
        for line in [
            format!("Background: {english}"),
            "System DNS: Failed: Probe timed out after 15 seconds: deadline has elapsed".into(),
            "System route HTTP (without proxy environment variables): Failed: Enter an HTTP/HTTPS URL".into(),
            "Proxy IPv4 exit: Failed: IP 查询返回 HTTP 東京，未获得有效出口地址".into(),
        ] {
            assert!(output.lines().any(|actual| actual == line), "{output}");
        }
        let output = render("zh-CN", &[], &report);
        assert!(
            output.lines().any(|line| line == format!("后台：{raw}")),
            "{output}"
        );
    }
}

#[test]
fn diagnose_keeps_json_and_saved_reports_in_the_original_language() {
    let report = report();
    let expected = json!({"schema": 1, "command": "diagnose", "data": report});
    for language in ["en", "zh-CN"] {
        let json = render(language, &["--json"], &report);
        assert_eq!(serde_json::from_str::<Value>(&json).unwrap(), expected);
    }

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("diagnose.json");
    super::super::save(&path, "diagnose", &report).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(path).unwrap()).unwrap(),
        expected
    );
}

#[test]
fn offline_diagnose_localizes_log_capture_state_and_preserves_json() {
    for (state, chinese, english) in [
        ("collected", "已采集", "Collected"),
        ("missing", "未找到", "Not found"),
        ("withheld", "已隐藏", "Withheld"),
        ("unavailable", "不可用", "Unavailable"),
        ("future-state", "future-state", "future-state"),
    ] {
        let report = json!({
            "offline": true,
            "local_evidence": {"daemon_log": {"state": state}},
        });
        assert!(
            render("zh-CN", &[], &report)
                .lines()
                .any(|line| line == format!("后台日志采集：{chinese}"))
        );
        assert!(
            render("en", &[], &report)
                .lines()
                .any(|line| line == format!("Background log capture: {english}"))
        );
        let rendered = render("en", &["--json"], &report);
        let value = serde_json::from_str::<Value>(&rendered).unwrap();
        assert_eq!(
            value["data"]["local_evidence"]["daemon_log"]["state"],
            state
        );
    }
}
