//! Desktop bundle discovery and core executable validation.
use anyhow::{Context, Result, bail};
pub use clash_client_core::geodata::{
    GEO_FILES, GeoFile, GeoManifest, MANIFEST, atomic_copy, begin_install, commit, hash_file,
    record_install, recover, verify_geo,
};
use std::path::{Path, PathBuf};

pub async fn download_geo(
    destination: &Path,
    port: Option<crate::network::DownloadPorts>,
) -> Result<GeoManifest> {
    clash_client_core::geodata::download_geo_via(
        destination,
        &crate::network::available_routes(port, "https"),
    )
    .await
}

pub fn discover() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let sibling = exe.parent().context("无法定位应用目录")?.join("resources");
    if sibling.is_dir() {
        return Ok(sibling);
    }
    #[cfg(target_os = "macos")]
    if let Some(contents) = exe.parent().and_then(Path::parent) {
        let resources = contents.join("Resources/resources");
        if contents.file_name().is_some_and(|name| name == "Contents") && resources.is_dir() {
            return Ok(resources);
        }
    }
    #[cfg(debug_assertions)]
    {
        let development =
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(if cfg!(windows) {
                    if cfg!(target_arch = "aarch64") {
                        "bundle/windows-arm64/resources".to_owned()
                    } else {
                        "bundle/resources".to_owned()
                    }
                } else {
                    format!(
                        "bundle/{}-{}/resources",
                        if cfg!(target_os = "macos") {
                            "macos"
                        } else {
                            "linux"
                        },
                        if cfg!(target_arch = "aarch64") {
                            "arm64"
                        } else {
                            "x64"
                        }
                    )
                });
        if development.is_dir() {
            return Ok(development);
        }
    }
    bail!(
        "安装资源不完整：程序 {} 未找到资源目录 {}；请从完整安装目录运行，或用 --resources PATH 指定资源目录",
        exe.display(),
        sibling.display()
    )
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

pub fn seed(resources: &Path, runtime: &Path) -> Result<GeoManifest> {
    core_path(resources)?;
    clash_client_core::geodata::seed(resources, runtime)
}
