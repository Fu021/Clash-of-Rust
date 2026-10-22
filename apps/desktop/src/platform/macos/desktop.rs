//! Dock reopen support. Slint/winit's existing delegate retains all lifecycle duties.
use objc2::{
    Encode, ffi,
    runtime::{AnyObject, Bool, Imp, Sel},
    sel,
};
use objc2_app_kit::{NSApplication, NSApplicationTerminateReply};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol};
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

static REOPEN: AtomicBool = AtomicBool::new(false);
static QUIT: AtomicBool = AtomicBool::new(false);
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

#[derive(Debug, Clone)]
pub(crate) struct Events {
    pub reopen: bool,
    pub quit: bool,
}

unsafe extern "C-unwind" fn quit(
    _delegate: &AnyObject,
    _selector: Sel,
    _app: &NSApplication,
) -> NSApplicationTerminateReply {
    // AppKit's menu, Command-Q and Dock Quit converge here. Cancel native
    // termination; the GUI saves geometry and stops the core before exiting.
    QUIT.store(true, Ordering::Release);
    NSApplicationTerminateReply::TerminateCancel
}

pub(crate) fn pump() -> Result<Events, String> {
    let mtm = MainThreadMarker::new().expect("Dock handling must run on the main thread");
    INSTALLED
        .get_or_init(|| {
            let app = NSApplication::sharedApplication(mtm);
            let delegate = app.delegate().ok_or("macOS 应用委托尚未就绪")?;
            let selector = sel!(applicationShouldHandleReopen:hasVisibleWindows:);
            if delegate.respondsToSelector(selector) {
                return Err("macOS 应用委托已包含其他窗口恢复处理程序".into());
            }
            let quit_selector = sel!(applicationShouldTerminate:);
            if delegate.respondsToSelector(quit_selector) {
                return Err("macOS 应用委托已包含其他退出处理程序".into());
            }
            // ProtocolObject has no AnyObject accessor in objc2 0.6.4. Its
            // Message contract guarantees the same Objective-C object layout.
            let class = unsafe {
                ffi::object_getClass(objc2::rc::Retained::as_ptr(&delegate).cast::<AnyObject>())
            };
            if class.is_null() {
                return Err("无法读取 macOS 应用委托类型".into());
            }
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
                    class.cast_mut(),
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
            let encoding =
                std::ffi::CString::new(format!("{}@:@", NSApplicationTerminateReply::ENCODING))
                    .unwrap();
            let added = unsafe {
                ffi::class_addMethod(
                    class.cast_mut(),
                    quit_selector,
                    std::mem::transmute::<
                        unsafe extern "C-unwind" fn(
                            &AnyObject,
                            Sel,
                            &NSApplication,
                        )
                            -> NSApplicationTerminateReply,
                        Imp,
                    >(quit),
                    encoding.as_ptr(),
                )
            };
            if !added.as_bool() {
                return Err("无法注册 macOS 应用退出事件".into());
            }
            // Refresh AppKit's optional-selector cache after adding the method.
            // The same retained winit delegate handles every existing callback.
            app.setDelegate(None);
            app.setDelegate(Some(&delegate));
            Ok(())
        })
        .as_ref()
        .map_err(Clone::clone)?;
    Ok(Events {
        reopen: REOPEN.swap(false, Ordering::AcqRel),
        quit: QUIT.swap(false, Ordering::AcqRel),
    })
}
