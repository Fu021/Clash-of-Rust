//! Root-only network preference writer; never prompts or starts a GUI.
use super::cf::{self, Owned, Ref};
use crate::platform::{ProxyState, ProxyValue};
use anyhow::{Result, bail, ensure};
use serde_json::Value;
use std::ptr;

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCPreferencesCreate(a: Ref, name: Ref, id: Ref) -> Ref;
    fn SCPreferencesPathGetValue(prefs: Ref, path: Ref) -> Ref;
    fn SCPreferencesPathSetValue(prefs: Ref, path: Ref, value: Ref) -> u8;
    fn SCPreferencesPathRemoveValue(prefs: Ref, path: Ref) -> u8;
    fn SCPreferencesLock(prefs: Ref, wait: u8) -> u8;
    fn SCPreferencesUnlock(prefs: Ref) -> u8;
    fn SCPreferencesCommitChanges(prefs: Ref) -> u8;
    fn SCPreferencesApplyChanges(prefs: Ref) -> u8;
    fn SCError() -> i32;
}

fn preferences() -> Result<Owned> {
    let name = cf::string("Clash of Rust")?;
    Owned::new(unsafe { SCPreferencesCreate(ptr::null(), name.0, ptr::null()) })
}
fn sc_check(ok: u8, action: &str) -> Result<()> {
    ensure!(
        ok != 0,
        "{action}失败（macOS SystemConfiguration {}），请检查授权与网络服务",
        unsafe { SCError() }
    );
    Ok(())
}
pub fn read() -> Result<ProxyState> {
    read_entity("Proxies")
}
pub(super) fn read_dns() -> Result<ProxyState> {
    read_entity("DNS")
}
fn read_entity(entity: &str) -> Result<ProxyState> {
    let prefs = preferences()?;
    let key = cf::string("/NetworkServices")?;
    let value = unsafe { SCPreferencesPathGetValue(prefs.0, key.0) };
    ensure!(!value.is_null(), "没有可配置的 macOS 网络服务");
    let mut state = ProxyState::new();
    for id in cf::dictionary_keys(value)? {
        let interface = cf::string(&format!("/NetworkServices/{id}/Interface"))?;
        if unsafe { SCPreferencesPathGetValue(prefs.0, interface.0) }.is_null() {
            continue;
        }
        let path = cf::string(&format!("/NetworkServices/{id}/{entity}"))?;
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
pub fn write(state: &ProxyState) -> Result<()> {
    write_entity("Proxies", state)
}
pub(super) fn write_dns(state: &ProxyState) -> Result<()> {
    write_entity("DNS", state)
}
fn write_entity(entity: &str, state: &ProxyState) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "网络设置必须通过已授权的网络助手修改"
    );
    let prefs = preferences()?;
    sc_check(
        unsafe { SCPreferencesLock(prefs.0, 0) },
        "锁定网络设置（其他程序占用时请重试）",
    )?;
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
            let path = cf::string(&format!("/NetworkServices/{id}/{entity}"))?;
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
