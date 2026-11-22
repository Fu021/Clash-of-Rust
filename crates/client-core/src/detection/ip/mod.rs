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

pub(crate) const BASIC_HTTP_DETAIL: &str = "基础 HTTP 响应检测";
pub(crate) const REGIONAL_HTTP_DETAIL: &str = "平台明确返回地区限制";
pub(crate) const EXIT_IP_DETAIL_PREFIX: &str = "内核访问 Cloudflare 的出口 IP · ";
pub(crate) const PLATFORM_ERROR_WITH_EXIT_SUMMARY: &str = "检测失败，未确认";

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
    pub millis: Option<u128>,
    pub authentication_check_skipped: bool,
    pub detail: String,
    pub target_exit: Option<TargetExit>,
}

impl CheckResult {
    pub fn response_millis(&self) -> Option<u128> {
        if self.authentication_check_skipped {
            None
        } else {
            self.millis
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TargetExit {
    pub host: String,
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

pub fn target_host(service: &Service) -> Option<String> {
    if service.id == "exit-ip" {
        return None;
    }
    let url = validate_url(&service.url).ok()?;
    url.host_str().map(str::to_owned)
}

fn target_trace_url(mut url: Url) -> Url {
    url.set_path("/cdn-cgi/trace");
    url.set_query(None);
    url.set_fragment(None);
    url
}

fn parse_target_exit(text: &str, host: &str) -> Option<TargetExit> {
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
        host: host.into(),
        ip,
        country: fields.get("loc").and_then(|code| country_name(code)),
    })
}

async fn target_exit(client: &Client, url: Url, host: &str) -> Option<TargetExit> {
    if url.host_str() != Some(host) || url.path() != "/cdn-cgi/trace" {
        return None;
    }
    let mut response = client
        .get(url.clone())
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .ok()?;
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
            summary: PLATFORM_ERROR_WITH_EXIT_SUMMARY.into(),
            country: String::new(),
            millis: None,
            authentication_check_skipped: false,
            detail: format!("{error:#}"),
            target_exit: None,
        },
        Err(error) => return Err(error),
    };
    result
        .detail
        .push_str(&target_exit_evidence_suffix(host, observed.as_ref()));
    result.target_exit = observed;
    Ok(result)
}

pub(crate) fn target_exit_evidence_suffix(host: &str, observed: Option<&TargetExit>) -> String {
    match observed {
        Some(exit) => format!("；{} 同域探针出口：{}", exit.host, exit.ip),
        None => format!("；{host} 同域探针出口未确认"),
    }
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
    tokio::time::timeout(Duration::from_secs(120), check_service(service, port))
        .await
        .context("检测超时")?
}

