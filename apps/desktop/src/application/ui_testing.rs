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
    static EPOCH: std::time::Instant = std::time::Instant::now();
    static ADVANCE: std::cell::Cell<std::time::Duration> = const { std::cell::Cell::new(std::time::Duration::ZERO) };
}
struct Headless;
impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(WINDOW.with(Clone::clone))
    }
    fn duration_since_start(&self) -> std::time::Duration {
        EPOCH.with(std::time::Instant::elapsed) + ADVANCE.with(std::cell::Cell::get)
    }
}

#[cfg(test)]
pub(super) fn advance(milliseconds: u64) {
    slint::platform::update_timers_and_animations();
    ADVANCE.with(|clock| clock.set(clock.get() + std::time::Duration::from_millis(milliseconds)));
    slint::platform::update_timers_and_animations();
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

#[cfg(test)]
pub(super) fn click(window: &slint::Window, x: f32, y: f32) {
    pointer_click(window, x, y, std::time::Duration::from_millis(50));
}

#[cfg(test)]
pub(super) fn rapid_click(window: &slint::Window, x: f32, y: f32) {
    pointer_click(window, x, y, std::time::Duration::ZERO);
}

#[cfg(test)]
fn pointer_click(window: &slint::Window, x: f32, y: f32, delay: std::time::Duration) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    // Match a real pointer sequence, including movement between adjacent hit
    // regions and the press delay used by Slint's own click test helper.
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    if !delay.is_zero() {
        std::thread::sleep(delay);
    }
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}
