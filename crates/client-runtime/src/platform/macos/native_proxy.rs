//! Root-only network preference writer; never prompts or starts a GUI.
#![warn(clippy::undocumented_unsafe_blocks)]

use super::cf::{self, Owned, Ref};
use crate::platform::{ProxyState, ProxyValue};
use anyhow::{Result, bail, ensure};
use serde_json::Value;
use std::ptr;

#[link(name = "SystemConfiguration", kind = "framework")]
// SAFETY: SystemConfiguration uses opaque CF pointers, u8 Boolean results and
// an i32 SCError status; all declared calls are synchronous and use the C ABI.
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
    // SAFETY: name is an owned CFString throughout this synchronous creation;
    // null allocator/ID select defaults. The new session reference is owned.
    Owned::new(unsafe { SCPreferencesCreate(ptr::null(), name.0, ptr::null()) })
}
fn sc_check(ok: u8, action: &str) -> Result<()> {
    ensure!(
        ok != 0,
        "{action}失败（macOS SystemConfiguration {}），请检查授权与网络服务",
        // SAFETY: SCError reads this thread's last SystemConfiguration error.
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
    read_entity_from(prefs.0, entity)
}
fn read_entity_from(prefs: Ref, entity: &str) -> Result<ProxyState> {
    let key = cf::string("/NetworkServices")?;
    // SAFETY: Callers keep the session owner alive throughout this function.
    // key is a live CFString; the Get result is borrowed, never independently freed.
    let value = unsafe { SCPreferencesPathGetValue(prefs, key.0) };
    ensure!(!value.is_null(), "没有可配置的 macOS 网络服务");
    let mut state = ProxyState::new();
    for id in cf::dictionary_keys(value)? {
        let interface = cf::string(&format!("/NetworkServices/{id}/Interface"))?;
        // SAFETY: The session and interface string remain live for this query.
        if unsafe { SCPreferencesPathGetValue(prefs, interface.0) }.is_null() {
            continue;
        }
        let path = cf::string(&format!("/NetworkServices/{id}/{entity}"))?;
        // SAFETY: The session and path remain live; the borrowed value is copied
        // into JSON before any session mutation or release can invalidate it.
        let value = unsafe { SCPreferencesPathGetValue(prefs, path.0) };
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
    edit_entity(entity, |_| Ok(state.clone())).map(|_| ())
}
pub fn restore(original: &ProxyState, owned: &ProxyState) -> Result<bool> {
    edit_entity("Proxies", |current| {
        super::proxy_preferences::restoration(current, original, owned)
    })
}
fn edit_entity(
    entity: &str,
    patch: impl FnOnce(&ProxyState) -> Result<ProxyState>,
) -> Result<bool> {
    ensure!(
        // SAFETY: This parameterless identity query has no borrowed resources.
        unsafe { libc::geteuid() } == 0,
        "网络设置必须通过已授权的网络助手修改"
    );
    let prefs = preferences()?;
    sc_check(
        // SAFETY: The locally owned session is not shared with another thread;
        // nonblocking lock failure returns before entering the edit closure.
        unsafe { SCPreferencesLock(prefs.0, 0) },
        "锁定网络设置（其他程序占用时请重试）",
    )?;
    let result = (|| {
        let current = read_entity_from(prefs.0, entity)?;
        let state = patch(&current)?;
        if state.is_empty() {
            return Ok(false);
        }
        for (id, value) in &state {
            ensure!(
                !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "网络服务标识无效"
            );
            let service = cf::string(&format!("/NetworkServices/{id}"))?;
            ensure!(
                // SAFETY: Both session and path are live; the borrowed result is
                // used only to check existence before the next session operation.
                !unsafe { SCPreferencesPathGetValue(prefs.0, service.0) }.is_null(),
                "网络服务已发生变化，请重新启用系统代理"
            );
            let path = cf::string(&format!("/NetworkServices/{id}/{entity}"))?;
            match value {
                Some(ProxyValue::Text(text)) => {
                    let value = cf::plist(&serde_json::from_str::<Value>(text)?)?;
                    sc_check(
                        // SAFETY: The locked session, path and immutable property
                        // list remain owned throughout this synchronous write.
                        unsafe { SCPreferencesPathSetValue(prefs.0, path.0, value.0) },
                        "写入网络服务代理",
                    )?;
                }
                None => {
                    // SAFETY: The locked session and path remain live; the Get
                    // result is inspected only for null before removing the key.
                    if !unsafe { SCPreferencesPathGetValue(prefs.0, path.0) }.is_null() {
                        sc_check(
                            // SAFETY: The same locked session and path are live.
                            unsafe { SCPreferencesPathRemoveValue(prefs.0, path.0) },
                            "恢复网络服务代理",
                        )?;
                    }
                }
                _ => bail!("网络服务代理属性无效"),
            }
        }
        sc_check(
            // SAFETY: The live session remains locked until after the closure.
            unsafe { SCPreferencesCommitChanges(prefs.0) },
            "保存网络设置",
        )?;
        sc_check(
            // SAFETY: The same owned, locked session has committed successfully.
            unsafe { SCPreferencesApplyChanges(prefs.0) },
            "应用网络设置",
        )?;
        Ok(true)
    })();
    // SAFETY: Lock succeeded and prefs is still owned, on success and all Result
    // error exits. If Rust unwinds earlier, Owned releases the session instead.
    unsafe { SCPreferencesUnlock(prefs.0) };
    result
}
