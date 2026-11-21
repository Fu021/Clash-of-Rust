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
    import { MobileReportBlocks } from "../ui/mobile/shared/reports.slint";
    import { PreviewReports } from "../ui/preview-reports.slint";
    import { ReportItem } from "../ui/state.slint";
    import { Theme } from "../ui/theme/palette.slint";
    export { ReportItem } from "../ui/state.slint";
    export { UiGeometry } from "../ui/components/geometry.slint";
    export component ReportBlocksProbe inherits Window {
        preferred-width: 393px; preferred-height: 852px;
        in-out property <[ReportItem]> reports;
        out property <[ReportItem]> preview-blocks: PreviewReports.blocks;
        in-out property <float> text-scale <=> Theme.text-scale;
        MobileReportBlocks { width: parent.width; items: root.reports; }
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
fn blocks_and_details_expand_without_exposing_other_sections() {
    let adapter = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(Headless(adapter.clone(), clock.clone()))).unwrap();
    adapter.set_size(slint::PhysicalSize::new(393, 852));
    let probe = ReportBlocksProbe::new().unwrap();
    assert!(
        probe
            .get_preview_blocks()
            .iter()
            .any(|row| row.tone == 0 && row.title == "实时保护与异常拦截")
    );
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
    let reports = Rc::new(slint::VecModel::from(vec![
            ReportItem {
                id: "quality".into(),
                title: "IP 归属与信誉".into(),
                value: "1 项来源不一致 · 机房或托管网络".into(),
                tone: 0,
                delay_tone: 2,
                block_items: 1,
                ..Default::default()
            },
            ReportItem {
                id: "quality".into(),
                title: "出口 IP 质量".into(),
                value: "机房或托管网络来源不一致".into(),
                detail: "来源不一致\nIPQuery 是，IPLogs 否。\n尚未确认\n爬虫地址：未提供\n查询未完成\nLayer3 Intel：请求超时".into(),
                tone: 2,
                ..Default::default()
            },
            ReportItem {
                id: "routes".into(),
                title: "Claude 域名选路".into(),
                value: "3 个域名 · 仅记录选路".into(),
                tone: 0,
                delay_tone: 2,
                block_items: 2,
                ..Default::default()
            },
            ReportItem {
                id: "routes".into(),
                title: "Claude 域名选路".into(),
                value: "3 个域名 · 仅记录选路".into(),
                detail: "api.anthropic.com\nclaude.ai\nclaude.com".into(),
                tone: 2,
                ..Default::default()
            },
            ReportItem {
                id: "routes".into(),
                title: "failed.example".into(),
                value: "连接失败".into(),
                tone: 3,
                ..Default::default()
            },
        ]));
    probe.set_reports(reports.clone().into());
    let guard_reports = Rc::new(slint::VecModel::from(vec![
        ReportItem {
            id: "实时保护与异常拦截".into(),
            title: "实时保护与异常拦截".into(),
            value: "已关闭 1 条异常连接".into(),
            tone: 0,
            delay_tone: 3,
            block_items: 2,
            ..Default::default()
        },
        ReportItem {
            id: "实时保护与异常拦截".into(),
            title: "实时保护与异常拦截".into(),
            value: "已观察 8 次请求".into(),
            detail: "说明：只处理保护范围内偏离指定节点的连接。".into(),
            source: "内核事件与连接控制接口".into(),
            tone: 3,
            ..Default::default()
        },
        ReportItem {
            id: "实时保护与异常拦截".into(),
            title: "closed.example".into(),
            value: "异常连接已关闭".into(),
            tone: 3,
            ..Default::default()
        },
    ]));
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
        render(width, height);
        let button = targets.borrow()["claude-block:quality"].clone();
        assert!(button[3] >= 48. && button[0] + button[2] <= width as f32);
        click(probe.window(), &button);
        render(width, height);
        let bounds = targets.borrow();
        let row = bounds
            .get("claude-block-item:quality:1")
            .unwrap_or_else(|| {
                panic!("quality block did not expand at {width}x{height}: {bounds:?}")
            });
        assert!(row[3] > 130.);
        assert!(bounds["claude-block-item:quality:3"][3] < 1.);
        assert!(!bounds.contains_key("claude-block-detail:quality:1"));
        drop(bounds);
        let mut header = reports.row_data(0).unwrap();
        header.value = format!("摘要已更新 {width}×{height} {scale}").into();
        reports.set_row_data(0, header);
        render(width, height);
        assert!(
            targets.borrow().contains_key("claude-block-item:quality:1"),
            "updating a section summary closed the expanded details"
        );
        let mut item = reports.row_data(1).unwrap();
        item.id = "replacement".into();
        reports.set_row_data(1, item);
        let mut header = reports.row_data(0).unwrap();
        header.id = "replacement".into();
        reports.set_row_data(0, header);
        render(width, height);
        assert!(
            !targets
                .borrow()
                .contains_key("claude-block-item:replacement:1"),
            "a replacement section inherited the previous section's expanded state"
        );
        let mut item = reports.row_data(1).unwrap();
        item.id = "quality".into();
        reports.set_row_data(1, item);
        let mut header = reports.row_data(0).unwrap();
        header.id = "quality".into();
        reports.set_row_data(0, header);
        render(width, height);
        let button = targets.borrow()["claude-block:quality"].clone();
        click(probe.window(), &button);
        render(width, height);
        assert!(targets.borrow().contains_key("claude-block-item:quality:1"));
        let button = targets.borrow()["claude-block:quality"].clone();
        click(probe.window(), &button);
        render(width, height);
        assert!(!targets.borrow().contains_key("claude-block-item:quality:1"));
        let button = targets.borrow()["claude-block:routes"].clone();
        click(probe.window(), &button);
        render(width, height);
        let row_height = targets.borrow()["claude-block-item:routes:3"][3];
        let detail = targets.borrow()["claude-block-detail:routes:3"].clone();
        assert!(detail[3] >= 48. && detail[0] + detail[2] <= width as f32);
        click(probe.window(), &detail);
        render(width, height);
        assert!(targets.borrow()["claude-block-item:routes:3"][3] > row_height);
        let mut header = reports.row_data(2).unwrap();
        header.value = format!("选路已更新 {width}×{height} {scale}").into();
        reports.set_row_data(2, header);
        render(width, height);
        assert!(
            targets.borrow()["claude-block-item:routes:3"][3] > row_height,
            "updating a section summary closed the nested details"
        );
        let button = targets.borrow()["claude-block:routes"].clone();
        click(probe.window(), &button);
        render(width, height);
        probe.set_reports(guard_reports.clone().into());
        render(width, height);
        let button = targets.borrow()["claude-block:实时保护与异常拦截"].clone();
        click(probe.window(), &button);
        render(width, height);
        let item_key = "claude-block-item:实时保护与异常拦截:1";
        let collapsed_height = targets.borrow()[item_key][3];
        let button = targets.borrow()["claude-block-detail:实时保护与异常拦截:1"].clone();
        click(probe.window(), &button);
        render(width, height);
        let expanded_height = targets.borrow()[item_key][3];
        assert!(expanded_height > collapsed_height);
        let mut status = guard_reports.row_data(1).unwrap();
        status.value = "已观察 9 次请求".into();
        guard_reports.set_row_data(1, status);
        render(width, height);
        assert_eq!(targets.borrow()[item_key][3], expanded_height);
        let mut replacement = guard_reports.row_data(1).unwrap();
        replacement.title = "其他状态".into();
        guard_reports.set_row_data(1, replacement);
        render(width, height);
        assert!(targets.borrow()[item_key][3] < expanded_height);
        let mut status = guard_reports.row_data(1).unwrap();
        status.title = "实时保护与异常拦截".into();
        guard_reports.set_row_data(1, status);
        probe.set_reports(reports.clone().into());
        render(width, height);
    }
    probe.hide().unwrap();
}
