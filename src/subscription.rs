//! Bounded subscription downloads using the same routes as client updates.
use crate::network::{Route, available_routes, request_error};
use anyhow::{Context, Result, bail};
use reqwest::{Client, Url};
use std::time::Duration;

const LIMIT: usize = 10 * 1024 * 1024;

pub(crate) async fn download(url: &Url, port: Option<u16>) -> Result<(String, Option<String>)> {
    download_routes(url, &available_routes(port, url.scheme())).await
}

async fn download_routes(url: &Url, routes: &[Route]) -> Result<(String, Option<String>)> {
    let mut errors = Vec::new();
    for route in routes {
        match download_route(url, route).await {
            Ok(subscription) => return Ok(subscription),
            Err(error) => errors.push(format!("{}：{error}", route.label)),
        }
    }
    bail!("{}（已尝试可用代理和直连）", errors.join("；"));
}

async fn download_route(url: &Url, route: &Route) -> Result<(String, Option<String>)> {
    let client = route
        .apply(
            Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .read_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(30))
                .user_agent(format!("clash-of-rust/{}", crate::VERSION)),
        )?
        .build()
        .context("无法创建订阅下载连接")?;
    let mut response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|error| request_error(error, "订阅下载"))?;
    if !response.status().is_success() {
        bail!("订阅服务器返回 HTTP {}", response.status().as_u16());
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
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| request_error(error, "订阅下载"))?
    {
        if chunk.len() > LIMIT.saturating_sub(bytes.len()) {
            bail!("订阅超过 10 MiB 限制");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((
        String::from_utf8(bytes).context("订阅不是 UTF-8 YAML 文本")?,
        usage,
    ))
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
}
