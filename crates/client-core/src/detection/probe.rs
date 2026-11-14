use anyhow::{Context, Result, bail};
use reqwest::Url;
use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub url: String,
    pub millis: u128,
    pub status: u16,
    pub detail: String,
}

pub fn validate_url(input: &str) -> Result<Url> {
    let url = Url::parse(input)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("请输入 HTTP/HTTPS 地址");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("测试地址不支持嵌入用户名密码");
    }
    Ok(url)
}

pub fn diagnostic_url(input: &str) -> Result<Url> {
    let input = input.trim();
    if input.is_empty() || input.len() > 2048 || input.chars().any(char::is_control) {
        bail!("请输入有效的网站地址");
    }
    let address = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    validate_url(&address).context("请输入域名或 HTTP/HTTPS 地址")
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticCheck {
    pub success: Option<bool>,
    pub detail: String,
}

impl DiagnosticCheck {
    fn success(detail: impl Into<String>) -> Self {
        Self {
            success: Some(true),
            detail: detail.into(),
        }
    }

    fn failure(detail: impl Into<String>) -> Self {
        Self {
            success: Some(false),
            detail: detail.into(),
        }
    }

    fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            success: None,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NetworkReport {
    pub target: String,
    pub system_dns: DiagnosticCheck,
    pub core_dns: DiagnosticCheck,
    pub proxy: DiagnosticCheck,
    pub https: DiagnosticCheck,
    pub http: DiagnosticCheck,
    pub response: DiagnosticCheck,
    pub route: DiagnosticCheck,
}

impl NetworkReport {
    pub fn lines(&self) -> Vec<String> {
        [
            ("目标", self.target.as_str()),
            ("系统 DNS", self.system_dns.detail.as_str()),
            ("内核 DNS", self.core_dns.detail.as_str()),
            ("本机代理", self.proxy.detail.as_str()),
            ("HTTPS", self.https.detail.as_str()),
            ("HTTP", self.http.detail.as_str()),
            ("采用协议", self.response.detail.as_str()),
            ("实际线路", self.route.detail.as_str()),
        ]
        .into_iter()
        .map(|(label, detail)| format!("{label} · {detail}"))
        .collect()
    }
}

fn addresses<I: IntoIterator<Item = IpAddr>>(values: I) -> DiagnosticCheck {
    let mut addresses: Vec<_> = values.into_iter().collect();
    addresses.sort();
    addresses.dedup();
    if addresses.is_empty() {
        return DiagnosticCheck::failure("未返回 IP 地址");
    }
    let extra = addresses.len().saturating_sub(16);
    let mut detail = addresses
        .iter()
        .take(16)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("、");
    if extra > 0 {
        detail.push_str(&format!("（另有 {extra} 项）"));
    }
    DiagnosticCheck::success(detail)
}

fn protocol_urls(input: &str) -> Result<(Url, Url)> {
    let mut https = diagnostic_url(input)?;
    let mut http = https.clone();
    https
        .set_scheme("https")
        .map_err(|_| anyhow::anyhow!("无法检查 HTTPS 地址"))?;
    http.set_scheme("http")
        .map_err(|_| anyhow::anyhow!("无法检查 HTTP 地址"))?;
    Ok((https, http))
}

async fn protocol_check(
    url: Url,
    port: Option<u16>,
    api: Option<crate::api::Api>,
) -> (DiagnosticCheck, DiagnosticCheck) {
    let Some(port) = port else {
        return (
            DiagnosticCheck::unavailable("内核未运行，未发起请求"),
            DiagnosticCheck::unavailable("未建立检测连接"),
        );
    };
    let result = async {
        let client = crate::network::client_builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let start = Instant::now();
        let response = client
            .get(url)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;
        let elapsed = start.elapsed().as_millis();
        let mut observer = crate::update::RouteObserver::new(&response, api);
        let route = observer.observation().await;
        Ok::<_, anyhow::Error>((response.status().as_u16(), elapsed, route))
    }
    .await;
    match result {
        Ok((status, millis, route)) => {
            let response = DiagnosticCheck::success(format!("HTTP {status} · {millis} ms"));
            let route = match route {
                Some(route) => {
                    let rule = match (route.rule.is_empty(), route.rule_payload.is_empty()) {
                        (true, _) => String::new(),
                        (false, true) => format!(" · 命中 {}", route.rule),
                        (false, false) => {
                            format!(" · 命中 {} ({})", route.rule, route.rule_payload)
                        }
                    };
                    DiagnosticCheck::success(format!("{}{rule}", route.chain))
                }
                None => DiagnosticCheck::unavailable("未捕获到此请求的连接记录"),
            };
            (response, route)
        }
        Err(error) => (
            DiagnosticCheck::failure(format!("{error:#}")),
            DiagnosticCheck::unavailable("请求未完成，无法确认线路"),
        ),
    }
}

pub async fn diagnose(
    input: &str,
    port: Option<u16>,
    api: Option<crate::api::Api>,
    protected: bool,
) -> Result<NetworkReport> {
    let (https_url, http_url) = protocol_urls(input)?;
    let url = &https_url;
    let host = url
        .host_str()
        .context("网站地址缺少域名")?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let address = host.parse::<IpAddr>().ok();
    let system = async {
        if let Some(address) = address {
            return DiagnosticCheck::unavailable(format!("{address}（IP 目标，无需解析）"));
        }
        if protected {
            return DiagnosticCheck::unavailable("保护已开启或状态未知，未单独查询系统解析器");
        }
        match dns(host.clone()).await {
            Ok(values) => addresses(values.into_iter().filter_map(|value| value.parse().ok())),
            Err(error) => DiagnosticCheck::failure(format!("{error:#}")),
        }
    };
    let core = async {
        if let Some(address) = address {
            return DiagnosticCheck::unavailable(format!("{address}（IP 目标，无需解析）"));
        }
        let Some(api) = api.as_ref() else {
            return DiagnosticCheck::unavailable("内核未运行或控制接口不可用");
        };
        match tokio::time::timeout(Duration::from_secs(5), api.resolve_domain(&host)).await {
            Ok(Ok(values)) => addresses(values),
            Ok(Err(error)) => DiagnosticCheck::failure(format!("{error:#}")),
            Err(_) => DiagnosticCheck::failure("查询超时"),
        }
    };
    let proxy = async {
        let Some(port) = port else {
            return DiagnosticCheck::unavailable("内核未运行");
        };
        let start = Instant::now();
        match tokio::time::timeout(
            Duration::from_secs(2),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await
        {
            Ok(Ok(_)) => {
                DiagnosticCheck::success(format!("端口可连接 · {} ms", start.elapsed().as_millis()))
            }
            Ok(Err(error)) => DiagnosticCheck::failure(format!("端口不可连接：{error}")),
            Err(_) => DiagnosticCheck::failure("端口连接超时"),
        }
    };
    let (system_dns, core_dns, proxy, (https, https_route), (http, http_route)) = tokio::join!(
        system,
        core,
        proxy,
        protocol_check(https_url.clone(), port, api.clone()),
        protocol_check(http_url.clone(), port, api.clone()),
    );
    let (target, response, route) = select_protocol(
        https_url.origin().ascii_serialization(),
        http_url.origin().ascii_serialization(),
        &https,
        &http,
        https_route,
        http_route,
    );
    Ok(NetworkReport {
        target,
        system_dns,
        core_dns,
        proxy,
        https,
        http,
        response,
        route,
    })
}

fn select_protocol(
    https_target: String,
    http_target: String,
    https: &DiagnosticCheck,
    http: &DiagnosticCheck,
    https_route: DiagnosticCheck,
    http_route: DiagnosticCheck,
) -> (String, DiagnosticCheck, DiagnosticCheck) {
    if https.success == Some(true) {
        (https_target, DiagnosticCheck::success("HTTPS"), https_route)
    } else if http.success == Some(true) {
        (
            http_target,
            DiagnosticCheck::success("HTTP（HTTPS 不可达）"),
            http_route,
        )
    } else {
        (
            https_target,
            if https.success.is_none() && http.success.is_none() {
                DiagnosticCheck::unavailable("未检查")
            } else {
                DiagnosticCheck::failure("HTTPS 与 HTTP 均未连通")
            },
            DiagnosticCheck::unavailable("未建立检测连接"),
        )
    }
}
pub async fn website(input: String, port: u16, exit_ip: bool) -> Result<ProbeResult> {
    website_with_route(input, Some(port), exit_ip).await
}

pub async fn website_system_route(input: String) -> Result<ProbeResult> {
    website_with_route(input, None, false).await
}

async fn website_with_route(
    input: String,
    port: Option<u16>,
    exit_ip: bool,
) -> Result<ProbeResult> {
    let url = validate_url(&input)?;
    let mut builder = crate::network::client_builder().no_proxy();
    if let Some(port) = port {
        builder = builder.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?);
    }
    let client = builder
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;
    let start = Instant::now();
    let response = client
        .get(url)
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;
    let millis = start.elapsed().as_millis();
    let status = response.status().as_u16();
    let detail = if exit_ip {
        if !response.status().is_success() {
            bail!("IP 查询返回 HTTP {status}，未获得有效出口地址");
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(reqwest::Error::without_url)?
        {
            if bytes.len() + chunk.len() > 8192 {
                bail!("IP 查询响应过大");
            }
            bytes.extend_from_slice(&chunk);
        }
        parse_exit_ip(&bytes)?.to_string()
    } else {
        format!("HTTP {status} · 响应头耗时，不是 ICMP 延迟")
    };
    Ok(ProbeResult {
        url: input,
        millis,
        status,
        detail,
    })
}

fn parse_exit_ip(bytes: &[u8]) -> Result<IpAddr> {
    let text = std::str::from_utf8(bytes).context("IP 查询响应不是有效 UTF-8 文本")?;
    if let Ok(address) = text.trim().parse() {
        return Ok(address);
    }
    #[derive(serde::Deserialize)]
    struct ExitAddress {
        ip: IpAddr,
    }
    serde_json::from_str::<ExitAddress>(text)
        .map(|response| response.ip)
        .map_err(|_| anyhow::anyhow!("IP 查询响应不是有效 IP 地址或包含有效 ip 字段的 JSON"))
}

pub async fn dns(host: String) -> Result<Vec<String>> {
    if host.is_empty() || host.contains(['/', ':', ' ']) {
        bail!("DNS 查询请输入域名，例如 github.com");
    }
    let addresses = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host.as_str(), 443)),
    )
    .await??;
    let mut result: Vec<_> = addresses.map(|a| a.ip().to_string()).collect();
    result.sort();
    result.dedup();
    Ok(result)
}

