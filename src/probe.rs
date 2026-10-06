use anyhow::{Result, bail};
use reqwest::{Client, Url};
use std::time::{Duration, Instant};

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

pub async fn website(input: String, port: u16, exit_ip: bool) -> Result<ProbeResult> {
    let url = validate_url(&input)?;
    let client = Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;
    let start = Instant::now();
    let response = client.get(url).send().await?;
    let millis = start.elapsed().as_millis();
    let status = response.status().as_u16();
    let detail = if exit_ip {
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 8192 {
                bail!("IP 查询响应过大");
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        value
            .get("ip")
            .and_then(|v| v.as_str())
            .unwrap_or("响应中没有 ip 字段")
            .to_string()
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

#[derive(Debug, Clone)]
pub struct SiteResult {
    pub status: u16,
    pub millis: u128,
    pub assessment: String,
    pub route: String,
    pub country: String,
}

pub fn assessment(status: u16) -> &'static str {
    match status {
        200..=399 => "网站可达（未验证登录及服务功能）",
        403 => "网站可达，但访问被拒绝或需要浏览器验证",
        429 => "网站可达，但请求受到限流",
        _ => "网站有响应，但返回异常状态",
    }
}

pub async fn site(input: String, port: u16, api: &crate::api::Api) -> Result<SiteResult> {
    let url = validate_url(&input)?;
    let host = url.host_str().unwrap().to_owned();
    let client = Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;
    let start = Instant::now();
    let response = client.get(url.clone()).send().await?;
    let millis = start.elapsed().as_millis();
    let status = response.status().as_u16();
    let final_host = response.url().host_str().unwrap_or(&host).to_owned();
    let connections: Result<crate::api::Connections> = api.get("connections").await;
    let route = connections
        .ok()
        .and_then(|c| {
            c.connections
                .into_iter()
                .find(|c| c.metadata.host == final_host)
        })
        .map(|c| format!("{} · {}", c.rule, c.chains.join(" → ")))
        .unwrap_or_else(|| "未获取到匹配连接；请求遵循当前内核规则".into());
    let country = website_country(&client, &url, response).await;
    Ok(SiteResult {
        status,
        millis,
        assessment: assessment(status).into(),
        route,
        country,
    })
}

async fn limited_body(mut response: reqwest::Response, limit: usize) -> Result<String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        let remaining = limit.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if bytes.len() == limit {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn website_country(client: &Client, original: &Url, response: reqwest::Response) -> String {
    // A site's region decision is not the same as an independent IP database.
    // Never infer it from language, a CDN datacenter, destination IP or another host.
    let cloudflare = response.headers().contains_key("cf-ray")
        || response
            .headers()
            .get("server")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("cloudflare"));
    let netflix_country = netflix_redirect_country(original, response.url());
    if let Some(country) = netflix_country {
        return country;
    }
    let body_country = if original.host_str() == Some("www.netflix.com") {
        limited_body(response, 1024 * 1024)
            .await
            .ok()
            .and_then(|body| netflix_request_country(original, &body))
    } else {
        drop(response);
        None
    };
    if let Some(country) = body_country {
        return country;
    }
    if cloudflare {
        let mut trace = original.clone();
        trace.set_path("/cdn-cgi/trace");
        trace.set_query(None);
        trace.set_fragment(None);
        if let Ok(response) = client.get(trace.clone()).send().await
            && response.status().is_success()
            && response.url().host_str() == trace.host_str()
            && let Ok(body) = limited_body(response, 8192).await
            && let Some(country) = trace_country(&body, trace.host_str().unwrap_or(""))
        {
            return country;
        }
    }
    "网站未提供地区".into()
}

fn trace_country(body: &str, host: &str) -> Option<String> {
    let fields: std::collections::BTreeMap<_, _> = body
        .lines()
        .filter_map(|line| line.trim().split_once('='))
        .collect();
    if fields.get("h").copied()? != host {
        return None;
    }
    country_name(fields.get("loc").copied()?)
}

fn netflix_redirect_country(original: &Url, final_url: &Url) -> Option<String> {
    if original.host_str()? != "www.netflix.com" || final_url.host_str()? != "www.netflix.com" {
        return None;
    }
    let segment = final_url.path_segments()?.next()?;
    let (country, language) = segment.split_once('-')?;
    if language.is_empty() {
        return None;
    }
    country_name(country)
}

fn netflix_request_country(original: &Url, body: &str) -> Option<String> {
    if original.host_str()? != "www.netflix.com" {
        return None;
    }
    // Only an explicit request-country field counts; UI locale and catalog language do not.
    let body = body.replace("\\\"", "\"");
    let (_, field) = body.split_once("\"requestCountry\"")?;
    let field = field.trim_start().strip_prefix(':')?.trim_start();
    let value = serde_json::Deserializer::from_str(field)
        .into_iter::<serde_json::Value>()
        .next()?
        .ok()?;
    country_name(value.as_str().or_else(|| value.get("id")?.as_str())?)
}

fn country_name(code: &str) -> Option<String> {
    let code = code.to_ascii_uppercase();
    const ISO_CODES: &str = "AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS YE YT ZA ZM ZW";
    if !ISO_CODES.split_whitespace().any(|valid| valid == code) {
        return None;
    }
    let name = match code.as_str() {
        "US" => "美国",
        "CN" => "中国",
        "HK" => "香港",
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
    #[test]
    fn restricts_probe_protocols() {
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("https://user:secret@example.com").is_err());
        assert!(validate_url("https://example.com").is_ok());
    }

    #[tokio::test]
    async fn site_keeps_http_denial_without_inventing_country() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = proxy.local_addr().unwrap().port();
        let proxy_task = tokio::spawn(async move {
            for _ in 0..1 {
                let (mut stream, _) = proxy.accept().await.unwrap();
                let mut buffer = [0; 4096];
                let read = stream.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..read]);
                assert!(request.starts_with("GET http://site.example/"));
                stream
                    .write_all(
                        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap();
            }
        });
        let controller = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let settings = crate::config::Settings {
            controller_port: controller.local_addr().unwrap().port(),
            ..Default::default()
        };
        let api = crate::api::Api::new(&settings).unwrap();
        let api_task = tokio::spawn(async move {
            let (mut stream, _) = controller.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let read = stream.read(&mut buffer).await.unwrap();
            assert!(String::from_utf8_lossy(&buffer[..read]).starts_with("GET /connections"));
            let body = r#"{"connections":[{"id":"test","metadata":{"host":"site.example"},"rule":"Domain","chains":["test-node"]}]}"#;
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        });
        let result = site("http://site.example/".into(), port, &api)
            .await
            .unwrap();
        assert_eq!(result.status, 403);
        assert!(result.assessment.contains("拒绝"));
        assert!(result.route.contains("test-node"));
        assert_eq!(result.country, "网站未提供地区");
        proxy_task.await.unwrap();
        api_task.await.unwrap();
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
        let original = Url::parse("https://www.netflix.com/").unwrap();
        assert_eq!(
            netflix_redirect_country(
                &original,
                &Url::parse("https://www.netflix.com/jp-en/").unwrap()
            ),
            Some("日本".into())
        );
        assert_eq!(
            netflix_request_country(&original, r#"{"requestCountry":{"id":"SG"}}"#),
            Some("新加坡".into())
        );
        assert!(
            netflix_request_country(
                &original,
                r#"{"lang":"en-US","requestCountry":{},"other":{"id":"US"}}"#
            )
            .is_none()
        );
        assert!(
            netflix_redirect_country(
                &original,
                &Url::parse("https://other.example/jp-en/").unwrap()
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn site_country_uses_target_host_trace() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = proxy.local_addr().unwrap().port();
        let proxy_task = tokio::spawn(async move {
            for index in 0..2 {
                let (mut socket, _) = proxy.accept().await.unwrap();
                let mut buffer = [0; 4096];
                let length = socket.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..length]);
                let body = if index == 0 {
                    assert!(request.starts_with("GET http://site.example/ HTTP"));
                    "hello"
                } else {
                    assert!(request.starts_with("GET http://site.example/cdn-cgi/trace HTTP"));
                    "h=site.example\nloc=JP\ncolo=SJC\nip=1.2.3.4\n"
                };
                socket.write_all(format!("HTTP/1.1 200 OK\r\nServer: cloudflare\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let controller = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = crate::api::Api::new(&crate::config::Settings {
            controller_port: controller.local_addr().unwrap().port(),
            ..Default::default()
        })
        .unwrap();
        let controller_task = tokio::spawn(async move {
            let (mut socket, _) = controller.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let length = socket.read(&mut buffer).await.unwrap();
            assert!(String::from_utf8_lossy(&buffer[..length]).starts_with("GET /connections"));
            let body = r#"{"connections":[]}"#;
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
        let result = site("http://site.example/".into(), port, &api)
            .await
            .unwrap();
        assert_eq!(result.country, "日本");
        assert_eq!(result.status, 200);
        proxy_task.await.unwrap();
        controller_task.await.unwrap();
    }
}
