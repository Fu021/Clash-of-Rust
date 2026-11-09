//! Exercise production virtual lists and rule gestures with a software window.
use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        WindowEventDispatchResult,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

slint::slint! {
    import { ListView, ListHeader, MField, MDragHandle, MButton } from "../ui/mobile/shared/controls.slint";
    import { Field, PageScroll } from "../ui/components/controls.slint";
    import { RuleDragHandle } from "../ui/components/rule-drag.slint";
    import { ListReorderGesture, ListReorderPreview } from "../ui/components/list-reorder.slint";
    import { PageSurface } from "../ui/components/motion.slint";
    import { MobileFrame } from "../ui/mobile/shared/frame.slint";
    import { MobileView } from "../ui/mobile/shared/state.slint";
    import { MobileData } from "../ui/mobile/shared/state.slint";
    import { MobileSubscriptions } from "../ui/mobile/shared/pages.slint";
    import { PageViews } from "../ui/components/view-toggle.slint";
    import { NoticeCountdown } from "../ui/components/motion.slint";
    export { ProfileItem } from "../ui/state.slint";
    export { MobileActions, MobileIntent } from "../ui/mobile/shared/state.slint";
    import { SettingsPage, SettingsNavigation } from "../ui/pages/settings.slint";
    import { UiData, ProfileItem } from "../ui/state.slint";
    import { Theme } from "../ui/theme/palette.slint";
    export { UiGeometry } from "../ui/components/geometry.slint";

    export component CompactButtonProbe inherits Window {
        preferred-width: 200px; preferred-height: 100px;
        background: #010203;
        in-out property <bool> enabled: true;
        out property <int> clicks;
        out property <length> target-height: action.height;
        action := MButton {
            x: 20px; y: 20px; width: 80px; text: "使用中";
            compact: true; primary: true; enabled: root.enabled;
            clicked => { root.clicks += 1; }
        }
    }

    export component CountdownProbe inherits Window {
        preferred-width: 300px; preferred-height: 80px;
        in-out property <float> elapsed <=> countdown.elapsed;
        in-out property <int> revision <=> countdown.revision;
        out property <float> remaining <=> countdown.value;
        countdown := NoticeCountdown { width: parent.width; track: #333; fill: #abc; }
    }

    export component MobileProfilesProbe inherits Window {
        preferred-width: 740px; preferred-height: 1100px;
        in-out property <[ProfileItem]> profiles <=> MobileData.profiles;
        in-out property <bool> list-view <=> PageViews.profiles-list;
        in-out property <string> selected-id <=> MobileView.selected-id;
        in-out property <int> detail <=> MobileView.detail;
        MobileSubscriptions { width: parent.width; height: parent.height; }
    }

    export component SettingsScrollProbe inherits Window {
        title: "SettingsScrollProbe";
        preferred-width: 740px; preferred-height: 360px;
        in-out property <length> offset <=> UiData.scroll-settings;
        in-out property <string> draft <=> UiData.mixed-port;
        out property <color> selected-color: Theme.accent-wash;
        property <int> navigation-request: -1;
        VerticalLayout {
            SettingsNavigation {
                active-section: settings.active-section;
                chosen(index) => { root.navigation-request = index; }
            }
            settings := SettingsPage {
                navigation-request <=> root.navigation-request;
            }
        }
    }

    export component DesktopScrollProbe inherits Window {
        preferred-width: 500px; preferred-height: 240px;
        in property <int> rows: 20;
        out property <length> offset <=> list.content-y;
        out property <length> extent: list.content-height;
        list := PageScroll {
            width: parent.width; height: parent.height;
            VerticalLayout {
                alignment: start; spacing: 0px;
                if root.rows > 0: VerticalLayout {
                    spacing: 0px;
                    for index in root.rows: Rectangle { height: 48px; }
                }
                Rectangle { height: 40px; }
            }
        }
    }

    export component MobileNoticeProbe inherits Window {
        preferred-width: 393px; preferred-height: 852px;
        forward-focus: surface;
        in-out property <string> notice <=> MobileView.notice;
        in-out property <bool> notice-error <=> MobileView.notice-error;
        in-out property <bool> notice-critical <=> MobileView.notice-critical;
        in-out property <bool> recover-network <=> MobileView.notice-recover-network;
        in-out property <bool> vpn-connected <=> MobileData.vpn-connected;
        in-out property <bool> core-running <=> MobileData.core-running;
        out property <[length]> close-target <=> surface.notice-close-target;
        in-out property <int> detail <=> MobileView.detail;
        in-out property <string> draft <=> MobileView.profile-name;
        public function notify(message: string) { MobileView.notify(message); }
        surface := MobileFrame {
            width: parent.width; height: parent.height;
            primary-pages: [0, 1, 2, 10, 9];
            sidebar-pages: [0, 1, 2, 3, 4, 5, 6, 7, 8];
            sidebar-footer-pages: [9];
        }
    }

    export component InputFieldsProbe inherits Window {
        preferred-width: 320px; preferred-height: 160px;
        in-out property <string> mobile-text <=> mobile.text;
        in-out property <string> desktop-text <=> desktop.text;
        out property <bool> mobile-placeholder: mobile.placeholder-visible;
        out property <bool> desktop-placeholder: desktop.placeholder-visible;
        public function focus-field(use-mobile: bool) {
            if use-mobile { mobile.focus(); } else { desktop.focus(); }
        }
        mobile := MField {
            x: 0px; y: 0px; width: parent.width;
            placeholder: "Mobile placeholder";
        }
        desktop := Field {
            x: 0px; y: 80px; width: parent.width;
            placeholder: "Desktop placeholder";
        }
    }

    export component LockedPageProbe inherits Window {
        preferred-width: 300px; preferred-height: 240px;
        in property <bool> locked;
        out property <int> clicks;
        out property <length> scroll-y <=> list.content-y;
        PageSurface {
            locked: root.locked;
            list := ListView {
                width: parent.width; height: parent.height;
                for index in 10: Rectangle {
                    height: 80px;
                    TouchArea { clicked => { root.clicks += 1; } }
                }
            }
        }
    }

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
        in-out property <length> scroll-y <=> list.content-y;
        out property <bool> dragging <=> gesture.reordering;
        out property <int> preview-index <=> gesture.drag-index;
        out property <length> preview-y <=> preview.preview-y;
        out property <int> source: -1;
        out property <int> target: -1;
        out property <int> drop-count;
        // The actual list uses recycled rows; the gesture lives beside it.
        list := ListView {
            width: root.width; height: root.height;
            for index in 41: Rectangle {
                height: index == 0 ? root.header-height : 80px;
                background: index == 1 ? #ff0000 : #e8edf4;
                opacity: gesture.reordering && index == gesture.drag-index + 1 ? 0.25 : 1;
            }
        }
        preview := ListReorderPreview {
            x: 0px; y: 0px; width: root.width; height: list.visible-height;
            active: gesture.reordering; row-y: gesture.drag-y; row-height: 80px;
            Rectangle { x: 0px; y: 0px; width: parent.width; height: 80px; background: #ff0000; }
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

fn headless(width: u32, height: u32) -> (Rc<MinimalSoftwareWindow>, Rc<Cell<Duration>>) {
    let adapter = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(Headless(adapter.clone(), clock.clone()))).unwrap();
    adapter.set_size(slint::PhysicalSize::new(width, height));
    (adapter, clock)
}

#[test]
fn mobile_list_stacks_rows_and_keeps_sparse_content_at_the_top() {
    let (adapter, _clock) = headless(300, 240);
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
}

#[test]
fn mobile_drag_waits_for_long_press_and_cancels_a_short_tap() {
    let (adapter, clock) = headless(300, 240);
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
}

#[test]
fn desktop_drag_starts_immediately_and_respects_busy_state() {
    let (adapter, _clock) = headless(300, 240);
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
}

#[test]
fn reorder_scroll_preserves_the_lifted_row_and_cancels_stale_content() {
    let (adapter, clock) = headless(300, 240);
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
        if hold_millis > 0 {
            assert_eq!(
                scrolling.get_preview_index(),
                -1,
                "the row must stay in place before the long press"
            );
        }
        advance(&clock, Duration::from_millis(700));
        assert!(scrolling.get_dragging());
        assert_eq!(scrolling.get_source(), 0);
        assert_eq!(scrolling.get_preview_index(), 0);
        assert_eq!(scrolling.get_preview_y(), 0.);
        scrolling
            .window()
            .dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(12., 120.),
            });
        assert_eq!(
            scrolling.get_preview_y(),
            96.,
            "the lifted row must preserve the original pointer offset"
        );
        let pixels = frame(&adapter, 300, 240);
        assert_eq!(
            pixels[120 * 300 + 100],
            slint::Rgb8Pixel::new(255, 0, 0),
            "the actual lifted row must render at the pointer"
        );
        assert_ne!(
            pixels[24 * 300 + 100],
            slint::Rgb8Pixel::new(255, 0, 0),
            "the source row must remain as a dim placeholder"
        );
        assert_ne!(
            pixels[200 * 300 + 100],
            slint::Rgb8Pixel::new(255, 0, 0),
            "the preview must use the row height and leave later rows visible"
        );
        scrolling
            .window()
            .dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(12., 232.),
            });
        let lifted_y = scrolling.get_preview_y();
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
        assert_eq!(scrolling.get_preview_index(), 0);
        assert_eq!(
            scrolling.get_preview_y(),
            lifted_y,
            "auto-scroll must not pull the lifted row away from the pointer"
        );
        let pixels = frame(&adapter, 300, 240);
        assert_eq!(
            pixels[200 * 300 + 100],
            slint::Rgb8Pixel::new(255, 0, 0),
            "the preview must survive recycling the source row"
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
        assert_eq!(scrolling.get_preview_index(), -1);
        let pixels = frame(&adapter, 300, 240);
        assert_ne!(
            pixels[120 * 300 + 100],
            slint::Rgb8Pixel::new(255, 0, 0),
            "dropping must remove the lifted row"
        );
        advance(&clock, Duration::from_millis(700));
        assert_eq!(scrolling.get_scroll_y(), centered);

        pointer(true, 120.);
        advance(&clock, Duration::from_millis(700));
        assert!(scrolling.get_dragging());
        scrolling.set_context("filtered".into());
        assert!(!scrolling.get_dragging());
        assert_eq!(scrolling.get_preview_index(), -1);
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
        assert_eq!(scrolling.get_preview_index(), -1);
        pointer(false, 120.);
        assert_eq!(
            scrolling.get_drop_count(),
            1,
            "disabling a gesture must cancel the pending drop"
        );
        scrolling.set_reorder_enabled(true);
        pointer(true, 120.);
        advance(&clock, Duration::from_millis(700));
        assert!(scrolling.get_dragging());
        scrolling.window().dispatch_event(WindowEvent::KeyPressed {
            text: slint::platform::Key::Escape.into(),
        });
        assert!(!scrolling.get_dragging());
        assert_eq!(scrolling.get_preview_index(), -1);
        pointer(false, 120.);
        assert_eq!(scrolling.get_drop_count(), 1, "Escape must cancel the drop");
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
            pointer(true, 120.);
            scrolling.set_scroll_y(scrolling.get_scroll_y() - 16.);
            advance(&clock, Duration::from_millis(700));
            assert!(
                !scrolling.get_dragging(),
                "scrolling before the long press must cancel the pending lift"
            );
            assert_eq!(scrolling.get_preview_index(), -1);
            pointer(false, 120.);
            assert_eq!(scrolling.get_drop_count(), 1);
        }
        scrolling.hide().unwrap();
        drop(scrolling);
    }
}

