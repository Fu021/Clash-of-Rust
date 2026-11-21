//! Native IP/HTTP checks and upstream-derived platform decisions.
use crate::probe::{country_name, trace_country, validate_url};
use anyhow::{Context, Result, bail};
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    net::IpAddr,
    sync::OnceLock,
    time::{Duration, Instant},
};

pub const SPLIT_EXIT_TARGETS: [(&str, &str, &str); 2] = [
    ("AIUnlockTest_Claude", "claude.ai", "Claude"),
    ("MediaUnlockTest_ChatGPT", "chatgpt.com", "ChatGPT"),
];

#[derive(Debug, Deserialize)]
pub struct Service {
    pub id: String,
    pub name: String,
    pub group: String,
    pub url: String,
}

impl Service {
    pub fn information_only(&self) -> bool {
        matches!(
            self.id.as_str(),
            "exit-ip"
                | "MediaUnlockTest_Google"
                | "MediaUnlockTest_NetflixCDN"
                | "MediaUnlockTest_Spotify"
                | "GameTest_Steam"
                | "MediaUnlockTest_YouTube_CDN"
        )
    }
}

pub fn services() -> &'static [Service] {
    static SERVICES: OnceLock<Vec<Service>> = OnceLock::new();
    SERVICES.get_or_init(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../resources/ip-check/services.json"
        )))
        .expect("bundled IP check catalog must be valid")
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Confirmed,
    Partial,
    Reachable,
    Identified,
    Restricted,
    Unknown,
    Failed,
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub state: State,
    pub summary: String,
    pub country: String,
    pub millis: u128,
    pub detail: String,
    pub target_exit: Option<TargetExit>,
}

