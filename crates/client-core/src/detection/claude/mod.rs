//! Observable region signals, with explicit provenance rather than a risk score.
pub mod additional;
pub mod audit;
pub mod browser;
pub mod enhanced;
pub mod guard;
pub mod observe;
pub mod preferences;
pub use crate::protection;

use crate::{api, probe::country_name};
use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, net::IpAddr, time::Duration};

pub const REPORT_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Environment {
    #[default]
    Core,
    Web,
    Desktop,
    Code,
}
impl Environment {
    pub fn name(self) -> &'static str {
        match self {
            Self::Core => "内核探测",
            Self::Web => "网页版",
            Self::Desktop => "Claude Desktop",
            Self::Code => "Claude Code",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Group {
    #[default]
    Exit,
    Browser,
    WebRtc,
    Dns,
    Desktop,
    Code,
    Quality,
    Cloudflare,
    Connectivity,
}
impl Group {
    pub const ALL: [Self; 9] = [
        Self::Exit,
        Self::Browser,
        Self::WebRtc,
        Self::Dns,
        Self::Desktop,
        Self::Code,
        Self::Quality,
        Self::Cloudflare,
        Self::Connectivity,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Exit => "网络出口与分流",
            Self::Browser => "浏览器地区特征",
            Self::WebRtc => "WebRTC",
            Self::Dns => "DNS 地区",
            Self::Desktop => "Claude Desktop",
            Self::Code => "Claude Code",
            Self::Quality => "IP 归属与信誉",
            Self::Cloudflare => "Cloudflare",
            Self::Connectivity => "连通性",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Evidence {
    Measured,
    Inferred,
    #[default]
    Unknown,
}
impl Evidence {
    pub fn name(self) -> &'static str {
        match self {
            Self::Measured => "实测",
            Self::Inferred => "辅助线索",
            Self::Unknown => "未确认",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub group: Group,
    pub title: String,
    pub value: String,
    pub source: String,
    pub evidence: Evidence,
    pub detail: String,
    pub advice: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub ip: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub risk: bool,
    /// The measured response was an interactive challenge, not evidence of a
    /// regional or routing failure. Keep its observation separate from risk.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub challenged: bool,
}
impl Signal {
    pub fn new(
        group: Group,
        title: &str,
        value: impl Into<String>,
        source: &str,
        evidence: Evidence,
        detail: &str,
    ) -> Self {
        Self {
            group,
            title: title.into(),
            value: value.into(),
            source: source.into(),
            evidence,
            detail: detail.into(),
            advice: String::new(),
            country: String::new(),
            ip: String::new(),
            risk: false,
            challenged: false,
        }
    }
    /// A real challenge response still cannot establish service usability.
    /// Present it with other contextual observations instead of a safe verdict.
    pub fn context_only(&self) -> bool {
        self.challenged || self.evidence != Evidence::Measured
    }
    pub fn unknown(group: Group, title: &str, source: &str, detail: impl Into<String>) -> Self {
        Self::new(
            group,
            title,
            "未确认",
            source,
            Evidence::Unknown,
            &detail.into(),
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Route {
    pub host: String,
    pub rule: String,
    pub chain: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub time: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub outcome: String,
}
impl Route {
    pub fn risky(&self) -> bool {
        self.outcome == "连接失败"
            || self.outcome.starts_with("异常连接")
            || self
                .chain
                .split(" → ")
                .any(|part| matches!(part.trim(), "DIRECT" | "REJECT" | "REJECT-DROP"))
            || self.chain.ends_with("[DIRECT]")
            || self.chain.ends_with("[REJECT]")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub schema: u8,
    pub environment: Environment,
    pub generated_at: String,
    pub context: String,
    pub signals: Vec<Signal>,
    #[serde(default)]
    pub routes: Vec<Route>,
}
impl Report {
    pub fn new(environment: Environment, context: &str) -> Self {
        Self {
            schema: 1,
            environment,
            generated_at: chrono::Utc::now().to_rfc3339(),
            context: context.into(),
            signals: vec![],
            routes: vec![],
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema != 1 || self.signals.len() > 80 || self.routes.len() > 32 {
            bail!("检测报告版本或项目数量无效");
        }
        for value in [self.context.as_str(), self.generated_at.as_str()]
            .into_iter()
            .chain(self.signals.iter().flat_map(|s| {
                [
                    s.title.as_str(),
                    s.value.as_str(),
                    s.source.as_str(),
                    s.detail.as_str(),
                    s.advice.as_str(),
                    s.country.as_str(),
                    s.ip.as_str(),
                ]
            }))
            .chain(self.routes.iter().flat_map(|r| {
                [
                    r.host.as_str(),
                    r.rule.as_str(),
                    r.chain.as_str(),
                    r.time.as_str(),
                    r.outcome.as_str(),
                ]
            }))
        {
            if value.len() > 2048 || value.chars().any(|c| c.is_control() && c != '\n') {
                bail!("检测报告字段过长或包含控制字符");
            }
        }
        for signal in &self.signals {
            if !signal.country.is_empty() && country_name(&signal.country).is_none() {
                bail!("检测报告包含无效地区");
            }
            if !signal.ip.is_empty() {
                let _: IpAddr = signal.ip.parse().context("检测报告包含无效 IP")?;
            }
        }
        Ok(())
    }
    pub fn baseline_country(&self) -> Option<&str> {
        self.signals
            .iter()
            .find(|s| {
                s.group == Group::Exit && s.evidence == Evidence::Measured && !s.country.is_empty()
            })
            .map(|s| s.country.as_str())
    }
    pub fn differs(&self, signal: &Signal) -> bool {
        // Preferences are not a physical-location assertion. Display the
        // difference and its evidence, never a regional-denial probability.
        signal.risk
            || (!signal.country.is_empty()
                && self
                    .baseline_country()
                    .is_some_and(|base| base != signal.country))
            || (signal.group == Group::WebRtc
                && !signal.ip.is_empty()
                && self
                    .signals
                    .iter()
                    .any(|s| s.group == Group::Exit && !s.ip.is_empty())
                && !self
                    .signals
                    .iter()
                    .any(|s| s.group == Group::Exit && s.ip == signal.ip))
    }
    pub fn counts(&self) -> (usize, usize, usize) {
        (
            self.signals
                .iter()
                .filter(|s| s.evidence == Evidence::Measured)
                .count(),
            self.signals.iter().filter(|s| self.differs(s)).count(),
            self.signals
                .iter()
                .filter(|s| s.evidence == Evidence::Unknown)
                .count(),
        )
    }
}

pub fn claude_host(host: &str) -> bool {
    protection::owned_host(host)
}
/// Deserialize one connection at a time and retain only relevant route facts.
/// Observing the Desktop must not retain another complete connection snapshot.
#[derive(Debug, Default, Deserialize)]
pub struct ObservedConnections {
    #[serde(default, deserialize_with = "filtered_connections")]
    pub connections: Vec<Route>,
}
fn filtered_connections<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<Route>, D::Error> {
    struct Filter;
    impl<'de> serde::de::Visitor<'de> for Filter {
        type Value = Vec<Route>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("connections or null")
        }
        fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(Vec::new())
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(Vec::new())
        }
        fn visit_some<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            deserializer.deserialize_seq(self)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut found = Vec::new();
            while found.len() < 32 {
                let Some(connection) = seq.next_element::<api::Connection>()? else {
                    return Ok(found);
                };
                if let Some(route) = connection_route(connection)
                    && !found.iter().any(|r: &Route| {
                        r.host == route.host && r.rule == route.rule && r.chain == route.chain
                    })
                {
                    found.push(route);
                }
            }
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
            Ok(found)
        }
    }
    deserializer.deserialize_option(Filter)
}

fn connection_route(connection: api::Connection) -> Option<Route> {
    let host = if connection.metadata.host.is_empty() {
        connection.metadata.sniff_host.as_str()
    } else {
        &connection.metadata.host
    };
    if !(claude_host(host) || protection::associated_host(host)) {
        return None;
    }
    Some(Route {
        host: host.chars().take(253).collect(),
        rule: format!(
            "{} {}",
            connection.rule.as_str(),
            connection.rule_payload.as_str()
        )
        .chars()
        .take(512)
        .collect(),
        chain: connection
            .chains
            .iter()
            .rev()
            .take(16)
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" → ")
            .chars()
            .take(512)
            .collect(),
        ..Default::default()
    })
}

pub fn client(port: Option<u16>) -> Result<Client> {
    let mut builder = Client::builder()
        .no_proxy()
        .user_agent("Clash-of-Rust/region-diagnostics")
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(port) = port {
        builder = builder.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?);
    }
    Ok(builder.build()?)
}

pub async fn response_bytes(mut response: reqwest::Response) -> Result<Vec<u8>> {
    response.error_for_status_ref()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > REPORT_LIMIT {
            bail!("检测响应超过 64 KiB");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub async fn locate(client: &Client, ip: &str) -> Option<(String, String)> {
    let ip: IpAddr = ip.parse().ok()?;
    if !public_ip(ip) {
        return None;
    }
    let response = client
        .get(format!("https://api.ip.sb/geoip/{ip}"))
        .send()
        .await
        .ok()?;
    let json: serde_json::Value =
        serde_json::from_slice(&response_bytes(response).await.ok()?).ok()?;
    if json.get("ip")?.as_str()?.parse::<IpAddr>().ok()? != ip {
        return None;
    }
    let code = json.get("country_code")?.as_str()?.to_ascii_uppercase();
    country_name(&code)?;
    let asn = json
        .get("asn")
        .map_or(String::new(), |value| value.to_string());
    let organization = json
        .get("asn_organization")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    Some((
        code,
        format!("AS{asn} · {organization}")
            .chars()
            .take(256)
            .collect(),
    ))
}

pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.octets()[0] == 0
                || ip.octets()[0] >= 240
                || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
                || (ip.octets()[0] == 198 && matches!(ip.octets()[1], 18 | 19)))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let octets = ip.octets();
            octets[0] & 0xe0 == 0x20
                && !(ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_multicast()
                    || octets[0] & 0xfe == 0xfc
                    || (octets[0] == 0xfe && octets[1] & 0xc0 == 0x80)
                    || (octets[..4] == [0x20, 0x01, 0x0d, 0xb8]))
        }
    }
}

async fn exit_signal(client: &Client, url: &str, title: &str, source: &str, v6: bool) -> Signal {
    let result: Result<String> = async {
        let bytes = response_bytes(client.get(url).send().await?).await?;
        let json: serde_json::Value = serde_json::from_slice(&bytes)?;
        let value = json
            .get("ip")
            .and_then(|v| v.as_str())
            .context("响应没有 IP")?;
        let ip: IpAddr = value.parse()?;
        if ip.is_ipv6() != v6 || !public_ip(ip) {
            bail!("响应 IP 类型或范围不符合预期");
        }
        Ok(ip.to_string())
    }
    .await;
    match result {
        Ok(ip) => {
            ip_signal(
                client,
                Group::Exit,
                title,
                &ip,
                source,
                "此测试域名看到的出口；不代表 Claude 目标域名出口。",
                Evidence::Measured,
            )
            .await
        }
        Err(_) => Signal::unknown(
            Group::Exit,
            title,
            source,
            "未获得有效结果；可能是未启用该协议或请求被阻止，不能据此判断地区。",
        ),
    }
}

pub async fn ip_signal(
    client: &Client,
    group: Group,
    title: &str,
    ip: &str,
    source: &str,
    detail: &str,
    evidence: Evidence,
) -> Signal {
    let mut signal = Signal::new(group, title, ip, source, evidence, detail);
    signal.ip = ip.into();
    if let Some((code, asn)) = locate(client, ip).await {
        signal.value = format!("{ip} · {}", country_name(&code).unwrap());
        signal.country = code;
        signal.detail.push_str(&format!(
            " 地区与运营商来自 IP.SB 数据库：{asn}；数据库可能与 Claude 的判断不同。"
        ));
    } else {
        signal
            .detail
            .push_str(" IP 已获得，地区数据库未返回有效结果。");
    }
    signal
}

pub async fn network(client: &Client, report: &mut Report, source: &str) {
    let (v4, v6) = tokio::join!(
        exit_signal(
            client,
            "https://api.ipify.org?format=json",
            "IPv4 出口",
            source,
            false
        ),
        exit_signal(
            client,
            "https://api6.ipify.org?format=json",
            "IPv6 出口",
            source,
            true
        )
    );
    report.signals.extend([v4, v6]);
    for host in ["claude.ai", "claude.com", "api.anthropic.com"] {
        let mut signal = Signal::unknown(
            Group::Exit,
            &format!("{host} 出口"),
            source,
            "目标站点未返回有效的同域 trace；没有用其他域名的出口代替。403、挑战页面或超时不直接归因于地区限制。",
        );
        if let Ok(response) = client
            .get(format!("https://{host}/cdn-cgi/trace"))
            .send()
            .await
            && let Ok(bytes) = response_bytes(response).await
            && let Ok(body) = std::str::from_utf8(&bytes)
            && let Some((ip, code)) = parse_trace(body, host)
        {
            signal = Signal::new(
                Group::Exit,
                &format!("{host} 出口"),
                format!("{ip} · {}", country_name(&code).unwrap()),
                source,
                Evidence::Measured,
                "地区来自此目标域名的 Cloudflare trace；未验证账号的服务资格。",
            );
            signal.ip = ip;
            signal.country = code;
        }
        report.signals.push(signal);
    }
}

pub fn parse_trace(body: &str, host: &str) -> Option<(String, String)> {
    let fields: std::collections::BTreeMap<_, _> =
        body.lines().filter_map(|l| l.split_once('=')).collect();
    if fields.get("h").copied()? != host {
        return None;
    }
    let ip: IpAddr = fields.get("ip")?.parse().ok()?;
    if !public_ip(ip) {
        return None;
    }
    let code = fields.get("loc")?.to_ascii_uppercase();
    country_name(&code)?;
    Some((ip.to_string(), code))
}

pub async fn dns_id(client: &Client) -> Result<String> {
    let bytes = response_bytes(client.get("https://bash.ws/id").send().await?).await?;
    let id = std::str::from_utf8(&bytes)?.trim();
    if !valid_dns_id(id) {
        bail!("DNS 测试服务未返回有效编号");
    }
    Ok(id.into())
}
pub fn valid_dns_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}
pub async fn dns_results(client: &Client, id: &str, source: &str) -> Result<Vec<Signal>> {
    if !valid_dns_id(id) {
        bail!("DNS 测试编号无效");
    }
    let bytes = response_bytes(
        client
            .get(format!("https://bash.ws/dnsleak/test/{id}?json=2"))
            .send()
            .await?,
    )
    .await?;
    parse_dns(&bytes, source)
}
fn parse_dns(bytes: &[u8], source: &str) -> Result<Vec<Signal>> {
    let records: Vec<serde_json::Value> = serde_json::from_slice(bytes)?;
    let mut signals = Vec::new();
    let mut seen = BTreeSet::new();
    for record in records {
        if record.get("type").and_then(|v| v.as_str()) != Some("dns") {
            continue;
        }
        let Some(ip) = record
            .get("ip")
            .and_then(|v| v.as_str())
            .and_then(|ip| ip.parse::<IpAddr>().ok())
        else {
            continue;
        };
        if !public_ip(ip) || !seen.insert(ip) {
            continue;
        }
        let code = record
            .get("country")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_ascii_uppercase();
        let mut signal = Signal::new(
            Group::Dns,
            "实际递归 DNS",
            ip.to_string(),
            source,
            Evidence::Measured,
            "bash.ws 权威 DNS 记录到的递归服务器；地区不一致是线索，不等同于泄漏或 Claude 拒绝服务的证据。",
        );
        signal.ip = ip.to_string();
        if let Some(name) = country_name(&code) {
            signal.country = code;
            signal.value = format!("{ip} · {name}");
        }
        signal.advice = "检查浏览器独立 DoH、系统 DNS 与 TUN DNS 劫持设置。".into();
        signals.push(signal);
        if signals.len() == 12 {
            break;
        }
    }
    if signals.is_empty() {
        bail!("权威 DNS 未返回递归服务器，不能判断是否存在泄漏");
    }
    Ok(signals)
}

pub async fn system_dns(client: &Client, source: &str) -> Vec<Signal> {
    let result: Result<Vec<Signal>> = async {
        let id = dns_id(client).await?;
        for index in 0..6 {
            // Local lookup is deliberate: it measures the system resolver,
            // not the HTTP proxy's resolver or the browser's independent DoH.
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                tokio::net::lookup_host((format!("{index}.{id}.bash.ws"), 443)),
            )
            .await;
        }
        dns_results(client, &id, source).await
    }
    .await;
    result.unwrap_or_else(|_| {
        vec![Signal::unknown(
            Group::Dns,
            "系统递归 DNS",
            source,
            "测试服务不可用或没有返回有效记录；没有把配置中的 DNS 地址当成实测结果。",
        )]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traces_require_host_country_and_public_ip() {
        assert_eq!(
            parse_trace("h=claude.ai\nip=8.8.8.8\nloc=US", "claude.ai"),
            Some(("8.8.8.8".into(), "US".into()))
        );
        for body in [
            "h=other.test\nip=8.8.8.8\nloc=US",
            "h=claude.ai\nip=127.0.0.1\nloc=US",
            "h=claude.ai\nip=8.8.8.8\nloc=XX",
            "challenge page",
        ] {
            assert!(parse_trace(body, "claude.ai").is_none());
        }
    }
    #[test]
    fn ipv6_and_private_candidates_are_classified_without_regex() {
        for ip in [
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:192.168.1.1",
            "198.18.0.1",
            "100.64.0.1",
            "203.0.113.2",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "2606:4700:4700::1111", "::ffff:8.8.8.8"] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
    }
    #[test]
    fn dns_uses_server_records_not_provider_conclusions() {
        let bytes = br#"[{"type":"conclusion","ip":"No leak"},{"type":"dns","ip":"8.8.8.8","country":"us"},{"type":"dns","ip":"8.8.8.8","country":"us"}]"#;
        let result = parse_dns(bytes, "browser").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].country, "US");
        assert!(parse_dns(b"[]", "browser").is_err());
        assert!(!valid_dns_id("../../secret"));
    }
    #[test]
    fn routing_does_not_match_lookalike_hosts_or_keep_unrelated_connections() {
        assert!(claude_host("API.ANTHROPIC.COM."));
        assert!(!claude_host("evilclaude.ai"));
        assert!(!claude_host("claude.ai.evil.test"));
        let document: ObservedConnections = serde_json::from_value(serde_json::json!({"connections":[{"id":"1","metadata":{"host":"api.anthropic.com"},"chains":["node","Proxy"],"rule":"MATCH"},{"id":"2","metadata":{"host":"example.com"}}]})).unwrap();
        assert_eq!(document.connections.len(), 1);
        assert_eq!(document.connections[0].chain, "Proxy → node");
    }
    #[test]
    fn observer_discards_unrelated_rows_and_bounds_retained_route_combinations() {
        let mut rows = (0..1000)
            .map(|i| serde_json::json!({"id":i.to_string(),"metadata":{"host":"example.com"}}))
            .collect::<Vec<_>>();
        rows.extend((0..60).map(|i| serde_json::json!({"id":i.to_string(),"metadata":{"host":format!("{i}.claude.ai")},"chains":["node","Proxy"]})));
        let document: ObservedConnections =
            serde_json::from_value(serde_json::json!({"connections":rows})).unwrap();
        assert_eq!(document.connections.len(), 32);
        let empty: ObservedConnections = serde_json::from_str(r#"{"connections":null}"#).unwrap();
        assert!(empty.connections.is_empty());
    }
}
