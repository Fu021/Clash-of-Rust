//! IP/HTTP checks plus a bridge to the bundled AGPL platform detector.
use crate::probe::{country_name, trace_country, validate_url};
use anyhow::{Context, Result, bail};
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::Value;
use std::{
    net::IpAddr,
    sync::OnceLock,
    time::{Duration, Instant},
};

#[derive(Debug, Deserialize)]
pub struct Service {
    pub id: String,
    pub name: String,
    pub group: String,
    pub url: String,
}

pub fn services() -> &'static [Service] {
    static SERVICES: OnceLock<Vec<Service>> = OnceLock::new();
    SERVICES.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/ip-check/services.json"))
            .expect("bundled IP check catalog must be valid")
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Confirmed,
    Reachable,
    Restricted,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub state: State,
    pub summary: String,
    pub country: String,
    pub millis: u128,
    pub detail: String,
}

fn client(port: u16) -> Result<Client> {
    Ok(Client::builder().no_proxy()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36")
        .connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(5)).build()?)
}

async fn body(mut response: reqwest::Response) -> Result<String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        let remaining = (1024 * 1024_usize).saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..remaining.min(chunk.len())]);
        if bytes.len() >= 1024 * 1024 {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub async fn check(id: &str, port: u16) -> Result<CheckResult> {
    let service = services()
        .iter()
        .find(|item| item.id == id)
        .context("检测项目不存在")?;
    tokio::time::timeout(Duration::from_secs(120), async {
        if matches!(id, "exit-ip" | "github") {
            check_with_client(service, &client(port)?).await
        } else {
            crate::region_check::check(service, port).await
        }
    })
    .await
    .context("检测超时")?
}

async fn check_with_client(service: &Service, client: &Client) -> Result<CheckResult> {
    let original = validate_url(&service.url)?;
    let started = Instant::now();
    let response = client
        .get(original.clone())
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await?;
    let millis = started.elapsed().as_millis();
    let status = response.status().as_u16();
    let final_url = response.url().clone();
    let cloudflare = response.headers().contains_key("cf-ray")
        || response
            .headers()
            .get("server")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("cloudflare"));
    let text = body(response).await?;
    let mut result = classify(service, status, &final_url, &text);
    result.millis = millis;
    if service.id == "exit-ip" {
        if !matches!(status, 200..=299) || final_url.host_str() != original.host_str() {
            bail!("出口 IP 查询未返回有效结果（HTTP {status}）");
        }
        let ip: IpAddr = text
            .lines()
            .find_map(|line| line.strip_prefix("ip="))
            .context("响应没有出口 IP")?
            .parse()?;
        result.state = State::Confirmed;
        result.summary = ip.to_string();
        result.country = trace_country(&text, original.host_str().unwrap_or_default())
            .unwrap_or_else(|| "未提供".into());
        result.detail = format!(
            "Cloudflare 返回的当前出口 IP；{} · HTTP {status}",
            if ip.is_ipv4() { "IPv4" } else { "IPv6" }
        );
        result.country = decorate_country(result.country);
        return Ok(result);
    }
    if result.country == "未提供" && cloudflare && final_url.host_str() == original.host_str() {
        let mut trace = original.clone();
        trace.set_path("/cdn-cgi/trace");
        trace.set_query(None);
        trace.set_fragment(None);
        if let Ok(response) = client.get(trace.clone()).send().await
            && response.status().is_success()
            && response.url().host_str() == trace.host_str()
            && let Ok(text) = body(response).await
            && let Some(country) = trace_country(&text, trace.host_str().unwrap_or_default())
        {
            result.country = country;
            result
                .detail
                .push_str("；地区来自该域名的 Cloudflare trace，未验证账号或播放地区");
        }
    }
    result.country = decorate_country(result.country);
    Ok(result)
}

pub(crate) fn decorate_country(name: String) -> String {
    let Some(code) = crate::probe::ISO_CODES
        .split_whitespace()
        .find(|code| country_name(code).as_deref() == Some(name.as_str()))
    else {
        return name;
    };
    if matches!(code, "HK" | "MO" | "TW") {
        return name;
    }
    let flag: String = code
        .bytes()
        .filter_map(|letter| char::from_u32(0x1f1e6 + u32::from(letter - b'A')))
        .collect();
    format!("{flag} {name}")
}

/// Small headless helper for upstream getent calls, available on Windows and Linux.
pub fn resolve_host(args: &[String]) -> Result<()> {
    use std::net::ToSocketAddrs;
    let [family, host] = args else {
        bail!("无效的解析参数");
    };
    anyhow::ensure!(
        matches!(family.as_str(), "ahostsv4" | "ahostsv6"),
        "无效的地址类型"
    );
    anyhow::ensure!(
        matches!(host.as_str(), "www.netflix.com" | "api.bilibili.com"),
        "检测域名不在允许列表"
    );
    let mut seen = std::collections::BTreeSet::new();
    for address in (host.as_str(), 0).to_socket_addrs()? {
        let ip = address.ip();
        if ip.is_ipv4() == (family == "ahostsv4") && seen.insert(ip) {
            println!("{ip} STREAM {host}");
        }
    }
    Ok(())
}

