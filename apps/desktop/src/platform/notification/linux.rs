use anyhow::{Context, Result};
use std::collections::HashMap;
use tokio::sync::Mutex;
use zbus::zvariant::Value;

// GNOME tracks the sender's unique bus name. Disconnecting immediately after
// Notify can destroy its notification source before the banner is presented.
static CONNECTION: Mutex<Option<zbus::Connection>> = Mutex::const_new(None);

pub(super) async fn show(title: &str, message: &str) -> Result<()> {
    let mut retained = CONNECTION.lock().await;
    if retained.as_ref().is_none_or(zbus::Connection::is_closed) {
        *retained = Some(
            zbus::Connection::session()
                .await
                .context("无法连接桌面通知会话")?,
        );
    }
    let connection = retained.as_ref().expect("connected notification session");
    let proxy = zbus::Proxy::new(
        connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await?;
    let hints = HashMap::from([
        ("desktop-entry", Value::from("clash-of-rust")),
        ("urgency", Value::from(1u8)),
        ("resident", Value::from(false)),
    ]);
    // The body supports markup; encode external host/node text as literal text.
    let body = escape(message);
    let _: u32 = proxy
        .call(
            "Notify",
            &(
                "Clash of Rust",
                0u32,
                "clash-of-rust",
                title,
                body,
                Vec::<&str>::new(),
                hints,
                -1i32,
            ),
        )
        .await
        .context("桌面未接受系统通知，请检查通知权限")?;
    Ok(())
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
#[cfg(test)]
mod tests {
    #[test]
    fn external_names_remain_literal_notification_text() {
        assert_eq!(super::escape("a<b>&c\n下一行"), "a&lt;b&gt;&amp;c\n下一行");
    }
}
