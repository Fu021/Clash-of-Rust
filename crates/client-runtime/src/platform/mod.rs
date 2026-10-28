//! OS-specific integration stays behind this module. Linux GUI/core code is identical.
use crate::config::atomic_write;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub mod memory;
pub mod notification;
mod proxy_state;
pub use proxy_state::{ProxyState, ProxyValue};
// Exercise the Mac preference semantics on all desktop CI hosts as pure logic.
#[cfg(all(test, not(target_os = "macos")))]
#[path = "macos/proxy_preferences.rs"]
mod macos_proxy_preferences;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    original: ProxyState,
    owned: ProxyState,
}

#[cfg(target_os = "linux")]
#[path = "linux/desktop.rs"]
mod desktop;
#[cfg(target_os = "linux")]
#[path = "linux/gio.rs"]
mod gio;
#[cfg(target_os = "linux")]
#[path = "linux/kde.rs"]
mod kde;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
#[path = "linux/tun/mod.rs"]
mod linux_tun;
#[cfg(windows)]
#[path = "windows/autostart.rs"]
mod windows_autostart;
#[cfg(target_os = "linux")]
pub use linux_tun::{
    authorize_tun, authorize_tun_for_user, authorize_tun_terminal, check_tun_environment,
    core_has_tun_permissions, core_launcher, tun_helper_main,
};
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
#[path = "windows/process.rs"]
mod windows_process;
#[cfg(windows)]
pub use windows_process::{BackgroundChild, spawn as spawn_background_console};
#[cfg(windows)]
#[path = "windows/service/mod.rs"]
pub mod windows_service;
#[cfg(target_os = "linux")]
use linux as native;
#[cfg(target_os = "macos")]
use macos as native;
#[cfg(windows)]
use windows as native;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod native {
    use super::*;
    pub fn read() -> Result<ProxyState> {
        bail!("当前平台尚未实现系统代理")
    }
    pub fn desired(_: u16) -> ProxyState {
        ProxyState::new()
    }
    pub fn write(_: &ProxyState) -> Result<()> {
        bail!("当前平台尚未实现系统代理")
    }
    pub fn description() -> &'static str {
        "当前平台系统代理暂不支持"
    }
}

pub fn description() -> &'static str {
    native::description()
}

/// Configure Linux GUI integration before any other initialization.
///
/// # Safety
/// Must be called before the process starts threads or libraries that read
/// environment variables concurrently.
#[cfg(target_os = "linux")]
pub unsafe fn initialize_desktop() {
    unsafe { desktop::initialize() }
}

#[cfg(target_os = "linux")]
pub fn ensure_tray_available() -> Result<()> {
    gio::ensure_tray_available()
}
#[cfg(windows)]
pub fn ensure_tray_available() -> Result<()> {
    windows::ensure_tray_available()
}

/// A session-wide marker prevents duplicate GUI instances and coordinates shutdown.
#[cfg(windows)]
pub struct ApplicationGuard(*mut std::ffi::c_void);
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(
        attributes: *mut std::ffi::c_void,
        owner: i32,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> *mut std::ffi::c_void;
    fn GetLastError() -> u32;
}
#[cfg(windows)]
pub fn application_guard() -> Result<Option<ApplicationGuard>> {
    let name: Vec<u16> = "Local\\ClashOfRust.Desktop"
        .encode_utf16()
        .chain([0])
        .collect();
    let handle = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
    if handle.is_null() {
        if unsafe { GetLastError() } == 5 {
            // CreateMutex requests full access, which a normal launch may not
            // have on the elevated TUN instance's mutex. Read access suffices.
            let existing = unsafe { OpenMutexW(0x100000, 0, name.as_ptr()) };
            if !existing.is_null() {
                unsafe {
                    CloseHandle(existing);
                }
                return Ok(None);
            }
        }
        return Err(std::io::Error::last_os_error()).context("创建桌面单实例锁失败");
    }
    if unsafe { GetLastError() } == 183 {
        unsafe {
            CloseHandle(handle);
        }
        return Ok(None);
    }
    let guard = ApplicationGuard(handle);
    windows::initialize_events().context("初始化桌面控制事件失败")?;
    Ok(Some(guard))
}
#[cfg(windows)]
impl Drop for ApplicationGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
#[cfg(not(windows))]
pub fn application_guard() {}

#[cfg(all(test, windows))]
mod application_guard_tests {
    use super::*;

    #[test]
    fn ordinary_user_can_initialize_desktop_control() {
        let guard = application_guard()
            .unwrap_or_else(|error| panic!("{error:#}"))
            .expect("test desktop must not have an existing GUI");
        assert!(application_guard().unwrap().is_none());
        drop(guard);
    }
}

