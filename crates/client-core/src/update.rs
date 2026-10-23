//! Release discovery and bounded, verified downloads for native hosts.
use anyhow::{Context, Result, bail, ensure};
use reqwest::Client;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
use tokio::io::AsyncWriteExt;
pub mod channel;
mod retry;
mod route;
pub use retry::check as retry_check;
pub use route::Observer as RouteObserver;

const API: &str = "https://api.github.com/repos/Fu021/Clash-of-Rust/releases?per_page=100";
const RELEASES: &str = "https://github.com/Fu021/Clash-of-Rust/releases";
const LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub received: u64,
    pub total: u64,
    pub bytes_per_second: u64,
    pub route: &'static str,
    pub outbound: Option<std::sync::Arc<str>>,
}
impl Progress {
    pub fn routing(&self) -> String {
        match self.outbound.as_deref() {
            Some(node) if node == "DIRECT" || node.starts_with("DIRECT → ") => {
                format!("{} · 直连 ({node})", self.route)
            }
            Some(node) => format!("{} · {node}", self.route),
            None => format!("{} · 正在确认出站", self.route),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Package {
    pub version: String,
    pub preview: bool,
    pub name: String,
    pub size: u64,
    pub url: String,
    pub checksum_url: String,
    digest: Option<String>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    digest: Option<String>,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}
fn client(route: &crate::network::Route, timeout: Duration) -> Result<Client> {
    route
        .apply(
            Client::builder()
                .no_proxy()
                .user_agent(format!("Clash-of-Rust/{}", crate::VERSION))
                .connect_timeout(Duration::from_secs(5))
                .read_timeout(Duration::from_secs(30))
                .timeout(timeout)
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    let u = attempt.url();
                    if attempt.previous().len() < 5
                        && u.scheme() == "https"
                        && u.username().is_empty()
                        && u.password().is_none()
                        && matches!(
                            u.host_str(),
                            Some(
                                "github.com"
                                    | "release-assets.githubusercontent.com"
                                    | "objects.githubusercontent.com"
                            )
                        )
                    {
                        attempt.follow()
                    } else {
                        attempt.error("更新下载重定向地址无效")
                    }
                })),
        )?
        .build()
        .context("无法创建更新连接")
}
async fn bounded(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    ensure!(
        response.status().is_success(),
        "更新服务器返回 HTTP {}",
        response.status()
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(bytes.len() + chunk.len() <= limit, "更新信息超过大小限制");
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
#[cfg(test)]
fn select(releases: Vec<Release>, current: &str, suffix: &str) -> Result<Option<Package>> {
    select_channel(releases, current, suffix, false)
}
fn select_channel(
    releases: Vec<Release>,
    current: &str,
    suffix: &str,
    include_prereleases: bool,
) -> Result<Option<Package>> {
    let current = Version::parse(current)?;
    let current_is_preview = channel::installed_preview(
        &current,
        releases
            .iter()
            .map(|release| (release.tag_name.as_str(), release.draft, release.prerelease)),
    );
    ensure!(
        suffix
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c)),
        "安装包平台无效"
    );
    let release = releases
        .into_iter()
        .filter_map(|r| {
            let v = channel::candidate(
                &r.tag_name,
                r.draft,
                r.prerelease,
                &current,
                include_prereleases,
                current_is_preview,
            )?;
            Some((v, r))
        })
        .max_by(|(a, _), (b, _)| a.cmp_precedence(b));
    if let Some((version, release)) = release {
        let name = format!("Clash-of-Rust-{version}-{suffix}");
        let matches = release
            .assets
            .iter()
            .filter(|a| a.name == name)
            .collect::<Vec<_>>();
        let sidecar = format!("{name}.sha256");
        ensure!(
            matches.len() == 1
                && release
                    .assets
                    .iter()
                    .filter(|a| a.name == sidecar && a.size > 0 && a.size <= 1024)
                    .count()
                    == 1,
            "版本 {version} 缺少本平台安装包或校验文件（{suffix}）"
        );
        let asset = matches[0];
        ensure!(asset.size > 0 && asset.size <= LIMIT, "安装包大小无效");
        let mut url = reqwest::Url::parse(RELEASES)?;
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("更新地址无效"))?
            .push("download")
            .push(&release.tag_name)
            .push(&name);
        return Ok(Some(Package {
            preview: release.prerelease || !version.pre.is_empty(),
            version: version.to_string(),
            name,
            size: asset.size,
            checksum_url: format!("{url}.sha256"),
            url: url.into(),
            digest: asset.digest.clone(),
        }));
    }
    Ok(None)
}
pub async fn check(current: &str, suffix: &str, port: Option<u16>) -> Result<Option<Package>> {
    check_with_prereleases(current, suffix, port, false).await
}
pub async fn check_with_prereleases(
    current: &str,
    suffix: &str,
    port: Option<u16>,
    include_prereleases: bool,
) -> Result<Option<Package>> {
    // Validate local arguments once; retry only the remote discovery operation.
    Version::parse(current).context("当前客户端版本号无效")?;
    ensure!(
        !suffix.is_empty()
            && suffix
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c)),
        "安装包平台无效"
    );
    let routes = crate::network::routes(port, None);
    retry_check(|| check_once_channel(current, suffix, &routes, API, include_prereleases)).await
}

