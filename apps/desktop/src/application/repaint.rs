//! Slint 1.18.1 software-surface compatibility, isolated from application state.
//! Remove the Windows damage hook when upgrading to a release containing
//! https://github.com/slint-ui/slint/pull/13568. The Slint version is pinned.
use slint::private_unstable_api::re_exports::{LogicalRect, WindowInner};

pub(super) fn invalidate_size(window: &slint::Window, width: f32, height: f32) {
    if width > 0.0 && height > 0.0 {
        let region = LogicalRect::new(Default::default(), [width, height].into());
        WindowInner::from_pub(window)
            .window_adapter()
            .renderer()
            .mark_dirty_region(region.into());
        window.request_redraw();
    }
}
pub(super) fn invalidate(window: &slint::Window) {
    let size = window.size().to_logical(window.scale_factor());
    invalidate_size(window, size.width, size.height);
}
pub(super) fn before_draw(_window: &slint::Window) {
    #[cfg(windows)]
    windows::before_draw(_window);
}

#[cfg(windows)]
mod windows {
    use super::*;
    use slint::winit_030::{
        WinitWindowAccessor,
        winit::raw_window_handle::{HasWindowHandle, RawWindowHandle},
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                KillTimer, SetTimer, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_NCDESTROY, WM_TIMER,
            },
        },
    };
    const DRAG_TIMER: usize = 0x434f5255;
    pub(super) fn before_draw(window: &slint::Window) {
        // Winit's native move/size loop delivers paints, but does not run its
        // usual NewEvents callback, which Slint relies on to advance timers.
        slint::platform::update_timers_and_animations();
        window.with_winit_window(|native| {
            let Ok(handle) = native.window_handle() else {
                return;
            };
            let RawWindowHandle::Win32(handle) = handle.as_raw() else {
                return;
            };
            let hwnd = handle.hwnd.get() as HWND;
            // The same procedure/id replaces itself; no heap state or dangling pointer.
            unsafe { SetWindowSubclass(hwnd, Some(drag_proc), DRAG_TIMER, 0) };
            let mut rectangle = ::windows::Win32::Foundation::RECT::default();
            let invalid = unsafe {
                ::windows::Win32::Graphics::Gdi::GetUpdateRect(
                    ::windows::Win32::Foundation::HWND(hwnd),
                    Some(&mut rectangle),
                    false,
                )
            }
            .as_bool();
            if invalid {
                let scale = window.scale_factor();
                let region = LogicalRect::new(
                    [
                        (rectangle.left as f32) / scale,
                        (rectangle.top as f32) / scale,
                    ]
                    .into(),
                    [
                        ((rectangle.right - rectangle.left) as f32) / scale,
                        ((rectangle.bottom - rectangle.top) as f32) / scale,
                    ]
                    .into(),
                );
                WindowInner::from_pub(window)
                    .window_adapter()
                    .renderer()
                    .mark_dirty_region(region.into());
            }
        });
    }
    unsafe extern "system" fn drag_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        match message {
            WM_ENTERSIZEMOVE => {
                unsafe { SetTimer(hwnd, DRAG_TIMER, 33, None) };
            }
            WM_EXITSIZEMOVE => {
                unsafe { KillTimer(hwnd, DRAG_TIMER) };
            }
            WM_TIMER if wparam == DRAG_TIMER => {
                // Request a normal winit paint instead of starting a nested Slint event loop.
                let _ = unsafe {
                    ::windows::Win32::Graphics::Gdi::RedrawWindow(
                        Some(::windows::Win32::Foundation::HWND(hwnd)),
                        None,
                        None,
                        ::windows::Win32::Graphics::Gdi::RDW_INTERNALPAINT,
                    )
                };
                return 0;
            }
            WM_NCDESTROY => unsafe {
                KillTimer(hwnd, DRAG_TIMER);
                RemoveWindowSubclass(hwnd, Some(drag_proc), DRAG_TIMER);
            },
            _ => {}
        }
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }
}
