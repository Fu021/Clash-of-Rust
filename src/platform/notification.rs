//! Native local alerts. No elevation, shell interpolation or account services.
use anyhow::{Context, Result, ensure};

pub async fn show(title: &str, message: &str) -> Result<()> {
    ensure!(
        title.len() <= 128
            && message.len() <= 2048
            && !title.contains('\0')
            && !message.contains('\0'),
        "系统通知内容无效"
    );
    #[cfg(target_os = "linux")]
    {
        let message = message
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let connection = zbus::Connection::session()
            .await
            .context("无法连接桌面通知服务")?;
        let proxy = zbus::Proxy::new(
            &connection,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        )
        .await?;
        let hints = std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::from([(
            "urgency",
            zbus::zvariant::Value::from(2u8),
        )]);
        let _: u32 = proxy
            .call(
                "Notify",
                &(
                    "Clash of Rust",
                    0u32,
                    "dialog-warning",
                    title,
                    message.as_str(),
                    Vec::<&str>::new(),
                    hints,
                    15000i32,
                ),
            )
            .await
            .context("系统未接受桌面通知")?;
    }
    #[cfg(target_os = "macos")]
    {
        let title = title.to_owned();
        let message = message.to_owned();
        let status = tokio::process::Command::new("/usr/bin/osascript")
            .args(["-e", "on run argv\n display dialog (item 2 of argv) with title (item 1 of argv) buttons {\"知道了\"} default button 1 with icon caution giving up after 15\nend run"])
            .arg(title).arg(message).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .kill_on_drop(true).status().await.context("无法调用 macOS 系统提示框")?;
        ensure!(status.success(), "macOS 未接受系统提示框，请检查桌面会话");
    }
    #[cfg(windows)]
    {
        struct Dismiss(Vec<u16>, bool);
        impl Drop for Dismiss {
            fn drop(&mut self) {
                if !self.1 {
                    return;
                }
                let title = self.0.clone();
                let _ = std::thread::Builder::new()
                    .name("claude-alert-close".into())
                    .spawn(move || {
                        use windows_sys::Win32::UI::WindowsAndMessaging::{
                            FindWindowW, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
                        };
                        // Check ownership before closing a native dialog, and allow
                        // creation to finish when cancellation races its UI thread.
                        for _ in 0..40 {
                            let window = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
                            let mut pid = 0;
                            if !window.is_null() {
                                unsafe { GetWindowThreadProcessId(window, &mut pid) };
                            }
                            if pid == std::process::id() {
                                unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }
                    });
            }
        }
        let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        let mut dismiss = Dismiss(title.clone(), true);
        let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        // A detached ordinary thread avoids keeping Tokio's runtime shutdown
        // waiting for a modal MessageBox to be acknowledged. The guard keeps
        // at most one alert active and continues closing offending connections.
        std::thread::Builder::new()
            .name("claude-alert".into())
            .spawn(move || {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MessageBoxW,
                };
                let result = unsafe {
                    MessageBoxW(
                        std::ptr::null_mut(),
                        message.as_ptr(),
                        title.as_ptr(),
                        MB_OK | MB_ICONWARNING | MB_SETFOREGROUND,
                    )
                };
                let _ = sender.send(result != 0);
            })?;
        ensure!(
            receiver.await.context("系统提示框未完成")?,
            "Windows 无法显示系统提示框"
        );
        dismiss.1 = false;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_invalid_content_before_os_calls() {
        assert!(show("bad\0title", "text").await.is_err());
        assert!(show("title", &"x".repeat(2049)).await.is_err());
    }
}
