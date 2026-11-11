//! Home-page network overview: reachability of well-known services through the
//! local proxy, and a timed system DNS lookup. Reachability only means an HTTP
//! response arrived; it never claims an account or region is unlocked.
use crate::probe::validate_url;
use anyhow::{Result, bail};
use std::time::{Duration, Instant};

#[path = "overview/presentation.rs"]
mod presentation;
pub use presentation::{HostInfo, Item, Presentation, Probe, View};

pub struct Target {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
}

/// Light endpoints first: the `generate_204` pages answer without a body.
pub const TARGETS: [Target; 8] = [
    Target {
        id: "google",
        name: "Google",
        url: "https://www.google.com/generate_204",
    },
    Target {
        id: "youtube",
        name: "YouTube",
        url: "https://www.youtube.com/generate_204",
    },
    Target {
        id: "github",
        name: "GitHub",
        url: "https://github.com/",
    },
    Target {
        id: "chatgpt",
        name: "ChatGPT",
        url: "https://chatgpt.com/",
    },
    Target {
        id: "claude",
        name: "Claude",
        url: "https://claude.ai/",
    },
    Target {
        id: "gemini",
        name: "Gemini",
        url: "https://gemini.google.com/",
    },
    Target {
        id: "telegram",
        name: "Telegram",
        url: "https://telegram.org/",
    },
    Target {
        id: "x",
        name: "X",
        url: "https://x.com/",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// 2xx or 3xx from the site itself.
    Ok,
    /// The site answered, but with a client error such as a bot challenge.
    Reachable,
    /// The site answered with a server error.
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub health: Health,
    pub status: u16,
    pub millis: u128,
}

pub fn classify(status: u16) -> Health {
    match status {
        200..=399 => Health::Ok,
        400..=499 => Health::Reachable,
        _ => Health::Error,
    }
}

fn failure(error: reqwest::Error) -> anyhow::Error {
    let error = error.without_url();
    if error.is_timeout() {
        anyhow::anyhow!("超时")
    } else if error.is_connect() {
        anyhow::anyhow!("无法连接")
    } else {
        anyhow::anyhow!("请求失败")
    }
}

/// Time the first response through the local mixed port. Redirects are not
/// followed so the figure is the target's own response time.
pub async fn check(url: &str, port: u16) -> Result<Status> {
    Checker::new(port)?.check(url).await
}

#[derive(Clone)]
pub struct Checker(reqwest::Client);

impl Checker {
    pub fn new(port: u16) -> Result<Self> {
        let client = crate::network::client_builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36")
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(0)
        .build()?;
        Ok(Self(client))
    }

    pub async fn check(&self, url: &str) -> Result<Status> {
        let url = validate_url(url)?;
        let started = Instant::now();
        let response = self.0.get(url).send().await.map_err(failure)?;
        let millis = started.elapsed().as_millis();
        let status = response.status().as_u16();
        Ok(Status {
            health: classify(status),
            status,
            millis,
        })
    }
}

/// Resolve one name with the system resolver and report how long it took.
pub async fn dns_lookup(host: &str) -> Result<(Vec<String>, u128)> {
    let started = Instant::now();
    let addresses = crate::probe::dns(host.into()).await?;
    if addresses.is_empty() {
        bail!("未返回地址");
    }
    Ok((addresses, started.elapsed().as_millis()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn catalog_is_unique_and_only_http() {
        let mut ids = std::collections::HashSet::new();
        for target in &TARGETS {
            assert!(ids.insert(target.id));
            assert!(validate_url(target.url).is_ok());
            assert!(!target.name.is_empty());
        }
    }

    #[test]
    fn statuses_separate_reachable_from_working_and_broken() {
        assert_eq!(classify(204), Health::Ok);
        assert_eq!(classify(307), Health::Ok);
        assert_eq!(classify(403), Health::Reachable);
        assert_eq!(classify(429), Health::Reachable);
        assert_eq!(classify(502), Health::Error);
    }

    #[tokio::test]
    async fn probe_goes_through_the_local_proxy_without_following_redirects() {
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = proxy.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
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
            assert!(String::from_utf8_lossy(&request).starts_with("GET http://site.example/ HTTP"));
            socket
                .write_all(b"HTTP/1.1 302 Found\r\nLocation: http://other.example/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let status =
            tokio::time::timeout(Duration::from_secs(5), check("http://site.example/", port))
                .await
                .unwrap()
                .unwrap();
        assert_eq!((status.health, status.status), (Health::Ok, 302));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn refused_proxy_is_reported_as_a_connection_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let error = check("http://site.example/", port).await.unwrap_err();
        assert_eq!(error.to_string(), "无法连接");
    }

    #[tokio::test]
    async fn shared_checker_keeps_its_proxy_for_concurrent_probes() {
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let checker = Checker::new(proxy.local_addr().unwrap().port()).unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = proxy.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buffer = [0; 1024];
                    let length = socket.read(&mut buffer).await.unwrap();
                    assert!(length > 0);
                    request.extend_from_slice(&buffer[..length]);
                    if request.windows(4).any(|part| part == b"\r\n\r\n") {
                        break;
                    }
                }
                assert!(
                    String::from_utf8_lossy(&request).starts_with("GET http://site.example/ HTTP")
                );
                socket
                    .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
            }
        });
        let cloned = checker.clone();
        let (first, second) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(
                checker.check("http://site.example/"),
                cloned.check("http://site.example/")
            )
        })
        .await
        .unwrap();
        assert_eq!(first.unwrap().status, 204);
        assert_eq!(second.unwrap().status, 204);
        server.await.unwrap();
        assert!(checker.check("file:///etc/hosts").await.is_err());
    }
}