#[test]
fn reorder_header_is_excluded_from_drag_targets() {
    let (adapter, clock) = headless(300, 240);
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
    assert_eq!(
        header.get_preview_y(),
        120.,
        "the toolbar height must be included in the lifted row origin"
    );
    header
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(12., 144.),
            button: PointerEventButton::Left,
        });
    assert_eq!(header.get_drop_count(), 1);
    header.hide().unwrap();
}

#[test]
fn search_header_retains_focus_during_keyboard_and_model_changes() {
    let (adapter, clock) = headless(300, 600);
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
    search.set_available_height(240.);
    search_rows.set_vec(vec![80.; 4]);
    frame(&adapter, 300, 600);
    type_text("b");
    assert_eq!(
        search.get_query().as_str(),
        "ab",
        "keyboard resizing and replacing virtual rows must retain search focus"
    );
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

    // Slint releases focus when a field scrolls out of view. Exercise scrolling
    // separately from keyboard/model changes that must retain an editable field.
    search.set_available_height(600.);
    search_rows.set_vec(vec![80.; 8]);
    frame(&adapter, 300, 600);
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
    search.set_scroll_y(-80.);
    frame(&adapter, 300, 600);
    assert_eq!(search.get_header_visible_height(), 0.);
    search_rows.set_vec(vec![]);
    frame(&adapter, 300, 600);
    assert!(search.get_header_visible_height() >= 48.);
    search.hide().unwrap();
}

