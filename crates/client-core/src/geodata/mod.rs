//! Portable Geo data validation, downloads and recoverable installation transactions.
use crate::config::{atomic_write, persist_file};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::AsyncWriteExt;

pub const GEO_FILES: [&str; 4] = ["GeoIP.dat", "GeoSite.dat", "Country.mmdb", "ASN.mmdb"];
pub const MANIFEST: &str = "geodata.json";
const JOURNAL: &str = "geo-transaction.json";
const LIMIT: u64 = 128 * 1024 * 1024;
const RELEASE: &str = "https://api.github.com/repos/MetaCubeX/meta-rules-dat/releases/latest";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoFile {
    pub name: String,
    pub source: String,
    pub sha256: String,
    pub size: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoManifest {
    pub version: String,
    pub updated: u64,
    pub files: Vec<GeoFile>,
}

pub use crate::file_hash::hash_file;

pub fn verify_geo(directory: &Path) -> Result<GeoManifest> {
    let manifest: GeoManifest = serde_json::from_slice(
        &std::fs::read(directory.join(MANIFEST)).context("Geo 数据清单缺失")?,
    )?;
    if manifest.files.len() != GEO_FILES.len() {
        bail!("Geo 数据清单不完整");
    }
    for name in GEO_FILES {
        let record = manifest
            .files
            .iter()
            .find(|f| f.name == name)
            .context("Geo 数据清单缺少必要文件")?;
        let (digest, size) =
            hash_file(&directory.join(name)).with_context(|| format!("Geo 数据 {name} 缺失"))?;
        if size == 0 || size > LIMIT || digest != record.sha256 || size != record.size {
            bail!("Geo 数据 {name} 校验失败");
        }
    }
    Ok(manifest)
}

pub fn atomic_copy(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination.parent().context("文件路径无效")?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    std::io::copy(&mut File::open(source)?, &mut file)?;
    file.as_file().sync_all()?;
    persist_file(file, destination)
}

pub fn seed(resources: &Path, runtime: &Path) -> Result<GeoManifest> {
    recover(runtime)?;
    // Preserve all four user-updated files as a set; only seed missing/invalid sets.
    if let Ok(manifest) = verify_geo(runtime) {
        return Ok(manifest);
    }
    verify_geo(resources).context("安装包离线 Geo 数据不完整，请重新安装")?;
    for name in GEO_FILES {
        atomic_copy(&resources.join(name), &runtime.join(name))?;
    }
    atomic_copy(&resources.join(MANIFEST), &runtime.join(MANIFEST))?;
    verify_geo(runtime)
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    published_at: String,
    assets: Vec<ReleaseAsset>,
}
#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    size: u64,
    digest: Option<String>,
    browser_download_url: String,
}

pub async fn download_geo(destination: &Path, proxy_port: Option<u16>) -> Result<GeoManifest> {
    let routes = crate::network::routes(proxy_port, None);
    let mut release = None;
    let mut errors = Vec::new();
    for route in &routes {
        match fetch_release(route).await {
            Ok(value) => {
                release = Some(value);
                break;
            }
            Err(error) => errors.push(format!("{}：{error:#}", route.label)),
        }
    }
    let release =
        release.with_context(|| format!("Geo 发布信息获取失败：{}", errors.join("；")))?;
    let mapping = [
        ("geoip.dat", "GeoIP.dat"),
        ("geosite.dat", "GeoSite.dat"),
        ("country.mmdb", "Country.mmdb"),
        ("GeoLite2-ASN.mmdb", "ASN.mmdb"),
    ];
    let mut manifest = GeoManifest {
        version: format!("{} · {}", release.tag_name, release.published_at),
        updated: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        files: vec![],
    };
    for (remote, local) in mapping {
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == remote)
            .with_context(|| format!("Geo 发布缺少 {remote}"))?;
        let digest = asset
            .digest
            .as_deref()
            .and_then(|v| v.strip_prefix("sha256:"))
            .context("Geo 发布缺少 SHA256 校验信息")?;
        if digest.len() != 64
            || !digest.chars().all(|c| c.is_ascii_hexdigit())
            || asset.size == 0
            || asset.size > LIMIT
        {
            bail!("Geo 数据校验信息无效");
        }
        let url = reqwest::Url::parse(&asset.browser_download_url)?;
        if url.scheme() != "https" || url.host_str() != Some("github.com") {
            bail!("Geo 下载源不是官方 HTTPS 发布");
        }
        download_asset(asset, &destination.join(local), &routes).await?;
        manifest.files.push(GeoFile {
            name: local.into(),
            source: asset.browser_download_url.clone(),
            sha256: digest.to_ascii_lowercase(),
            size: asset.size,
        });
    }
    atomic_write(
        &destination.join(MANIFEST),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    verify_geo(destination).context("新下载的 Geo 数据无效")
}

