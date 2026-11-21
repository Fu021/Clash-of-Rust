use slint::{
    ComponentHandle,
    platform::{
        Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::Cell, rc::Rc, time::Duration};

slint::slint! {
    #[edition = "1.19"]
    import { ListView, MScroll } from "../ui/mobile/shared/controls.slint";

    export component MobileScrollProbe inherits Window {
        in property <bool> mounted: true;
        in property <int> rows: 100;
        in-out property <length> offset;
        if root.mounted: MScroll {
            width: parent.width; height: parent.height;
            content-y <=> root.offset;
            for index in root.rows: Rectangle { height: 80px; }
        }
    }

    export component MobileListProbe inherits Window {
        in property <bool> mounted: true;
        in property <int> rows: 100;
        in-out property <length> offset;
        if root.mounted: ListView {
            width: parent.width; height: parent.height;
            content-y <=> root.offset;
            for index in root.rows: Rectangle { height: 80px; }
        }
    }
}

struct Headless(Rc<MinimalSoftwareWindow>, Rc<Cell<Duration>>);

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.1.get()
    }
}

fn headless() -> Rc<Cell<Duration>> {
    let adapter = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    adapter.set_size(slint::PhysicalSize::new(300, 240));
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(Headless(adapter, clock.clone()))).unwrap();
    clock
}

fn frame(window: &slint::Window, clock: &Cell<Duration>) {
    slint::platform::update_timers_and_animations();
    window.take_snapshot().unwrap();
    clock.set(clock.get() + Duration::from_millis(16));
    slint::platform::update_timers_and_animations();
    window.take_snapshot().unwrap();
}

#[test]
fn mobile_scroll_remount_preserves_valid_offsets_and_clamps_short_content() {
    let clock = headless();
    let window = MobileScrollProbe::new().unwrap();
    window.show().unwrap();
    frame(window.window(), &clock);
    window.set_offset(-500.);
    frame(window.window(), &clock);
    assert_eq!(window.get_offset(), -500.);
    window.set_mounted(false);
    frame(window.window(), &clock);
    window.set_mounted(true);
    frame(window.window(), &clock);
    assert_eq!(window.get_offset(), -500.);
    window.set_mounted(false);
    frame(window.window(), &clock);
    window.set_rows(2);
    window.set_mounted(true);
    frame(window.window(), &clock);
    assert_eq!(window.get_offset(), 0.);
    window.hide().unwrap();
}

#[test]
fn mobile_list_remount_preserves_valid_offsets_and_clamps_short_content() {
    let clock = headless();
    let window = MobileListProbe::new().unwrap();
    window.show().unwrap();
    frame(window.window(), &clock);
    window.set_offset(-500.);
    frame(window.window(), &clock);
    assert_eq!(window.get_offset(), -500.);
    window.set_mounted(false);
    frame(window.window(), &clock);
    window.set_mounted(true);
    frame(window.window(), &clock);
    assert_eq!(window.get_offset(), -500.);
    window.set_mounted(false);
    frame(window.window(), &clock);
    window.set_rows(2);
    window.set_mounted(true);
    frame(window.window(), &clock);
    assert_eq!(window.get_offset(), 0.);
    window.hide().unwrap();
}
