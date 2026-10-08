//! Bounded Release discovery and streaming, checksum-verified installer downloads.
mod discovery;
mod install;
use anyhow::{Context, Result, bail};
pub use install::{
    InstallSession, cleanup, ensure_installable, handoff, helper_main, startup_outcome,
};
use reqwest::{Client, StatusCode, Url};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::Duration};
use tokio::io::AsyncWriteExt;

pub const RELEASES_URL: &str = "https://github.com/Fu021/Clash-of-Rust/releases";
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const API_URL: &str = "https://api.github.com/repos/Fu021/Clash-of-Rust/releases?per_page=100";
const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;
const PACKAGE_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub asset: Asset,
    pub url: String,
    pub checksum_url: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Progress {
    pub received: u64,
    pub total: u64,
}

#[derive(Debug)]
pub struct Downloaded {
    directory: tempfile::TempDir,
    package: PathBuf,
    sha256: String,
    version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    pub version: String,
    pub url: String,
    pub preview: bool,
    pub package: Option<Package>,
}

#[derive(Default)]
pub struct State {
    pub checking: bool,
    pub started: bool,
    pub available: Option<Available>,
    pub status: String,
    pub progress: Option<Progress>,
    pub installing: Option<InstallSession>,
    pub install_pending: bool,
    pub failure: Option<String>,
    pub outcome: Option<String>,
}

impl State {
    /// Coalesce automatic and manual checks instead of queuing more requests.
    pub fn begin(&mut self) -> bool {
        if self.checking || self.busy() {
            return false;
        }
        self.checking = true;
        self.started = true;
        true
    }

    pub fn busy(&self) -> bool {
        self.progress.is_some() || self.installing.is_some() || self.install_pending
    }

    pub fn fail(&mut self, reason: String) {
        self.progress = None;
        self.installing = None;
        self.install_pending = false;
        self.failure = Some(format!("更新失败：{reason}"));
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
    #[serde(default)]
    assets: Vec<Asset>,
}

fn package_name(version: &str) -> Option<String> {
    let suffix = if cfg!(all(windows, target_arch = "x86_64")) {
        "windows-x64-setup.exe"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-amd64.deb"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "linux-arm64.deb"
    } else {
        return None;
    };
    Some(format!("Clash-of-Rust-{version}-{suffix}"))
}

fn package(release: &Release, version: &str) -> Option<Package> {
    let name = package_name(version)?;
    let mut matches = release.assets.iter().filter(|asset| asset.name == name);
    let asset = matches.next()?.clone();
    if matches.next().is_some() || asset.size == 0 || asset.size > PACKAGE_LIMIT {
        return None;
    }
    if release
        .assets
        .iter()
        .filter(|asset| {
            asset.name == format!("{name}.sha256") && asset.size > 0 && asset.size <= 1024
        })
        .count()
        != 1
    {
        return None;
    }
    let mut url = Url::parse(RELEASES_URL).ok()?;
    url.path_segments_mut()
        .ok()?
        .push("download")
        .push(&release.tag_name)
        .push(&name);
    let mut checksum = url.clone();
    checksum
        .path_segments_mut()
        .ok()?
        .pop()
        .push(&format!("{name}.sha256"));
    Some(Package {
        asset,
        url: url.into(),
        checksum_url: checksum.into(),
    })
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
            package: package(&release, &version.to_string()),
            version: version.to_string(),
            url: url.into(),
            preview: release.prerelease || !version.pre.is_empty(),
        }
    }))
}

fn download_client(proxy_port: Option<u16>) -> Result<Client> {
    let proxy = proxy_port.map(|port| format!("http://127.0.0.1:{port}"));
    download_client_via(proxy.as_deref(), Duration::from_secs(30 * 60))
}

fn download_client_via(proxy: Option<&str>, timeout: Duration) -> Result<Client> {
    let mut builder = Client::builder()
        .no_proxy()
        .user_agent(format!("Clash-of-Rust/{}", crate::VERSION))
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            let trusted = matches!(
                url.host_str(),
                Some(
                    "github.com"
                        | "release-assets.githubusercontent.com"
                        | "objects.githubusercontent.com"
                )
            );
            if attempt.previous().len() >= 5
                || url.scheme() != "https"
                || !trusted
                || !url.username().is_empty()
                || url.password().is_some()
            {
                attempt.error("下载重定向地址无效")
            } else {
                attempt.follow()
            }
        }));
    if let Some(proxy) = proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy)?);
    }
    builder.build().context("无法创建下载连接")
}

fn checksum(bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes)
        .context("校验文件格式无效")?
        .trim();
    // Our release sidecars contain exactly a single SHA256, not shell commands.
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("校验文件格式无效");
    }
    Ok(text.to_ascii_lowercase())
}