#[test]
fn locked_pages_block_clicks_and_scrolling() {
    let (adapter, _clock) = headless(300, 240);
    let locked = LockedPageProbe::new().unwrap();
    locked.show().unwrap();
    frame(&adapter, 300, 240);
    let click = || {
        for event in [
            WindowEvent::PointerPressed {
                position: LogicalPosition::new(20., 20.),
                button: PointerEventButton::Left,
            },
            WindowEvent::PointerReleased {
                position: LogicalPosition::new(20., 20.),
                button: PointerEventButton::Left,
            },
        ] {
            locked.window().dispatch_event(event);
        }
    };
    click();
    assert_eq!(locked.get_clicks(), 1);
    locked.set_locked(true);
    frame(&adapter, 300, 240);
    click();
    locked
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(20., 20.),
            delta_x: 0.,
            delta_y: -80.,
        });
    frame(&adapter, 300, 240);
    assert_eq!(
        locked.get_clicks(),
        1,
        "locked desktop/mobile pages must swallow pointer input"
    );
    assert_eq!(locked.get_scroll_y(), 0., "locked pages must not scroll");
    locked.set_locked(false);
    frame(&adapter, 300, 240);
    click();
    assert_eq!(locked.get_clicks(), 2);
    locked.hide().unwrap();
}

