//! Nonmodal OS notifications, shared by the logo and Claude protection.
use anyhow::{Context, Result, ensure};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

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
}
