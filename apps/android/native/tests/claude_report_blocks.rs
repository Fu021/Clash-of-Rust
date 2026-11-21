use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    time::Duration,
};

slint::slint! {
    #[edition = "1.19"]
    import { MobileReportBlocks, MobileReport } from "../../../../crates/client-ui/ui/mobile/shared/reports.slint";
    import { MobileView } from "../../../../crates/client-ui/ui/mobile/shared/state.slint";
    import { ActionButton } from "../../../../crates/client-ui/ui/components/controls.slint";
    import { PreviewReports } from "../../../../crates/client-ui/ui/preview-reports.slint";
    import { ReportItem } from "../../../../crates/client-ui/ui/models.slint";
    import { Theme } from "../../../../crates/client-ui/ui/theme/palette.slint";
    export { ReportItem } from "../../../../crates/client-ui/ui/models.slint";
    export { UiGeometry } from "../../../../crates/client-ui/ui/components/geometry.slint";
    export component ReportBlocksProbe inherits Window {
        preferred-width: 393px; preferred-height: 852px;
        in-out property <[ReportItem]> reports;
        in-out property <string> section;
        out property <[ReportItem]> preview-blocks: PreviewReports.blocks;
        in-out property <float> text-scale <=> Theme.text-scale;
        if root.section == "": MobileReportBlocks {
            width: parent.width; items: root.reports;
            show-details(section) => { root.section = section; }
        }
        if root.section != "": MobileReport {
            width: parent.width; items: root.reports; section: root.section;
        }
    }

    export component MobileControlAccessProbe inherits Window {
        preferred-width: 320px; preferred-height: 160px;
        in-out property <int> detail <=> MobileView.detail;
        in-out property <string> notice <=> MobileView.notice;
        in-out property <bool> notice-critical <=> MobileView.notice-critical;
        out property <int> clicks;
        ActionButton {
            x: 10px; y: 10px; width: 100px; text: "Test";
            clicked => { root.clicks += 1; }
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

fn frame(adapter: &MinimalSoftwareWindow, clock: &Cell<Duration>, width: u32, height: u32) {
    for _ in 0..10 {
        clock.set(clock.get() + Duration::from_millis(10));
        slint::platform::update_timers_and_animations();
    }
    let mut pixels = vec![slint::Rgb8Pixel::default(); width as usize * height as usize];
    adapter.window().request_redraw();
    assert!(adapter.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    }));
}

fn click(window: &slint::Window, bounds: &[f32]) {
    let position = LogicalPosition::new(bounds[0] + bounds[2] / 2., bounds[1] + bounds[3] / 2.);
    for event in [
        WindowEvent::PointerMoved { position },
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    ] {
        window.dispatch_event(event);
    }
}

#[test]
fn mobile_shared_controls_follow_modal_and_notice() {
    let adapter = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(Headless(adapter.clone(), clock.clone()))).unwrap();
    adapter.set_size(slint::PhysicalSize::new(320, 160));
    let probe = MobileControlAccessProbe::new().unwrap();
    probe.show().unwrap();
    frame(&adapter, &clock, 320, 160);
    let tap = || click(probe.window(), &[10., 10., 100., 40.]);
    tap();
    assert_eq!(probe.get_clicks(), 1);
    probe.set_detail(4);
    frame(&adapter, &clock, 320, 160);
    tap();
    assert_eq!(probe.get_clicks(), 1);
    probe.set_detail(0);
    frame(&adapter, &clock, 320, 160);
    tap();
    assert_eq!(probe.get_clicks(), 2);
    probe.set_notice("critical".into());
    probe.set_notice_critical(true);
    frame(&adapter, &clock, 320, 160);
    tap();
    assert_eq!(probe.get_clicks(), 2);
    probe.set_notice_critical(false);
    frame(&adapter, &clock, 320, 160);
    tap();
    assert_eq!(probe.get_clicks(), 3);
    probe.hide().unwrap();
}

