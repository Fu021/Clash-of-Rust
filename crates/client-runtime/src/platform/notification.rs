//! Nonmodal OS notifications, shared by the logo and Claude protection.
use anyhow::{Context, Result, ensure};
use clash_client_core::config::Language;
use std::borrow::Cow;

const AUTHORIZATION_TIMEOUT: &str = "等待 macOS 通知授权超时";
const BACKEND_TIMEOUT: &str = "系统通知服务响应超时";

pub fn localize_error(message: &str, language: Language) -> Cow<'_, str> {
    if language != Language::English {
        return Cow::Borrowed(message);
    }
    let mut rest = message;
    let mut translated = String::new();
    loop {
        let (summary, cause) = rest
            .split_once(": ")
            .map_or((rest, None), |(summary, cause)| (summary, Some(cause)));
        let Some(label) = english_context(summary) else {
            if translated.is_empty() {
                return Cow::Borrowed(message);
            }
            translated.push_str(rest);
            return Cow::Owned(translated);
        };
        let Some(cause) = cause else {
            if translated.is_empty() {
                return Cow::Borrowed(label);
            }
            translated.push_str(label);
            return Cow::Owned(translated);
        };
        translated.push_str(label);
        translated.push_str(": ");
        rest = cause;
    }
}

fn english_context(summary: &str) -> Option<&'static str> {
    Some(match summary {
        AUTHORIZATION_TIMEOUT => "Waiting for macOS notification authorization timed out",
        BACKEND_TIMEOUT => "The system notification service timed out",
        "系统通知内容无效" => "Invalid system notification",
        "macOS 通知授权需要应用程序包" => {
            "macOS notification authorization requires an app bundle"
        }
        "无法打开系统通知设置" => "Could not open system notification settings",
        "系统通知设置未打开" => "System notification settings did not open",
        "当前系统没有此通知设置入口" => {
            "This platform has no shortcut to notification settings"
        }
        "无法连接桌面通知会话" => "Could not connect to the desktop notification session",
        "桌面未接受系统通知，请检查通知权限" => {
            "The desktop did not accept the notification; check notification permissions"
        }
        "macOS 系统通知需要从 Clash of Rust.app 启动" => {
            "Launch Clash of Rust.app to use macOS notifications"
        }
        "macOS 通知设置查询未完成" => {
            "The macOS notification settings query did not complete"
        }
        "macOS 通知授权未完成" => "macOS notification authorization did not complete",
        "macOS 通知提交未完成" => "macOS notification delivery did not complete",
        "无法注册 Windows 通知应用" => "Could not register the Windows notification app",
        "无法注册应用通知身份" => "Could not register the app's notification identity",
        "无法定位应用通知目录" => "Could not locate the app notification directory",
        "无法定位当前用户开始菜单" => "Could not locate the current user's Start menu",
        "无法分配 Windows 通知标识" => {
            "Could not allocate the Windows notification identity"
        }
        "Windows 无法创建应用通知发送器" => {
            "Windows could not create the app notification sender"
        }
        "Windows 未接受通知" => "Windows did not accept the notification",
        "Windows 通知发送失败" => "Windows notification delivery failed",
        "Windows 无法读取应用通知设置" => {
            "Windows could not read the app notification settings"
        }
        "Windows 已禁用应用通知，请在系统设置中允许 Clash of Rust 发送通知" => {
            "Windows has disabled app notifications; allow Clash of Rust to send notifications in system settings"
        }
        _ => return None,
    })
}

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
                .context(AUTHORIZATION_TIMEOUT)??;
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
        .context(AUTHORIZATION_TIMEOUT)??;
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
        .context(BACKEND_TIMEOUT)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn timeout_errors_localize_for_display_without_changing_the_error_chain() {
        for (context, english) in [
            (
                AUTHORIZATION_TIMEOUT,
                "Waiting for macOS notification authorization timed out",
            ),
            (BACKEND_TIMEOUT, "The system notification service timed out"),
        ] {
            let error =
                tokio::time::timeout(std::time::Duration::ZERO, std::future::pending::<()>())
                    .await
                    .context(context)
                    .unwrap_err();
            let message = format!("{error:#}");
            assert_eq!(error.to_string(), context);
            let cause = error.downcast_ref::<tokio::time::error::Elapsed>().unwrap();
            assert_eq!(message, format!("{context}: {cause}"));
            assert_eq!(
                localize_error(&message, Language::English),
                format!("{english}: {cause}")
            );
            assert_eq!(
                localize_error(&message, Language::SimplifiedChinese),
                message
            );
        }
        let external = "桌面外部错误: native detail";
        assert_eq!(localize_error(external, Language::English), external);
    }

    #[test]
    fn translates_client_frames_but_never_descends_into_a_native_message() {
        let native = "原生错误 0x80070005: 无法打开系统通知设置: 设备:\n原始细节";
        let error = anyhow::anyhow!(native)
            .context("无法分配 Windows 通知标识")
            .context("无法注册应用通知身份");
        let message = format!("{error:#}");
        assert_eq!(
            localize_error(&message, Language::English),
            format!(
                "Could not register the app's notification identity: Could not allocate the Windows notification identity: {native}"
            )
        );
        assert_eq!(
            localize_error(&message, Language::SimplifiedChinese),
            message
        );
        assert_eq!(error.root_cause().to_string(), native);

        assert_eq!(
            localize_error(
                "无法注册 Windows 通知应用: 无法定位应用通知目录",
                Language::English,
            ),
            "Could not register the Windows notification app: Could not locate the app notification directory"
        );
        assert_eq!(
            localize_error(
                "无法连接桌面通知会话: org.freedesktop.DBus.Error.NoReply",
                Language::English
            ),
            "Could not connect to the desktop notification session: org.freedesktop.DBus.Error.NoReply"
        );
    }

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
