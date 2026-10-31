//! Bounded subscription downloads using the same routes as client updates.
use crate::network::{Route, request_error};
use anyhow::{Context, Result, bail};
use reqwest::{Client, Url, header};
use std::time::Duration;
mod compatibility;
mod info;
pub mod schedule;
pub use info::summary;

const LIMIT: usize = 10 * 1024 * 1024;

pub async fn download_routes(url: &Url, routes: &[Route]) -> Result<(String, Option<String>)> {
    let mut errors = Vec::new();
    for (index, route) in routes.iter().enumerate() {
        match download_route(url, route, index + 1 < routes.len()).await {
            Ok(subscription) => return Ok(subscription),
            Err(error) => errors.push(format!("{}：{error}", route.label)),
        }
    }
    bail!("{}（已尝试可用代理和直连）", errors.join("；"));
}

async fn download_route(
    url: &Url,
    route: &Route,
    retry_slow: bool,
) -> Result<(String, Option<String>)> {
    let client = route
        .apply(
            crate::network::client_builder()
                .connect_timeout(Duration::from_secs(5))
                .read_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(30)),
        )?
        .build()
        .context("无法创建订阅下载连接")?;
    // Both format attempts share the existing route deadline.
    tokio::time::timeout(Duration::from_secs(30), async {
        match download_agent(&client, url, retry_slow, false).await {
            Err(error) if error.is::<compatibility::Incompatible>() => {
                download_agent(&client, url, retry_slow, true)
                    .await
                    .map_err(|fallback| anyhow::anyhow!("{error}；Mihomo 兼容请求：{fallback}"))
            }
            result => result,
        }
    })
    .await
    .context("订阅下载超时")?
}