#[test]
fn report_cards_open_only_the_selected_section_at_representative_sizes() {
    let adapter = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(Headless(adapter.clone(), clock.clone()))).unwrap();
    adapter.set_size(slint::PhysicalSize::new(393, 852));
    let probe = ReportBlocksProbe::new().unwrap();
    let mut rows = probe.get_preview_blocks().iter().collect::<Vec<_>>();
    assert_eq!(rows.iter().take(5).filter(|row| row.tone == 0).count(), 5);
    for index in 0..15 {
        rows.push(ReportItem {
            id: "Claude 连接与选路".into(),
            title: format!("额外连接 {index}").into(),
            value: "仅记录选路".into(),
            tone: 2,
            ..Default::default()
        });
    }
    rows[1].block_items += 15;
    let reports = Rc::new(slint::VecModel::from(rows));
    probe.set_reports(reports.clone().into());
    let targets = Rc::new(RefCell::new(BTreeMap::<String, Vec<f32>>::new()));
    let observed = targets.clone();
    probe
        .global::<UiGeometry>()
        .on_measured(move |key, bounds| {
            observed
                .borrow_mut()
                .insert(key.to_string(), bounds.iter().collect());
        });
    probe.global::<UiGeometry>().set_enabled(true);
    probe.show().unwrap();
    let render = |width, height| {
        frame(&adapter, &clock, width, height);
        targets.borrow_mut().clear();
        let geometry = probe.global::<UiGeometry>();
        geometry.set_revision(geometry.get_revision().wrapping_add(1));
        frame(&adapter, &clock, width, height);
    };
    for (width, height, scale) in [
        (360, 640, 1.),
        (360, 640, 1.3),
        (393, 852, 1.),
        (800, 360, 1.),
        (800, 1280, 1.),
        (1280, 800, 1.),
    ] {
        adapter.set_size(slint::PhysicalSize::new(width, height));
        probe.set_text_scale(scale);
        probe.set_section("".into());
        render(width, height);
        let button = targets.borrow()["claude-block:实时保护与异常拦截"].clone();
        assert!(button[3] >= 48. && button[0] + button[2] <= width as f32);
        let card = targets.borrow()["claude-card:实时保护与异常拦截"].clone();
        click(probe.window(), &[card[0] + 12., card[1] + 12., 1., 1.]);
        render(width, height);
        assert_eq!(probe.get_section(), "实时保护与异常拦截");
        let bounds = targets.borrow();
        assert!(bounds["claude-result:5"][3] > 0.);
        assert!(bounds["claude-result:6"][3] > 0.);
        assert!(bounds["claude-result:7"][3] < 1.);
        drop(bounds);
        let collapsed_height = targets.borrow()["claude-result:5"][3];
        let detail = targets.borrow()["claude-result-detail:5"].clone();
        click(probe.window(), &detail);
        render(width, height);
        let expanded_height = targets.borrow()["claude-result:5"][3];
        assert!(expanded_height > collapsed_height);
        let original = reports.row_data(5).unwrap();
        let mut updated = original.clone();
        updated.value = "已观察 9 次请求".into();
        reports.set_row_data(5, updated.clone());
        render(width, height);
        let updated_height = targets.borrow()["claude-result:5"][3];
        assert!(updated_height > collapsed_height);
        updated.title = "其他状态".into();
        reports.set_row_data(5, updated);
        render(width, height);
        assert!(targets.borrow()["claude-result:5"][3] < updated_height);
        reports.set_row_data(5, original);

        probe.set_section("Claude 连接与选路".into());
        render(width, height);
        let bounds = targets.borrow();
        assert!(bounds["claude-result:7"][3] > 0.);
        assert!(bounds["claude-result:8"][3] > 0.);
        assert!(bounds["claude-result:10"][3] < 1.);
        assert!(bounds[&format!("claude-result:{}", reports.row_count() - 1)][3] > 0.);
    }
    probe.hide().unwrap();
}