pub fn autostart_enabled() -> Result<bool> {
    native::autostart_enabled()
}
#[cfg(windows)]
pub fn headless_startup(
    data: &Path,
    resources: Option<&Path>,
    enabled: Option<bool>,
) -> Result<bool> {
    let backend = windows_autostart::Backend::headless(data, resources)?;
    if let Some(enabled) = enabled {
        backend.set(enabled)?;
    }
    backend.enabled()
}
/// The ordinary autostart/update target remains the desktop executable even
/// when requested from its sibling console program. Headless startup is separate.
pub fn autostart_executable() -> Result<std::path::PathBuf> {
    let executable = std::env::current_exe()?;
    if executable.file_stem().is_some_and(|name| name == "cor") {
        let gui = executable.with_file_name(if cfg!(windows) {
            "clash-of-rust.exe"
        } else {
            "clash-of-rust"
        });
        if gui.is_file() {
            return Ok(gui);
        }
    }
    Ok(executable)
}
pub fn set_autostart(enabled: bool) -> Result<()> {
    native::set_autostart(enabled)
}
pub fn open_directory(path: &Path) -> Result<()> {
    native::open_directory(path)
}
pub fn open_url(url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url)?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() || url.contains('\0') {
        bail!("网页地址无效");
    }
    native::open_url(url)
}
/// Only the short-lived region diagnostic receiver may open an HTTP URL.
pub fn open_diagnostic(url: &str) -> Result<()> {
    crate::claude_check::browser::validate_submit_url(&format!("{url}report"))?;
    native::open_url(url)
}
pub fn configured_proxy(scheme: &str) -> Result<Option<String>> {
    native::configured_proxy(scheme)
}
#[cfg(windows)]
pub fn is_elevated() -> bool {
    windows::is_elevated()
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn is_elevated() -> bool {
    unsafe { libc::geteuid() == 0 }
}
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn is_elevated() -> bool {
    false
}
#[cfg(windows)]
pub use windows::{autostart_helper_main, migrate_autostart};
#[cfg(windows)]
pub use windows::{installed_executable, launch_update_installer};
#[cfg(windows)]
pub fn exit_requested() -> bool {
    windows::exit_requested()
}
#[cfg(not(windows))]
pub fn exit_requested() -> bool {
    false
}
#[cfg(windows)]
pub fn show_requested() -> bool {
    windows::show_requested()
}
#[cfg(windows)]
pub fn show_existing() {
    windows::signal_show();
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn show_requested() -> bool {
    native::show_requested()
}
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn show_requested() -> bool {
    false
}

pub fn enable(journal_path: &Path, port: u16) -> Result<()> {
    if port == 0 {
        bail!("代理端口无效");
    }
    if journal_path.exists() {
        bail!("存在未恢复的系统代理，请先关闭系统代理或重新启动客户端");
    }
    let original = native::read()?;
    #[cfg(target_os = "macos")]
    let owned = native::desired(&original, port)?;
    #[cfg(not(target_os = "macos"))]
    let owned = native::desired(port);
    let journal = Journal { original, owned };
    // Persist restoration information before changing any OS setting.
    atomic_write(journal_path, &serde_json::to_vec_pretty(&journal)?)?;
    if let Err(error) = native::write(&journal.owned) {
        if native::write(&journal.original).is_ok() {
            std::fs::remove_file(journal_path)?;
        }
        return Err(error.context("系统代理启用失败，已尝试恢复原设置"));
    }
    if !owns(&native::read()?, &journal.owned)? {
        bail!("系统代理设置未生效，恢复记录已保留");
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn auxiliary_key(key: &str) -> bool {
    matches!(
        key,
        "ProxyOverride"
            | "org.gnome.system.proxy|ignore-hosts"
            | "kde|NoProxyFor"
            | "kde|ReversedException"
    )
}

#[cfg(not(target_os = "macos"))]
fn same_route_value(
    key: &str,
    current: Option<&Option<ProxyValue>>,
    owned: Option<&Option<ProxyValue>>,
) -> bool {
    if key == "AutoConfigURL" {
        let disabled = |value: Option<&Option<ProxyValue>>| {
            matches!(value, Some(None))
                || matches!(value, Some(Some(ProxyValue::Text(text))) if text.is_empty())
        };
        if disabled(current) && disabled(owned) {
            return true;
        }
    }
    current == owned
}

fn owns(current: &ProxyState, owned: &ProxyState) -> Result<bool> {
    #[cfg(target_os = "macos")]
    {
        native::matches_managed(current, owned)
    }
    #[cfg(not(target_os = "macos"))]
    {
        if !owned.is_empty()
            && owned.keys().any(|key| !auxiliary_key(key))
            && current.len() == owned.len()
            && owned.iter().all(|(key, value)| {
                current.contains_key(key)
                    && (auxiliary_key(key) || same_route_value(key, current.get(key), Some(value)))
            })
        {
            return Ok(true);
        }
        #[cfg(target_os = "linux")]
        {
            Ok(linux::owns_legacy(current, owned))
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(false)
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn restoration(current: &ProxyState, journal: &Journal) -> Result<ProxyState> {
    if !owns(current, &journal.owned)? {
        return Ok(ProxyState::new());
    }
    Ok(journal
        .original
        .iter()
        .filter(|(key, _)| {
            journal.owned.contains_key(*key) && current.get(*key) == journal.owned.get(*key)
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

/// Restore our routing settings while preserving later user changes.
pub fn restore(journal_path: &Path) -> Result<bool> {
    if !journal_path.exists() {
        return Ok(false);
    }
    let journal: Journal =
        serde_json::from_slice(&std::fs::read(journal_path)?).context("代理恢复记录损坏")?;
    #[cfg(target_os = "macos")]
    let restored = native::restore_write(&journal.original, &journal.owned)?;
    #[cfg(not(target_os = "macos"))]
    let restored = {
        let patch = restoration(&native::read()?, &journal)?;
        if !patch.is_empty() {
            native::write(&patch)?;
        }
        !patch.is_empty()
    };
    std::fs::remove_file(journal_path)?;
    Ok(restored)
}

pub fn is_owned(journal_path: &Path) -> Result<bool> {
    if !journal_path.exists() {
        return Ok(false);
    }
    let journal: Journal = serde_json::from_slice(&std::fs::read(journal_path)?)?;
    owns(&native::read()?, &journal.owned)
}

#[cfg(target_os = "macos")]
pub fn adopt_owned_proxy(journal_path: &Path) -> Result<()> {
    if !journal_path.exists() {
        return Ok(());
    }
    let journal: Journal = serde_json::from_slice(&std::fs::read(journal_path)?)?;
    let current = native::read()?;
    if owns(&current, &journal.owned)? {
        native::service::adopt_proxy(&journal.original, &journal.owned)?;
    } else {
        // A restarted helper may already have recovered a stale GUI journal.
        // Still restore surviving owned services before discarding the journal.
        restore(journal_path)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn is_owned_at_port(journal_path: &Path, port: u16) -> Result<bool> {
    if !journal_path.exists() {
        return Ok(false);
    }
    let journal: Journal = serde_json::from_slice(&std::fs::read(journal_path)?)?;
    Ok(
        owns(&journal.owned, &native::desired(&journal.original, port)?)?
            && owns(&native::read()?, &journal.owned)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_journal_never_changes_os_settings() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!restore(&tmp.path().join("absent.json")).unwrap());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn windows_empty_pac_url_is_disabled_but_an_external_url_loses_ownership() {
        let owned = ProxyState::from([
            ("ProxyEnable".into(), Some(ProxyValue::Number(1))),
            (
                "ProxyServer".into(),
                Some(ProxyValue::Text("127.0.0.1:7890".into())),
            ),
            ("AutoConfigURL".into(), None),
        ]);
        let mut current = owned.clone();
        current.insert(
            "AutoConfigURL".into(),
            Some(ProxyValue::Text(String::new())),
        );
        assert!(owns(&current, &owned).unwrap());
        current.insert(
            "AutoConfigURL".into(),
            Some(ProxyValue::Text(
                "https://external.example/proxy.pac".into(),
            )),
        );
        assert!(!owns(&current, &owned).unwrap());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn bypass_edits_do_not_hide_proxy_or_get_overwritten_during_restore() {
        for (mode, endpoint, bypass) in [
            ("ProxyEnable", "ProxyServer", "ProxyOverride"),
            (
                "org.gnome.system.proxy|mode",
                "org.gnome.system.proxy.http|port",
                "org.gnome.system.proxy|ignore-hosts",
            ),
            ("kde|ProxyType", "kde|httpProxy", "kde|NoProxyFor"),
            ("kde|ProxyType", "kde|httpProxy", "kde|ReversedException"),
        ] {
            let original = ProxyState::from([
                (mode.into(), Some(ProxyValue::Text("off".into()))),
                (endpoint.into(), Some(ProxyValue::Text("old".into()))),
                (bypass.into(), Some(ProxyValue::Text("previous".into()))),
            ]);
            let owned = ProxyState::from([
                (mode.into(), Some(ProxyValue::Text("on".into()))),
                (endpoint.into(), Some(ProxyValue::Text("local".into()))),
                (bypass.into(), Some(ProxyValue::Text("default".into()))),
            ]);
            let journal = Journal {
                original: original.clone(),
                owned: owned.clone(),
            };
            assert!(owns(&owned, &owned).unwrap());
            assert_eq!(restoration(&owned, &journal).unwrap(), original);
            let mut current = owned.clone();
            current.insert(bypass.into(), Some(ProxyValue::Text("later edit".into())));
            assert!(owns(&current, &owned).unwrap(), "{bypass}");
            let patch = restoration(&current, &journal).unwrap();
            assert!(!patch.contains_key(bypass));
            current.extend(patch);
            assert_eq!(current[mode], original[mode]);
            assert_eq!(current[endpoint], original[endpoint]);
            assert_eq!(current[bypass], Some(ProxyValue::Text("later edit".into())));
            assert!(!owns(&current, &owned).unwrap());
            let mut external = owned.clone();
            external.insert(endpoint.into(), Some(ProxyValue::Text("external".into())));
            assert!(!owns(&external, &owned).unwrap());
            assert!(restoration(&external, &journal).unwrap().is_empty());
            external = owned.clone();
            external.insert(mode.into(), Some(ProxyValue::Text("off".into())));
            assert!(!owns(&external, &owned).unwrap());
        }
        assert!(!owns(&ProxyState::new(), &ProxyState::new()).unwrap());
    }
}