async fn check_service(service: &Service, port: u16) -> Result<CheckResult> {
    if service.id == "exit-ip" {
        return check_with_client(service, &client(port)?).await;
    }
    let original = validate_url(&service.url)?;
    let host = original.host_str().context("检测项目缺少有效域名")?;
    let trace = async {
        let trace_client = client_with_redirect(port, reqwest::redirect::Policy::none()).ok()?;
        target_exit(&trace_client, target_trace_url(original.clone()), host).await
    };
    let platform = async {
        if service.id == "github" {
            check_with_client(service, &client(port)?).await
        } else {
            crate::region_check::check(service, port).await
        }
    };
    let (mut result, observed) = tokio::join!(platform, trace);
    if service.id == "github"
        && let Ok(result) = &mut result
        && let Some(country) = observed.as_ref().and_then(|exit| exit.country.clone())
    {
        result.country = decorate_country(country);
    }
    combine_target_exit(result, observed, host)
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
    let text = body(response).await?;
    let mut result = classify(service, status, &final_url, &text);
    result.millis = Some(millis);
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
            "{EXIT_IP_DETAIL_PREFIX}{} · HTTP {status}",
            if ip.is_ipv4() { "IPv4" } else { "IPv6" }
        );
        result.country = decorate_country(result.country);
        return Ok(result);
    }
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
        millis: None,
        authentication_check_skipped: false,
        detail: format!(
            "HTTP {status} · {}",
            if regional_error {
                REGIONAL_HTTP_DETAIL
            } else {
                BASIC_HTTP_DETAIL
            }
        ),
        target_exit: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    async fn proxy_request(socket: &mut TcpStream) -> String {
        let mut request = Vec::new();
        loop {
            let mut buffer = [0; 1024];
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
            if request.windows(4).any(|part| part == b"\r\n\r\n") {
                return String::from_utf8(request).unwrap();
            }
        }
    }

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
            "h=claude.ai\nip=2001:2::1\nloc=US",
            "h=claude.ai\nip=3fff:fff::1\nloc=US",
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

    #[test]
    fn combined_target_exit_localizes_only_the_appended_evidence() {
        use crate::{config::Language, ip_report, support};

        let service = Service {
            id: "fixture".into(),
            name: "Fixture".into(),
            group: "Fixture".into(),
            url: "https://fixture.example".into(),
        };
        for (observed, raw_suffix, projected) in [
            (
                Some(TargetExit {
                    host: "fixture.example".into(),
                    ip: "203.0.113.10".parse().unwrap(),
                    country: None,
                }),
                "；fixture.example 同域探针出口：203.0.113.10",
                "203.0.113.10",
            ),
            (None, "；fixture.example 同域探针出口未确认", "Unconfirmed"),
        ] {
            let upstream = format!("原始说明；另一段; 保留上游记录{raw_suffix}");
            let mut result = classify(&service, 200, &Url::parse(&service.url).unwrap(), "");
            result.detail = format!("原生地区检测结果：{upstream}");
            let outcome = Ok(combine_target_exit(Ok(result), observed, "fixture.example").unwrap());
            let raw = format!("原生地区检测结果：{upstream}{raw_suffix}");

            assert_eq!(
                ip_report::detail_in(&service, &outcome, Language::SimplifiedChinese),
                raw
            );
            assert_eq!(
                ip_report::detail_in(&service, &outcome, Language::English),
                format!(
                    "Native region check result: {upstream}\nSame-domain probe exit (fixture.example): {projected}"
                )
            );
            assert_eq!(support::ip_result(outcome.as_ref().unwrap())["detail"], raw);
        }
        let external = "外部错误；fixture.example 同域探针出口未确认";
        let outcome = Err(external.into());
        assert_eq!(
            ip_report::detail_in(&service, &outcome, Language::English),
            external
        );
        let raw_error = "原生地区检测结果：外部错误；保留原文";
        let observed = TargetExit {
            host: "fixture.example".into(),
            ip: "203.0.113.10".parse().unwrap(),
            country: None,
        };
        let wrapped = Ok(combine_target_exit(
            Err(anyhow::anyhow!(raw_error)),
            Some(observed.clone()),
            "fixture.example",
        )
        .unwrap());
        assert_eq!(
            ip_report::detail_in(&service, &wrapped, Language::English),
            format!("{raw_error}\nSame-domain probe exit (fixture.example): 203.0.113.10")
        );
        let raw = "上游记录；fixture.example 同域探针出口：203.0.113.11";
        let mut mismatched = classify(&service, 200, &Url::parse(&service.url).unwrap(), "");
        mismatched.detail = raw.into();
        mismatched.target_exit = Some(observed);
        assert_eq!(
            ip_report::detail_in(&service, &Ok(mismatched), Language::English),
            raw
        );
    }

    #[test]
    fn combined_http_details_localize_known_templates_and_preserve_json() {
        use crate::{config::Language, ip_report, support};

        let service = Service {
            id: "github".into(),
            name: "Fixture".into(),
            group: "Fixture".into(),
            url: "https://fixture.example".into(),
        };
        let url = Url::parse(&service.url).unwrap();
        for (status, body, chinese, english) in [
            (200, "", "基础 HTTP 响应检测", "Basic HTTP response check"),
            (
                403,
                r#"{"error":{"code":"unsupported_country"}}"#,
                "平台明确返回地区限制",
                "Service reported a regional restriction",
            ),
        ] {
            let result = classify(&service, status, &url, body);
            let outcome = Ok(combine_target_exit(Ok(result), None, "fixture.example").unwrap());
            let raw = format!("HTTP {status} · {chinese}；fixture.example 同域探针出口未确认");

            assert_eq!(
                ip_report::detail_in(&service, &outcome, Language::SimplifiedChinese),
                raw
            );
            assert_eq!(
                ip_report::detail_in(&service, &outcome, Language::English),
                format!(
                    "HTTP {status} · {english}\nSame-domain probe exit (fixture.example): Unconfirmed"
                )
            );
            assert_eq!(support::ip_result(outcome.as_ref().unwrap())["detail"], raw);
        }
        for raw in [
            "HTTP 20 · 基础 HTTP 响应检测",
            "HTTP 200 · 平台明确返回地区限制；上游还有说明",
            "上游记录；other.example 同域探针出口未确认",
            "上游记录；fixture.example 同域探针出口未确认；附录",
        ] {
            let mut result = classify(&service, 200, &url, "");
            result.detail = raw.into();
            assert_eq!(
                ip_report::detail_in(&service, &Ok(result), Language::English),
                raw
            );
        }
    }

    #[tokio::test]
    async fn exit_ip_details_localize_ipv4_and_ipv6_without_changing_json() {
        use crate::{config::Language, ip_report, support};

        let service = Service {
            id: "exit-ip".into(),
            name: "Fixture".into(),
            group: "Fixture".into(),
            url: "http://fixture.example".into(),
        };
        for (ip, family) in [("203.0.113.10", "IPv4"), ("2001:db8::1", "IPv6")] {
            let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = proxy.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (mut socket, _) = proxy.accept().await.unwrap();
                let request = proxy_request(&mut socket).await;
                assert!(request.starts_with("GET http://fixture.example/ HTTP/"));
                let body = format!("h=fixture.example\nip={ip}\nloc=JP\n");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            });
            let result =
                tokio::time::timeout(Duration::from_secs(5), check_service(&service, port))
                    .await
                    .unwrap()
                    .unwrap();
            server.await.unwrap();
            let outcome = Ok(result);
            let raw = format!("内核访问 Cloudflare 的出口 IP · {family} · HTTP 200");
            assert_eq!(
                ip_report::detail_in(&service, &outcome, Language::SimplifiedChinese),
                raw
            );
            assert_eq!(
                ip_report::detail_in(&service, &outcome, Language::English),
                format!("Core egress IP observed by Cloudflare · {family} · HTTP 200")
            );
            assert_eq!(support::ip_result(outcome.as_ref().unwrap())["detail"], raw);
        }
    }

    #[tokio::test]
    async fn target_exit_rejects_redirects_errors_and_oversized_responses() {
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
    async fn github_uses_one_proxied_trace_for_the_address_and_country() {
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = proxy.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let mut paths = std::collections::BTreeSet::new();
            for _ in 0..2 {
                let (mut socket, _) = proxy.accept().await.unwrap();
                let request = proxy_request(&mut socket).await;
                let url = request.lines().next().unwrap().split(' ').nth(1).unwrap();
                assert!(paths.insert(url.to_owned()));
                let text = match url {
                    "http://site.example:8080/?fixture=1" => "hello",
                    "http://site.example:8080/cdn-cgi/trace" => {
                        "h=site.example\nloc=JP\nip=1.2.3.4\n"
                    }
                    _ => panic!("Unexpected proxy request: {url}"),
                };
                socket.write_all(format!("HTTP/1.1 200 OK\r\nServer: cloudflare\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len()).as_bytes()).await.unwrap();
            }
        });
        let service = Service {
            id: "github".into(),
            name: "test".into(),
            group: "test".into(),
            url: "http://site.example:8080/?fixture=1".into(),
        };
        let result = tokio::time::timeout(Duration::from_secs(5), check_service(&service, port))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.state, State::Reachable);
        assert_eq!(result.country, "🇯🇵 日本");
        let exit = result.target_exit.unwrap();
        assert_eq!(exit.country.as_deref(), Some("日本"));
        assert_eq!(exit.ip, "1.2.3.4".parse::<IpAddr>().unwrap());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn skipped_authentication_preserves_same_origin_exit_without_measuring_the_platform() {
        for (id, use_catalog) in [
            "MediaUnlockTest_DisneyPlus",
            "MediaUnlockTest_ESPNPlus",
            "MediaUnlockTest_NeonTV",
            "MediaUnlockTest_HBOMax",
            "MediaUnlockTest_NLZIET",
            "MediaUnlockTest_EurosportRO",
            "MediaUnlockTest_OptusSports",
            "MediaUnlockTest_HuluUS",
            "MediaUnlockTest_Catchplay",
            "MediaUnlockTest_TrueID",
            "MediaUnlockTest_EPIX",
            "MediaUnlockTest_Joyn",
            "MediaUnlockTest_SetantaSports",
            "MediaUnlockTest_TVer",
            "MediaUnlockTest_Tving",
            "MediaUnlockTest_Wavve",
            "MediaUnlockTest_DMM",
            "MediaUnlockTest_encoreTVB",
            "MediaUnlockTest_SKY_DE",
            "MediaUnlockTest_MaoriTV",
            "MediaUnlockTest_SpotvNow",
            "MediaUnlockTest_NetflixCDN",
            "MediaUnlockTest_7plus",
            "MediaUnlockTest_NaverTV",
            "MediaUnlockTest_TV360",
            "MediaUnlockTest_KPlus",
            "MediaUnlockTest_ofiii",
            "MediaUnlockTest_NowE",
            "MediaUnlockTest_ViuTV",
            "MediaUnlockTest_4GTV",
            "MediaUnlockTest_Channel5",
        ]
        .into_iter()
        .flat_map(|id| [(id, false), (id, true)])
        {
            let catalog_service = services().iter().find(|service| service.id == id).unwrap();
            let catalog_url = validate_url(&catalog_service.url).unwrap();
            assert!(catalog_url.path() == "/", "catalog path for {id}");
            assert!(catalog_url.query().is_none(), "catalog query for {id}");
            assert!(
                catalog_url.fragment().is_none(),
                "catalog fragment for {id}"
            );
            let host = target_host(catalog_service).unwrap();
            let service = Service {
                id: id.into(),
                name: "Fixture".into(),
                group: "Fixture".into(),
                url: "http://site.example:8080/platform?fixture=1#fragment".into(),
            };
            let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = proxy.local_addr().unwrap().port();
            let expected = if use_catalog {
                format!("CONNECT {host}:443 HTTP/")
            } else {
                "GET http://site.example:8080/cdn-cgi/trace HTTP/".into()
            };
            let server = tokio::spawn(async move {
                let (mut socket, _) = proxy.accept().await.unwrap();
                let request = proxy_request(&mut socket).await;
                assert!(request.starts_with(&expected));
                assert!(!request.lines().any(|line| {
                    line.split_once(':').is_some_and(|(name, _)| {
                        name.eq_ignore_ascii_case("cookie")
                            || name.eq_ignore_ascii_case("authorization")
                    })
                }));
                let response = if use_catalog {
                    "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_owned()
                } else {
                    let body = "h=site.example\nloc=JP\nip=1.2.3.4\n";
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                socket.write_all(response.as_bytes()).await.unwrap();
                proxy.into_std().unwrap()
            });
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                if use_catalog {
                    check(id, port).await
                } else {
                    check_service(&service, port).await
                }
            })
            .await
            .unwrap()
            .unwrap();
            let proxy = server.await.unwrap();
            assert_eq!(
                proxy.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
            assert_eq!(result.state, State::Unknown, "{id}");
            assert!(result.authentication_check_skipped, "{id}");
            assert_eq!(result.country, "未提供", "{id}");
            assert_eq!(result.response_millis(), None, "{id}");
            let report = crate::support::ip_result(&result);
            assert_eq!(report["state"], "Unknown", "{id}");
            assert_eq!(report["country"], "未提供", "{id}");
            assert_eq!(report["millis"], serde_json::Value::Null, "{id}");
            assert_eq!(report["authentication_check_skipped"], true, "{id}");
            if use_catalog {
                assert!(result.target_exit.is_none(), "{id}");
                assert_eq!(
                    report["detail"],
                    format!("未执行认证检测；{host} 同域探针出口未确认")
                );
            } else {
                assert_eq!(
                    report["detail"],
                    "未执行认证检测；site.example 同域探针出口：1.2.3.4"
                );
                let exit = result.target_exit.unwrap();
                assert_eq!(exit.host, "site.example");
                assert_eq!(exit.ip, "1.2.3.4".parse::<IpAddr>().unwrap());
                assert_eq!(exit.country.as_deref(), Some("日本"));
            }
        }
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
            assert_eq!(
                target_host(item).as_deref(),
                (item.id != "exit-ip").then(|| url.host_str().unwrap())
            );
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