#[test]
fn input_composition_hides_placeholders_on_mobile_and_desktop() {
    let (adapter, _clock) = headless(320, 160);
    let fields = InputFieldsProbe::new().unwrap();
    fields.show().unwrap();
    for mobile in [true, false] {
        fields.invoke_focus_field(mobile);
        let placeholder = || {
            if mobile {
                fields.get_mobile_placeholder()
            } else {
                fields.get_desktop_placeholder()
            }
        };
        let text = || {
            if mobile {
                fields.get_mobile_text()
            } else {
                fields.get_desktop_text()
            }
        };
        let composition = |preedit: &str, committed: &str, commit: bool| {
            let mut key_event = i_slint_core::input::KeyEvent::default();
            key_event.text = committed.into();
            fields.window().dispatch_event(WindowEvent::internal(
                i_slint_core::input::InternalKeyEvent {
                    event_type: if commit {
                        i_slint_core::input::KeyEventType::CommitComposition
                    } else {
                        i_slint_core::input::KeyEventType::UpdateComposition
                    },
                    preedit_text: preedit.into(),
                    key_event,
                    ..Default::default()
                },
            ));
            frame(&adapter, 320, 160);
        };
        assert!(placeholder());
        composition("pin", "", false);
        assert_eq!(text(), "");
        assert!(
            !placeholder(),
            "uncommitted IME text must hide the placeholder on both hosts"
        );
        composition("", "", false);
        assert!(placeholder(), "cancelled composition must restore the hint");
        composition("拼", "", false);
        assert!(!placeholder());
        composition("", "拼音 🙂", true);
        assert_eq!(text(), "拼音 🙂");
        assert!(!placeholder());
        if mobile {
            fields.set_mobile_text("".into());
        } else {
            fields.set_desktop_text("".into());
        }
        frame(&adapter, 320, 160);
        assert!(
            placeholder(),
            "clearing committed text must restore the hint"
        );
    }
    fields.hide().unwrap();
}