#[cfg(test)]
async fn check_once(
    current: &str,
    suffix: &str,
    routes: &[crate::network::Route],
    endpoint: &str,
) -> Result<Option<Package>> {
    check_once_channel(current, suffix, routes, endpoint, false).await
}
async fn check_once_channel(
    current: &str,
    suffix: &str,
    routes: &[crate::network::Route],
    endpoint: &str,
    include_prereleases: bool,
) -> Result<Option<Package>> {
    let mut errors = Vec::new();
    for route in routes {
        let result = async {
            let bytes = bounded(
                client(route, Duration::from_secs(20))?
                    .get(endpoint)
                    .header(reqwest::header::CACHE_CONTROL, "no-cache")
                    .send()
                    .await?,
                2 * 1024 * 1024,
            )
            .await?;
            select_channel(
                serde_json::from_slice(&bytes)?,
                current,
                suffix,
                include_prereleases,
            )
        }
        .await;
        match result {
            Ok(p) => return Ok(p),
            Err(e) => errors.push(format!("{}：{e}", route.label)),
        }
    }
    bail!("检查更新失败：{}", errors.join("；"))
}
fn checksum(bytes: &[u8], name: &str) -> Result<String> {
    let text = std::str::from_utf8(bytes)?.trim();
    let words = text.split_whitespace().collect::<Vec<_>>();
    ensure!(
        !words.is_empty()
            && words.len() <= 2
            && words[0].len() == 64
            && words[0].bytes().all(|b| b.is_ascii_hexdigit())
            && (words.len() == 1 || words[1].trim_start_matches('*') == name),
        "更新校验文件格式无效"
    );
    Ok(words[0].to_ascii_lowercase())
}
pub async fn download(
    package: &Package,
    port: Option<u16>,
    path: &Path,
    api: Option<&crate::api::Api>,
    mut progress: impl FnMut(Progress),
) -> Result<()> {
    ensure!(package.size > 0 && package.size <= LIMIT, "安装包大小无效");
    let parent = path.parent().context("更新缓存路径无效")?;
    tokio::fs::create_dir_all(parent).await?;
    let partial = path.with_extension("partial");
    let mut errors = Vec::new();
    for route in crate::network::routes(port, None) {
        progress(Progress {
            total: package.size,
            route: route.label,
            ..Progress::default()
        });
        let result = async {
            let client = client(&route, Duration::from_secs(30 * 60))?;
            let expected = checksum(
                &bounded(client.get(&package.checksum_url).send().await?, 1024).await?,
                &package.name,
            )?;
            if let Some(digest) = &package.digest {
                ensure!(
                    digest == &format!("sha256:{expected}"),
                    "发布元数据与校验文件不一致"
                );
            }
            let mut response = client.get(&package.url).send().await?;
            ensure!(
                response.status().is_success(),
                "安装包下载返回 HTTP {}",
                response.status()
            );
            ensure!(
                response
                    .content_length()
                    .is_none_or(|size| size == package.size),
                "安装包大小与发布信息不一致"
            );
            let observer = RouteObserver::new(&response, api.cloned(), route.proxy.is_none());
            let file = tokio::fs::File::create(&partial).await?;
            let mut file = tokio::io::BufWriter::with_capacity(256 * 1024, file);
            let mut count = 0u64;
            let mut hash = Sha256::new();
            let mut last = std::time::Instant::now();
            let mut rate_started = last;
            let mut rate_received = 0;
            while let Some(chunk) = response.chunk().await? {
                count += chunk.len() as u64;
                ensure!(
                    count <= package.size && count <= LIMIT,
                    "安装包超过预期大小"
                );
                file.write_all(&chunk).await?;
                hash.update(&chunk);
                if count == chunk.len() as u64 || last.elapsed() >= Duration::from_millis(250) {
                    let elapsed = rate_started.elapsed().as_secs_f64();
                    progress(Progress {
                        received: count,
                        total: package.size,
                        bytes_per_second: ((count - rate_received) as f64 / elapsed.max(0.001))
                            as u64,
                        route: route.label,
                        outbound: observer.outbound(),
                    });
                    if elapsed >= 1.0 {
                        rate_started = std::time::Instant::now();
                        rate_received = count;
                    }
                    last = std::time::Instant::now();
                }
            }
            ensure!(
                count == package.size && format!("{:x}", hash.finalize()) == expected,
                "安装包大小或 SHA256 校验失败"
            );
            file.flush().await?;
            file.into_inner().sync_all().await?;
            tokio::fs::rename(&partial, path).await?;
            progress(Progress {
                received: count,
                total: package.size,
                bytes_per_second: ((count - rate_received) as f64
                    / rate_started.elapsed().as_secs_f64().max(0.001))
                    as u64,
                route: route.label,
                outbound: observer.outbound(),
            });
            Ok::<(), anyhow::Error>(())
        }
        .await;
        match result {
            Ok(()) => return Ok(()),
            Err(e) => {
                let _ = tokio::fs::remove_file(&partial).await;
                errors.push(format!("{}：{e}", route.label));
            }
        }
    }
    bail!("下载更新失败：{}", errors.join("；"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn buffered_download_flushes_verified_bytes_and_reports_the_route() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let body = vec![42; 128 * 1024 + 3];
        let expected = format!("{:x}", Sha256::digest(&body));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let sent = body.clone();
        let sidecar = expected.clone();
        let server = tokio::spawn(async move {
            for (suffix, bytes) in [(".sha256", sidecar.into_bytes()), ("", sent)] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 8192];
                let mut count = 0;
                while !request[..count].ends_with(b"\r\n\r\n") {
                    let n = socket.read(&mut request[count..]).await.unwrap();
                    assert!(n > 0);
                    count += n;
                }
                assert!(
                    String::from_utf8_lossy(&request[..count])
                        .starts_with(&format!("GET /update.apk{suffix} HTTP/1.1"))
                );
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                for part in bytes.chunks(1024) {
                    socket.write_all(part).await.unwrap();
                }
            }
        });
        let package = Package {
            version: "0.6.2".into(),
            preview: false,
            name: "update.apk".into(),
            size: body.len() as u64,
            url: format!("{base}/update.apk"),
            checksum_url: format!("{base}/update.apk.sha256"),
            digest: Some(format!("sha256:{expected}")),
        };
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("update.apk");
        let mut progress = Vec::new();
        download(&package, None, &path, None, |value| progress.push(value))
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), body);
        assert!(!path.with_extension("partial").exists());
        assert_eq!(progress.first().unwrap().received, 0);
        let last = progress.last().unwrap();
        assert_eq!(last.received, package.size);
        assert_eq!(last.route, "直连");
        assert!(last.bytes_per_second > 0);
        server.await.unwrap();
    }
    #[test]
    fn stable_updates_match_platform_and_require_a_checksum() {
        let data = serde_json::json!([
            {"tag_name":"v0.6.0","draft":false,"prerelease":true,"assets":[]},
            {"tag_name":"v0.5.13","draft":false,"prerelease":false,"assets":[
                {"name":"Clash-of-Rust-0.5.13-android-arm64.apk","size":42,"digest":null},
                {"name":"Clash-of-Rust-0.5.13-android-arm64.apk.sha256","size":64,"digest":null}]}
        ]);
        let parse = || serde_json::from_value::<Vec<Release>>(data.clone()).unwrap();
        assert_eq!(
            select(parse(), "0.5.12-dev.1", "android-arm64.apk")
                .unwrap()
                .unwrap()
                .version,
            "0.5.13"
        );
        // A newer stable release with incomplete assets is retryable, not
        // evidence that the installed client is already up to date.
        assert!(select(parse(), "0.5.12", "android-x64.apk").is_err());
        assert!(
            select(parse(), "0.5.13", "android-arm64.apk")
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn android_channels_offer_previews_and_return_to_the_newest_complete_stable_package() {
        let releases = serde_json::json!([
            {"tag_name":"v0.6.4-dev.2","draft":false,"prerelease":true,"assets":[
                {"name":"Clash-of-Rust-0.6.4-dev.2-android-arm64.apk","size":42},
                {"name":"Clash-of-Rust-0.6.4-dev.2-android-arm64.apk.sha256","size":64}]},
            {"tag_name":"v0.6.3","draft":false,"prerelease":false,"assets":[
                {"name":"Clash-of-Rust-0.6.3-android-arm64.apk","size":42},
                {"name":"Clash-of-Rust-0.6.3-android-arm64.apk.sha256","size":64}]}
        ]);
        let parse = || serde_json::from_value::<Vec<Release>>(releases.clone()).unwrap();
        let preview = select_channel(parse(), "0.6.3", "android-arm64.apk", true)
            .unwrap()
            .unwrap();
        assert_eq!(preview.version, "0.6.4-dev.2");
        assert!(preview.preview);
        let stable = select_channel(parse(), "0.6.4-dev.1", "android-arm64.apk", false)
            .unwrap()
            .unwrap();
        assert_eq!(stable.version, "0.6.3");
        assert!(!stable.preview);
        assert!(select_channel(parse(), "0.6.4-dev.1", "android-x64.apk", false).is_err());
        assert!(
            select_channel(parse(), "0.6.3", "android-arm64.apk", false)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn checksum_sidecars_accept_our_formats_but_not_another_filename() {
        let digest = "a".repeat(64);
        for text in [digest.clone(), format!("{digest}  update.apk\n")] {
            assert_eq!(checksum(text.as_bytes(), "update.apk").unwrap(), digest);
        }
        assert!(checksum(format!("{digest}  other.apk").as_bytes(), "update.apk").is_err());
        assert!(checksum(b"not a digest", "update.apk").is_err());
    }

    #[tokio::test]
    async fn check_refetches_after_http_errors_and_incomplete_newest_assets() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/releases", listener.local_addr().unwrap());
        let name = "Clash-of-Rust-0.6.3-android-arm64.apk";
        let package = serde_json::json!({"name":name,"size":42});
        let sidecar = serde_json::json!({"name":format!("{name}.sha256"),"size":64});
        let older = serde_json::json!({
            "tag_name":"v0.6.2","draft":false,"prerelease":false,"assets":[
                {"name":"Clash-of-Rust-0.6.2-android-arm64.apk","size":42},
                {"name":"Clash-of-Rust-0.6.2-android-arm64.apk.sha256","size":64}
            ]
        });
        let bodies = [vec![], vec![package.clone()], vec![package, sidecar]].map(|assets| {
            serde_json::json!([
                {"tag_name":"v0.6.3","draft":false,"prerelease":false,"assets":assets},
                older.clone()
            ])
            .to_string()
        });
        let server = tokio::spawn(async move {
            for (status, body) in [
                ("503 Service Unavailable", String::new()),
                ("200 OK", bodies[0].clone()),
                ("200 OK", bodies[1].clone()),
                ("200 OK", bodies[2].clone()),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = [0; 8192];
                let mut count = 0;
                while !bytes[..count].ends_with(b"\r\n\r\n") {
                    let n = socket.read(&mut bytes[count..]).await.unwrap();
                    assert!(n > 0);
                    count += n;
                }
                let request = String::from_utf8_lossy(&bytes[..count]).to_ascii_lowercase();
                assert!(request.starts_with("get /releases http/1.1"));
                assert!(request.contains("cache-control: no-cache"));
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let routes = crate::network::routes(None, None);
        let package = retry_check(|| check_once("0.6.1", "android-arm64.apk", &routes, &endpoint))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(package.version, "0.6.3");
        assert_eq!(package.name, name);
        assert_eq!(package.checksum_url, format!("{}.sha256", package.url));
        server.await.unwrap();
    }

    #[test]
    fn incomplete_newest_release_does_not_fall_back_to_an_older_installer() {
        let name = "Clash-of-Rust-0.6.3-android-arm64.apk";
        for assets in [
            serde_json::json!([]),
            serde_json::json!([{"name":name,"size":42}]),
            serde_json::json!([
                {"name":name,"size":42},
                {"name":name,"size":42},
                {"name":format!("{name}.sha256"),"size":64}
            ]),
        ] {
            let releases = serde_json::json!([
                {"tag_name":"v0.6.3","draft":false,"prerelease":false,"assets":assets},
                {"tag_name":"v0.6.2","draft":false,"prerelease":false,"assets":[
                    {"name":"Clash-of-Rust-0.6.2-android-arm64.apk","size":42},
                    {"name":"Clash-of-Rust-0.6.2-android-arm64.apk.sha256","size":64}
                ]}
            ]);
            let result = select(
                serde_json::from_value(releases).unwrap(),
                "0.6.1",
                "android-arm64.apk",
            );
            assert!(result.unwrap_err().to_string().contains("0.6.3"));
        }
    }
}
