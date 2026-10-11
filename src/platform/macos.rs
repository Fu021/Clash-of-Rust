//! Native macOS proxy preferences and per-user login agent. The GUI never runs as root.
use super::{ProxyState, ProxyValue};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    ptr,
};
mod cf;
use cf::{Owned, Ref};
pub mod tun;

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCPreferencesCreate(a: Ref, name: Ref, id: Ref) -> Ref;
    fn SCPreferencesCreateWithAuthorization(a: Ref, name: Ref, id: Ref, auth: Ref) -> Ref;
    fn SCPreferencesPathGetValue(prefs: Ref, path: Ref) -> Ref;
    fn SCPreferencesPathSetValue(prefs: Ref, path: Ref, value: Ref) -> u8;
    fn SCPreferencesPathRemoveValue(prefs: Ref, path: Ref) -> u8;
    fn SCPreferencesLock(prefs: Ref, wait: u8) -> u8;
    fn SCPreferencesUnlock(prefs: Ref) -> u8;
    fn SCPreferencesCommitChanges(prefs: Ref) -> u8;
    fn SCPreferencesApplyChanges(prefs: Ref) -> u8;
    fn SCDynamicStoreCopyProxies(store: Ref) -> Ref;
    fn SCError() -> i32;
}
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn AuthorizationCreate(rights: Ref, environment: Ref, flags: u32, auth: *mut Ref) -> i32;
    fn AuthorizationFree(auth: Ref, flags: u32) -> i32;
}
struct Authorization(Ref);
impl Drop for Authorization {
    fn drop(&mut self) {
        unsafe { AuthorizationFree(self.0, 0) };
    }
}
fn preferences(auth: Option<&Authorization>) -> Result<Owned> {
    let name = cf::string("Clash of Rust")?;
    Owned::new(unsafe {
        match auth {
            Some(auth) => {
                SCPreferencesCreateWithAuthorization(ptr::null(), name.0, ptr::null(), auth.0)
            }
            None => SCPreferencesCreate(ptr::null(), name.0, ptr::null()),
        }
    })
}
fn sc_check(ok: u8, action: &str) -> Result<()> {
    ensure!(
        ok != 0,
        "{action}失败（macOS SystemConfiguration {}），请检查授权与网络服务",
        unsafe { SCError() }
    );
    Ok(())
}
pub fn description() -> &'static str {
    "macOS 网络服务代理（HTTP / HTTPS / SOCKS）"
}
pub fn read() -> Result<ProxyState> {
    let prefs = preferences(None)?;
    let key = cf::string("/NetworkServices")?;
    let value = unsafe { SCPreferencesPathGetValue(prefs.0, key.0) };
    ensure!(!value.is_null(), "没有可配置的 macOS 网络服务");
    let mut state = ProxyState::new();
    for id in cf::dictionary_keys(value)? {
        let interface = cf::string(&format!("/NetworkServices/{id}/Interface"))?;
        if unsafe { SCPreferencesPathGetValue(prefs.0, interface.0) }.is_null() {
            continue;
        }
        let path = cf::string(&format!("/NetworkServices/{id}/Proxies"))?;
        let value = unsafe { SCPreferencesPathGetValue(prefs.0, path.0) };
        let proxy = if value.is_null() {
            None
        } else {
            Some(ProxyValue::Text(cf::json(value)?.to_string()))
        };
        state.insert(id, proxy);
    }
    ensure!(!state.is_empty(), "没有可配置的 macOS 网络服务");
    Ok(state)
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
    let mut auth = ptr::null();
    ensure!(
        unsafe { AuthorizationCreate(ptr::null(), ptr::null(), 0, &mut auth) } == 0,
        "无法创建系统代理授权"
    );
    let auth = Authorization(auth);
    let prefs = preferences(Some(&auth))?;
    sc_check(unsafe { SCPreferencesLock(prefs.0, 1) }, "锁定网络设置")?;
    let result = (|| {
        for (id, value) in state {
            ensure!(
                !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "网络服务标识无效"
            );
            let service = cf::string(&format!("/NetworkServices/{id}"))?;
            ensure!(
                !unsafe { SCPreferencesPathGetValue(prefs.0, service.0) }.is_null(),
                "网络服务已发生变化，请重新启用系统代理"
            );
            let path = cf::string(&format!("/NetworkServices/{id}/Proxies"))?;
            match value {
                Some(ProxyValue::Text(text)) => {
                    let value = cf::plist(&serde_json::from_str::<Value>(text)?)?;
                    sc_check(
                        unsafe { SCPreferencesPathSetValue(prefs.0, path.0, value.0) },
                        "写入网络服务代理",
                    )?;
                }
                None => {
                    if !unsafe { SCPreferencesPathGetValue(prefs.0, path.0) }.is_null() {
                        sc_check(
                            unsafe { SCPreferencesPathRemoveValue(prefs.0, path.0) },
                            "恢复网络服务代理",
                        )?;
                    }
                }
                _ => bail!("网络服务代理属性无效"),
            }
        }
        sc_check(
            unsafe { SCPreferencesCommitChanges(prefs.0) },
            "保存网络设置",
        )?;
        sc_check(
            unsafe { SCPreferencesApplyChanges(prefs.0) },
            "应用网络设置",
        )
    })();
    unsafe { SCPreferencesUnlock(prefs.0) };
    result
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
        let _ = Command::new("/bin/launchctl")
            .args([
                "bootout",
                &format!("gui/{}/{LABEL}", unsafe { libc::getuid() }),
            ])
            .output();
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

/// Quote separately for the shell and the AppleScript string literal.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
pub fn administrator(script: &str) -> Result<()> {
    let literal = serde_json::to_string(script)?;
    let output = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            &format!("do shell script {literal} with administrator privileges"),
        ])
        .output()?;
    ensure!(
        output.status.success(),
        "管理员授权失败或已取消：{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
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
