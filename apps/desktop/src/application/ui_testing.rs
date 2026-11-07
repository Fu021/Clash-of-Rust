//! Headless software window for deterministic rendering and input tests.
//! Uses the same renderer, compiled components and models as the desktop.
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    static PRIMARY: Rc<MinimalSoftwareWindow> = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    static WINDOWS: RefCell<Vec<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(Vec::new()) };
    static PRIMARY_CREATED: Cell<bool> = const { Cell::new(false) };
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    static ADVANCE: Cell<std::time::Duration> = const { Cell::new(std::time::Duration::ZERO) };
}
struct Headless;
impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = if PRIMARY_CREATED.with(|created| created.replace(true)) {
            MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer)
        } else {
            PRIMARY.with(Clone::clone)
        };
        WINDOWS.with(|windows| windows.borrow_mut().push(window.clone()));
        Ok(window)
    }
    fn duration_since_start(&self) -> std::time::Duration {
        ADVANCE.with(Cell::get)
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
    WINDOWS.with(|windows| windows.borrow_mut().clear());
    PRIMARY_CREATED.with(|created| created.set(false));
    let window = PRIMARY.with(Clone::clone);
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
pub(super) fn popup_click(x: f32, y: f32) {
    let popup = WINDOWS
        .with(|windows| {
            windows
                .borrow()
                .iter()
                .skip(1)
                .rev()
                .find(|window| window.window().is_visible())
                .cloned()
        })
        .expect("popup window is not open");
    pointer_click(popup.window(), x, y, std::time::Duration::from_millis(50));
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
    ADVANCE.with(|clock| clock.set(clock.get() + delay));
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}