pub async fn download(
    available: Available,
    proxy_port: Option<u16>,
    report: impl FnMut(Progress),
) -> Result<Downloaded> {
    if cfg!(windows)
        && Version::parse(&available.version)
            .context("新版本号无效")?
            .cmp_precedence(&Version::parse("0.4.6-0").unwrap())
            .is_lt()
    {
        bail!("该版本不支持自动安装，请从 Release 手动升级");
    }
    download_package(available, proxy_port, report).await
}

async fn download_package(
    available: Available,
    proxy_port: Option<u16>,
    report: impl FnMut(Progress),
) -> Result<Downloaded> {
    let package = available
        .package
        .context("该版本缺少本平台安装包或校验文件")?;
    let client = download_client(proxy_port)?;
    let mut response = client
        .get(&package.checksum_url)
        .send()
        .await
        .context("无法下载校验文件，请检查网络或代理")?;
    if response.status() != StatusCode::OK {
        bail!("校验文件下载失败（HTTP {}）", response.status().as_u16());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.context("校验文件下载中断")? {
        if bytes.len() + chunk.len() > 1024 {
            bail!("校验文件过大");
        }
        bytes.extend_from_slice(&chunk);
    }
    let expected = checksum(&bytes)?;
    if let Some(digest) = &package.asset.digest
        && digest != &format!("sha256:{expected}")
    {
        bail!("Release 校验信息不一致");
    }
    let directory = tempfile::Builder::new()
        .prefix("clash-of-rust-update-")
        .tempdir()
        .context("无法创建下载目录，请检查磁盘权限")?;
    let path = directory.path().join(&package.asset.name);
    transfer(
        &client,
        &package.url,
        package.asset.size,
        &expected,
        &path,
        report,
    )
    .await?;
    Ok(Downloaded {
        directory,
        package: path,
        sha256: expected,
        version: available.version,
    })
}

async fn transfer(
    client: &Client,
    url: &str,
    size: u64,
    expected: &str,
    path: &std::path::Path,
    mut report: impl FnMut(Progress),
) -> Result<()> {
    let mut response = client
        .get(url)
        .send()
        .await
        .context("无法连接下载地址，请检查网络或代理")?;
    if response.status() != StatusCode::OK {
        bail!("安装包下载失败（HTTP {}）", response.status().as_u16());
    }
    if size == 0
        || size > PACKAGE_LIMIT
        || response
            .content_length()
            .is_some_and(|length| length != size)
    {
        bail!("安装包大小与 Release 不一致");
    }
    let mut file = tokio::fs::File::create(path)
        .await
        .context("无法写入安装包，请检查磁盘空间与权限")?;
    let mut received = 0;
    let mut digest = Sha256::new();
    let mut last_report = std::time::Instant::now();
    while let Some(chunk) = response.chunk().await.context("下载中断或超时，请重试")? {
        received += chunk.len() as u64;
        if received > size {
            bail!("安装包大小与 Release 不一致");
        }
        file.write_all(&chunk)
            .await
            .context("写入失败，请检查磁盘空间与权限")?;
        digest.update(&chunk);
        if last_report.elapsed() >= Duration::from_millis(100) || received == size {
            report(Progress {
                received,
                total: size,
            });
            last_report = std::time::Instant::now();
        }
    }
    file.flush().await.context("无法保存安装包")?;
    drop(file);
    if received != size {
        bail!("安装包下载不完整");
    }
    if format!("{:x}", digest.finalize()) != expected {
        bail!("安装包 SHA256 校验失败，请重试");
    }
    Ok(())
}

pub async fn check(proxy_port: Option<u16>) -> Result<Option<Available>> {
    discovery::check(proxy_port).await
}

async fn fetch(client: &Client, endpoint: &str, current: &str) -> Result<Option<Available>> {
    let response = client
        .get(endpoint)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| discovery::request_error(error, "GitHub API"))?;
    decode(response, current).await
}

async fn decode(response: reqwest::Response, current: &str) -> Result<Option<Available>> {
    let bytes = release_bytes(response).await?;
    let releases = serde_json::from_slice(&bytes).context("Release 信息格式无效")?;
    newest(releases, current)
}