async fn download_agent(
    client: &Client,
    url: &Url,
    retry_slow: bool,
    meta_only: bool,
) -> Result<(String, Option<String>)> {
    let mut response = client
        .get(url.clone())
        .header(header::USER_AGENT, compatibility::user_agent(meta_only))
        .header(
            header::ACCEPT,
            "application/yaml, text/yaml, text/x-yaml, */*;q=0.1",
        )
        .send()
        .await
        .map_err(|error| request_error(error, "订阅下载"))?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let message = format!("订阅服务器返回 HTTP {status}");
        if matches!(status, 400 | 403 | 406 | 415 | 422) {
            bail!(compatibility::Incompatible(message));
        }
        bail!(message);
    }
    if response
        .content_length()
        .is_some_and(|size| size > LIMIT as u64)
    {
        bail!("订阅超过 10 MiB 限制");
    }
    let usage = response
        .headers()
        .get("subscription-userinfo")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut bytes = Vec::new();
    let mut watchdog = crate::download::Watchdog::new(retry_slow);
    while let Some(chunk) = watchdog
        .chunk(&mut response)
        .await
        .context("订阅下载失败")?
    {
        let chunk = chunk.as_ref();
        if chunk.len() > LIMIT.saturating_sub(bytes.len()) {
            bail!("订阅超过 10 MiB 限制");
        }
        bytes.extend_from_slice(chunk);
    }
    let raw = String::from_utf8(bytes).context("订阅不是 UTF-8 YAML 文本")?;
    // Captive portals and error pages can return HTTP 200. Try the next route
    // before allowing any caller to replace its cached subscription.
    compatibility::validate(&raw)?;
    Ok((raw, usage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::routes;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn server(wire: Vec<u8>) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let count = socket.read(&mut request).await.unwrap();
            socket.write_all(&wire).await.unwrap();
            String::from_utf8_lossy(&request[..count]).into_owned()
        });
        (url, task)
    }

    fn response() -> Vec<u8> {
        b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nsubscription-userinfo: upload=1; download=2; total=3\r\n\r\nproxies: []\n".to_vec()
    }

    async fn negotiating_server(
        first_status: u16,
        first_body: &str,
        second_body: &str,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let replies = [
            (first_status, first_body.to_owned()),
            (200, second_body.to_owned()),
        ];
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (index, (status, body)) in replies.into_iter().enumerate() {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut chunk = [0; 1024];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0 && request.len() + count <= 8192);
                    request.extend_from_slice(&chunk[..count]);
                }
                requests.push(String::from_utf8(request).unwrap());
                let wire = format!(
                    "HTTP/1.1 {status} Test\r\nConnection: close\r\nContent-Length: {}\r\nsubscription-userinfo: total={}\r\n\r\n{body}",
                    body.len(),
                    index + 1,
                );
                socket.write_all(wire.as_bytes()).await.unwrap();
            }
            requests
        });
        (url, task)
    }

    fn agent(request: &str) -> &str {
        request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("user-agent")
                    .then(|| value.trim())
            })
            .unwrap()
    }

    #[tokio::test]
    async fn subscription_aliases_fall_back_to_meta_yaml_without_converting_nodes() {
        let yaml = "proxies: [{name: real, type: vless, server: example.test, port: 443, uuid: fixture}]\n";
        // A panel can select its V2rayNG formatter before its Clash formatter.
        let (url, task) = negotiating_server(200, "dmxlc3M6Ly9maXh0dXJl", yaml).await;
        let url = Url::parse(&format!("{url}/private-subscription?token=secret")).unwrap();
        let (raw, usage) = download_routes(&url, &routes(None, None)).await.unwrap();
        assert_eq!(raw, yaml);
        assert_eq!(usage.as_deref(), Some("total=2"));
        let requests = task.await.unwrap();
        assert!(requests.iter().all(|request| {
            request.starts_with("GET /private-subscription?token=secret HTTP/1.1")
        }));
        let primary = agent(&requests[0]);
        for alias in [
            "clash-verge",
            "ClashVerge",
            "ClashMetaForAndroid",
            "clash.meta",
            "mihomo",
            "v2rayN",
            "v2rayNG",
        ] {
            assert!(primary.contains(alias), "{primary}");
        }
        assert!(primary.contains(&format!("clash-of-rust/{}", crate::VERSION)));
        let fallback = agent(&requests[1]);
        assert!(fallback.starts_with("clash.meta mihomo "));
        assert!(!fallback.to_ascii_lowercase().contains("v2ray"));
        assert!(!fallback.contains("ClashVerge"));
        assert!(requests[0].contains("application/yaml"));
    }

    #[tokio::test]
    async fn rejected_agents_and_dummy_help_nodes_negotiate_before_accepting_a_profile() {
        let hints = "proxies: [{name: 当前Clash客户端不支持本机场协议, type: ss, server: 127.0.0.1, port: 1, cipher: aes-128-gcm, password: placeholder}]\n";
        for (status, body) in [(403, ""), (406, ""), (200, hints)] {
            let (url, task) = negotiating_server(status, body, "proxies: []\n").await;
            let (raw, _) = download_routes(&Url::parse(&url).unwrap(), &routes(None, None))
                .await
                .unwrap();
            assert_eq!(raw, "proxies: []\n");
            assert_eq!(task.await.unwrap().len(), 2);
        }
    }

    #[tokio::test]
    async fn repeated_help_nodes_fail_without_exposing_tokens_or_placeholder_credentials() {
        let hints = "proxies: [{name: 当前Clash客户端不支持本机场协议, type: ss, server: 127.0.0.1, port: 1, cipher: aes-128-gcm, password: private-placeholder}]\n";
        let (url, task) = negotiating_server(200, hints, hints).await;
        let url = Url::parse(&format!("{url}/private-subscription?token=secret")).unwrap();
        let error = download_routes(&url, &routes(None, None))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("兼容提示节点"));
        for secret in ["private-subscription", "secret", "private-placeholder"] {
            assert!(!error.contains(secret));
        }
        assert_eq!(task.await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn authentication_rate_limits_and_server_errors_do_not_retry_the_agent() {
        for status in [401, 404, 429, 500, 503] {
            let wire = format!("HTTP/1.1 {status} Test\r\nContent-Length: 0\r\n\r\n");
            let (url, task) = server(wire.into_bytes()).await;
            let error = download_routes(&Url::parse(&url).unwrap(), &routes(None, None))
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(&format!("HTTP {status}")));
            assert!(!error.contains("Mihomo 兼容请求"));
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn unavailable_core_falls_back_to_system_proxy_with_usage_metadata() {
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let (proxy, task) = server(response()).await;
        let url = Url::parse("http://subscription.invalid/config").unwrap();
        let (raw, usage) = download_routes(&url, &routes(Some(port), Some(proxy)))
            .await
            .unwrap();
        assert_eq!(raw, "proxies: []\n");
        assert_eq!(usage.as_deref(), Some("upload=1; download=2; total=3"));
        assert!(
            task.await
                .unwrap()
                .starts_with("GET http://subscription.invalid/config HTTP/1.1")
        );
    }

    #[tokio::test]
    async fn failed_proxy_falls_back_to_direct() {
        let (proxy, proxy_task) =
            server(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n".to_vec()).await;
        let (origin, origin_task) = server(response()).await;
        let url = Url::parse(&format!("{origin}/config")).unwrap();
        assert_eq!(
            download_routes(&url, &routes(None, Some(proxy)))
                .await
                .unwrap()
                .0,
            "proxies: []\n"
        );
        assert!(
            proxy_task
                .await
                .unwrap()
                .starts_with(&format!("GET {origin}/config HTTP/1.1"))
        );
        assert!(
            origin_task
                .await
                .unwrap()
                .starts_with("GET /config HTTP/1.1")
        );
    }

    #[tokio::test]
    async fn incomplete_oversized_and_non_utf8_subscriptions_are_rejected() {
        for wire in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\npartial".to_vec(),
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", LIMIT + 1).into_bytes(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n\xff".to_vec(),
        ] {
            let (url, task) = server(wire).await;
            let error = download_routes(&Url::parse(&url).unwrap(), &routes(None, None))
                .await
                .unwrap_err();
            assert!(error.to_string().contains("直连"));
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn invalid_success_response_falls_back_to_a_valid_route() {
        for raw in ["", "<html>Sign in</html>", "proxies: ["] {
            let wire = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{raw}",
                raw.len()
            );
            let (proxy, proxy_task) = server(wire.into_bytes()).await;
            let (origin, origin_task) = server(response()).await;
            let (raw, usage) =
                download_routes(&Url::parse(&origin).unwrap(), &routes(None, Some(proxy)))
                    .await
                    .unwrap();
            assert_eq!(raw, "proxies: []\n");
            assert_eq!(usage.as_deref(), Some("upload=1; download=2; total=3"));
            proxy_task.await.unwrap();
            origin_task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn all_failed_routes_return_an_error_without_including_the_subscription_url() {
        let (proxy, proxy_task) =
            server(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n".to_vec()).await;
        let (origin, origin_task) =
            server(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec()).await;
        let url = Url::parse(&format!("{origin}/private-subscription?token=secret")).unwrap();
        let error = download_routes(&url, &routes(None, Some(proxy)))
            .await
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("HTTP 503"));
        assert!(message.contains("订阅响应不是有效"));
        assert!(message.contains("已尝试可用代理和直连"));
        assert!(!message.contains("private-subscription") && !message.contains("secret"));
        proxy_task.await.unwrap();
        origin_task.await.unwrap();
    }
}