#[test]
fn notice_countdown_restarts_and_expires_after_five_seconds() {
    let (adapter, clock) = headless(300, 80);
    let countdown = CountdownProbe::new().unwrap();
    countdown.show().unwrap();
    advance(&clock, Duration::from_millis(2500));
    assert!((countdown.get_remaining() - 0.5).abs() < 0.03);
    countdown.set_revision(1);
    advance(&clock, Duration::ZERO);
    frame(&adapter, 300, 80);
    assert_eq!(countdown.get_remaining(), 1.);
    countdown.set_elapsed(4.);
    advance(&clock, Duration::ZERO);
    frame(&adapter, 300, 80);
    assert!((countdown.get_remaining() - 0.2).abs() < 0.001);
    advance(&clock, Duration::from_millis(1100));
    assert_eq!(countdown.get_remaining(), 0.);
    countdown.hide().unwrap();
    drop(countdown);
}

#[test]
fn mobile_notices_reflow_and_preserve_the_underlying_draft() {
    let (adapter, clock) = headless(393, 852);
    let notice = MobileNoticeProbe::new().unwrap();
    notice.set_detail(3);
    notice.set_draft("Unchanged subscription draft".into());
    notice.show().unwrap();
    notice.set_notice_error(false);
    notice.set_notice_critical(false);
    notice.invoke_notify("已保存".into());
    frame(&adapter, 393, 852);
    advance(&clock, Duration::from_secs(4));
    assert_eq!(notice.get_notice(), "已保存");
    notice.invoke_notify("已保存".into());
    advance(&clock, Duration::from_secs(4));
    assert_eq!(notice.get_notice(), "已保存");
    advance(&clock, Duration::from_millis(1100));
    assert!(notice.get_notice().is_empty());
    for critical in [false, true] {
        notice.set_notice_error(true);
        notice.set_notice_critical(critical);
        notice.set_notice("A recoverable or critical error".into());
        frame(&adapter, 393, 852);
        for recovery in [true, false, true] {
            notice.set_recover_network(recovery);
            notice.set_vpn_connected(recovery);
            notice.set_core_running(recovery);
            for width in [320, 740, 393] {
                adapter.set_size(slint::PhysicalSize::new(width, 852));
                advance(&clock, Duration::from_millis(250));
                frame(&adapter, width as usize, 852);
                let close = notice.get_close_target();
                assert_eq!(close.row_count(), 4);
                let coordinates = close.iter().collect::<Vec<f32>>();
                assert!(coordinates[2] > 0. && coordinates[3] >= 48.);
                assert!(coordinates[0] >= 0. && coordinates[1] >= 0.);
                assert!(coordinates[0] + coordinates[2] <= width as f32 + 1.);
                assert!(coordinates[1] + coordinates[3] <= 853.);
            }
        }
        advance(&clock, Duration::from_secs(6));
        assert!(!notice.get_notice().is_empty());
        let pressed = notice
            .window()
            .dispatch_event_with_result(WindowEvent::KeyPressed {
                text: slint::platform::Key::Back.into(),
            })
            .unwrap();
        let released = notice
            .window()
            .dispatch_event_with_result(WindowEvent::KeyReleased {
                text: slint::platform::Key::Back.into(),
            })
            .unwrap();
        assert!(
            matches!(pressed, WindowEventDispatchResult::Accepted)
                || matches!(released, WindowEventDispatchResult::Accepted),
            "Android Back must be consumed instead of finishing the Activity"
        );
        assert!(notice.get_notice().is_empty());
        assert_eq!(notice.get_detail(), 3);
        assert_eq!(notice.get_draft().as_str(), "Unchanged subscription draft");
    }
    notice.hide().unwrap();
    drop(notice);
}

