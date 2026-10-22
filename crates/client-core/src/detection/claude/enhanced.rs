//! Bounded external evidence. Missing provider fields never become safe verdicts.
use super::{Evidence, Group, Report, Signal, public_ip, response_bytes};
use crate::probe::country_name;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeSet, net::IpAddr};

#[derive(Deserialize)]
struct Regions {
    source: String,
    source_date: String,
    reviewed_at: String,
    codes: String,
}
pub fn region_supported(code: &str) -> bool {
    let regions: Regions = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../resources/claude-check/supported-regions.json"
    )))
    .expect("reviewed region catalog");
    regions
        .codes
        .split_ascii_whitespace()
        .any(|c| c.eq_ignore_ascii_case(code))
}
pub fn supported_region(code: &str, ip: &str) -> Signal {
    let regions: Regions = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../resources/claude-check/supported-regions.json"
    )))
    .expect("reviewed region catalog");
    let supported = region_supported(code);
    let mut signal = Signal::new(
        Group::Exit,
        "Claude 官方支持地区",
        format!(
            "{} · {}",
            country_name(code).unwrap_or_else(|| code.into()),
            if supported {
                "列于支持地区"
            } else {
                "未列于支持地区"
            }
        ),
        &regions.source,
        Evidence::Inferred,
        &format!(
            "依据目标域名实测的出口地区与官方列表对照；列表日期 {}，核对日期 {}。不读取账号，也不保证账号资格。",
            regions.source_date, regions.reviewed_at
        ),
    );
    signal.ip = ip.into();
    signal.country = code.into();
    signal.risk = !supported;
    signal
}
fn clip(value: &str) -> String {
    value.chars().take(256).collect()
}
pub fn cloudflare_trace(body: &str, host: &str, source: &str) -> Signal {
    if super::parse_trace(body, host).is_none() {
        return Signal::unknown(
            Group::Cloudflare,
            "Cloudflare 网络环境",
            source,
            "未取得有效同域 trace，不能判断边缘节点或 WARP 状态。",
        );
    }
    let fields: std::collections::BTreeMap<_, _> =
        body.lines().filter_map(|l| l.split_once('=')).collect();
    let value = [
        ("colo", "边缘节点"),
        ("http", "HTTP"),
        ("tls", "TLS"),
        ("warp", "WARP"),
    ]
    .iter()
    .filter_map(|(key, label)| fields.get(key).map(|v| format!("{label}：{}", clip(v))))
    .collect::<Vec<_>>()
    .join(" · ");
    Signal::new(
        Group::Cloudflare,
        "Cloudflare 网络环境",
        value,
        source,
        Evidence::Measured,
        "边缘机房代码不代表出口 IP 的国家，也不是 Claude 或 Cloudflare 的官方风控评分。",
    )
}

