//! Headless software window for deterministic rendering and input tests.
//! Uses the same renderer, compiled components and models as the desktop.
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::rc::Rc;

thread_local! {
    static WINDOW: Rc<MinimalSoftwareWindow> = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
struct Headless;
impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(WINDOW.with(Clone::clone))
    }
}
pub fn initialize(width: u32, height: u32) -> Rc<MinimalSoftwareWindow> {
    INSTALLED.with(|installed| {
        if !installed.get() {
            slint::platform::set_platform(Box::new(Headless))
                .expect("install isolated Slint test backend");
            installed.set(true);
        }
    });
    let window = WINDOW.with(Clone::clone);
    window.set_size(slint::PhysicalSize::new(width, height));
    window
}
