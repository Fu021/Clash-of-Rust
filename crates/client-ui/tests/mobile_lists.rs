//! Exercise production virtual lists and rule gestures with a software window.
use slint::{
    ComponentHandle,
    platform::{
        Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::Cell, rc::Rc, time::Duration};

slint::slint! {
    import { ListView, ListHeader, MField, MDragHandle } from "../ui/mobile/shared/controls.slint";
    import { RuleDragHandle } from "../ui/components/rule-drag.slint";
    import { ListReorderGesture } from "../ui/components/list-reorder.slint";

    export component MobileListProbe inherits Window {
        preferred-width: 300px; preferred-height: 240px;
        in property <[float]> rows;
        out property <length> extent <=> list.content-height;
        out property <length> horizontal-extent <=> list.content-width;
        out property <length> visible-width <=> list.visible-width;
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
                    // Outside a layout Slint centers fixed-size items by default.
                    x: 0px; y: 0px;
                    row-height: 80px;
                    started => { root.started = true; }
                    dropped(delta) => { root.delta = delta; }
                }
            }
        }
    }
    export component DesktopDragProbe inherits Window {
        preferred-width: 300px; preferred-height: 240px;
        in property <bool> handle-enabled: true;
        in property <string> context;
        out property <bool> started;
        out property <bool> cancelled;
        in-out property <int> delta: 99;
        RuleDragHandle {
            x: 0px; y: 0px; row-height: 80px; enabled: root.handle-enabled;
            context: root.context;
            started => { root.started = true; }
            dropped(delta) => { root.delta = delta; }
            cancelled => { root.cancelled = true; }
        }
    }
    export component ReorderScrollProbe inherits Window {
        preferred-width: 300px; preferred-height: 240px;
        in property <string> context: "initial";
        in property <bool> reorder-enabled: true;
        in property <length> header-height;
        in property <duration> hold-duration: 550ms;
        out property <length> scroll-y <=> list.content-y;
        out property <bool> dragging <=> gesture.reordering;
        out property <int> source: -1;
        out property <int> target: -1;
        out property <int> drop-count;
        // The actual list uses recycled rows; the gesture lives beside it.
        list := ListView {
            width: root.width; height: root.height;
            for index in 41: Rectangle {
                height: index == 0 ? root.header-height : 80px;
                background: index == 1 ? #ff0000 : #e8edf4;
            }
        }
        gesture := ListReorderGesture {
            x: 0px; y: 0px; height: list.visible-height;
            enabled: root.reorder-enabled;
            hold-duration: root.hold-duration;
            context: root.context; row-count: 40; row-height: 80px;
            header-height: root.header-height;
            scroll-content-height: list.content-height; scroll-y <=> list.content-y;
            started(index) => { root.source = index; }
            targeted(index) => { root.target = index; }
            dropped(source, target) => { root.source = source; root.target = target; root.drop-count += 1; }
        }
    }
    export component HeaderSearchProbe inherits Window {
        preferred-width: 300px; preferred-height: 600px;
        in property <[float]> rows;
        in property <length> available-height: 600px;
        in-out property <string> query;
        in-out property <length> scroll-y <=> list.content-y;
        out property <length> header-visible-height <=> header.height;
        Rectangle {
            x: 0px; y: 0px; width: parent.width; height: root.available-height; clip: true;
            list := ListView {
                width: parent.width; height: parent.height;
                for index in root.rows.length + 1: Rectangle {
                    height: index == 0 ? header.header-height : root.rows[index - 1] * 1px;
                    background: index == 0 ? transparent : #e8edf4;
                }
            }
            header := ListHeader {
                x: 0px; y: 0px; width: list.visible-width;
                list-height: list.visible-height; list-content-height: list.content-height; content-y <=> list.content-y;
                MField { text <=> root.query; }
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
    assert_eq!(probe.get_horizontal_extent(), probe.get_visible_width());
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
    assert_eq!(probe.get_horizontal_extent(), probe.get_visible_width());
    assert_eq!(pixels[5 * 900 + 700], slint::Rgb8Pixel::new(255, 0, 0));
    assert_eq!(pixels[50 * 900 + 700], slint::Rgb8Pixel::new(0, 255, 0));
    assert_eq!(pixels[110 * 900 + 700], slint::Rgb8Pixel::new(0, 0, 255));
    probe.hide().unwrap();
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
    assert!(
        drag_probe.get_started(),
        "holding the handle must start reordering after the long-press threshold"
    );
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
    drag_probe.hide().unwrap();
    drop(drag_probe);

    let desktop = DesktopDragProbe::new().unwrap();
    desktop.show().unwrap();
    frame(&adapter, 300, 240);
    let press = || {
        desktop
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position: LogicalPosition::new(12., 24.),
                button: PointerEventButton::Left,
            })
    };
    let release = || {
        desktop
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position: LogicalPosition::new(12., 184.),
                button: PointerEventButton::Left,
            })
    };
    press();
    assert!(
        desktop.get_started(),
        "desktop dragging must not require a long press"
    );
    desktop.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(12., 184.),
    });
    release();
    assert_eq!(desktop.get_delta(), 2);
    desktop.set_delta(99);
    press();
    desktop.window().dispatch_event(WindowEvent::KeyPressed {
        text: slint::platform::Key::Escape.into(),
    });
    release();
    assert!(desktop.get_cancelled());
    assert_eq!(
        desktop.get_delta(),
        99,
        "Escape must cancel rather than drop"
    );
    press();
    desktop.set_context("filtered".into());
    release();
    assert_eq!(
        desktop.get_delta(),
        99,
        "filtering during a desktop drag must not reorder a replacement row"
    );
    desktop.set_handle_enabled(false);
    frame(&adapter, 300, 240);
    press();
    release();
    assert_eq!(desktop.get_delta(), 99, "a busy handle must not reorder");
    desktop.hide().unwrap();
    drop(desktop);

    for hold_millis in [550, 0] {
        let scrolling = ReorderScrollProbe::new().unwrap();
        scrolling.set_hold_duration(hold_millis);
        scrolling.show().unwrap();
        frame(&adapter, 300, 240);
        let pointer = |pressed: bool, y: f32| {
            let position = LogicalPosition::new(12., y);
            let event = if pressed {
                WindowEvent::PointerPressed {
                    position,
                    button: PointerEventButton::Left,
                }
            } else {
                WindowEvent::PointerReleased {
                    position,
                    button: PointerEventButton::Left,
                }
            };
            scrolling.window().dispatch_event(event);
        };
        pointer(true, 24.);
        advance(&clock, Duration::from_millis(700));
        assert!(scrolling.get_dragging());
        assert_eq!(scrolling.get_source(), 0);
        scrolling
            .window()
            .dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(12., 232.),
            });
        advance(&clock, Duration::from_secs(10));
        frame(&adapter, 300, 240);
        assert!(scrolling.get_scroll_y() < -800.);
        assert_eq!(
            scrolling.get_target(),
            39,
            "edge scrolling must reach the final rule"
        );
        assert!(
            scrolling.get_dragging(),
            "recycling the source row must retain the gesture"
        );
        let near_bottom = scrolling.get_scroll_y();
        let lower_target = scrolling.get_target();
        scrolling
            .window()
            .dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(12., 8.),
            });
        advance(&clock, Duration::from_secs(1));
        assert!(scrolling.get_scroll_y() > near_bottom);
        assert!(scrolling.get_target() < lower_target);
        scrolling
            .window()
            .dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(12., 120.),
            });
        let centered = scrolling.get_scroll_y();
        advance(&clock, Duration::from_millis(500));
        assert_eq!(
            scrolling.get_scroll_y(),
            centered,
            "the center must not auto-scroll"
        );
        pointer(false, 120.);
        assert_eq!(scrolling.get_source(), 0);
        assert_eq!(scrolling.get_drop_count(), 1);
        assert!(!scrolling.get_dragging());
        advance(&clock, Duration::from_millis(700));
        assert_eq!(scrolling.get_scroll_y(), centered);

        pointer(true, 120.);
        advance(&clock, Duration::from_millis(700));
        assert!(scrolling.get_dragging());
        scrolling.set_context("filtered".into());
        assert!(!scrolling.get_dragging());
        pointer(false, 120.);
        assert_eq!(
            scrolling.get_drop_count(),
            1,
            "a context change must cancel the pending drop"
        );
        pointer(true, 120.);
        advance(&clock, Duration::from_millis(700));
        assert!(scrolling.get_dragging());
        scrolling.set_reorder_enabled(false);
        assert!(!scrolling.get_dragging());
        pointer(false, 120.);
        assert_eq!(
            scrolling.get_drop_count(),
            1,
            "disabling a gesture must cancel the pending drop"
        );
        scrolling.set_reorder_enabled(true);
        if hold_millis > 0 {
            pointer(true, 120.);
            scrolling
                .window()
                .dispatch_event(WindowEvent::PointerMoved {
                    position: LogicalPosition::new(12., 200.),
                });
            advance(&clock, Duration::from_millis(700));
            assert!(
                !scrolling.get_dragging(),
                "an ordinary swipe must not start reordering"
            );
            assert!(scrolling.get_scroll_y() > centered);
            pointer(false, 200.);
            assert_eq!(scrolling.get_drop_count(), 1);
        }
        scrolling.hide().unwrap();
        drop(scrolling);
    }

    let header = ReorderScrollProbe::new().unwrap();
    header.set_header_height(120.);
    header.show().unwrap();
    frame(&adapter, 300, 240);
    header.window().dispatch_event(WindowEvent::PointerPressed {
        position: LogicalPosition::new(12., 24.),
        button: PointerEventButton::Left,
    });
    advance(&clock, Duration::from_millis(700));
    assert!(
        !header.get_dragging(),
        "inline toolbars must not start a rule drag"
    );
    header
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(12., 24.),
            button: PointerEventButton::Left,
        });
    header.window().dispatch_event(WindowEvent::PointerPressed {
        position: LogicalPosition::new(12., 144.),
        button: PointerEventButton::Left,
    });
    advance(&clock, Duration::from_millis(700));
    assert!(header.get_dragging());
    assert_eq!(header.get_source(), 0);
    header
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(12., 144.),
            button: PointerEventButton::Left,
        });
    assert_eq!(header.get_drop_count(), 1);
    header.hide().unwrap();

    adapter.set_size(slint::PhysicalSize::new(300, 600));
    let search = HeaderSearchProbe::new().unwrap();
    let search_rows = Rc::new(slint::VecModel::from(vec![80.; 8]));
    search.set_rows(search_rows.clone().into());
    search.show().unwrap();
    frame(&adapter, 300, 600);
    for event in [
        WindowEvent::PointerPressed {
            position: LogicalPosition::new(24., 24.),
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position: LogicalPosition::new(24., 24.),
            button: PointerEventButton::Left,
        },
    ] {
        search.window().dispatch_event(event);
    }
    advance(&clock, Duration::from_millis(100));
    let type_text = |text: &str| {
        search
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
        search
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: text.into() });
    };
    type_text("a");
    assert_eq!(search.get_query().as_str(), "a");
    search
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(24., 24.),
            delta_x: 0.,
            delta_y: -120.,
        });
    frame(&adapter, 300, 600);
    assert!(
        search.get_scroll_y() < 0.,
        "scrolling over the search header must scroll the shared list"
    );
    assert_eq!(search.get_header_visible_height(), 0.);
    search
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(24., 24.),
            delta_x: 0.,
            delta_y: 120.,
        });
    frame(&adapter, 300, 600);
    assert_eq!(search.get_scroll_y(), 0.);
    search.set_available_height(240.);
    search_rows.set_vec(vec![80.; 4]);
    frame(&adapter, 300, 600);
    type_text("b");
    assert_eq!(
        search.get_query().as_str(),
        "ab",
        "keyboard resizing and replacing virtual rows must retain search focus"
    );
    search.set_scroll_y(-80.);
    frame(&adapter, 300, 600);
    assert_eq!(search.get_header_visible_height(), 0.);
    search_rows.set_vec(vec![]);
    frame(&adapter, 300, 600);
    advance(&clock, Duration::from_millis(50));
    type_text("c");
    assert_eq!(
        search.get_query().as_str(),
        "abc",
        "empty search results must retain the editable header"
    );
    assert!(search.get_header_visible_height() >= 48.);
    search.hide().unwrap();
}