fn download_client(route: &crate::network::Route) -> Result<reqwest::Client> {
    Ok(route
        .apply(
            crate::network::client_builder()
                .user_agent(format!("clash-of-rust/{}", crate::VERSION))
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(30 * 60)),
        )?
        .build()?)
}

async fn fetch_release(route: &crate::network::Route) -> Result<Release> {
    let mut response = download_client(route)?
        .get(RELEASE)
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > (1024 * 1024usize).saturating_sub(bytes.len()) {
            bail!("Geo 发布信息超过大小限制");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}

async fn download_asset(
    asset: &ReleaseAsset,
    destination: &Path,
    routes: &[crate::network::Route],
) -> Result<()> {
    let mut errors = Vec::new();
    for (index, route) in routes.iter().enumerate() {
        let attempt = async {
            let mut response = download_client(route)?
                .get(&asset.browser_download_url)
                .send()
                .await?
                .error_for_status()?;
            if response
                .content_length()
                .is_some_and(|size| size != asset.size)
            {
                bail!("Geo 数据大小与发布信息不一致");
            }
            let temporary =
                tempfile::NamedTempFile::new_in(destination.parent().context("Geo 下载路径无效")?)?;
            let file = tokio::fs::File::from_std(temporary.reopen()?);
            let mut file =
                tokio::io::BufWriter::with_capacity(crate::download::BUFFER_CAPACITY, file);
            let mut watchdog = crate::download::Watchdog::new(index + 1 < routes.len());
            let mut hash = Sha256::new();
            let mut size = 0;
            while let Some(chunk) = watchdog.chunk(&mut response).await? {
                let chunk = chunk.as_ref();
                size += chunk.len() as u64;
                if size > asset.size || size > LIMIT {
                    bail!("Geo 数据超过预期大小");
                }
                hash.update(chunk);
                file.write_all(chunk).await?;
            }
            let actual = format!("sha256:{}", crate::file_hash::hex(hash.finalize()));
            if size != asset.size
                || !asset
                    .digest
                    .as_ref()
                    .is_some_and(|digest| digest.eq_ignore_ascii_case(&actual))
            {
                bail!("Geo 数据 SHA256/大小校验失败，已有数据保持不变");
            }
            file.flush().await?;
            file.into_inner().sync_all().await?;
            persist_file(temporary, destination)?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        match attempt {
            Ok(()) => return Ok(()),
            Err(error) => errors.push(format!("{}：{error:#}", route.label)),
        }
    }
    bail!("Geo 下载失败：{}", errors.join("；"))
}

#[derive(Serialize, Deserialize)]
struct Transaction {
    backup: String,
    #[serde(default)]
    files: Option<Vec<BackupFile>>,
}
#[derive(Serialize, Deserialize)]
struct BackupFile {
    name: String,
    contents: Option<BackupContents>,
}
#[derive(Serialize, Deserialize)]
struct BackupContents {
    sha256: String,
    size: u64,
}
fn backup_path(runtime: &Path, journal: &Transaction) -> Result<PathBuf> {
    if !journal.backup.starts_with(".geo-backup-") || journal.backup.contains(['/', '\\', ':']) {
        bail!("Geo 恢复记录路径无效");
    }
    Ok(runtime.join(&journal.backup))
}

pub fn begin_install(staged: &Path, runtime: &Path) -> Result<()> {
    verify_geo(staged).context("新下载的 Geo 数据无效")?;
    if runtime.join(JOURNAL).exists() {
        bail!("存在尚未恢复的 Geo 更新事务");
    }
    let backup = tempfile::Builder::new()
        .prefix(".geo-backup-")
        .tempdir_in(runtime)?;
    let mut files = Vec::new();
    for name in GEO_FILES.into_iter().chain([MANIFEST]) {
        let source = runtime.join(name);
        let contents = if source.try_exists()? {
            atomic_copy(&source, &backup.path().join(name))?;
            let (sha256, size) = hash_file(&backup.path().join(name))?;
            Some(BackupContents { sha256, size })
        } else {
            None
        };
        files.push(BackupFile {
            name: name.into(),
            contents,
        });
    }
    let journal = Transaction {
        backup: backup
            .path()
            .file_name()
            .context("备份路径无效")?
            .to_string_lossy()
            .into_owned(),
        files: Some(files),
    };
    atomic_write(&runtime.join(JOURNAL), &serde_json::to_vec(&journal)?)?;
    backup.keep();
    let result = (|| {
        for name in GEO_FILES.into_iter().chain([MANIFEST]) {
            atomic_copy(&staged.join(name), &runtime.join(name))?;
        }
        Ok::<_, anyhow::Error>(())
    })();
    if let Err(error) = result {
        recover(runtime)?;
        return Err(error);
    }
    Ok(())
}

pub fn commit(runtime: &Path) -> Result<()> {
    let journal: Transaction = serde_json::from_slice(&std::fs::read(runtime.join(JOURNAL))?)?;
    let backup = backup_path(runtime, &journal)?;
    std::fs::remove_file(runtime.join(JOURNAL))?;
    std::fs::remove_dir_all(backup)?;
    Ok(())
}

pub fn recover(runtime: &Path) -> Result<()> {
    let path = runtime.join(JOURNAL);
    if !path.exists() {
        return Ok(());
    }
    let journal: Transaction = serde_json::from_slice(&std::fs::read(&path)?)?;
    let backup = backup_path(runtime, &journal)?;
    if let Some(files) = &journal.files {
        let expected = GEO_FILES.into_iter().chain([MANIFEST]);
        if files.len() != GEO_FILES.len() + 1
            || !files
                .iter()
                .zip(expected)
                .all(|(file, name)| file.name == name)
        {
            bail!("Geo 更新备份记录无效，无法自动恢复");
        }
        for file in files {
            if let Some(contents) = &file.contents {
                let (sha256, size) = hash_file(&backup.join(&file.name))
                    .context("Geo 更新备份不完整，无法自动恢复")?;
                if sha256 != contents.sha256 || size != contents.size {
                    bail!("Geo 更新备份校验失败，无法自动恢复");
                }
            }
        }
        for file in files {
            let destination = runtime.join(&file.name);
            if file.contents.is_some() {
                atomic_copy(&backup.join(&file.name), &destination)?;
            } else if let Err(error) = std::fs::remove_file(destination)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(error.into());
            }
        }
    } else {
        // Journals from earlier versions used the original manifest for verification.
        verify_geo(&backup).context("Geo 更新备份不完整，无法自动恢复")?;
        for name in GEO_FILES.into_iter().chain([MANIFEST]) {
            atomic_copy(&backup.join(name), &runtime.join(name))?;
        }
    }
    commit(runtime)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn asset_server(bytes: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 8192];
            let mut count = 0;
            while !request[..count].ends_with(b"\r\n\r\n") {
                let n = socket.read(&mut request[count..]).await.unwrap();
                assert!(n > 0);
                count += n;
            }
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
            for chunk in bytes.chunks(1024) {
                socket.write_all(chunk).await.unwrap();
            }
        });
        (url, task)
    }

    #[tokio::test]
    async fn corrupt_route_preserves_old_file_and_buffered_fallback_installs_verified_bytes() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("GeoIP.dat");
        std::fs::write(&destination, b"old-data").unwrap();
        let body = vec![42; 128 * 1024 + 3];
        let (bad_url, bad) = asset_server(vec![43; body.len()]).await;
        let (url, good) = asset_server(body.clone()).await;
        let asset = ReleaseAsset {
            name: "geoip.dat".into(),
            size: body.len() as u64,
            digest: Some(format!(
                "sha256:{}",
                crate::file_hash::hex(Sha256::digest(&body))
            )),
            browser_download_url: url,
        };
        let proxy = crate::network::Route {
            label: "测试代理",
            proxy: Some(bad_url),
        };
        assert!(
            download_asset(&asset, &destination, std::slice::from_ref(&proxy))
                .await
                .is_err()
        );
        bad.await.unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"old-data");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);

        let (bad_url, bad) = asset_server(vec![43; body.len()]).await;
        let routes = [
            crate::network::Route {
                proxy: Some(bad_url),
                ..proxy
            },
            crate::network::Route {
                label: "直连",
                proxy: None,
            },
        ];
        download_asset(&asset, &destination, &routes).await.unwrap();
        bad.await.unwrap();
        good.await.unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), body);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    fn fixture(path: &Path, contents: &[u8]) {
        std::fs::create_dir_all(path).unwrap();
        let mut manifest = GeoManifest {
            version: "test".into(),
            updated: 1,
            files: vec![],
        };
        for name in GEO_FILES {
            std::fs::write(path.join(name), contents).unwrap();
            let (sha256, size) = hash_file(&path.join(name)).unwrap();
            manifest.files.push(GeoFile {
                name: name.into(),
                source: "test".into(),
                sha256,
                size,
            });
        }
        atomic_write(
            &path.join(MANIFEST),
            &serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn interrupted_update_restores_whole_set() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let staged = tmp.path().join("new");
        fixture(&old, b"old-data");
        fixture(&staged, b"new-data");
        begin_install(&staged, &old).unwrap();
        assert_eq!(std::fs::read(old.join("GeoSite.dat")).unwrap(), b"new-data");
        recover(&old).unwrap();
        for name in GEO_FILES {
            assert_eq!(std::fs::read(old.join(name)).unwrap(), b"old-data");
        }
        assert!(!old.join(JOURNAL).exists());
    }
    #[test]
    fn corrupted_download_cannot_replace_existing_data() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let staged = tmp.path().join("new");
        fixture(&old, b"old-data");
        fixture(&staged, b"new-data");
        std::fs::write(staged.join("GeoIP.dat"), b"corrupt").unwrap();
        assert!(begin_install(&staged, &old).is_err());
        assert_eq!(std::fs::read(old.join("GeoSite.dat")).unwrap(), b"old-data");
    }

    #[test]
    fn inconsistent_or_missing_old_data_can_be_replaced_and_restored_exactly() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let staged = tmp.path().join("new");
        fixture(&old, b"old-data");
        fixture(&staged, b"new-data");
        std::fs::write(old.join("GeoIP.dat"), b"old-modified").unwrap();
        std::fs::remove_file(old.join("ASN.mmdb")).unwrap();
        let original_manifest = std::fs::read(old.join(MANIFEST)).unwrap();
        assert!(verify_geo(&old).is_err());

        begin_install(&staged, &old).unwrap();
        assert_eq!(verify_geo(&old).unwrap().version, "test");
        recover(&old).unwrap();
        assert_eq!(
            std::fs::read(old.join("GeoIP.dat")).unwrap(),
            b"old-modified"
        );
        assert_eq!(std::fs::read(old.join("GeoSite.dat")).unwrap(), b"old-data");
        assert!(!old.join("ASN.mmdb").exists());
        assert_eq!(
            std::fs::read(old.join(MANIFEST)).unwrap(),
            original_manifest
        );

        begin_install(&staged, &old).unwrap();
        commit(&old).unwrap();
        assert_eq!(std::fs::read(old.join("GeoIP.dat")).unwrap(), b"new-data");
        assert!(verify_geo(&old).is_ok());
    }

    #[test]
    fn corrupted_backup_does_not_partially_restore_and_legacy_journal_still_works() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let staged = tmp.path().join("new");
        fixture(&old, b"old-data");
        fixture(&staged, b"new-data");
        begin_install(&staged, &old).unwrap();
        let journal: Transaction =
            serde_json::from_slice(&std::fs::read(old.join(JOURNAL)).unwrap()).unwrap();
        let backup = backup_path(&old, &journal).unwrap();
        std::fs::write(backup.join("GeoIP.dat"), b"damaged").unwrap();
        assert!(recover(&old).is_err());
        assert_eq!(std::fs::read(old.join("GeoSite.dat")).unwrap(), b"new-data");
        assert!(old.join(JOURNAL).exists());

        std::fs::write(backup.join("GeoIP.dat"), b"old-data").unwrap();
        let legacy = serde_json::json!({ "backup": journal.backup });
        atomic_write(&old.join(JOURNAL), &serde_json::to_vec(&legacy).unwrap()).unwrap();
        recover(&old).unwrap();
        assert_eq!(std::fs::read(old.join("GeoIP.dat")).unwrap(), b"old-data");
        assert!(verify_geo(&old).is_ok());
    }
}