pub fn quality_signals(value: &Value, ip: &str) -> Vec<Signal> {
    if value
        .get("ip")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<IpAddr>().ok())
        != ip.parse::<IpAddr>().ok()
        || value.get("error").is_some()
    {
        return vec![Signal::unknown(
            Group::Quality,
            "IP 归属与信誉",
            "ipapi.is",
            "供应商未返回匹配的 IP 记录，无法确认。",
        )];
    }
    let mut signals = vec![];
    let title = format!("IP 归属 · {ip}");
    let company = value
        .get("company")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/company/name").and_then(Value::as_str))
        .unwrap_or("未提供");
    let asn = value
        .get("asn")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| value.pointer("/asn/asn").map(|v| format!("AS{v}")))
        .unwrap_or_else(|| "未提供".into());
    let country = value
        .get("country")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/location/country").and_then(Value::as_str))
        .unwrap_or("未提供");
    signals.push(Signal::new(Group::Quality,&title,format!("{} · {} · {}",clip(company),clip(&asn),clip(country)),"ipapi.is 独立 IP 数据库",Evidence::Measured,"记录注册归属、ASN 和地区来源；与 IP.SB、目标 trace 的判断分别展示，不从公司名称猜测住宅或原生 IP。"));
    for (key, title, risk) in [
        ("is_datacenter", "机房地址", false),
        ("is_mobile", "移动网络", false),
        ("is_proxy", "代理地址标签", false),
        ("is_vpn", "VPN 地址标签", false),
        ("is_tor", "Tor 出口", true),
        ("is_crawler", "爬虫地址标签", true),
        ("is_abuser", "滥用记录", true),
    ] {
        let mut signal = if let Some(flag) = value.get(key).and_then(Value::as_bool) {
            Signal::new(
                Group::Quality,
                &format!("{title} · {ip}"),
                if flag {
                    "供应商标记为是"
                } else {
                    "供应商未标记"
                },
                "ipapi.is 信誉数据库",
                Evidence::Inferred,
                "第三方标签可能过时或误报；未标记不等于 Claude 判定可信，代理标签本身不作为异常流量拦截依据。",
            )
        } else {
            Signal::unknown(
                Group::Quality,
                &format!("{title} · {ip}"),
                "ipapi.is",
                "该接口没有返回此字段；匿名额度只提供归属信息，完整信誉标签需要用户自己的 API 密钥。",
            )
        };
        signal.ip = ip.into();
        signal.risk = risk && value.get(key).and_then(Value::as_bool) == Some(true);
        signals.push(signal);
    }
    if let Some(score) = value
        .pointer("/company/abuser_score")
        .or_else(|| value.pointer("/asn/abuser_score"))
        .and_then(Value::as_str)
    {
        signals.push(Signal::new(
            Group::Quality,
            "供应商滥用评分",
            clip(score),
            "ipapi.is",
            Evidence::Inferred,
            "保留供应商原始值；不是 Claude 官方评分，不自行合成可用性分数。",
        ));
    }
    signals
}

