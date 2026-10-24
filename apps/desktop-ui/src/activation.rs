//! Showing a recreated window must wait for its native handle, then activate it.
use crate::{repaint, view_state};
use clash_client_ui::AppWindow;
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};

pub(super) async fn show(window: &AppWindow) -> Result<(), slint::PlatformError> {
    window.show()?;
    // Slint may defer native window creation until the event loop is running.
    // Do not retain the returned Arc: that would prevent hide() destroying it.
    drop(window.window().winit_window().await?);
    if !window.window().is_visible() {
        return Ok(()); // A close command overtook this pending activation.
    }
    window.window().set_minimized(false);
    view_state::ensure_reachable(window);
    window.window().with_winit_window(|native| {
        native.set_minimized(false);
        native.set_visible(true);
        #[cfg(windows)]
        reveal(native);
        native.focus_window();
    });
    repaint::invalidate(window.window());
    Ok(())
}

#[cfg(windows)]
fn reveal(native: &slint::winit_030::winit::window::Window) {
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{IsIconic, SW_RESTORE, SW_SHOW, ShowWindow},
    };
    let Ok(handle) = native.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as HWND;
    // Slint show() is a no-op when it already considers a window visible.
    // Restore the actual Win32 window too, preserving a maximized window.
    unsafe {
        ShowWindow(
            hwnd,
            if IsIconic(hwnd) != 0 {
                SW_RESTORE
            } else {
                SW_SHOW
            },
        )
    };
}
