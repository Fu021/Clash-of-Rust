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
    let executable = std::env::current_exe()?;
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
        assert_eq!(value["HTTPPort"], 7890);
        assert_eq!(value["ProxyAutoConfigEnable"], 0);
        assert_eq!(value["ExceptionsList"], json!(["*.local"]));
        let native = cf::plist(&value).unwrap();
        assert_eq!(cf::json(native.0).unwrap(), value);
    }
    #[test]
    #[ignore = "changes proxy settings only on a disposable macOS Actions runner"]
    fn native_proxy_restore_round_trip() {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
        assert!(crate::platform::is_elevated());
        let temp = tempfile::tempdir().unwrap();
        let journal = temp.path().join("proxy.json");
        let original = read().unwrap();
        // Restore even if an assertion fails, before allowing the panic out.
        let result = std::panic::catch_unwind(|| {
            crate::platform::enable(&journal, 17890).unwrap();
            assert!(crate::platform::is_owned(&journal).unwrap());
            assert!(crate::platform::restore(&journal).unwrap());
            assert_eq!(read().unwrap(), original);
            let store = crate::config::Store::at(temp.path().join("startup")).unwrap();
            let startup_journal = store.root.join("system-proxy.json");
            let settings = crate::config::Settings {
                mixed_port: 17890,
                proxy_mode: crate::proxy::ProxyMode::System,
                ..Default::default()
            };
            store.save_settings(&settings).unwrap();
            crate::platform::enable(&startup_journal, settings.mixed_port).unwrap();
            let recorded = std::fs::read(&startup_journal).unwrap();
            let configured = read().unwrap();
            assert!(crate::platform::is_owned_at_port(&startup_journal, 17890).unwrap());
            assert!(!crate::platform::is_owned_at_port(&startup_journal, 17891).unwrap());
            let resources = PathBuf::from(std::env::var_os("MIHOMO_TEST_RESOURCES").unwrap());
            let mut engine = crate::engine::Engine::with_resources(store, resources).unwrap();
            assert_eq!(std::fs::read(&startup_journal).unwrap(), recorded);
            assert_eq!(read().unwrap(), configured);
            assert!(crate::platform::is_owned(&startup_journal).unwrap());
            let occupied = std::net::TcpListener::bind(("127.0.0.1", settings.mixed_port)).unwrap();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            assert!(runtime.block_on(engine.start()).is_err());
            assert!(
                !startup_journal.exists(),
                "failed startup must restore the previous OS proxy"
            );
            assert_eq!(read().unwrap(), original);
            drop(occupied);
            drop(engine);
            assert!(!startup_journal.exists());
            assert_eq!(read().unwrap(), original);
            crate::platform::enable(&journal, 17891).unwrap();
            let later = desired(&original, 17892).unwrap();
            write(&later).unwrap();
            assert!(!crate::platform::restore(&journal).unwrap());
            assert_eq!(read().unwrap(), later);
        });
        write(&original).unwrap();
        result.unwrap();
    }
    #[test]
    #[ignore = "temporarily changes only this runner user's login agent"]
    fn native_autostart_round_trip() {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
        let path = agent_path().unwrap();
        assert!(!path.exists(), "do not overwrite an existing login agent");
        let result = std::panic::catch_unwind(|| {
            set_autostart(true).unwrap();
            assert!(autostart_enabled().unwrap());
            set_autostart(false).unwrap();
            assert!(!autostart_enabled().unwrap());
        });
        let _ = std::fs::remove_file(path);
        result.unwrap();
    }
    #[test]
    #[ignore = "reads actual macOS networking without changing preferences"]
    fn native_network_services_can_be_read() {
        assert!(!read().unwrap().is_empty());
        configured_proxy("https").unwrap();
    }
}
