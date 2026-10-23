//! Native macOS proxy preferences and per-user login agent. The GUI never runs as root.
use super::{ProxyState, ProxyValue};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    ptr,
};
mod authorization;
mod cf;
pub use authorization::{administrator, run as run_command, shell_quote};
use cf::{Owned, Ref};
pub mod tun;

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCopyProxies(store: Ref) -> Ref;
}
pub mod dns;
mod native_proxy;
pub mod service;
pub use native_proxy::read;
pub fn description() -> &'static str {
    "macOS 网络服务代理（HTTP / HTTPS / SOCKS）"
}
pub fn desired(original: &ProxyState, port: u16) -> Result<ProxyState> {
    original
        .iter()
        .map(|(id, value)| {
            let mut value: Value = match value {
                Some(ProxyValue::Text(text)) => serde_json::from_str(text)?,
                None => json!({}),
                _ => bail!("网络服务代理属性无效"),
            };
            ensure!(value.is_object(), "网络服务代理属性无效");
            for prefix in ["HTTP", "HTTPS", "SOCKS"] {
                value[format!("{prefix}Enable")] = 1.into();
                value[format!("{prefix}Proxy")] = "127.0.0.1".into();
                value[format!("{prefix}Port")] = port.into();
            }
            for key in ["ProxyAutoConfigEnable", "ProxyAutoDiscoveryEnable"] {
                value[key] = 0.into();
            }
            Ok((id.clone(), Some(ProxyValue::Text(value.to_string()))))
        })
        .collect()
}
pub fn write(state: &ProxyState) -> Result<()> {
    let current = read()?;
    if state
        .iter()
        .all(|(id, value)| current.get(id) == Some(value))
    {
        return Ok(());
    }
    if unsafe { libc::geteuid() } == 0 {
        return native_proxy::write(state);
    }
    service::write_proxy(state)
}
pub fn restore_write(state: &ProxyState) -> Result<()> {
    if unsafe { libc::geteuid() } == 0 {
        return native_proxy::write(state);
    }
    service::restore_proxy(state)
}
pub fn configured_proxy(scheme: &str) -> Result<Option<String>> {
    let proxies = Owned::new(unsafe { SCDynamicStoreCopyProxies(ptr::null()) })?;
    let value = cf::json(proxies.0)?;
    for (prefix, protocol) in [
        (if scheme == "https" { "HTTPS" } else { "HTTP" }, "http"),
        ("SOCKS", "socks5h"),
    ] {
        if value[format!("{prefix}Enable")].as_i64() != Some(1) {
            continue;
        }
        let Some(host) = value[format!("{prefix}Proxy")].as_str() else {
            continue;
        };
        let Some(port) = value[format!("{prefix}Port")]
            .as_u64()
            .filter(|p| (1..=65535).contains(p))
        else {
            continue;
        };
        let mut url = reqwest::Url::parse(&format!("{protocol}://localhost"))?;
        url.set_host(Some(host))
            .map_err(|_| anyhow::anyhow!("系统代理主机无效"))?;
        url.set_port(Some(port as u16))
            .map_err(|_| anyhow::anyhow!("系统代理端口无效"))?;
        return Ok(Some(url.into()));
    }
    Ok(None)
}
const LABEL: &str = "org.clashofrust.desktop";
fn agent_path() -> Result<PathBuf> {
    let home = directories::BaseDirs::new().context("无法定位用户目录")?;
    Ok(home
        .home_dir()
        .join(format!("Library/LaunchAgents/{LABEL}.plist")))
}
fn agent_text() -> Result<String> {
    let executable = crate::platform::autostart_executable()?;
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>Label</key><string>{LABEL}</string><key>ProgramArguments</key><array><string>{}</string><string>--background</string></array><key>RunAtLoad</key><true/><key>LimitLoadToSessionType</key><string>Aqua</string><key>ProcessType</key><string>Interactive</string></dict></plist>\n",
        cf::escape(&executable.to_string_lossy())
    ))
}
pub fn autostart_enabled() -> Result<bool> {
    let path = agent_path()?;
    if !path.exists() {
        return Ok(false);
    }
    Ok(std::fs::read_to_string(path)? == agent_text()?)
}
pub fn set_autostart(enabled: bool) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } != 0,
        "开机自启应在普通用户会话设置"
    );
    let path = agent_path()?;
    if enabled {
        super::super::config::atomic_write(&path, agent_text()?.as_bytes())?;
    } else if path.exists() {
        std::fs::remove_file(path)?;
        // The plist determines startup at the next login. Booting out this
        // LaunchAgent can terminate the very GUI changing its own setting.
    }
    Ok(())
}
pub fn open_directory(path: &Path) -> Result<()> {
    open(path.as_os_str())
}
pub fn open_url(url: &str) -> Result<()> {
    open(std::ffi::OsStr::new(url))
}
fn open(value: &std::ffi::OsStr) -> Result<()> {
    ensure!(
        Command::new("/usr/bin/open")
            .arg("--")
            .arg(value)
            .status()?
            .success(),
        "macOS 无法打开目标"
    );
    Ok(())
}
pub fn show_requested() -> bool {
    crate::config::Store::discover()
        .ok()
        .is_some_and(|store| std::fs::remove_file(store.root.join("show-window")).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_round_trip_preserves_pac_and_exceptions() {
        let original = ProxyState::from([("service".into(), Some(ProxyValue::Text(json!({"ExceptionsList":["*.local"],"ProxyAutoConfigEnable":1,"ProxyAutoConfigURLString":"https://example.com/proxy.pac","HTTPPort":80}).to_string())))]);
        let expected = desired(&original, 7890).unwrap();
        let Some(ProxyValue::Text(text)) = &expected["service"] else {
            panic!()
        };
        let value: Value = serde_json::from_str(text).unwrap();
        for prefix in ["HTTP", "HTTPS", "SOCKS"] {
            assert_eq!(value[format!("{prefix}Enable")], 1);
            assert_eq!(value[format!("{prefix}Proxy")], "127.0.0.1");
            assert_eq!(value[format!("{prefix}Port")], 7890);
        }
        assert_eq!(value["ProxyAutoConfigEnable"], 0);
        assert_eq!(value["ExceptionsList"], json!(["*.local"]));
        let native = cf::plist(&value).unwrap();
        assert_eq!(cf::json(native.0).unwrap(), value);
    }
}