fn classify(service: &Service, status: u16, final_url: &Url, text: &str) -> CheckResult {
    let error = serde_json::from_str::<Value>(text).ok().and_then(|json| {
        json.pointer("/error/code")
            .or_else(|| json.get("errorCode"))
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
    });
    let same_host = validate_url(&service.url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| final_url.host_str() == Some(host.as_str()));
    let regional_error = same_host
        && (error.as_deref().is_some_and(|code| {
            matches!(
                code,
                "unsupported_country_region_territory"
                    | "unsupported_country"
                    | "geo_restricted"
                    | "forbidden_location"
            )
        }) || (final_url.host_str() == Some("claude.ai")
            && final_url.path().starts_with("/unavailable")));
    let (state, summary) = if regional_error {
        (State::Restricted, "地区受限")
    } else if matches!(status, 403 | 451) {
        (State::Restricted, "访问受限")
    } else if status == 429 {
        (State::Restricted, "请求限流")
    } else if matches!(status, 200..=399) {
        (State::Reachable, "可达 · 解锁未确认")
    } else {
        (State::Unknown, "结果未确认")
    };
    CheckResult {
        state,
        summary: summary.into(),
        country: "未提供".into(),
        millis: 0,
        detail: format!(
            "HTTP {status} · {}",
            if regional_error {
                "平台明确返回地区限制"
            } else {
                "基础 HTTP 检测；未验证登录、订阅或播放，不能视为原脚本完整解锁结果"
            }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn requests_use_proxy_and_target_country_evidence() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = proxy.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            for (path, text) in [
                ("/", "hello"),
                ("/cdn-cgi/trace", "h=site.example\nloc=JP\nip=1.2.3.4\n"),
            ] {
                let (mut socket, _) = proxy.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buffer = [0; 1024];
                    let len = socket.read(&mut buffer).await.unwrap();
                    assert!(len > 0);
                    request.extend_from_slice(&buffer[..len]);
                    if request.windows(4).any(|part| part == b"\r\n\r\n") {
                        break;
                    }
                }
                assert!(
                    String::from_utf8_lossy(&request)
                        .starts_with(&format!("GET http://site.example{path} HTTP"))
                );
                socket.write_all(format!("HTTP/1.1 200 OK\r\nServer: cloudflare\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len()).as_bytes()).await.unwrap();
            }
        });
        let service = Service {
            id: "test".into(),
            name: "test".into(),
            group: "test".into(),
            url: "http://site.example/".into(),
        };
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            check_with_client(&service, &client(port).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.state, State::Reachable);
        assert_eq!(result.country, "🇯🇵 日本");
        server.await.unwrap();
    }

    #[test]
    fn catalog_is_complete_unique_and_only_http() {
        assert_eq!(services().len(), 183);
        let mut ids = std::collections::HashSet::new();
        for item in services() {
            assert!(ids.insert(&item.id));
            let url = validate_url(&item.url).unwrap();
            assert!(url.host_str().is_some());
            assert!(!item.name.is_empty() && !item.group.is_empty());
            assert!(!item.url.contains('$'));
        }
    }
    #[test]
    fn availability_never_means_full_unlock() {
        let service = Service {
            id: "test".into(),
            name: "test".into(),
            group: "AI".into(),
            url: "https://claude.ai".into(),
        };
        let url = validate_url(&service.url).unwrap();
        assert_eq!(classify(&service, 200, &url, "ok").state, State::Reachable);
        assert_eq!(classify(&service, 401, &url, "").state, State::Unknown);
        assert_eq!(classify(&service, 403, &url, "").summary, "访问受限");
        assert_eq!(
            classify(
                &service,
                200,
                &url,
                r#"{"error":{"code":"unsupported_country_region_territory"}}"#
            )
            .summary,
            "地区受限"
        );
        let unrelated = Url::parse("https://other.example/unavailable").unwrap();
        assert_eq!(
            classify(
                &service,
                200,
                &unrelated,
                r#"{"error":{"code":"unsupported_country"}}"#
            )
            .state,
            State::Reachable
        );
    }
    #[test]
    fn country_only_uses_target_specific_evidence() {
        assert_eq!(decorate_country("美国".into()), "🇺🇸 美国");
        for name in ["香港", "澳门", "台湾", "未提供"] {
            assert_eq!(decorate_country(name.into()), name);
        }
    }
}