#[test]
fn desktop_scroll_clamps_after_content_and_viewport_changes() {
    let (adapter, clock) = headless(500, 240);
    let desktop = DesktopScrollProbe::new().unwrap();
    desktop.show().unwrap();
    frame(&adapter, 500, 240);
    assert!(desktop.get_extent() >= 1000.);
    let wheel = |delta_y| {
        desktop
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(100., 100.),
                delta_x: 0.,
                delta_y,
            });
    };
    wheel(-10000.);
    advance(&clock, Duration::from_secs(1));
    frame(&adapter, 500, 240);
    assert_eq!(desktop.get_offset(), 240. - desktop.get_extent());
    wheel(10000.);
    advance(&clock, Duration::from_secs(1));
    frame(&adapter, 500, 240);
    assert_eq!(desktop.get_offset(), 0.);
    wheel(-200.);
    advance(&clock, Duration::from_secs(1));
    frame(&adapter, 500, 240);
    desktop.set_rows(0);
    frame(&adapter, 500, 240);
    assert_eq!(desktop.get_extent(), 240.);
    assert_eq!(desktop.get_offset(), 0.);
    desktop.set_rows(20);
    frame(&adapter, 500, 240);
    wheel(-200.);
    advance(&clock, Duration::from_secs(1));
    frame(&adapter, 500, 240);
    assert!(desktop.get_offset() < 0.);
    adapter.set_size(slint::PhysicalSize::new(500, 1400));
    frame(&adapter, 500, 1400);
    assert_eq!(desktop.get_offset(), 0.);
    desktop.hide().unwrap();
    drop(desktop);
}

#[test]
fn settings_navigation_tracks_scroll_and_preserves_drafts() {
    let (adapter, clock) = headless(740, 360);
    let settings = SettingsScrollProbe::new().unwrap();
    settings.set_draft("7897".into());
    let targets = Rc::new(RefCell::new(std::collections::BTreeMap::new()));
    let measured_targets = targets.clone();
    settings
        .global::<UiGeometry>()
        .on_measured(move |key, bounds| {
            measured_targets
                .borrow_mut()
                .insert(key.to_string(), bounds.iter().collect::<Vec<f32>>());
        });
    let geometry = settings.global::<UiGeometry>();
    geometry.set_revision(geometry.get_revision().wrapping_add(1));
    settings.show().unwrap();
    frame(&adapter, 740, 360);
    advance(&clock, Duration::from_secs(1));
    let active_section = |pixels: &[slint::Rgb8Pixel]| {
        let selected = settings.get_selected_color();
        let selected = slint::Rgb8Pixel::new(selected.red(), selected.green(), selected.blue());
        let bounds = targets.borrow();
        let highlighted: Vec<_> = ["常规", "网络", "外观", "更新", "计划任务"]
            .iter()
            .enumerate()
            .filter_map(|(index, _)| {
                let target = &bounds[&format!("settings-section:{index}")];
                let horizontal = (target[0] + 5.) as usize;
                let vertical = (target[1] + target[3] / 2.) as usize;
                (pixels[vertical * 740 + horizontal] == selected).then_some(index)
            })
            .collect();
        assert_eq!(highlighted.len(), 1);
        highlighted[0]
    };
    assert_eq!(active_section(&frame(&adapter, 740, 360)), 0);
    for (delta_y, expected) in [(-48., vec![0, 1, 2, 3, 4]), (48., vec![4, 3, 2, 1, 0])] {
        let mut visited = vec![active_section(&frame(&adapter, 740, 360))];
        for _ in 0..80 {
            let previous = settings.get_offset();
            settings
                .window()
                .dispatch_event(WindowEvent::PointerScrolled {
                    position: LogicalPosition::new(600., 180.),
                    delta_x: 0.,
                    delta_y,
                });
            advance(&clock, Duration::from_secs(1));
            let active = active_section(&frame(&adapter, 740, 360));
            if active != *visited.last().unwrap() {
                visited.push(active);
            }
            if settings.get_offset() == previous {
                break;
            }
        }
        assert_eq!(visited, expected);
    }
    for expected in [0, 1, 2] {
        let bounds = targets.borrow()[&format!("settings-section:{expected}")].clone();
        let position = LogicalPosition::new(bounds[0] + bounds[2] / 2., bounds[1] + bounds[3] / 2.);
        for event in [
            WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            },
            WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            },
        ] {
            settings.window().dispatch_event(event);
        }
        advance(&clock, Duration::from_secs(1));
        assert_eq!(active_section(&frame(&adapter, 740, 360)), expected);
    }
    adapter.set_size(slint::PhysicalSize::new(740, 2400));
    frame(&adapter, 740, 2400);
    advance(&clock, Duration::from_secs(1));
    let pixels = frame(&adapter, 740, 2400);
    assert_eq!(settings.get_offset(), 0.);
    assert_eq!(active_section(&pixels), 0);
    assert_eq!(settings.get_draft(), "7897");
    settings.hide().unwrap();
    drop(settings);
}

