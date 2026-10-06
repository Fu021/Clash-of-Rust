//! Offline resources and transactional Geo updates, shared by Windows and Linux.
use crate::config::atomic_write;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

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

pub fn discover() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let sibling = exe.parent().context("无法定位应用目录")?.join("resources");
    if sibling.is_dir() {
        return Ok(sibling);
    }
    #[cfg(debug_assertions)]
    {
        let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("bundle/resources");
        if development.is_dir() {
            return Ok(development);
        }
    }
    bail!("安装资源不完整，请重新安装 Clash of Rust")
}

pub fn core_path(resources: &Path) -> Result<PathBuf> {
    let path = resources.join(if cfg!(windows) {
        "mihomo.exe"
    } else {
        "mihomo"
    });
    let path = std::fs::canonicalize(path).context("安装包内核缺失，请重新安装")?;
    if !path.is_file() {
        bail!("安装包内核不是有效文件");
    }
    Ok(path)
}

pub fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut size = 0;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((format!("{:x}", hash.finalize()), size))
}

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
    file.persist(destination).map_err(|e| e.error)?;
    Ok(())
}

pub fn seed(resources: &Path, runtime: &Path) -> Result<GeoManifest> {
    recover(runtime)?;
    core_path(resources)?;
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
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .user_agent(concat!("clash-of-rust/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180));
    if let Some(port) = proxy_port {
        builder = builder.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?);
    }
    let client = builder.build()?;
    let release: Release = client
        .get(RELEASE)
        .send()
        .await
        .context("Geo 发布信息获取失败")?
        .error_for_status()?
        .json()
        .await?;
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
        let mut response = client
            .get(url)
            .send()
            .await
            .with_context(|| format!("{local} 下载失败"))?
            .error_for_status()?;
        let mut file = File::create(destination.join(local))?;
        let mut hash = Sha256::new();
        let mut size = 0;
        while let Some(chunk) = response.chunk().await? {
            size += chunk.len() as u64;
            if size > LIMIT {
                bail!("Geo 数据超过大小限制");
            }
            hash.update(&chunk);
            file.write_all(&chunk)?;
        }
        file.sync_all()?;
        let actual = format!("{:x}", hash.finalize());
        if actual != digest.to_ascii_lowercase() || size != asset.size {
            bail!("{local} SHA256/大小校验失败，已有数据保持不变");
        }
        manifest.files.push(GeoFile {
            name: local.into(),
            source: asset.browser_download_url.clone(),
            sha256: actual,
            size,
        });
    }
    atomic_write(
        &destination.join(MANIFEST),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    verify_geo(destination)
}

#[derive(Serialize, Deserialize)]
struct Transaction {
    backup: String,
}
fn backup_path(runtime: &Path, journal: &Transaction) -> Result<PathBuf> {
    if !journal.backup.starts_with(".geo-backup-") || journal.backup.contains(['/', '\\', ':']) {
        bail!("Geo 恢复记录路径无效");
    }
    Ok(runtime.join(&journal.backup))
}

pub fn begin_install(staged: &Path, runtime: &Path) -> Result<()> {
    verify_geo(staged)?;
    verify_geo(runtime)?;
    if runtime.join(JOURNAL).exists() {
        bail!("存在尚未恢复的 Geo 更新事务");
    }
    let backup = tempfile::Builder::new()
        .prefix(".geo-backup-")
        .tempdir_in(runtime)?;
    for name in GEO_FILES.into_iter().chain([MANIFEST]) {
        atomic_copy(&runtime.join(name), &backup.path().join(name))?;
    }
    let path = backup.keep();
    let journal = Transaction {
        backup: path
            .file_name()
            .context("备份路径无效")?
            .to_string_lossy()
            .into_owned(),
    };
    atomic_write(&runtime.join(JOURNAL), &serde_json::to_vec(&journal)?)?;
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
    verify_geo(&backup).context("Geo 更新备份不完整，无法自动恢复")?;
    for name in GEO_FILES.into_iter().chain([MANIFEST]) {
        atomic_copy(&backup.join(name), &runtime.join(name))?;
    }
    commit(runtime)
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
