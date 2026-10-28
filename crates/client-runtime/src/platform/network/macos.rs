use super::{Route, Routes};
use crate::platform::macos::cf;
use anyhow::{Context, Result, ensure};
use std::ptr;

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCreate(
        allocator: cf::Ref,
        name: cf::Ref,
        callback: Option<unsafe extern "C" fn(cf::Ref, cf::Ref, *mut std::ffi::c_void)>,
        context: *mut std::ffi::c_void,
    ) -> cf::Ref;
    fn SCDynamicStoreCopyValue(store: cf::Ref, key: cf::Ref) -> cf::Ref;
    fn SCError() -> i32;
}

pub(super) fn read() -> Result<Routes> {
    let name = cf::string("Clash of Rust network recovery")?;
    let store = cf::Owned::new(unsafe {
        SCDynamicStoreCreate(ptr::null(), name.0, None, ptr::null_mut())
    })?;
    let mut routes = Vec::new();
    for family in ["IPv4", "IPv6"] {
        let key = cf::string(&format!("State:/Network/Global/{family}"))?;
        let value = unsafe { SCDynamicStoreCopyValue(store.0, key.0) };
        if value.is_null() {
            let status = unsafe { SCError() };
            ensure!(status == 1004, "读取系统默认网络失败：{status}");
            continue;
        }
        let value = cf::Owned::new(value)?;
        let value = cf::json(value.0)?;
        let interface = value["PrimaryInterface"]
            .as_str()
            .context("系统默认网络接口无效")?;
        let gateway = value["Router"].as_str().unwrap_or_default();
        routes.push(Route {
            interface: interface.into(),
            gateway: format!("{family}:{gateway}"),
        });
    }
    Ok(routes)
}
