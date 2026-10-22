//! Nonmodal OS notifications, shared by the logo and Claude protection.
use anyhow::{Context, Result, ensure};

#[derive(Debug)]
pub struct PermissionDenied;
impl std::fmt::Display for PermissionDenied {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("请在系统设置的通知中允许 Clash of Rust 发送通知")
    }
}
impl std::error::Error for PermissionDenied {}

#[derive(Debug, Clone, Copy)]
pub struct Preparation {
    pub allowed: bool,
    pub offer_settings: bool,
}

#[cfg(any(target_os = "macos", test))]
fn record_recovery(store: &crate::config::Store, identity: &[u8], offer: bool) -> Result<bool> {
    let marker = store.root.join("notification-permission.json");
    let seen = std::fs::read(&marker).ok().as_deref() == Some(identity);
    if !seen {
        crate::config::atomic_write(&marker, identity)?;
    }
    Ok(!seen && offer)
}

/// Inspect the current OS state on every launch, including post-update launches.
/// Reports authorization and whether this installation needs a settings guide.
pub async fn prepare_startup(store: &crate::config::Store) -> Result<Preparation> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::MetadataExt;
        let permission =
            tokio::time::timeout(std::time::Duration::from_secs(120), macos::permission())
                .await
                .context("等待 macOS 通知授权超时")??;
        let marker = store.root.join("notification-permission.json");
        if permission == macos::Permission::Allowed {
            if marker.exists() {
                std::fs::remove_file(&marker)?;
            }
            return Ok(Preparation {
                allowed: true,
                offer_settings: false,
            });
        }
        // A new bundle (even a reinstall of the same version) gets a fresh
        // recovery offer. User dismissal persists for this installation only.
        let executable = std::env::current_exe()?;
        let bundle = executable
            .ancestors()
            .find(|path| path.extension().is_some_and(|s| s == "app"))
            .context("macOS 通知授权需要应用程序包")?;
        let time = |value: std::io::Result<std::time::SystemTime>| {
            value
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos().to_string())
        };
        let bundle_meta = std::fs::metadata(bundle)?;
        let executable_meta = std::fs::metadata(&executable)?;
        let identity = serde_json::to_vec(&serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"), "bundle": bundle,
            "created": time(bundle_meta.created()), "modified": time(executable_meta.modified()),
            "bundle_inode": bundle_meta.ino(), "executable_inode": executable_meta.ino(),
        }))?;
        let offer_settings =
            record_recovery(store, &identity, permission == macos::Permission::Settings)?;
        Ok(Preparation {
            allowed: false,
            offer_settings,
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = store;
        initialize().await?;
        Ok(Preparation {
            allowed: true,
            offer_settings: false,
        })
    }
}

/// Explicit user action after the system has remembered a denial.
pub fn open_settings() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension")
            .status()
            .context("无法打开系统通知设置")?;
        ensure!(status.success(), "系统通知设置未打开");
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    anyhow::bail!("当前系统没有此通知设置入口")
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// Prepare native notification identity/permission without sending a toast.
/// Permission preparation does not send a test notification or block core startup.
pub async fn initialize() -> Result<()> {
    #[cfg(windows)]
    windows::initialize().await?;
    #[cfg(target_os = "macos")]
    tokio::time::timeout(std::time::Duration::from_secs(120), macos::initialize())
        .await
        .context("等待 macOS 通知授权超时")??;
    Ok(())
}

#[cfg(windows)]
pub fn prepare_process() -> Result<()> {
    windows::prepare_process()
}

pub async fn show(title: &str, message: &str) -> Result<()> {
    ensure!(
        title.len() <= 128
            && message.len() <= 2048
            && !title.contains('\0')
            && !message.contains('\0'),
        "系统通知内容无效"
    );
    #[cfg(target_os = "linux")]
    let send = linux::show(title, message);
    #[cfg(target_os = "macos")]
    let send = macos::show(title, message);
    #[cfg(windows)]
    let send = windows::show(title, message);
    tokio::time::timeout(std::time::Duration::from_secs(30), send)
        .await
        .context("系统通知服务响应超时")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_invalid_content_before_os_calls() {
        assert!(show("bad\0title", "text").await.is_err());
        assert!(show("title", &"x".repeat(2049)).await.is_err());
    }
    #[test]
    fn notification_recovery_is_renewed_for_installation_changes_without_reprompting_a_new_denial()
    {
        let directory = tempfile::tempdir().unwrap();
        let store = crate::config::Store::at(directory.path().to_owned()).unwrap();
        assert!(record_recovery(&store, b"install-one", true).unwrap());
        assert!(!record_recovery(&store, b"install-one", true).unwrap());
        assert!(record_recovery(&store, b"updated-bundle", true).unwrap());
        assert!(record_recovery(&store, b"same-version-reinstalled", true).unwrap());
        assert!(!record_recovery(&store, b"freshly-declined", false).unwrap());
        assert!(!record_recovery(&store, b"freshly-declined", true).unwrap());
    }
}