#[test]
fn compact_mobile_buttons_keep_their_touch_targets() {
    let (adapter, _clock) = headless(200, 100);
    let compact = CompactButtonProbe::new().unwrap();
    compact.show().unwrap();
    let pixels = frame(&adapter, 200, 100);
    assert!(compact.get_target_height() >= 48.);
    assert_eq!(pixels[21 * 200 + 60], slint::Rgb8Pixel::new(1, 2, 3));
    assert_ne!(pixels[26 * 200 + 60], slint::Rgb8Pixel::new(1, 2, 3));
    for enabled in [true, false] {
        compact.set_enabled(enabled);
        for event in [
            WindowEvent::PointerPressed {
                position: LogicalPosition::new(60., 21.),
                button: PointerEventButton::Left,
            },
            WindowEvent::PointerReleased {
                position: LogicalPosition::new(60., 21.),
                button: PointerEventButton::Left,
            },
        ] {
            compact.window().dispatch_event(event);
        }
        assert_eq!(compact.get_clicks(), 1);
    }
    compact.hide().unwrap();
    drop(compact);
}

#[test]
fn mobile_subscriptions_reflow_and_keep_local_profile_actions() {
    let (adapter, clock) = headless(740, 1100);
    let profiles = MobileProfilesProbe::new().unwrap();
    let profile_items = Rc::new(slint::VecModel::from(vec![
        ProfileItem {
            id: "imported".into(),
            name: "Imported file".into(),
            local: true,
            active: true,
            ..Default::default()
        },
        ProfileItem {
            id: "builtin".into(),
            name: "默认直连".into(),
            source: "default.yaml".into(),
            local: true,
            builtin: true,
            ..Default::default()
        },
    ]));
    profiles.set_profiles(profile_items.clone().into());
    let targets = Rc::new(RefCell::new(std::collections::BTreeMap::new()));
    let measured_targets = targets.clone();
    profiles
        .global::<UiGeometry>()
        .on_measured(move |key, bounds| {
            measured_targets
                .borrow_mut()
                .insert(key.to_string(), bounds.iter().collect::<Vec<f32>>());
        });
    let events = Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    profiles
        .global::<MobileActions>()
        .on_request(move |intent, first, second| {
            recorded.borrow_mut().push((intent, first, second))
        });
    profiles.show().unwrap();
    advance(&clock, Duration::from_secs(1));
    frame(&adapter, 740, 1100);
    let mut renamed = profile_items.row_data(0).unwrap();
    renamed.name = "Renamed imported file".into();
    profile_items.set_row_data(0, renamed);
    profile_items.push(ProfileItem {
        id: "temporary".into(),
        name: "Temporary remote subscription".into(),
        ..Default::default()
    });
    advance(&clock, Duration::from_secs(1));
    frame(&adapter, 740, 1100);
    {
        let targets = targets.borrow();
        let first = &targets["profile:imported"];
        let third = &targets["profile:temporary"];
        assert!(third[1] >= first[1] + first[3]);
        assert!((third[0] - first[0]).abs() < 1.);
    }
    profile_items.remove(2);
    let restored = profile_items.iter().collect::<Vec<_>>();
    profile_items.set_vec(Vec::new());
    advance(&clock, Duration::from_secs(1));
    frame(&adapter, 740, 1100);
    profile_items.set_vec(restored);
    advance(&clock, Duration::from_secs(1));
    frame(&adapter, 740, 1100);
    let bounds = |key: &str| targets.borrow()[key].clone();
    for width in [320, 740] {
        adapter.set_size(slint::PhysicalSize::new(width, 1100));
        advance(&clock, Duration::from_secs(1));
        frame(&adapter, width as usize, 1100);
        let first = bounds("profile:imported");
        let second = bounds("profile:builtin");
        assert!(first[0] >= 0. && first[0] + first[2] <= width as f32 + 1.);
        assert!(second[0] >= 0. && second[0] + second[2] <= width as f32 + 1.);
        if width == 320 {
            assert!(second[1] >= first[1] + first[3]);
        } else {
            assert!((first[1] - second[1]).abs() < 1.);
        }
    }
    let click = |key: &str| {
        let bounds = bounds(key);
        let position = LogicalPosition::new(bounds[0] + bounds[2] / 2., bounds[1] + bounds[3] / 2.);
        profiles
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
        advance(&clock, Duration::from_millis(50));
        profiles
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        advance(&clock, Duration::from_secs(1));
        frame(&adapter, 740, 1100);
    };
    assert!((bounds("profile:imported")[1] - bounds("profile:builtin")[1]).abs() < 1.);
    for key in [
        "profile-use:imported",
        "profile-update:imported",
        "profile-update:builtin",
        "profile-edit:builtin",
    ] {
        click(key);
    }
    assert!(events.borrow().is_empty());
    assert_eq!(profiles.get_detail(), 0);
    click("profile-edit:imported");
    assert_eq!(profiles.get_detail(), 3);
    assert_eq!(profiles.get_selected_id(), "imported");
    profiles.set_detail(0);
    profiles.set_selected_id("".into());
    click("profiles-view");
    assert!(profiles.get_list_view());
    for width in [320, 740] {
        adapter.set_size(slint::PhysicalSize::new(width, 1100));
        advance(&clock, Duration::from_secs(1));
        frame(&adapter, width as usize, 1100);
        let first = bounds("profile:imported");
        let second = bounds("profile:builtin");
        assert!(first[0] >= 0. && first[0] + first[2] <= width as f32 + 1.);
        assert!((first[0] - second[0]).abs() < 1.);
        assert!(second[1] >= first[1] + first[3]);
    }
    let imported = bounds("profile:imported");
    let builtin = bounds("profile:builtin");
    assert!(
        imported[3] < 130.,
        "mobile list rows must stay compact: {imported:?}"
    );
    assert!((imported[0] - builtin[0]).abs() < 1.);
    assert!(builtin[1] >= imported[1] + imported[3]);
    click("profile-detail:imported");
    assert!(targets.borrow().contains_key("profile-delete:imported"));
    click("profile-detail:builtin");
    assert!(targets.borrow().contains_key("profile-delete:builtin"));
    click("profile-delete:builtin");
    assert_eq!(profiles.get_detail(), 0);
    assert!(profiles.get_selected_id().is_empty());
    assert!(events.borrow().is_empty());
    click("profile-detail:imported");
    click("profile-delete:imported");
    assert_eq!(profiles.get_detail(), 4);
    assert_eq!(profiles.get_selected_id(), "imported");
    profiles.hide().unwrap();
}
