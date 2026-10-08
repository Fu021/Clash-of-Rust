//! OS-specific integration stays behind this module. Linux GUI/core code is identical.
use crate::config::atomic_write;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProxyValue {
    Text(String),
    Number(u32),
}
pub type ProxyState = BTreeMap<String, Option<ProxyValue>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    original: ProxyState,
    owned: ProxyState,
}

#[cfg(target_os = "linux")]
mod desktop;
#[cfg(target_os = "linux")]
mod gio;
#[cfg(target_os = "linux")]
mod kde;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
use linux as native;
#[cfg(windows)]
use windows as native;

#[cfg(not(any(windows, target_os = "linux")))]
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
        return Err(std::io::Error::last_os_error().into());
    }
    if unsafe { GetLastError() } == 183 {
        unsafe {
            CloseHandle(handle);
        }
        return Ok(None);
    }
    let guard = ApplicationGuard(handle);
    windows::initialize_events()?;
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

pub fn autostart_enabled() -> Result<bool> {
    native::autostart_enabled()
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
pub fn configured_proxy(scheme: &str) -> Result<Option<String>> {
    native::configured_proxy(scheme)
}
#[cfg(windows)]
pub fn is_elevated() -> bool {
    windows::is_elevated()
}
#[cfg(target_os = "linux")]
pub fn is_elevated() -> bool {
    unsafe { libc::geteuid() == 0 }
}
#[cfg(not(any(windows, target_os = "linux")))]
pub fn is_elevated() -> bool {
    false
}
#[cfg(windows)]
pub fn elevate(path: &Path) -> Result<()> {
    windows::elevate(path)
}
#[cfg(windows)]
pub use windows::{installed_executable, launch_update_installer};
#[cfg(not(windows))]
pub fn elevate(_: &Path) -> Result<()> {
    bail!("当前平台不支持 Windows UAC 提权")
}
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
#[cfg(not(windows))]
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
    let journal = Journal {
        original: native::read()?,
        owned: native::desired(port),
    };
    // Persist restoration information before changing any OS setting.
    atomic_write(journal_path, &serde_json::to_vec_pretty(&journal)?)?;
    if let Err(error) = native::write(&journal.owned) {
        if native::write(&journal.original).is_ok() {
            std::fs::remove_file(journal_path)?;
        }
        return Err(error.context("系统代理启用失败，已尝试恢复原设置"));
    }
    if native::read()? != journal.owned {
        bail!("系统代理设置未生效，恢复记录已保留");
    }
    Ok(())
}

/// Restore only if the current configuration is still exactly the one we wrote.
/// A different client's later changes must not be overwritten.
pub fn restore(journal_path: &Path) -> Result<bool> {
    if !journal_path.exists() {
        return Ok(false);
    }
    let journal: Journal =
        serde_json::from_slice(&std::fs::read(journal_path)?).context("代理恢复记录损坏")?;
    let current = native::read()?;
    let restored = current == journal.owned;
    if restored {
        native::write(&journal.original)?;
    }
    std::fs::remove_file(journal_path)?;
    Ok(restored)
}

pub fn is_owned(journal_path: &Path) -> Result<bool> {
    if !journal_path.exists() {
        return Ok(false);
    }
    let journal: Journal = serde_json::from_slice(&std::fs::read(journal_path)?)?;
    Ok(native::read()? == journal.owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_journal_never_changes_os_settings() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!restore(&tmp.path().join("absent.json")).unwrap());
    }
}