pub fn trace_country(body: &str, host: &str) -> Option<String> {
    let fields: std::collections::BTreeMap<_, _> = body
        .lines()
        .filter_map(|line| line.trim().split_once('='))
        .collect();
    if fields.get("h").copied()? != host {
        return None;
    }
    country_name(fields.get("loc").copied()?)
}

pub const ISO_CODES: &str = "AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS YE YT ZA ZM ZW";

pub fn country_name(code: &str) -> Option<String> {
    let code = code.to_ascii_uppercase();
    if !ISO_CODES.split_whitespace().any(|valid| valid == code) {
        return None;
    }
    let name = match code.as_str() {
        "US" => "美国",
        "CN" => "中国",
        "HK" => "香港",
        "MO" => "澳门",
        "TW" => "台湾",
        "JP" => "日本",
        "SG" => "新加坡",
        "KR" => "韩国",
        "GB" => "英国",
        "CA" => "加拿大",
        "AU" => "澳大利亚",
        "DE" => "德国",
        "FR" => "法国",
        "NL" => "荷兰",
        "IN" => "印度",
        "MY" => "马来西亚",
        "ID" => "印度尼西亚",
        "TH" => "泰国",
        "VN" => "越南",
        "PH" => "菲律宾",
        "NZ" => "新西兰",
        "IT" => "意大利",
        "ES" => "西班牙",
        "CH" => "瑞士",
        "SE" => "瑞典",
        "FI" => "芬兰",
        "NO" => "挪威",
        "RU" => "俄罗斯",
        "BR" => "巴西",
        "MX" => "墨西哥",
        "AR" => "阿根廷",
        "ZA" => "南非",
        "AE" => "阿联酋",
        "TR" => "土耳其",
        "IE" => "爱尔兰",
        "IS" => "冰岛",
        "PL" => "波兰",
        "PT" => "葡萄牙",
        "BE" => "比利时",
        "AT" => "奥地利",
        "IL" => "以色列",
        "SA" => "沙特阿拉伯",
        "CZ" => "捷克",
        _ => return Some(code),
    };
    Some(name.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn request_headers(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        while !request.ends_with(b"\r\n\r\n") {
            let count = socket.read(&mut buffer).await.unwrap();
            if count == 0 {
                assert!(request.is_empty());
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            assert!(request.len() <= 8192);
        }
        request
    }

    async fn probe_response(status: &str, body: &str, exit_ip: bool) -> Result<ProbeResult> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.ends_with(b"\r\n\r\n") {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&buffer[..read]);
                assert!(request.len() <= 8192);
            }
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let result = website("http://probe.invalid/".into(), port, exit_ip).await;
        tokio::time::timeout(Duration::from_secs(15), server)
            .await
            .unwrap()
            .unwrap();
        result
    }

    #[test]
    fn restricts_probe_protocols() {
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("https://user:secret@example.com").is_err());
        assert!(validate_url("https://example.com").is_ok());
    }

    #[test]
    fn diagnostic_accepts_bare_hosts_without_accepting_credentials_or_other_schemes() {
        for (input, expected) in [
            ("example.com", "https://example.com/"),
            ("  example.com/path  ", "https://example.com/path"),
            ("http://example.com:8080/", "http://example.com:8080/"),
            ("https://example.com/", "https://example.com/"),
        ] {
            assert_eq!(diagnostic_url(input).unwrap().as_str(), expected);
        }
        for input in [
            "",
            "not a host",
            "file:///etc/passwd",
            "https://name:password@example.com/",
            "https://example.com/\nother",
        ] {
            assert!(diagnostic_url(input).is_err(), "{input}");
        }
    }

    #[test]
    fn diagnostic_protocol_pair_preserves_paths_and_custom_ports() {
        for (input, https, http) in [
            ("example.com", "https://example.com/", "http://example.com/"),
            (
                "http://example.com/path?q=1",
                "https://example.com/path?q=1",
                "http://example.com/path?q=1",
            ),
            (
                "https://example.com/",
                "https://example.com/",
                "http://example.com/",
            ),
            (
                "example.com:8443/path",
                "https://example.com:8443/path",
                "http://example.com:8443/path",
            ),
            ("[::1]:8080/", "https://[::1]:8080/", "http://[::1]:8080/"),
        ] {
            let pair = protocol_urls(input).unwrap();
            assert_eq!(pair.0.as_str(), https);
            assert_eq!(pair.1.as_str(), http);
        }
    }

    #[test]
    fn protocol_selection_prefers_https_responses_and_keeps_the_matching_route() {
        for https_status in [200, 301, 403, 500] {
            let (target, response, route) = select_protocol(
                "https://example.test".into(),
                "http://example.test".into(),
                &DiagnosticCheck::success(format!("HTTP {https_status}")),
                &DiagnosticCheck::success("HTTP 200"),
                DiagnosticCheck::success("HTTPS 节点"),
                DiagnosticCheck::success("HTTP 节点"),
            );
            assert_eq!(target, "https://example.test");
            assert_eq!(response.detail, "HTTPS");
            assert_eq!(route.detail, "HTTPS 节点");
        }
        let (_, response, route) = select_protocol(
            "https://example.test".into(),
            "http://example.test".into(),
            &DiagnosticCheck::failure("TLS 握手失败"),
            &DiagnosticCheck::success("HTTP 301"),
            DiagnosticCheck::unavailable("未确认"),
            DiagnosticCheck::success("HTTP 节点"),
        );
        assert!(response.detail.contains("HTTPS 不可达"));
        assert_eq!(route.detail, "HTTP 节点");
        for success in [None, Some(false)] {
            let failed = || DiagnosticCheck {
                success,
                detail: "不可用".into(),
            };
            let (_, response, route) = select_protocol(
                "https://example.test".into(),
                "http://example.test".into(),
                &failed(),
                &failed(),
                DiagnosticCheck::success("旧线路"),
                DiagnosticCheck::success("旧线路"),
            );
            assert_eq!(response.success, success);
            assert_eq!(route.success, None);
            assert!(!route.detail.contains("旧线路"));
        }
    }

    #[tokio::test]
    async fn diagnostic_reports_the_exact_proxy_connection_and_does_not_follow_redirects() {
        let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let controller = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_port = proxy.local_addr().unwrap().port();
        let api = crate::api::Api::new(&crate::config::Settings {
            controller_port: controller.local_addr().unwrap().port(),
            secret: "probe-test-secret".into(),
            ..Default::default()
        })
        .unwrap();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let (finish, wait) = tokio::sync::oneshot::channel();
        let proxy_task = tokio::spawn(async move {
            let mut sender = Some(sender);
            let mut wait = Some(wait);
            let mut transfer = None;
            for _ in 0..3 {
                let (mut socket, peer) = proxy.accept().await.unwrap();
                let request = request_headers(&mut socket).await;
                if request.is_empty() {
                    continue;
                }
                if String::from_utf8_lossy(&request).starts_with("CONNECT ") {
                    socket
                        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await
                        .unwrap();
                    continue;
                }
                assert!(String::from_utf8_lossy(&request).starts_with("GET http://"));
                sender.take().unwrap().send(peer).unwrap();
                socket
                    .write_all(b"HTTP/1.1 302 Found\r\nLocation: http://other.invalid/\r\nContent-Length: 1\r\n\r\nx")
                    .await
                    .unwrap();
                let wait = wait.take().unwrap();
                transfer = Some(tokio::spawn(async move {
                    let _ = wait.await;
                    drop(socket);
                }));
            }
            if let Some(transfer) = transfer {
                transfer.await.unwrap();
            }
        });
        let controller_task = tokio::spawn(async move {
            let (mut socket, _) = controller.accept().await.unwrap();
            let request = request_headers(&mut socket).await;
            assert!(
                String::from_utf8_lossy(&request)
                    .to_ascii_lowercase()
                    .contains("authorization: bearer probe-test-secret")
            );
            let source = receiver.await.unwrap();
            let body = serde_json::json!({"connections":[{"metadata":{
                "sourceIP":source.ip().to_string(),"sourcePort":source.port(),
                "network":"tcp","host":"127.0.0.1"},
                "chains":["🇺🇸 节点","AUTO","PROXY"],"rule":"DomainSuffix",
                "rulePayload":"example.test"}]})
            .to_string();
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
        });
        let report = diagnose(
            "http://127.0.0.1/private?token=test",
            Some(proxy_port),
            Some(api),
            false,
        )
        .await
        .unwrap();
        assert_eq!(report.https.success, Some(false));
        assert!(report.http.detail.contains("HTTP 302"));
        assert!(report.response.detail.contains("HTTPS 不可达"));
        assert_eq!(report.target, "http://127.0.0.1");
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("token=test")
        );
        assert!(report.route.detail.contains("PROXY → AUTO → 🇺🇸 节点"));
        assert!(report.route.detail.contains("DomainSuffix"));
        assert!(report.route.detail.contains("example.test"));
        finish.send(()).unwrap();
        proxy_task.await.unwrap();
        controller_task.await.unwrap();
    }

    #[tokio::test]
    async fn diagnostic_without_core_never_starts_or_bypasses_it() {
        let report = diagnose("localhost", None, None, true).await.unwrap();
        assert_eq!(report.system_dns.success, None);
        assert_eq!(report.core_dns.success, None);
        assert_eq!(report.proxy.success, None);
        assert_eq!(report.https.success, None);
        assert_eq!(report.http.success, None);
        assert_eq!(report.response.success, None);
        assert_eq!(report.route.success, None);
        let report = diagnose("[::1]", None, None, false).await.unwrap();
        assert_eq!(report.system_dns.success, None);
        assert_eq!(report.core_dns.success, None);
        assert!(report.system_dns.detail.contains("无需解析"));
        assert_eq!(report.response.success, None);
    }

    #[test]
    fn exit_ip_accepts_plain_text_and_json_addresses() {
        for (body, expected) in [
            ("203.0.113.9", "203.0.113.9"),
            (" \r\n203.0.113.9\n", "203.0.113.9"),
            (r#"{"ip":"203.0.113.9"}"#, "203.0.113.9"),
            ("2001:db8::9", "2001:db8::9"),
            (r#"{"ip":"2001:db8::9","extra":true}"#, "2001:db8::9"),
        ] {
            assert_eq!(
                parse_exit_ip(body.as_bytes()).unwrap(),
                expected.parse::<IpAddr>().unwrap()
            );
        }
    }

    #[test]
    fn exit_ip_rejects_invalid_responses_without_echoing_body() {
        for body in [
            "",
            "203.0.113.9 trailing-data",
            r#""203.0.113.9""#,
            r#"{"ip":"203.0.113.9"} trailing-data"#,
            r#"{"ip":null}"#,
            r#"{"ip":123}"#,
            r#"{"address":"203.0.113.9"}"#,
            r#"{"ip":"203.0.113.9:443"}"#,
            r#"{"ip":"example.invalid"}"#,
            r#"{"ip":"{{PRIVATE_RESPONSE_VALUE}}"}"#,
            "<html>{{PRIVATE_RESPONSE_VALUE}}</html>",
        ] {
            let error = parse_exit_ip(body.as_bytes()).unwrap_err().to_string();
            assert!(error.contains("IP 查询响应"));
            assert!(!error.contains("PRIVATE_RESPONSE_VALUE"));
        }
        assert!(parse_exit_ip(&[0xff]).is_err());
    }

    #[tokio::test]
    async fn exit_ip_queries_accept_text_and_json_through_proxy() {
        for (body, expected) in [
            ("203.0.113.9", "203.0.113.9"),
            (r#"{"ip":"2001:db8::9"}"#, "2001:db8::9"),
        ] {
            let result = probe_response("200 OK", body, true).await.unwrap();
            assert_eq!(result.status, 200);
            assert_eq!(result.detail, expected);
        }
    }

    #[tokio::test]
    async fn exit_ip_queries_reject_http_errors_and_oversized_bodies() {
        let error = probe_response("503 Service Unavailable", r#"{"ip":"203.0.113.9"}"#, true)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("HTTP 503"));
        let error = probe_response("200 OK", &" ".repeat(8193), true)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("IP 查询响应过大"));
        let response = probe_response("403 Forbidden", "<html>Forbidden</html>", false)
            .await
            .unwrap();
        assert_eq!(response.status, 403);
    }

    #[test]
    fn country_requires_explicit_same_site_evidence() {
        assert_eq!(
            trace_country("h=chatgpt.com\nloc=JP\ncolo=LAX\nip=1.2.3.4", "chatgpt.com"),
            Some("日本".into())
        );
        assert!(trace_country("h=other.example\nloc=US", "chatgpt.com").is_none());
        assert!(trace_country("h=chatgpt.com\nloc=XX", "chatgpt.com").is_none());
        assert!(country_name("EN").is_none());
    }
}
