//! Release metadata is short lived; only the selected version and URL survive a check.
use anyhow::{Context, Result, bail};
use reqwest::{Client, StatusCode, Url};
use semver::Version;
use serde::Deserialize;
use std::time::Duration;

pub const RELEASES_URL: &str = "https://github.com/Fu021/Clash-of-Rust/releases";
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const API_URL: &str = "https://api.github.com/repos/Fu021/Clash-of-Rust/releases?per_page=100";
const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    pub version: String,
    pub url: String,
    pub preview: bool,
}

#[derive(Default)]
pub struct State {
    pub checking: bool,
    pub started: bool,
    pub available: Option<Available>,
    pub status: String,
}

impl State {
    /// Coalesce automatic and manual checks instead of queuing more requests.
    pub fn begin(&mut self) -> bool {
        if self.checking {
            return false;
        }
        self.checking = true;
        self.started = true;
        true
    }

    pub fn finish(&mut self, result: Result<Option<Available>, String>) {
        self.checking = false;
        let description = match result {
            Ok(available) => {
                self.available = available;
                match &self.available {
                    Some(update) => format!("发现新版本 {}", update.version),
                    None => "未发现新版本".into(),
                }
            }
            // A temporary network error must not erase an existing update prompt.
            Err(error) => format!("检查失败：{error}"),
        };
        self.status = format!(
            "{} · {description}",
            chrono::Local::now().format("%m-%d %H:%M")
        );
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn newest(releases: Vec<Release>, current: &str) -> Result<Option<Available>> {
    let current = Version::parse(current).context("当前客户端版本号无效")?;
    let selected = releases
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let version = Version::parse(
                release
                    .tag_name
                    .strip_prefix('v')
                    .unwrap_or(&release.tag_name),
            )
            .ok()?;
            version
                .cmp_precedence(&current)
                .is_gt()
                .then_some((version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp_precedence(right));
    Ok(selected.map(|(version, release)| {
        // Build the link from our repository, never an arbitrary URL in metadata.
        let mut url = Url::parse(RELEASES_URL).expect("constant release URL");
        url.path_segments_mut()
            .expect("GitHub URL has a path")
            .push("tag")
            .push(&release.tag_name);
        Available {
            version: version.to_string(),
            url: url.into(),
            preview: release.prerelease || !version.pre.is_empty(),
        }
    }))
}

pub async fn check(proxy_port: Option<u16>) -> Result<Option<Available>> {
    let mut builder = Client::builder()
        .no_proxy()
        .user_agent(format!("Clash-of-Rust/{}", crate::VERSION))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(port) = proxy_port {
        builder = builder.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?);
    }
    // Include the project's earlier releases, even those GitHub omits from /latest.
    fetch(&builder.build()?, API_URL, crate::VERSION).await
}

async fn fetch(client: &Client, endpoint: &str, current: &str) -> Result<Option<Available>> {
    let response = client
        .get(endpoint)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("无法连接 GitHub，请检查网络或代理")?;
    decode(response, current).await
}

async fn decode(mut response: reqwest::Response, current: &str) -> Result<Option<Available>> {
    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS
        || (status == StatusCode::FORBIDDEN
            && response
                .headers()
                .get("x-ratelimit-remaining")
                .is_some_and(|remaining| remaining == "0"))
    {
        bail!("GitHub 暂时限制请求，请稍后再试");
    }
    if !status.is_success() {
        bail!("GitHub 返回 HTTP {status}");
    }
    if response
        .content_length()
        .is_some_and(|size| size > RESPONSE_LIMIT as u64)
    {
        bail!("Release 响应超过 2 MiB 限制");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.context("读取 Release 信息失败")? {
        if chunk.len() > RESPONSE_LIMIT.saturating_sub(bytes.len()) {
            bail!("Release 响应超过 2 MiB 限制");
        }
        bytes.extend_from_slice(&chunk);
    }
    let releases = serde_json::from_slice(&bytes).context("Release 信息格式无效")?;
    newest(releases, current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn release(tag: &str, preview: bool) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: preview,
        }
    }

    #[test]
    fn compares_numeric_versions_and_includes_github_previews() {
        let releases = vec![
            release("v0.4.9", false),
            release("v0.4.10", true),
            release("v0.4.2", false),
        ];
        let update = newest(releases, "0.4.1").unwrap().unwrap();
        assert_eq!(update.version, "0.4.10");
        assert!(update.preview);
        assert_eq!(update.url, format!("{RELEASES_URL}/tag/v0.4.10"));
    }

    #[test]
    fn ignores_drafts_invalid_tags_and_build_metadata_changes() {
        let mut draft = release("v9.0.0", false);
        draft.draft = true;
        let releases = vec![
            draft,
            release("nightly", false),
            release("v0.4.1+other", false),
        ];
        assert!(newest(releases, "0.4.1+local").unwrap().is_none());
        assert!(
            newest(vec![release("v0.4.0", false)], "0.4.1")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn stable_version_replaces_same_version_beta() {
        let releases = vec![release("v0.5.0-beta.10", true), release("v0.5.0", false)];
        let update = newest(releases, "0.5.0-beta.2").unwrap().unwrap();
        assert_eq!(update.version, "0.5.0");
        assert!(!update.preview);
        assert!(
            newest(vec![release("v0.5.0-beta.10", true)], "0.5.0")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn coalesces_checks_and_keeps_prompt_after_failure() {
        let mut state = State::default();
        assert!(state.begin());
        assert!(!state.begin());
        let update = newest(vec![release("v0.5.0", true)], "0.4.1").unwrap();
        state.finish(Ok(update.clone()));
        assert!(state.begin());
        state.finish(Err("offline".into()));
        assert_eq!(state.available, update);
        assert!(!state.checking);
        assert!(state.status.contains("offline"));
        assert!(state.begin());
        state.finish(Ok(None));
        assert!(state.available.is_none());
    }

    async fn mock(wire: Vec<u8>, delay: Duration) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).await;
            tokio::time::sleep(delay).await;
            let _ = socket.write_all(&wire).await;
        });
        (url, server)
    }

    fn client(timeout: Duration) -> Client {
        Client::builder()
            .no_proxy()
            .user_agent(format!("Clash-of-Rust/{}", crate::VERSION))
            .timeout(timeout)
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn reads_preview_release_metadata_without_retaining_notes() {
        let body = br#"[{"tag_name":"v0.4.2","draft":false,"prerelease":true,"body":"notes","html_url":"https://example.com"}]"#;
        let wire =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        let (url, server) = mock([wire, body.to_vec()].concat(), Duration::ZERO).await;
        let update = fetch(&client(Duration::from_secs(2)), &url, "0.4.1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(update.url, format!("{RELEASES_URL}/tag/v0.4.2"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn reports_http_errors_malformed_json_and_timeouts() {
        for wire in [
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n{".to_vec(),
        ] {
            let (url, server) = mock(wire, Duration::ZERO).await;
            assert!(
                fetch(&client(Duration::from_secs(2)), &url, "0.4.1")
                    .await
                    .is_err()
            );
            server.await.unwrap();
        }
        let (url, server) = mock(Vec::new(), Duration::from_secs(2)).await;
        assert!(
            fetch(&client(Duration::from_millis(50)), &url, "0.4.1")
                .await
                .is_err()
        );
        server.abort();
    }

    #[tokio::test]
    async fn caps_both_declared_and_streamed_response_sizes() {
        let wire = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            RESPONSE_LIMIT + 1
        )
        .into_bytes();
        let (url, server) = mock(wire, Duration::ZERO).await;
        assert!(
            fetch(&client(Duration::from_secs(2)), &url, "0.4.1")
                .await
                .unwrap_err()
                .to_string()
                .contains("2 MiB")
        );
        server.await.unwrap();

        let mut wire = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        wire.extend(format!("{:x}\r\n", RESPONSE_LIMIT + 1).as_bytes());
        wire.extend(vec![b' '; RESPONSE_LIMIT + 1]);
        wire.extend(b"\r\n0\r\n\r\n");
        let (url, server) = mock(wire, Duration::ZERO).await;
        assert!(
            fetch(&client(Duration::from_secs(2)), &url, "0.4.1")
                .await
                .unwrap_err()
                .to_string()
                .contains("2 MiB")
        );
        server.await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires access to the public GitHub API"]
    async fn checks_official_release_feed() {
        let result = check(None).await.unwrap();
        eprintln!("Client {}: {result:?}", crate::VERSION);
        if let Some(update) = result {
            assert!(
                Version::parse(&update.version)
                    .unwrap()
                    .cmp_precedence(&Version::parse(crate::VERSION).unwrap())
                    .is_gt()
            );
        }
        // An older client must detect the project's published preview releases.
        let update = fetch(&client(Duration::from_secs(15)), API_URL, "0.0.0")
            .await
            .unwrap()
            .unwrap();
        assert!(update.url.starts_with(&format!("{RELEASES_URL}/tag/")));
    }
}
