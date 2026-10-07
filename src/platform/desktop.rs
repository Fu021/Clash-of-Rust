//! Select WSLg's XWayland path before GUI libraries start any threads.
use std::ffi::OsStr;

fn prefer_x11(wsl: bool, display: Option<&OsStr>) -> bool {
    wsl && display.is_some_and(|value| !value.is_empty())
}

/// # Safety
/// Call once at process startup, before creating threads or invoking libraries
/// that may read the environment on another thread.
pub(super) unsafe fn initialize() {
    let wsl =
        std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSL_INTEROP").is_some();
    if prefer_x11(wsl, std::env::var_os("DISPLAY").as_deref()) {
        // Winit prefers Wayland whenever these variables exist. WSLg's
        // software-rendered Wayland path can disconnect during window startup;
        // XWayland handles the same window without changing the user's shell.
        unsafe {
            std::env::remove_var("WAYLAND_DISPLAY");
            std::env::remove_var("WAYLAND_SOCKET");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_uses_x11_only_when_an_x_display_is_available() {
        assert!(prefer_x11(true, Some(OsStr::new(":0"))));
        assert!(!prefer_x11(true, None));
        assert!(!prefer_x11(true, Some(OsStr::new(""))));
        assert!(!prefer_x11(false, Some(OsStr::new(":0"))));
    }
}
