//! Dock reopen support. Iced/winit's existing delegate retains all lifecycle duties.
use objc2::{
    Encode, Message, ffi,
    runtime::{AnyObject, Bool, Imp, Sel},
    sel,
};
use objc2_app_kit::NSApplication;
use objc2_foundation::{MainThreadMarker, NSObjectProtocol};
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

static REOPEN: AtomicBool = AtomicBool::new(false);
static INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();

// AppKit BOOL has different encodings on Intel and Apple Silicon. Use objc2's
// native Bool for both arguments and the result, rather than a hard-coded i8.
unsafe extern "C-unwind" fn reopen(
    _delegate: &AnyObject,
    _selector: Sel,
    _app: &NSApplication,
    _has_visible_windows: Bool,
) -> Bool {
    // Restore even when an auxiliary window is visible or the app is already
    // active. DidBecomeActive alone misses clicks made after closing its window.
    REOPEN.store(true, Ordering::Release);
    Bool::NO // The GUI restores/focuses its retained window; skip AppKit defaults.
}

pub(crate) fn pump() -> Result<bool, String> {
    let mtm = MainThreadMarker::new().expect("Dock handling must run on the main thread");
    INSTALLED
        .get_or_init(|| {
            let app = NSApplication::sharedApplication(mtm);
            let delegate = app.delegate().ok_or("macOS 应用委托尚未就绪")?;
            let selector = sel!(applicationShouldHandleReopen:hasVisibleWindows:);
            if delegate.respondsToSelector(selector) {
                return Err("macOS 应用委托已包含其他窗口恢复处理程序".into());
            }
            let class = delegate.as_any_object().class();
            // Add just the missing optional selector. Replacing the delegate
            // would break winit's launch, termination and event-loop callbacks.
            let encoding =
                std::ffi::CString::new(format!("{}@:@{}", Bool::ENCODING, Bool::ENCODING))
                    .expect("Objective-C method encoding contains no NUL");
            // SAFETY: installation happens once on AppKit's main thread. This
            // selector is absent, the callback has the exact Objective-C ABI,
            // and the registered class and implementation live for the process.
            let added = unsafe {
                ffi::class_addMethod(
                    (class as *const objc2::runtime::AnyClass).cast_mut(),
                    selector,
                    std::mem::transmute::<
                        unsafe extern "C-unwind" fn(&AnyObject, Sel, &NSApplication, Bool) -> Bool,
                        Imp,
                    >(reopen),
                    encoding.as_ptr(),
                )
            };
            if !added.as_bool() {
                return Err("无法注册 macOS Dock 窗口恢复事件".into());
            }
            Ok(())
        })
        .as_ref()
        .map_err(Clone::clone)?;
    Ok(REOPEN.swap(false, Ordering::AcqRel))
}

#[cfg(feature = "ui-preview")]
pub(crate) fn native_window() -> objc2::rc::Retained<objc2_app_kit::NSWindow> {
    let mtm = MainThreadMarker::new().unwrap();
    NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .find(|window| window.title().to_string().starts_with("Clash of Rust"))
        .expect("native application window exists")
        .retain()
}

#[cfg(feature = "ui-preview")]
pub(crate) fn simulate_dock_reopen(has_visible_windows: bool) {
    let mtm = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    let delegate = app.delegate().expect("native application delegate exists");
    assert!(delegate.respondsToSelector(sel!(applicationShouldHandleReopen:hasVisibleWindows:)));
    // Exercise the selector that AppKit delivers for a Dock click, including its
    // architecture-specific argument/result ABI, rather than calling ShowWindow.
    let handled: Bool = unsafe {
        objc2::msg_send![&*delegate, applicationShouldHandleReopen: &*app,
                        hasVisibleWindows: Bool::new(has_visible_windows)]
    };
    assert!(!handled.as_bool());
}