async fn release_bytes(mut response: reqwest::Response) -> Result<Vec<u8>> {
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
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| discovery::request_error(error, "Release 信息"))?
    {
        if chunk.len() > RESPONSE_LIMIT.saturating_sub(bytes.len()) {
            bail!("Release 响应超过 2 MiB 限制");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
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
            assets: Vec::new(),
        }
    }

    #[test]
    fn selects_only_the_exact_platform_package_with_a_sidecar() {
        let name = package_name("0.4.6").unwrap();
        let mut release = release("v0.4.6", false);
        release.assets = vec![
            Asset {
                name: name.clone(),
                size: 4096,
                digest: None,
            },
            Asset {
                name: format!("{name}.sha256"),
                size: 65,
                digest: None,
            },
            Asset {
                name: "unrelated.exe".into(),
                size: 100,
                digest: None,
            },
        ];
        let selected = package(&release, "0.4.6").unwrap();
        assert_eq!(selected.asset.name, name);
        assert_eq!(
            selected.url,
            format!("{RELEASES_URL}/download/v0.4.6/{name}")
        );
        release.assets.remove(1);
        assert!(package(&release, "0.4.6").is_none());
        release.assets.push(Asset {
            name: format!("{name}.sha256"),
            size: 65,
            digest: None,
        });
        release.assets[0].size = PACKAGE_LIMIT + 1;
        assert!(package(&release, "0.4.6").is_none());
    }

    #[test]
    fn every_failure_clears_progress_and_unlocks_retry_without_losing_the_version() {
        for reason in [
            "连接超时",
            "校验失败",
            "授权取消",
            "磁盘空间不足",
            "安装失败",
        ] {
            let available = newest(vec![release("v0.4.6", false)], "0.4.5").unwrap();
            let mut state = State {
                available: available.clone(),
                progress: Some(Progress {
                    received: 2048,
                    total: 4096,
                }),
                install_pending: true,
                installing: None,
                ..State::default()
            };
            assert!(!state.begin());
            state.fail(reason.into());
            assert!(state.progress.is_none());
            assert!(!state.busy());
            assert_eq!(state.available, available);
            assert!(state.failure.unwrap().contains(reason));
        }
    }

    #[test]
    fn checksum_rejects_html_commands_and_trailing_content() {
        let hash = "a".repeat(64);
        assert_eq!(checksum(format!("{hash}\n").as_bytes()).unwrap(), hash);
        for bad in [
            "<html>error</html>".to_owned(),
            format!("{hash} run.exe"),
            "x".repeat(64),
            String::new(),
        ] {
            assert!(checksum(bad.as_bytes()).is_err());
        }
    }

    #[tokio::test]
    async fn streams_progress_and_verifies_the_exact_downloaded_bytes() {
        let body = vec![42; 256 * 1024];
        let expected = format!("{:x}", Sha256::digest(&body));
        let wire = [
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes(),
            body.clone(),
        ]
        .concat();
        let (url, server) = mock(wire, Duration::ZERO).await;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("package");
        let mut progress = Vec::new();
        transfer(
            &client(Duration::from_secs(2)),
            &url,
            body.len() as u64,
            &expected,
            &path,
            |value| progress.push(value),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), body);
        assert_eq!(progress.last().unwrap().received, body.len() as u64);
        assert!(
            progress
                .windows(2)
                .all(|values| values[0].received <= values[1].received)
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn refuses_corrupt_truncated_oversized_and_error_downloads() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("package");
        for (wire, size, expected) in [
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nBAD".to_vec(),
                3,
                "0".repeat(64),
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nBAD".to_vec(),
                5,
                "0".repeat(64),
            ),
            (
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nBAD\r\n0\r\n\r\n"
                    .to_vec(),
                2,
                "0".repeat(64),
            ),
            (
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
                3,
                "0".repeat(64),
            ),
        ] {
            let (url, server) = mock(wire, Duration::ZERO).await;
            assert!(
                transfer(
                    &client(Duration::from_secs(2)),
                    &url,
                    size,
                    &expected,
                    &path,
                    |_| {}
                )
                .await
                .is_err()
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn download_does_not_follow_an_untrusted_or_insecure_redirect() {
        let wire = b"HTTP/1.1 302 Found\r\nLocation: http://example.com/package.exe\r\nContent-Length: 0\r\n\r\n".to_vec();
        let (url, server) = mock(wire, Duration::ZERO).await;
        let error = download_client(None)
            .unwrap()
            .get(url)
            .send()
            .await
            .unwrap_err();
        assert!(error.is_redirect());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn proxy_download_uses_the_selected_loopback_port() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let count = socket.read(&mut request).await.unwrap();
            assert!(
                String::from_utf8_lossy(&request[..count])
                    .starts_with("GET http://download.invalid/package HTTP/1.1")
            );
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc")
                .await
                .unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        transfer(
            &download_client(Some(port)).unwrap(),
            "http://download.invalid/package",
            3,
            &format!("{:x}", Sha256::digest(b"abc")),
            &directory.path().join("package"),
            |_| {},
        )
        .await
        .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    #[ignore = "downloads the current official installer without executing it"]
    async fn downloads_and_verifies_the_official_installer() {
        let proxy = std::env::var("UPDATE_TEST_PROXY_PORT")
            .ok()
            .map(|value| value.parse::<u16>().unwrap());
        let available = fetch(&client(Duration::from_secs(20)), API_URL, "0.0.0")
            .await
            .unwrap()
            .unwrap();
        let downloaded = download_package(available, proxy, |_| {}).await.unwrap();
        assert!(downloaded.package.is_file());
        eprintln!(
            "Verified {}: {} bytes, SHA256 {} (not installed)",
            downloaded.version,
            std::fs::metadata(&downloaded.package).unwrap().len(),
            downloaded.sha256
        );
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
        let error = fetch(&client(Duration::from_millis(50)), &url, "0.4.1")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("GitHub API连接超时"));
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
