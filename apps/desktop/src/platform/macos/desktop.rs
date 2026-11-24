//! Dock reopen support. Slint/winit's existing delegate retains all lifecycle duties.
use crate::messages;
use clash_of_rust::config::Language;
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
static INSTALLED: OnceLock<Result<(), DockFailure>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
pub(crate) enum DockFailure {
    DelegateNotReady,
    ReopenHandlerExists,
    QuitHandlerExists,
    DelegateClassMissing,
    ReopenRegistrationFailed,
    QuitRegistrationFailed,
}

impl DockFailure {
    pub(crate) fn message(self, language: Language) -> &'static str {
        match self {
            Self::DelegateNotReady => messages::dock_delegate_not_ready(language),
            Self::ReopenHandlerExists => messages::dock_reopen_handler_exists(language),
            Self::QuitHandlerExists => messages::dock_quit_handler_exists(language),
            Self::DelegateClassMissing => messages::dock_delegate_class_missing(language),
            Self::ReopenRegistrationFailed => messages::dock_reopen_registration_failed(language),
            Self::QuitRegistrationFailed => messages::dock_quit_registration_failed(language),
        }
    }

    pub(crate) fn translate(text: &str, language: Language) -> Option<&'static str> {
        [
            Self::DelegateNotReady,
            Self::ReopenHandlerExists,
            Self::QuitHandlerExists,
            Self::DelegateClassMissing,
            Self::ReopenRegistrationFailed,
            Self::QuitRegistrationFailed,
        ]
        .into_iter()
        .find(|failure| failure.message(Language::SimplifiedChinese) == text)
        .map(|failure| failure.message(language))
    }
}

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

pub(crate) fn pump() -> Result<Events, DockFailure> {
    let mtm = MainThreadMarker::new().expect("Dock handling must run on the main thread");
    INSTALLED
        .get_or_init(|| {
            let app = NSApplication::sharedApplication(mtm);
            let delegate = app.delegate().ok_or(DockFailure::DelegateNotReady)?;
            let selector = sel!(applicationShouldHandleReopen:hasVisibleWindows:);
            if delegate.respondsToSelector(selector) {
                return Err(DockFailure::ReopenHandlerExists);
            }
            let quit_selector = sel!(applicationShouldTerminate:);
            if delegate.respondsToSelector(quit_selector) {
                return Err(DockFailure::QuitHandlerExists);
            }
            // ProtocolObject has no AnyObject accessor in objc2 0.6.4. Its
            // Message contract guarantees the same Objective-C object layout.
            let class = unsafe {
                ffi::object_getClass(objc2::rc::Retained::as_ptr(&delegate).cast::<AnyObject>())
            };
            if class.is_null() {
                return Err(DockFailure::DelegateClassMissing);
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
                return Err(DockFailure::ReopenRegistrationFailed);
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
                return Err(DockFailure::QuitRegistrationFailed);
            }
            // Refresh AppKit's optional-selector cache after adding the method.
            // The same retained winit delegate handles every existing callback.
            app.setDelegate(None);
            app.setDelegate(Some(&delegate));
            Ok(())
        })
        .as_ref()
        .map_err(|error| *error)?;
    Ok(Events {
        reopen: REOPEN.swap(false, Ordering::AcqRel),
        quit: QUIT.swap(false, Ordering::AcqRel),
    })
}
