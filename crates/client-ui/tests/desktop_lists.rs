use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

slint::slint! {
    #[edition = "1.19"]
    import { ActionButton, Field, PageScroll } from "../ui/components/controls.slint";
    import { RuleDragHandle } from "../ui/components/rule-drag.slint";
    import { SettingsPage, SettingsNavigation } from "../ui/pages/settings.slint";
    import { NetworkPage } from "../ui/pages/network.slint";
    import { UiData } from "../ui/state.slint";
    import { Theme } from "../ui/theme/palette.slint";
    export { UiGeometry } from "../ui/components/geometry.slint";

    export component DesktopNetworkToolboxProbe inherits Window {
        preferred-width: 950px; preferred-height: 700px;
        in-out property <float> text-scale <=> Theme.text-scale;
        in-out property <int> tool <=> UiData.network-tool;
        in-out property <[string]> results <=> UiData.network-results;
        NetworkPage { width: parent.width; height: parent.height; }
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

    export component DesktopInputProbe inherits Window {
        preferred-width: 320px; preferred-height: 160px;
        in-out property <string> text <=> field.text;
        out property <bool> placeholder: field.placeholder-visible;
        public function focus-field() { field.focus(); }
        field := Field {
            x: 0px; y: 80px; width: parent.width;
            placeholder: "Desktop placeholder";
        }
    }

    export component DesktopControlAccessProbe inherits Window {
        preferred-width: 320px; preferred-height: 160px;
        in-out property <int> modal <=> UiData.modal;
        in-out property <string> notice <=> UiData.notice;
        in-out property <bool> notice-critical <=> UiData.notice-critical;
        in-out property <string> focus-target <=> UiData.focus-target;
        in-out property <int> focus-revision <=> UiData.focus-revision;
        in-out property <string> text <=> field.text;
        out property <int> clicks;
        ActionButton {
            x: 10px; y: 10px; width: 100px; text: "Test";
            clicked => { root.clicks += 1; }
        }
        field := Field {
            x: 10px; y: 80px; width: 200px; focus-id: "test-field";
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
fn desktop_network_toolbox_keeps_a_stable_detail_viewport() {
    let (adapter, clock) = headless(950, 700);
    let desktop = DesktopNetworkToolboxProbe::new().unwrap();
    let targets = Rc::new(RefCell::new(
        std::collections::BTreeMap::<String, Vec<f32>>::new(),
    ));
    let observed = targets.clone();
    desktop
        .global::<UiGeometry>()
        .on_measured(move |key, bounds| {
            observed
                .borrow_mut()
                .insert(key.to_string(), bounds.iter().collect());
        });
    desktop.global::<UiGeometry>().set_enabled(true);
    desktop.set_results(Rc::new(slint::VecModel::from(vec!["结果 · 示例".into()])).into());
    desktop.show().unwrap();
    for (width, height, scale) in [(800, 450, 1.3), (950, 700, 1.), (1280, 800, 1.)] {
        adapter.set_size(slint::PhysicalSize::new(width, height));
        desktop.set_text_scale(scale);
        let mut baseline = None;
        for tool in [0, 4, 9, 10, 11] {
            desktop.set_tool(tool);
            advance(&clock, Duration::from_millis(300));
            frame(&adapter, width as usize, height as usize);
            targets.borrow_mut().clear();
            let geometry = desktop.global::<UiGeometry>();
            geometry.set_revision(geometry.get_revision().wrapping_add(1));
            frame(&adapter, width as usize, height as usize);
            let bounds = targets.borrow();
            let viewport = &bounds["network-detail-viewport"];
            let expected_width = *baseline.get_or_insert(viewport[2]);
            assert!((viewport[2] - expected_width).abs() < 2., "{viewport:?}");
            assert!(viewport[2] > width as f32 / 2., "{viewport:?}");
            assert!((viewport[0] + viewport[2] - width as f32).abs() < 2.);
        }
    }
    desktop.hide().unwrap();
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
}

#[test]
fn desktop_input_composition_hides_placeholder() {
    let (adapter, _clock) = headless(320, 160);
    let field = DesktopInputProbe::new().unwrap();
    field.show().unwrap();
    field.invoke_focus_field();
    let composition = |preedit: &str, committed: &str, commit: bool| {
        let mut key_event = i_slint_core::input::KeyEvent::default();
        key_event.text = committed.into();
        field.window().dispatch_event(WindowEvent::internal(
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
    assert!(field.get_placeholder());
    composition("pin", "", false);
    assert_eq!(field.get_text(), "");
    assert!(!field.get_placeholder());
    composition("", "", false);
    assert!(field.get_placeholder());
    composition("拼", "", false);
    assert!(!field.get_placeholder());
    composition("", "拼音 🙂", true);
    assert_eq!(field.get_text(), "拼音 🙂");
    assert!(!field.get_placeholder());
    field.set_text("".into());
    frame(&adapter, 320, 160);
    assert!(field.get_placeholder());
    field.hide().unwrap();
}

#[test]
fn desktop_shared_controls_follow_modal_notice_and_focus() {
    let (adapter, clock) = headless(320, 160);
    let probe = DesktopControlAccessProbe::new().unwrap();
    probe.show().unwrap();
    frame(&adapter, 320, 160);
    let click = || {
        for event in [
            WindowEvent::PointerPressed {
                position: LogicalPosition::new(30., 25.),
                button: PointerEventButton::Left,
            },
            WindowEvent::PointerReleased {
                position: LogicalPosition::new(30., 25.),
                button: PointerEventButton::Left,
            },
        ] {
            probe.window().dispatch_event(event);
        }
    };
    click();
    assert_eq!(probe.get_clicks(), 1);
    probe.set_modal(1);
    frame(&adapter, 320, 160);
    click();
    assert_eq!(probe.get_clicks(), 1);
    probe.set_modal(10);
    frame(&adapter, 320, 160);
    click();
    assert_eq!(probe.get_clicks(), 2);
    probe.set_modal(0);
    probe.set_notice("critical".into());
    probe.set_notice_critical(true);
    frame(&adapter, 320, 160);
    click();
    assert_eq!(probe.get_clicks(), 2);
    probe.set_notice_critical(false);
    frame(&adapter, 320, 160);
    click();
    assert_eq!(probe.get_clicks(), 3);
    probe.set_focus_target("test-field".into());
    probe.set_focus_revision(1);
    advance(&clock, Duration::from_millis(20));
    frame(&adapter, 320, 160);
    probe
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: "x".into() });
    assert_eq!(probe.get_text(), "x");
    probe.hide().unwrap();
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
    geometry.set_enabled(true);
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
}
