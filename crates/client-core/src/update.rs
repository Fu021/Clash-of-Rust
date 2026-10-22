//! Stable-release discovery and bounded, verified downloads for native hosts.
use anyhow::{Context, Result, bail, ensure};
use reqwest::Client;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
use tokio::io::AsyncWriteExt;
mod route;
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
fn select(releases: Vec<Release>, current: &str, suffix: &str) -> Result<Option<Package>> {
    let current = Version::parse(current)?;
    ensure!(
        suffix
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c)),
        "安装包平台无效"
    );
    let mut releases = releases
        .into_iter()
        .filter_map(|r| {
            let v = Version::parse(r.tag_name.trim_start_matches('v')).ok()?;
            (!r.draft && !r.prerelease && v.pre.is_empty() && v.cmp_precedence(&current).is_gt())
                .then_some((v, r))
        })
        .collect::<Vec<_>>();
    releases.sort_by(|(a, _), (b, _)| b.cmp_precedence(a));
    for (version, release) in releases {
        let name = format!("Clash-of-Rust-{version}-{suffix}");
        let matches = release
            .assets
            .iter()
            .filter(|a| a.name == name)
            .collect::<Vec<_>>();
        let sidecar = format!("{name}.sha256");
        if matches.len() != 1
            || release
                .assets
                .iter()
                .filter(|a| a.name == sidecar && a.size > 0 && a.size <= 1024)
                .count()
                != 1
        {
            continue;
        }
        let asset = matches[0];
        if asset.size == 0 || asset.size > LIMIT {
            continue;
        }
        let mut url = reqwest::Url::parse(RELEASES)?;
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("更新地址无效"))?
            .push("download")
            .push(&release.tag_name)
            .push(&name);
        return Ok(Some(Package {
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
    let mut errors = Vec::new();
    for route in crate::network::routes(port, None) {
        let result = async {
            let bytes = bounded(
                client(&route, Duration::from_secs(20))?
                    .get(API)
                    .send()
                    .await?,
                2 * 1024 * 1024,
            )
            .await?;
            select(serde_json::from_slice(&bytes)?, current, suffix)
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
        assert!(
            select(parse(), "0.5.12", "android-x64.apk")
                .unwrap()
                .is_none()
        );
        assert!(
            select(parse(), "0.5.13", "android-arm64.apk")
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
}