impl CheckResult {
    pub fn response_millis(&self) -> Option<u128> {
        (self.state != State::Failed || self.millis > 0).then_some(self.millis)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TargetExit {
    pub host: &'static str,
    pub ip: IpAddr,
    pub country: Option<String>,
}

fn client(port: u16) -> Result<Client> {
    client_with_redirect(port, reqwest::redirect::Policy::limited(5))
}

fn client_with_redirect(port: u16, redirect: reqwest::redirect::Policy) -> Result<Client> {
    Ok(crate::network::client_builder().no_proxy()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36")
        .connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(12))
        .redirect(redirect).build()?)
}

fn parse_target_exit(text: &str, host: &'static str) -> Option<TargetExit> {
    let fields: BTreeMap<_, _> = text
        .lines()
        .filter_map(|line| line.trim().split_once('='))
        .collect();
    if fields.get("h").copied()? != host {
        return None;
    }
    let ip: IpAddr = fields.get("ip")?.parse().ok()?;
    if !crate::claude_check::public_ip(ip) {
        return None;
    }
    Some(TargetExit {
        host,
        ip,
        country: fields.get("loc").and_then(|code| country_name(code)),
    })
}

async fn target_exit(client: &Client, url: Url, host: &'static str) -> Option<TargetExit> {
    if url.host_str() != Some(host) || url.path() != "/cdn-cgi/trace" {
        return None;
    }
    let mut response = client.get(url.clone()).send().await.ok()?;
    if !response.status().is_success() || response.url() != &url {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if chunk.len() > 8192usize.saturating_sub(bytes.len()) {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_target_exit(std::str::from_utf8(&bytes).ok()?, host)
}

fn combine_target_exit(
    result: Result<CheckResult>,
    observed: Option<TargetExit>,
    host: &str,
) -> Result<CheckResult> {
    let mut result = match result {
        Ok(result) => result,
        Err(error) if observed.is_some() => CheckResult {
            state: State::Failed,
            summary: "检测失败，未确认".into(),
            country: String::new(),
            millis: 0,
            detail: format!("{error:#}"),
            target_exit: None,
        },
        Err(error) => return Err(error),
    };
    result.detail.push_str(&match &observed {
        Some(exit) => format!("；{} 同域探针出口：{}", exit.host, exit.ip),
        None => format!("；{host} 同域探针出口未确认"),
    });
    result.target_exit = observed;
    Ok(result)
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
            return check_with_client(service, &client(port)?).await;
        }
        let Some((_, host, _)) = SPLIT_EXIT_TARGETS
            .iter()
            .find(|(target, _, _)| *target == id)
        else {
            return crate::region_check::check(service, port).await;
        };
        let trace = async {
            let trace_client =
                client_with_redirect(port, reqwest::redirect::Policy::none()).ok()?;
            let url = Url::parse(&format!("https://{host}/cdn-cgi/trace")).ok()?;
            target_exit(&trace_client, url, host).await
        };
        let (result, observed) = tokio::join!(crate::region_check::check(service, port), trace);
        combine_target_exit(result, observed, host)
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
        result.state = State::Identified;
        result.summary = ip.to_string();
        result.country = trace_country(&text, original.host_str().unwrap_or_default())
            .unwrap_or_else(|| "未提供".into());
        result.detail = format!(
            "内核访问 Cloudflare 的出口 IP · {} · HTTP {status}",
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
                .push_str("；地区来自该域名的 Cloudflare trace");
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
    let Some(flag) = crate::countries::flag_emoji(code) else {
        return name;
    };
    format!("{flag} {name}")
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
                "基础 HTTP 响应检测"
            }
        ),
        target_exit: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_exit_requires_the_same_host_and_a_public_address() {
        let observed = parse_target_exit("h=claude.ai\nip=8.8.8.8\nloc=US\n", "claude.ai").unwrap();
        assert_eq!(observed.host, "claude.ai");
        assert_eq!(observed.ip.to_string(), "8.8.8.8");
        assert_eq!(observed.country.as_deref(), Some("美国"));
        let observed = parse_target_exit(
            "h=claude.ai\nip=2606:4700:4700::1111\nloc=XX\n",
            "claude.ai",
        )
        .unwrap();
        assert!(observed.ip.is_ipv6());
        assert!(observed.country.is_none());
        for text in [
            "h=other.example\nip=8.8.8.8\nloc=US",
            "h=claude.ai\nip=127.0.0.1\nloc=US",
            "h=claude.ai\nip=invalid\nloc=US",
            "h=claude.ai\nloc=US",
        ] {
            assert!(parse_target_exit(text, "claude.ai").is_none(), "{text}");
        }
    }

    #[test]
    fn measured_target_exit_survives_an_independent_platform_error() {
        let observed = parse_target_exit("h=chatgpt.com\nip=8.8.8.8\n", "chatgpt.com").unwrap();
        let result = combine_target_exit(
            Err(anyhow::anyhow!("检测响应不是有效 JSON")),
            Some(observed),
            "chatgpt.com",
        )
        .unwrap();
        assert_eq!(result.state, State::Failed);
        assert_eq!(result.response_millis(), None);
        assert!(result.detail.contains("检测响应不是有效 JSON"));
        assert!(result.detail.contains("chatgpt.com 同域探针出口：8.8.8.8"));
        assert_eq!(result.target_exit.unwrap().ip.to_string(), "8.8.8.8");
        assert!(
            combine_target_exit(Err(anyhow::anyhow!("检测失败")), None, "chatgpt.com").is_err()
        );
    }

    #[tokio::test]
    async fn target_exit_rejects_redirects_errors_and_oversized_responses() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        for (status, body, expected) in [
            (
                "200 OK",
                "h=claude.ai\nip=8.8.8.8\nloc=US\n".to_owned(),
                true,
            ),
            (
                "403 Forbidden",
                "h=claude.ai\nip=8.8.8.8\nloc=US\n".to_owned(),
                false,
            ),
            (
                "302 Found",
                "h=claude.ai\nip=8.8.8.8\nloc=US\n".to_owned(),
                false,
            ),
            (
                "200 OK",
                format!("h=claude.ai\nip=8.8.8.8\n{}", "x".repeat(8192)),
                false,
            ),
        ] {
            let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = proxy.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (mut socket, _) = proxy.accept().await.unwrap();
                let mut request = [0; 2048];
                let count = socket.read(&mut request).await.unwrap();
                assert!(
                    String::from_utf8_lossy(&request[..count])
                        .starts_with("GET http://claude.ai/cdn-cgi/trace HTTP/")
                );
                socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nLocation: http://other.example/\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            });
            let client = client_with_redirect(port, reqwest::redirect::Policy::none()).unwrap();
            let url = Url::parse("http://claude.ai/cdn-cgi/trace").unwrap();
            assert_eq!(
                target_exit(&client, url, "claude.ai").await.is_some(),
                expected
            );
            server.await.unwrap();
        }
    }

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
        assert_eq!(services().len(), 182);
        assert!(
            !services()
                .iter()
                .any(|item| item.id == "MediaUnlockTest_Sora")
        );
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
        for name in ["香港", "澳门", "台湾"] {
            assert_eq!(decorate_country(name.into()), format!("🇨🇳 {name}"));
        }
        assert_eq!(decorate_country("未提供".into()), "未提供");
    }
}
