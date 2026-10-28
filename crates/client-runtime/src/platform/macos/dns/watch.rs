use super::super::cf;
use anyhow::{Result, ensure};
use std::{
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

static CHANGED: AtomicBool = AtomicBool::new(true);
#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCreate(
        allocator: cf::Ref,
        name: cf::Ref,
        callback: Option<unsafe extern "C" fn(cf::Ref, cf::Ref, *mut std::ffi::c_void)>,
        context: *mut std::ffi::c_void,
    ) -> cf::Ref;
    fn SCDynamicStoreSetNotificationKeys(store: cf::Ref, keys: cf::Ref, patterns: cf::Ref) -> u8;
    fn SCDynamicStoreCreateRunLoopSource(a: cf::Ref, store: cf::Ref, order: isize) -> cf::Ref;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: cf::Ref;
    fn CFRunLoopGetCurrent() -> cf::Ref;
    fn CFRunLoopAddSource(run_loop: cf::Ref, source: cf::Ref, mode: cf::Ref);
    fn CFRunLoopRemoveSource(run_loop: cf::Ref, source: cf::Ref, mode: cf::Ref);
    fn CFRunLoopRunInMode(mode: cf::Ref, seconds: f64, return_after_source: u8) -> i32;
}
unsafe extern "C" fn changed(_: cf::Ref, _: cf::Ref, _: *mut std::ffi::c_void) {
    CHANGED.store(true, Ordering::Release);
}

pub(crate) struct Changes {
    _store: cf::Owned,
    source: cf::Owned,
    run_loop: cf::Ref,
}
impl Changes {
    pub fn new() -> Result<Self> {
        let name = cf::string("Clash of Rust DNS changes")?;
        let store = cf::Owned::new(unsafe {
            SCDynamicStoreCreate(ptr::null(), name.0, Some(changed), ptr::null_mut())
        })?;
        let patterns = cf::plist(&serde_json::json!(super::DNS_PATTERNS))?;
        ensure!(
            unsafe { SCDynamicStoreSetNotificationKeys(store.0, ptr::null(), patterns.0) } != 0,
            "无法监测系统 DNS 变化"
        );
        let source =
            cf::Owned::new(unsafe { SCDynamicStoreCreateRunLoopSource(ptr::null(), store.0, 0) })?;
        let run_loop = unsafe { CFRunLoopGetCurrent() };
        unsafe { CFRunLoopAddSource(run_loop, source.0, kCFRunLoopDefaultMode) };
        Ok(Self {
            _store: store,
            source,
            run_loop,
        })
    }

    pub fn changed(&mut self) -> bool {
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.001, 1) };
        CHANGED.swap(false, Ordering::AcqRel)
    }
}
impl Drop for Changes {
    fn drop(&mut self) {
        unsafe { CFRunLoopRemoveSource(self.run_loop, self.source.0, kCFRunLoopDefaultMode) };
    }
}
