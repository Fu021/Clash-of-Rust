//! Exercise the production mobile virtual-list wrapper with a software window.
use slint::{
    ComponentHandle,
    platform::{
        Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::Cell, rc::Rc, time::Duration};

slint::slint! {
    import { ListView, MDragHandle } from "../ui/mobile/shared/controls.slint";

    export component MobileListProbe inherits Window {
        preferred-width: 300px; preferred-height: 240px;
        in property <[float]> rows;
        out property <length> extent <=> list.content-height;
        list := ListView {
            width: parent.width; height: parent.height;
            for row[index] in root.rows: Rectangle {
                height: row * 1px;
                background: index == 0 ? #ff0000 : index == 1 ? #00ff00 : #0000ff;
            }
        }
    }
    export component MobileDragProbe inherits Window {
        preferred-width: 300px; preferred-height: 240px;
        out property <bool> started;
        out property <int> delta: 99;
        ListView {
            width: parent.width; height: parent.height;
            for index in 10: Rectangle {
                height: 80px;
                MDragHandle {
                    row-height: 80px;
                    started => { root.started = true; }
                    dropped(delta) => { root.delta = delta; }
                }
            }
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

fn frame(adapter: &MinimalSoftwareWindow, width: usize, height: usize) -> Vec<slint::Rgb8Pixel> {
    let mut pixels = vec![slint::Rgb8Pixel::default(); width * height];
    adapter.window().request_redraw();
    assert!(adapter.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width);
    }));
    pixels
}

fn advance(clock: &Cell<Duration>, duration: Duration) {
    let deadline = clock.get() + duration;
    // Scroll gestures defer pointer forwarding; process each virtual event-loop
    // tick so forwarded presses and timer bindings precede their deadlines.
    slint::platform::update_timers_and_animations();
    while clock.get() < deadline {
        clock.set((clock.get() + Duration::from_millis(10)).min(deadline));
        slint::platform::update_timers_and_animations();
    }
}

#[test]
fn mobile_list_stacks_rows_and_keeps_sparse_content_at_the_top() {
    let adapter = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(Headless(adapter.clone(), clock.clone()))).unwrap();
    adapter.set_size(slint::PhysicalSize::new(300, 240));
    let probe = MobileListProbe::new().unwrap();
    let rows = Rc::new(slint::VecModel::from(vec![48.; 20]));
    probe.set_rows(rows.clone().into());
    probe.show().unwrap();

    let pixels = frame(&adapter, 300, 240);
    assert!(
        probe.get_extent() >= 20. * 48.,
        "all rows must contribute to the scrollable content extent"
    );
    assert_eq!(pixels[10 * 300 + 10], slint::Rgb8Pixel::new(255, 0, 0));
    assert_eq!(pixels[58 * 300 + 10], slint::Rgb8Pixel::new(0, 255, 0));
    assert_eq!(pixels[106 * 300 + 10], slint::Rgb8Pixel::new(0, 0, 255));

    rows.set_vec(vec![48.]);
    let pixels = frame(&adapter, 300, 240);
    assert_eq!(pixels[10 * 300 + 10], slint::Rgb8Pixel::new(255, 0, 0));
    assert!(
        probe.get_extent() < 100.,
        "removed rows must release their scroll extent"
    );

    adapter.set_size(slint::PhysicalSize::new(900, 240));
    assert_eq!(probe.window().size(), slint::PhysicalSize::new(900, 240));
    rows.set_vec(vec![40., 64., 80.]);
    let pixels = frame(&adapter, 900, 240);
    assert_eq!(pixels[5 * 900 + 700], slint::Rgb8Pixel::new(255, 0, 0));
    assert_eq!(pixels[50 * 900 + 700], slint::Rgb8Pixel::new(0, 255, 0));
    assert_eq!(pixels[110 * 900 + 700], slint::Rgb8Pixel::new(0, 0, 255));
    drop(probe);

    // Slint installs one platform per test process, so exercise the drag probe
    // after releasing the first window instead of installing a second platform.
    use slint::{
        LogicalPosition,
        platform::{PointerEventButton, WindowEvent},
    };
    adapter.set_size(slint::PhysicalSize::new(300, 240));
    let drag_probe = MobileDragProbe::new().unwrap();
    drag_probe.show().unwrap();
    frame(&adapter, 300, 240);
    let press = || {
        drag_probe
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position: LogicalPosition::new(12., 24.),
                button: PointerEventButton::Left,
            })
    };
    press();
    slint::platform::update_timers_and_animations();
    drag_probe
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(12., 24.),
            button: PointerEventButton::Left,
        });
    advance(&clock, Duration::from_millis(700));
    assert!(
        !drag_probe.get_started(),
        "a normal tap must not start reordering"
    );
    assert_eq!(drag_probe.get_delta(), 99);
    press();
    advance(&clock, Duration::from_millis(500));
    assert!(
        !drag_probe.get_started(),
        "reordering must wait for the long-press threshold"
    );
    advance(&clock, Duration::from_millis(300));
    assert!(drag_probe.get_started());
    drag_probe
        .window()
        .dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(12., 184.),
        });
    drag_probe
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(12., 184.),
            button: PointerEventButton::Left,
        });
    assert_eq!(drag_probe.get_delta(), 2);
}