pub async fn extend(client: &Client, report: &mut Report, reputation_key: Option<&str>) {
    let targets: Vec<_> = report
        .signals
        .iter()
        .filter(|s| {
            s.group == Group::Exit
                && s.evidence == Evidence::Measured
                && s.title.contains("claude")
                && !s.country.is_empty()
        })
        .map(|s| (s.country.clone(), s.ip.clone()))
        .collect();
    let mut seen = BTreeSet::new();
    for (code, ip) in targets {
        if seen.insert(code.clone()) {
            report.signals.push(supported_region(&code, &ip));
        }
    }
    let ips: Vec<_> = report
        .signals
        .iter()
        .filter(|s| s.group == Group::Exit && !s.ip.is_empty())
        .map(|s| s.ip.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(2)
        .collect();
    for ip in ips {
        if !ip.parse::<IpAddr>().is_ok_and(public_ip) {
            continue;
        }
        let mut payload = serde_json::json!({"q":ip});
        if let Some(key) = reputation_key.filter(|s| !s.is_empty()) {
            payload["key"] = Value::from(key);
        }
        let result = async {
            let response = client
                .post("https://api.ipapi.is/")
                .json(&payload)
                .send()
                .await?;
            anyhow::ensure!(response.status().is_success(), "信誉服务未返回有效响应");
            let value: Value = serde_json::from_slice(&response_bytes(response).await?)?;
            Ok::<_, anyhow::Error>(quality_signals(&value, &ip))
        }
        .await;
        report.signals.extend(result.unwrap_or_else(|_| {
            vec![Signal::unknown(
                Group::Quality,
                &format!("IP 归属与信誉 · {ip}"),
                "ipapi.is",
                "请求被阻止、额度不足或供应商未返回有效数据；没有据此判定安全，也不会反复重试。",
            )]
        }));
    }
    let trace = async {
        let response = client
            .get("https://www.cloudflare.com/cdn-cgi/trace")
            .send()
            .await?;
        let bytes = response_bytes(response).await?;
        Ok::<_, anyhow::Error>(cloudflare_trace(
            std::str::from_utf8(&bytes)?,
            "www.cloudflare.com",
            "通过内核访问 Cloudflare 同域 trace",
        ))
    }
    .await;
    report.signals.push(trace.unwrap_or_else(|_| {
        Signal::unknown(
            Group::Cloudflare,
            "Cloudflare 网络环境",
            "Cloudflare trace",
            "未得到有效结果，不能确认。",
        )
    }));
    let (web, console, api) = tokio::join!(
        connectivity(client, "claude.ai"),
        connectivity(client, "platform.claude.com"),
        connectivity(client, "api.anthropic.com")
    );
    report.signals.extend([web, console, api]);
}

async fn connectivity(client: &Client, host: &str) -> Signal {
    connectivity_at(client, host, &format!("https://{host}/")).await
}

async fn connectivity_at(client: &Client, host: &str, url: &str) -> Signal {
    let mut timings = Vec::new();
    let mut status = None;
    let mut challenge = false;
    for _ in 0..3 {
        let started = std::time::Instant::now();
        if let Ok(response) = client.head(url).send().await {
            timings.push(started.elapsed().as_millis());
            status = Some(response.status().as_u16());
            challenge |= response
                .headers()
                .get("cf-mitigated")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.eq_ignore_ascii_case("challenge"));
        }
    }
    timings.sort_unstable();
    let mut signal = if timings.is_empty() {
        Signal::unknown(
            Group::Connectivity,
            &format!("{host} 连通性"),
            "通过内核发起轻量 HTTPS 请求",
            "三次请求均未取得响应，不能确认原因。",
        )
    } else {
        Signal::new(
            Group::Connectivity,
            &format!("{host} 连通性"),
            format!(
                "响应 {}/3 · 中位 {} ms · HTTP {}",
                timings.len(),
                timings[timings.len() / 2],
                status.unwrap()
            ),
            "通过内核发起轻量 HTTPS 请求",
            Evidence::Measured,
            "测量包含连接、TLS 与服务器响应，不是 ICMP Ping；收到 HTTP 响应不等于可登录、可对话或验证码通过。",
        )
    };
    if challenge {
        signal.challenged = true;
        signal.value.push_str(" · 需要浏览器验证");
        signal.detail.push_str(" 服务器要求 Cloudflare 人机验证；此响应不能证明地区或线路存在风险。检测未自动完成验证码，也没有提交账号信息。");
        signal.advice = "可用浏览器打开 Claude，确认是否需要验证及能否正常使用。".into();
    }
    signal
}

pub async fn configuration(
    api: &crate::api::Api,
    settings: &crate::config::Settings,
) -> Vec<Signal> {
    let Some(protection) = &settings.claude_protection else {
        return vec![];
    };
    let result = api.get::<Value>("configs").await;
    let mut signals = vec![];
    for (title, pointer, expected) in [
        ("TUN 实际状态", "/tun/enable", Value::Bool(true)),
        ("严格路由实际状态", "/tun/strict-route", Value::Bool(true)),
        ("运行模式", "/mode", Value::from(protection.run_mode())),
        ("IPv6 实际状态", "/ipv6", Value::Bool(true)),
    ] {
        let value = result.as_ref().ok().and_then(|v| v.pointer(pointer));
        let mut signal = if let Some(value) = value {
            Signal::new(
                Group::Dns,
                title,
                value.to_string(),
                "内核当前配置接口",
                Evidence::Measured,
                "当前接口值与保护要求对照；仍需结合浏览器、DNS 和 UDP 实测核验接管范围。",
            )
        } else {
            Signal::unknown(
                Group::Dns,
                title,
                "内核当前配置接口",
                "接口未返回字段或无法连接，不能按保存的设置代替实际状态。",
            )
        };
        signal.risk = value.is_some_and(|v| *v != expected);
        signals.push(signal);
    }
    signals.push(Signal::new(Group::Dns,"公网 DNS 保护策略",format!("DoH 经 {}",protection.node),"保护运行时配置",Evidence::Inferred,"公网解析指定保护节点；节点引导解析使用直连加密 DNS，局域网名称保留本地解析。浏览器独立 DoH 不由这个配置直接控制。"));
    signals
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_flags_are_not_safe_and_provider_labels_do_not_trigger_network_blocking() {
        let signals = quality_signals(
            &serde_json::json!({"ip":"1.1.1.1","company":"Cloudflare","asn":"AS13335","country":"Australia"}),
            "1.1.1.1",
        );
        assert_eq!(
            signals
                .iter()
                .filter(|s| s.evidence == Evidence::Unknown)
                .count(),
            7
        );
        assert!(!signals.iter().any(|s| s.risk));
        let flags = quality_signals(
            &serde_json::json!({"ip":"1.1.1.1","is_proxy":true,"is_vpn":true,"is_abuser":true}),
            "1.1.1.1",
        );
        assert_eq!(flags.iter().filter(|s| s.risk).count(), 1);
    }
    #[test]
    fn supported_regions_and_edge_locations_are_separate() {
        assert!(region_supported("US") && region_supported("TW"));
        assert!(!region_supported("CN") && !region_supported("HK"));
        assert!(supported_region("CN", "1.1.1.1").risk);
        assert_eq!(
            cloudflare_trace(
                "h=www.cloudflare.com\nip=1.1.1.1\nloc=US\ncolo=HKG\nwarp=off\n",
                "www.cloudflare.com",
                "test"
            )
            .country,
            ""
        );
        assert_eq!(
            cloudflare_trace(
                "h=evil.test\nip=1.1.1.1\nloc=US",
                "www.cloudflare.com",
                "test"
            )
            .evidence,
            Evidence::Unknown
        );
    }
    #[tokio::test]
    async fn challenge_responses_remain_measured_context_without_becoming_regional_risks() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (header, status, challenged) in [
            ("cf-mitigated: challenge\r\n", 403, true),
            ("CF-Mitigated: ChAlLeNgE\r\n", 403, true),
            ("", 403, false),
            ("", 200, false),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let response = format!(
                "HTTP/1.1 {status} Test\r\n{header}Content-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let server = tokio::spawn(async move {
                let mut requests = Vec::new();
                for _ in 0..3 {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        stream.read_exact(&mut byte).await.unwrap();
                        request.push(byte[0]);
                        assert!(request.len() < 8192);
                    }
                    requests.push(String::from_utf8(request).unwrap());
                    stream.write_all(response.as_bytes()).await.unwrap();
                }
                requests
            });
            let signal =
                connectivity_at(&super::super::client(None).unwrap(), "claude.ai", &url).await;
            let requests = tokio::time::timeout(std::time::Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap();
            assert!(
                requests
                    .iter()
                    .all(|r| r.starts_with("HEAD / HTTP/1.1\r\n"))
            );
            assert!(
                requests
                    .iter()
                    .all(|r| !r.to_ascii_lowercase().contains("cookie:")
                        && !r.to_ascii_lowercase().contains("authorization:"))
            );
            assert_eq!(signal.evidence, Evidence::Measured);
            assert!(!signal.risk);
            assert_eq!(signal.challenged, challenged);
            assert_eq!(signal.context_only(), challenged);
            assert!(signal.value.contains("响应 3/3"));
            assert!(signal.value.contains(&format!("HTTP {status}")));
            let mut report = Report::new(super::super::Environment::Core, "test");
            report.signals.push(signal.clone());
            report.validate().unwrap();
            assert!(!report.differs(&signal));
            let json = serde_json::to_value(&signal).unwrap();
            let restored: Signal = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(restored.challenged, challenged);
            // Existing reports have no challenge field and stay importable.
            let mut legacy = json;
            legacy.as_object_mut().unwrap().remove("challenged");
            assert!(!serde_json::from_value::<Signal>(legacy).unwrap().challenged);
        }
    }
}
