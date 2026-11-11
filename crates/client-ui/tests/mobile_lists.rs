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
    import { MobileSubscriptions, MobileConnections, MobileRules, RulesRow } from "../ui/mobile/shared/pages.slint";
    import { MobileModal } from "../ui/mobile/shared/details.slint";
    import { AndroidSurface } from "../ui/mobile/android/app.slint";
    import { AndroidApplications } from "../ui/mobile/android/applications.slint";
    import { AndroidApplicationPolicies } from "../ui/mobile/android/application-policies.slint";
    import { AndroidData, ApplicationItem } from "../ui/mobile/android/state.slint";
    export { ApplicationItem } from "../ui/mobile/android/state.slint";
    export { AndroidData, AndroidActions, AndroidIntent } from "../ui/mobile/android/state.slint";
    import { IosSurface } from "../ui/mobile/ios/app.slint";
    import { PageViews } from "../ui/components/view-toggle.slint";
    import { NoticeCountdown } from "../ui/components/motion.slint";
    export { ProfileItem } from "../ui/state.slint";
    export { ConnectionItem } from "../ui/state.slint";
    export { ReportItem } from "../ui/state.slint";
    export { MobileActions, MobileIntent } from "../ui/mobile/shared/state.slint";
    import { SettingsPage, SettingsNavigation } from "../ui/pages/settings.slint";
    import { UiData, ProfileItem, ConnectionItem, ReportItem } from "../ui/state.slint";
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
        in-out property <bool> protected <=> MobileData.protected;
        in-out property <bool> busy <=> MobileData.profile-busy;
        MobileSubscriptions { width: parent.width; height: parent.height; }
    }

    export component MobileRuleRowsProbe inherits Window {
        preferred-width: 393px; preferred-height: 400px;
        in-out property <float> text-scale <=> Theme.text-scale;
        in-out property <bool> protected <=> MobileData.protected;
        in-out property <int> detail <=> MobileView.detail;
        in-out property <string> selected-id <=> MobileView.selected-id;
        out property <length> viewport-width: root.width;
        out property <length> custom-height: custom.height;
        out property <length> subscription-height: subscription.height;
        out property <length> shortcut-height: shortcut.height;
        Rectangle {
            width: parent.width; height: parent.height;
            custom := RulesRow {
                x: 0px; y: 0px; width: parent.width;
                item: { id: "custom", position: "1", kind: "DOMAIN-SUFFIX", value: "a-very-long-domain-name.example.com", policy: "DIRECT", source: "自定义", enabled: true };
            }
            subscription := RulesRow {
                x: 0px; y: custom.y + custom.height + 1px; width: parent.width;
                item: { id: "subscription", position: "2", kind: "MATCH", policy: "A very long policy name", source: "订阅", enabled: true };
            }
            shortcut := RulesRow {
                x: 0px; y: subscription.y + subscription.height + 1px; width: parent.width;
                item: { id: "shortcut", position: "3", kind: "GEOIP", value: "CN", policy: "DIRECT", source: "快捷规则", enabled: true };
            }
        }
    }

    export component MobileConnectionsProbe inherits Window {
        preferred-width: 393px; preferred-height: 852px;
        in-out property <float> text-scale <=> Theme.text-scale;
        in-out property <[ConnectionItem]> connections <=> MobileData.connections;
        in-out property <string> query <=> MobileView.query;
        in-out property <int> detail <=> MobileView.detail;
        in-out property <bool> protected <=> MobileData.protected;
        init => {
            MobileData.connection-count = "12";
            MobileData.connection-page = "1 / 1 · 12 条";
        }
        MobileConnections { width: parent.width; height: parent.height; }
    }

    export component MobileRulesProbe inherits Window {
        preferred-width: 393px; preferred-height: 720px;
        in-out property <float> text-scale <=> Theme.text-scale;
        in-out property <string> query <=> MobileView.query;
        out property <string> drag-id <=> MobileView.rule-drag-id;
        out property <string> drag-target <=> MobileView.rule-drag-target;
        init => {
            MobileData.rule-profiles = ["我的订阅"];
            MobileData.rule-profile = "我的订阅";
            MobileData.rules = [
                { id: "first", position: "1", kind: "DOMAIN-SUFFIX", value: "first.test", policy: "DIRECT", source: "自定义", enabled: true },
                { id: "second", position: "2", kind: "DOMAIN-SUFFIX", value: "second.test", policy: "Proxy", source: "订阅", enabled: true },
                { id: "third", position: "3", kind: "MATCH", policy: "DIRECT", source: "订阅", enabled: true }
            ];
        }
        MobileRules { width: parent.width; height: parent.height; }
    }

    export component ApplicationsProbe inherits Window {
        preferred-width: 393px; preferred-height: 852px;
        in-out property <[ApplicationItem]> applications <=> AndroidData.applications;
        in-out property <length> offset <=> AndroidData.applications-scroll-y;
        in-out property <string> query <=> AndroidData.applications-query;
        in-out property <float> text-scale <=> Theme.text-scale;
        in-out property <bool> protected <=> MobileData.protected;
        in-out property <bool> protection-pending <=> MobileData.protection-pending;
        in-out property <bool> dirty <=> MobileView.draft-dirty;
        in-out property <bool> loaded <=> AndroidData.applications-loaded;
        in-out property <string> error <=> AndroidData.applications-error;
        in-out property <int> mode <=> AndroidData.applications-mode;
        in-out property <int> filter <=> AndroidData.applications-filter;
        in-out property <bool> opened: true;
        if root.opened: AndroidApplications { width: parent.width; height: parent.height; }
        init => { AndroidData.applications-loaded = true; }
    }

    export component MobileStartupProbe inherits Window {
        preferred-width: 393px; preferred-height: 852px;
        in property <bool> use-ios;
        in-out property <int> page <=> MobileView.page;
        in-out property <float> text-scale <=> Theme.text-scale;
        in-out property <bool> core-running <=> MobileData.core-running;
        if !root.use-ios: AndroidSurface { width: parent.width; height: parent.height; }
        if root.use-ios: IosSurface { width: parent.width; height: parent.height; }
    }

    export component ApplicationPoliciesProbe inherits Window {
        preferred-width: 393px; preferred-height: 852px;
        in-out property <bool> supported <=> AndroidData.policies-supported;
        in-out property <bool> protected <=> MobileData.protected;
        in-out property <bool> protection-pending <=> MobileData.protection-pending;
        in-out property <bool> dirty <=> MobileView.draft-dirty;
        in-out property <float> text-scale <=> Theme.text-scale;
        in-out property <length> offset <=> AndroidData.policies-scroll-y;
        in-out property <bool> opened: true;
        init => {
            AndroidData.policies-profile = "A very long subscription name";
            AndroidData.policies-applications = ["Browser · org.example.browser"];
            AndroidData.policies-application = "Browser · org.example.browser";
            AndroidData.policies-targets = ["跟随配置", "直连", "拦截"];
            AndroidData.policies-target = "直连";
            AndroidData.policies-status = "只作用于进入 VPN 的应用；未指定的应用跟随配置。";
        }
        if root.opened: AndroidApplicationPolicies { width: parent.width; height: parent.height; }
    }

    export component MobileAccentProbe inherits Window {
        preferred-width: 393px; preferred-height: 852px;
        in-out property <bool> protected <=> MobileData.protected;
        in-out property <bool> protection-pending <=> MobileData.protection-pending;
        in-out property <bool> modal <=> MobileView.modal;
        in-out property <int> detail <=> MobileView.detail;
        MobileModal { width: parent.width; height: parent.height; }
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
        in-out property <bool> protected <=> MobileData.protected;
        in-out property <bool> protection-pending <=> MobileData.protection-pending;
        in-out property <bool> update-available <=> surface.update-available;
        in-out property <bool> update-downloading <=> surface.update-downloading;
        in-out property <string> update-status <=> surface.update-status;
        in-out property <float> update-progress <=> surface.update-progress;
        out property <length> update-height: surface.update-banner-height;
        in-out property <int> page <=> MobileView.page;
        out property <[length]> close-target <=> surface.notice-close-target;
        in-out property <int> detail <=> MobileView.detail;
        in-out property <[ReportItem]> reports <=> MobileData.claude-results;
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
fn mobile_connections_keep_actions_reachable_and_scroll_with_long_names() {
    let (adapter, clock) = headless(393, 852);
    let probe = MobileConnectionsProbe::new().unwrap();
    let connections = Rc::new(slint::VecModel::from(
        (0..12)
            .map(|index| ConnectionItem {
                id: index.to_string().into(),
                host: "very-long-service-name.api.github.example.com:443".into(),
                detail: "TCP · 10:31:20".into(),
                route: "Proxy → Very long subscription and node name".into(),
                rule: "DOMAIN-SUFFIX very-long-service-name.example.com".into(),
                down: "128.9 MiB/s".into(),
                up: "128.9 KiB/s".into(),
                download: "123.4 GiB".into(),
                upload: "123.4 MiB".into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    ));
    probe.set_connections(connections.clone().into());
    let targets = Rc::new(RefCell::new(
        std::collections::BTreeMap::<String, Vec<f32>>::new(),
    ));
    let observed = targets.clone();
    probe
        .global::<UiGeometry>()
        .on_measured(move |key, bounds| {
            observed
                .borrow_mut()
                .insert(key.to_string(), bounds.iter().collect());
        });
    probe.global::<UiGeometry>().set_enabled(true);
    let events = Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    probe
        .global::<MobileActions>()
        .on_request(move |intent, subject, _| {
            recorded.borrow_mut().push((intent, subject.to_string()));
        });
    probe.show().unwrap();
    let render = || {
        advance(&clock, Duration::from_millis(300));
        let size = probe.window().size();
        frame(&adapter, size.width as usize, size.height as usize);
        targets.borrow_mut().clear();
        let geometry = probe.global::<UiGeometry>();
        geometry.set_revision(geometry.get_revision().wrapping_add(1));
        advance(&clock, Duration::ZERO);
        frame(&adapter, size.width as usize, size.height as usize);
    };
    for (width, height, text_scale) in [(393, 852, 1.), (320, 568, 1.3), (1280, 800, 1.)] {
        adapter.set_size(slint::PhysicalSize::new(width, height));
        probe.set_text_scale(text_scale);
        probe.set_query(format!("viewport-{width}").into());
        probe.set_detail(0);
        render();
        let bounds = |key: &str| targets.borrow()[key].clone();
        let panel = bounds("connections-card");
        let list = bounds("connections-list");
        assert!(panel[0] >= 0. && panel[0] + panel[2] <= width as f32 + 1.);
        assert!(list[0] > panel[0] && list[0] + list[2] < panel[0] + panel[2]);
        let first = bounds("connection-row:0");
        let name = bounds("connection-name:0");
        let close = bounds("connection-close:0");
        let sort = bounds("connections-sort");
        let close_all = bounds("connections-close-all");
        assert!(first[1] >= panel[1]);
        assert!(close[2] >= 48. && close[3] >= 48.);
        assert!(name[2] > 0. && name[0] + name[2] <= close[0]);
        for control in [close.clone(), sort, close_all] {
            assert!(control[0] >= panel[0]);
            assert!(control[0] + control[2] <= panel[0] + panel[2] + 1.);
            assert!(control[1] + control[3] <= first[1] + first[3] + 1.);
        }
        let click = |target: &[f32]| {
            let position =
                LogicalPosition::new(target[0] + target[2] / 2., target[1] + target[3] / 2.);
            probe.window().dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
            advance(&clock, Duration::from_millis(50));
            probe.window().dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
            render();
        };
        events.borrow_mut().clear();
        click(&close);
        assert_eq!(
            &*events.borrow(),
            &[(MobileIntent::CloseConnection, "0".to_owned())]
        );
        assert_eq!(probe.get_detail(), 0);
        probe.set_protected(true);
        events.borrow_mut().clear();
        click(&close);
        assert!(events.borrow().is_empty());
        probe.set_protected(false);
        click(&name);
        assert_eq!(probe.get_detail(), 5);
        assert_eq!(
            &*events.borrow(),
            &[(MobileIntent::ConnectionDetail, "0".to_owned())]
        );
        probe.set_detail(0);
        for _ in 0..8 {
            probe.window().dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(list[0] + list[2] / 2., list[1] + list[3] / 2.),
                delta_x: 0.,
                delta_y: -(height as f32),
            });
            render();
        }
        let last = bounds("connection-row:11");
        assert!(last[1] >= list[1] - 1.);
        assert!(last[1] + last[3] <= list[1] + list[3] + 1.);
    }
    connections.set_vec(Vec::new());
    probe.set_query("no-matching-connections".into());
    render();
    let search = targets.borrow()["connections-search"].clone();
    assert!(search[2] > 0. && search[3] >= 48.);
    assert!(!targets.borrow().contains_key("connection-row:0"));
    probe.hide().unwrap();
}

#[test]
fn report_categories_share_one_panel_without_gaps_from_hidden_rows() {
    let (adapter, clock) = headless(393, 852);
    let probe = MobileNoticeProbe::new().unwrap();
    let targets = Rc::new(RefCell::new(
        std::collections::BTreeMap::<String, Vec<f32>>::new(),
    ));
    let observed = targets.clone();
    probe
        .global::<UiGeometry>()
        .on_measured(move |key, bounds| {
            observed
                .borrow_mut()
                .insert(key.to_string(), bounds.iter().collect());
        });
    probe.global::<UiGeometry>().set_enabled(true);
    probe.set_page(8);
    let tones = [1, 3, 2, 3, 1];
    probe.set_reports(
        Rc::new(slint::VecModel::from(
            tones
                .iter()
                .map(|tone| ReportItem {
                    title: "检测项目".into(),
                    value: "2001:db8:1234:5678:abcd:ef01:2345:6789".into(),
                    country: "香港".into(),
                    flag: "🇨🇳".into(),
                    flagged: true,
                    detail: "说明：检测结果的说明按可用宽度换行。".into(),
                    source: "固定测试来源".into(),
                    delay: "218 ms".into(),
                    delay_tone: 1,
                    tone: *tone,
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
    probe.show().unwrap();
    for (width, height) in [(393, 852), (800, 1280), (1280, 800)] {
        adapter.set_size(slint::PhysicalSize::new(width, height));
        for (detail, tone) in [(7, 1), (1, 3), (9, 2)] {
            targets.borrow_mut().clear();
            probe.set_detail(detail);
            advance(&clock, Duration::from_millis(300));
            frame(&adapter, width as usize, height as usize);
            let geometry = probe.global::<UiGeometry>();
            geometry.set_revision(geometry.get_revision().wrapping_add(1));
            advance(&clock, Duration::ZERO);
            frame(&adapter, width as usize, height as usize);
            let measured = targets.borrow();
            let panel = &measured["claude-report"];
            let mut previous_bottom = None;
            for (index, item_tone) in tones.iter().enumerate() {
                let bounds = &measured[&format!("claude-result:{index}")];
                if *item_tone != tone {
                    assert!(bounds[3] < 1., "filtered records must occupy no space");
                    continue;
                }
                assert!(bounds[2] > if width > 1000 { 800. } else { 250. });
                assert!(bounds[3] > 0.);
                assert!((bounds[0] - panel[0]).abs() < 1.);
                assert!((bounds[2] - panel[2]).abs() < 1.);
                if let Some(bottom) = previous_bottom {
                    assert!(
                        (bounds[1] - bottom).abs() < 1.,
                        "visible rows must remain adjacent inside the single panel"
                    );
                } else {
                    assert!((bounds[1] - panel[1]).abs() < 1.);
                }
                previous_bottom = Some(bounds[1] + bounds[3]);
            }
            assert!(
                (previous_bottom.unwrap() - panel[1] - panel[3]).abs() < 1.,
                "the panel must end with the last visible row"
            );
        }
    }
    targets.borrow_mut().clear();
    probe.set_reports(Default::default());
    advance(&clock, Duration::from_millis(300));
    frame(&adapter, 1280, 800);
    let geometry = probe.global::<UiGeometry>();
    geometry.set_revision(geometry.get_revision().wrapping_add(1));
    advance(&clock, Duration::ZERO);
    frame(&adapter, 1280, 800);
    assert!(
        targets.borrow()["claude-report"][3] < 1.,
        "clearing results must not leave an empty report card"
    );
    probe.hide().unwrap();
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
fn mobile_rule_rows_keep_actions_aligned_for_all_sources_and_sizes() {
    let (adapter, clock) = headless(393, 400);
    let probe = MobileRuleRowsProbe::new().unwrap();
    let targets = Rc::new(RefCell::new(
        std::collections::BTreeMap::<String, Vec<f32>>::new(),
    ));
    let observed = targets.clone();
    probe
        .global::<UiGeometry>()
        .on_measured(move |key, bounds| {
            observed
                .borrow_mut()
                .insert(key.to_string(), bounds.iter().collect());
        });
    let requests = Rc::new(RefCell::new(Vec::new()));
    let captured = requests.clone();
    probe
        .global::<MobileActions>()
        .on_request(move |intent, first, second| {
            captured.borrow_mut().push((intent, first, second));
        });
    probe.global::<UiGeometry>().set_enabled(true);
    probe.show().unwrap();
    for (width, scale) in [(320, 1.), (393, 1.), (800, 1.), (320, 1.3)] {
        adapter.set_size(slint::PhysicalSize::new(width, 400));
        probe.set_text_scale(scale);
        advance(&clock, Duration::from_millis(300));
        targets.borrow_mut().clear();
        let geometry = probe.global::<UiGeometry>();
        geometry.set_revision(geometry.get_revision().wrapping_add(1));
        frame(&adapter, width as usize, 400);
        advance(&clock, Duration::ZERO);
        assert!(
            (probe.get_viewport_width() - width as f32).abs() < 1.,
            "rule probe width {} does not match requested width {width}",
            probe.get_viewport_width()
        );
        let heights = [
            probe.get_custom_height(),
            probe.get_subscription_height(),
            probe.get_shortcut_height(),
        ];
        assert!(heights[0] >= 48.);
        assert!((heights[0] - heights[1]).abs() < 1.);
        assert!((heights[0] - heights[2]).abs() < 1.);
        for id in ["custom", "subscription", "shortcut"] {
            let targets = targets.borrow();
            let toggle = &targets[&format!("rule-switch:{id}")];
            let detail = &targets[&format!("rule-detail:{id}")];
            for target in [toggle, detail] {
                assert!(
                    target[2] >= 48. && target[3] >= 48.,
                    "{id} target is too small at width {width}, scale {scale}: {target:?}"
                );
                assert!(
                    target[0] >= 0. && target[0] + target[2] <= width as f32 + 1.,
                    "{id} target is outside width {width}, scale {scale}: {target:?}"
                );
            }
            assert!(toggle[0] + toggle[2] <= detail[0] + 1.);
        }
    }
    let click = |key: &str| {
        let bounds = targets.borrow()[key].clone();
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
            probe.window().dispatch_event(event);
        }
        advance(&clock, Duration::from_millis(50));
    };
    click("rule-switch:custom");
    assert_eq!(requests.borrow().len(), 1);
    assert_eq!(requests.borrow()[0].0, MobileIntent::RuleEnable);
    assert_eq!(requests.borrow()[0].1.as_str(), "custom");
    click("rule-switch:subscription");
    assert_eq!(requests.borrow().len(), 1);
    click("rule-detail:shortcut");
    assert_eq!(probe.get_detail(), 15);
    assert_eq!(probe.get_selected_id().as_str(), "shortcut");
    probe.set_detail(0);
    probe.set_protected(true);
    frame(&adapter, 320, 400);
    click("rule-switch:custom");
    click("rule-detail:shortcut");
    assert_eq!(requests.borrow().len(), 1);
    assert_eq!(probe.get_detail(), 0);
    probe.hide().unwrap();
}

#[test]
fn mobile_rules_drag_real_rows_at_phone_pad_and_large_text_sizes() {
    let (adapter, clock) = headless(393, 720);
    let probe = MobileRulesProbe::new().unwrap();
    let targets = Rc::new(RefCell::new(
        std::collections::BTreeMap::<String, Vec<f32>>::new(),
    ));
    let observed = targets.clone();
    probe
        .global::<UiGeometry>()
        .on_measured(move |key, bounds| {
            observed
                .borrow_mut()
                .insert(key.to_string(), bounds.iter().collect());
        });
    let requests = Rc::new(RefCell::new(Vec::new()));
    let captured = requests.clone();
    probe
        .global::<MobileActions>()
        .on_request(move |intent, first, second| {
            captured.borrow_mut().push((intent, first, second));
        });
    probe.show().unwrap();
    probe.global::<UiGeometry>().set_enabled(true);
    for (width, height, scale) in [(393, 720, 1.), (800, 720, 1.), (320, 568, 1.3)] {
        adapter.set_size(slint::PhysicalSize::new(width, height));
        probe.set_text_scale(scale);
        advance(&clock, Duration::from_millis(300));
        targets.borrow_mut().clear();
        let geometry = probe.global::<UiGeometry>();
        geometry.set_revision(geometry.get_revision().wrapping_add(1));
        frame(&adapter, width as usize, height as usize);
        advance(&clock, Duration::ZERO);
        let bounds = targets.borrow()["mobile-rule-drag"].clone();
        let start = LogicalPosition::new(bounds[0] + 8., bounds[1] + bounds[3] / 2.);
        let end = LogicalPosition::new(start.x, start.y + bounds[3]);
        probe.window().dispatch_event(WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        });
        advance(&clock, Duration::from_millis(500));
        assert!(probe.get_drag_id().is_empty());
        advance(&clock, Duration::from_millis(200));
        assert_eq!(probe.get_drag_id().as_str(), "first");
        probe
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: end });
        advance(&clock, Duration::from_millis(20));
        assert_eq!(probe.get_drag_target().as_str(), "second");
        frame(&adapter, width as usize, height as usize);
        probe.window().dispatch_event(WindowEvent::PointerReleased {
            position: end,
            button: PointerEventButton::Left,
        });
        let requests = requests.borrow();
        let last = requests.last().expect("dragging must dispatch a rule move");
        assert_eq!(last.0, MobileIntent::RuleDrop);
        assert_eq!(last.1.as_str(), "first");
        assert_eq!(last.2.as_str(), "second");
        assert!(probe.get_drag_id().is_empty());
    }
    probe.hide().unwrap();
}

#[test]
fn application_controls_distinguish_scope_filter_save_and_protection() {
    let (adapter, clock) = headless(393, 852);
    let probe = ApplicationsProbe::new().unwrap();
    let targets = Rc::new(RefCell::new(
        std::collections::BTreeMap::<String, Vec<f32>>::new(),
    ));
    let observed = targets.clone();
    probe
        .global::<UiGeometry>()
        .on_measured(move |key, bounds| {
            observed
                .borrow_mut()
                .insert(key.to_string(), bounds.iter().collect());
        });
    probe.global::<UiGeometry>().set_enabled(true);
    let requests = Rc::new(RefCell::new(Vec::new()));
    let captured = requests.clone();
    probe
        .global::<AndroidActions>()
        .on_request(move |intent, value| {
            captured.borrow_mut().push((intent, value.to_string()));
        });
    probe.set_mode(1);
    probe.show().unwrap();
    let click = |key: &str| {
        let geometry = probe.global::<UiGeometry>();
        geometry.set_revision(geometry.get_revision() + 1);
        frame(&adapter, 393, 852);
        advance(&clock, Duration::ZERO);
        let bounds = targets.borrow()[key].clone();
        assert!(bounds[2] > 0. && bounds[3] >= 48.);
        assert!(bounds[0] >= 0. && bounds[0] + bounds[2] <= 394.);
        assert!(bounds[1] >= 0. && bounds[1] + bounds[3] <= 853.);
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
            probe.window().dispatch_event(event);
        }
        advance(&clock, Duration::from_millis(50));
    };
    click("preview-applications-save");
    assert!(requests.borrow().is_empty());
    probe.set_dirty(true);
    click("preview-applications-save");
    assert_eq!(
        requests.borrow().last().unwrap().0,
        AndroidIntent::ApplicationSave
    );
    click("preview-applications-revert");
    assert_eq!(
        requests.borrow().last().unwrap().0,
        AndroidIntent::ApplicationRevert
    );
    click("preview-applications-mode:2");
    assert_eq!(
        requests.borrow().last().unwrap(),
        &(AndroidIntent::ApplicationMode, "2".to_string())
    );
    click("preview-applications-filter:1");
    assert_eq!(
        requests.borrow().last().unwrap(),
        &(AndroidIntent::ApplicationFilter, "1".to_string())
    );
    probe.set_mode(0);
    let before = requests.borrow().len();
    targets.borrow_mut().clear();
    let geometry = probe.global::<UiGeometry>();
    geometry.set_revision(geometry.get_revision() + 1);
    frame(&adapter, 393, 852);
    advance(&clock, Duration::ZERO);
    assert!(
        !targets
            .borrow()
            .contains_key("preview-applications-filter:1")
    );
    assert_eq!(requests.borrow().len(), before);
    probe.set_loaded(false);
    probe.set_error("读取应用列表失败".into());
    click("preview-applications-retry");
    assert_eq!(
        requests.borrow().last().unwrap().0,
        AndroidIntent::ApplicationRefresh
    );
    probe.set_error("".into());
    probe.set_loaded(true);
    probe.set_mode(1);
    let before = requests.borrow().len();
    for pending in [false, true] {
        probe.set_protected(!pending);
        probe.set_protection_pending(pending);
        for key in [
            "preview-applications-save",
            "preview-applications-mode:1",
            "preview-applications-filter:0",
        ] {
            click(key);
        }
        assert_eq!(requests.borrow().len(), before);
    }
    probe.hide().unwrap();
}

#[test]
fn applications_reopen_with_active_geometry_observers_across_modes_and_sizes() {
    let (adapter, clock) = headless(393, 852);
    let probe = ApplicationsProbe::new().unwrap();
    probe.set_opened(false);
    let targets = Rc::new(RefCell::new(
        std::collections::BTreeMap::<String, Vec<f32>>::new(),
    ));
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
    for (width, height, scale) in [(393, 852, 1.), (320, 640, 1.6), (1100, 740, 1.)] {
        adapter.set_size(slint::PhysicalSize::new(width, height));
        probe.set_text_scale(scale);
        for mode in [0, 1, 2, 1, 0] {
            probe.set_opened(false);
            advance(&clock, Duration::from_millis(50));
            frame(&adapter, width as usize, height as usize);
            targets.borrow_mut().clear();
            probe.set_mode(mode);
            probe.set_opened(true);
            advance(&clock, Duration::from_millis(250));
            frame(&adapter, width as usize, height as usize);
            for button in 0..3 {
                let measured = targets.borrow();
                let bounds = &measured[&format!("preview-applications-mode:{button}")];
                assert_eq!(bounds.len(), 4);
                assert!(bounds[2] > 0. && bounds[3] >= 48.);
                assert!(bounds[0] >= 0. && bounds[0] + bounds[2] <= width as f32 + 1.);
                assert!(bounds[1] >= 0. && bounds[1] + bounds[3] <= height as f32 + 1.);
            }
        }
    }
    probe.hide().unwrap();
}

#[test]
fn application_policy_form_respects_platform_protection_and_scrolls() {
    let (adapter, clock) = headless(393, 852);
    let probe = ApplicationPoliciesProbe::new().unwrap();
    let targets = Rc::new(RefCell::new(
        std::collections::BTreeMap::<String, Vec<f32>>::new(),
    ));
    let observed = targets.clone();
    probe
        .global::<UiGeometry>()
        .on_measured(move |key, bounds| {
            observed
                .borrow_mut()
                .insert(key.to_string(), bounds.iter().collect());
        });
    let requests = Rc::new(RefCell::new(Vec::new()));
    let recorded = requests.clone();
    probe
        .global::<MobileActions>()
        .on_request(move |intent, action, value| {
            recorded
                .borrow_mut()
                .push((intent, action.to_string(), value.to_string()));
        });
    probe.global::<UiGeometry>().set_enabled(true);
    probe.set_supported(true);
    probe.show().unwrap();
    for (width, height, scale) in [
        (393, 852, 1.),
        (320, 568, 1.5),
        (852, 320, 1.),
        (800, 1280, 1.),
    ] {
        adapter.set_size(slint::PhysicalSize::new(width, height));
        probe.set_text_scale(scale);
        probe.set_offset(0.);
        probe.set_dirty(true);
        advance(&clock, Duration::from_millis(300));
        frame(&adapter, width as usize, height as usize);
        let geometry = probe.global::<UiGeometry>();
        geometry.set_revision(geometry.get_revision().wrapping_add(1));
        advance(&clock, Duration::ZERO);
        frame(&adapter, width as usize, height as usize);
        let bounds = targets.borrow()["application-policy-save"].clone();
        probe.set_offset(-(bounds[1] + bounds[3] - height as f32 + 24.).max(0.));
        advance(&clock, Duration::from_millis(300));
        frame(&adapter, width as usize, height as usize);
        geometry.set_revision(geometry.get_revision().wrapping_add(1));
        advance(&clock, Duration::ZERO);
        frame(&adapter, width as usize, height as usize);
        let bounds = targets.borrow()["application-policy-save"].clone();
        let position = LogicalPosition::new(bounds[0] + bounds[2] / 2., bounds[1] + bounds[3] / 2.);
        assert!(bounds[2] > 0. && bounds[3] >= 48.);
        assert!(position.x > 0. && position.x < width as f32);
        assert!(position.y > 0. && position.y < height as f32);
        for (protected, pending, expected) in
            [(false, false, 1), (true, false, 0), (false, true, 0)]
        {
            probe.set_protected(protected);
            probe.set_protection_pending(pending);
            requests.borrow_mut().clear();
            frame(&adapter, width as usize, height as usize);
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
                probe.window().dispatch_event(event);
            }
            advance(&clock, Duration::from_millis(50));
            assert_eq!(requests.borrow().len(), expected);
            if expected == 1 {
                assert_eq!(
                    requests.borrow()[0],
                    (
                        MobileIntent::ApplicationPolicy,
                        "save".into(),
                        "Browser · org.example.browser\n直连\nfalse".into(),
                    )
                );
            }
        }
        probe.set_protection_pending(false);
    }
    probe.set_supported(false);
    probe.set_opened(false);
    frame(&adapter, 800, 1280);
    targets.borrow_mut().clear();
    probe.set_opened(true);
    frame(&adapter, 800, 1280);
    assert!(!targets.borrow().contains_key("application-policy-save"));
    probe.hide().unwrap();
}

#[test]
fn applications_scroll_across_phone_and_tablet_viewports() {
    let (adapter, clock) = headless(393, 852);
    let probe = ApplicationsProbe::new().unwrap();
    let applications = Rc::new(slint::VecModel::from(
        (0..50)
            .map(|index| ApplicationItem {
                name: format!("Application {index}").into(),
                package: format!("org.example.application{index}").into(),
                selected: false,
                icon: Default::default(),
                has_icon: false,
            })
            .collect::<Vec<_>>(),
    ));
    probe.set_applications(applications.clone().into());
    probe.set_mode(1);
    probe.show().unwrap();
    let scroll = |delta_y| {
        probe.window().dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(24., probe.window().size().height as f32 - 24.),
            delta_x: 0.,
            delta_y,
        });
    };
    for (width, height) in [(393, 852), (852, 240), (800, 1100), (1280, 650)] {
        adapter.set_size(slint::PhysicalSize::new(width, height));
        probe.set_offset(0.);
        frame(&adapter, width as usize, height as usize);
        scroll(-160.);
        advance(&clock, Duration::from_secs(1));
        frame(&adapter, width as usize, height as usize);
        assert!(
            probe.get_offset() < 0.,
            "application header and rows must share scrolling at {width}x{height}"
        );
        scroll(-10000.);
        advance(&clock, Duration::from_secs(1));
        frame(&adapter, width as usize, height as usize);
        scroll(10000.);
        advance(&clock, Duration::from_secs(1));
        frame(&adapter, width as usize, height as usize);
        assert_eq!(probe.get_offset(), 0.);
        let drag_start = LogicalPosition::new(width as f32 / 2., height as f32 - 24.);
        probe.window().dispatch_event(WindowEvent::PointerPressed {
            position: drag_start,
            button: PointerEventButton::Left,
        });
        for step in 1..=6 {
            advance(&clock, Duration::from_millis(16));
            probe.window().dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(drag_start.x, drag_start.y - step as f32 * 24.),
            });
            frame(&adapter, width as usize, height as usize);
        }
        probe.window().dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(drag_start.x, drag_start.y - 144.),
            button: PointerEventButton::Left,
        });
        advance(&clock, Duration::from_secs(1));
        frame(&adapter, width as usize, height as usize);
        assert!(
            probe.get_offset() < 0.,
            "application rows must scroll under a drag gesture at {width}x{height}"
        );
        probe.set_text_scale(1.3);
        frame(&adapter, width as usize, height as usize);
        assert!(
            probe.get_offset().is_finite(),
            "changing the system font size must not invalidate the application viewport"
        );
        probe.set_text_scale(1.);
    }
    for mode in [0, 2, 0, 1, 0, 2, 1] {
        probe.set_mode(mode);
        advance(&clock, Duration::from_millis(50));
        frame(&adapter, 1280, 650);
        assert_eq!(probe.get_offset(), 0.);
        scroll(-160.);
        advance(&clock, Duration::from_secs(1));
        frame(&adapter, 1280, 650);
        if mode == 0 {
            assert_eq!(
                probe.get_offset(),
                0.,
                "all applications must hide the list"
            );
        } else {
            assert!(
                probe.get_offset() < 0.,
                "selection modes must show the list"
            );
        }
    }
    scroll(-160.);
    advance(&clock, Duration::from_secs(1));
    frame(&adapter, 1280, 650);
    applications.set_vec(vec![]);
    probe.set_query("no results".into());
    frame(&adapter, 1280, 650);
    assert_eq!(probe.get_offset(), 0.);
    probe.hide().unwrap();
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
    advance(&clock, Duration::from_millis(30));
    assert!(
        (0.98..1.).contains(&countdown.get_remaining()),
        "the countdown must move before the old 100 ms timer would tick"
    );
    advance(&clock, Duration::from_millis(2470));
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
fn android_application_entry_tracks_update_banner_layout_and_opens_the_correct_page() {
    let (adapter, clock) = headless(393, 852);
    let probe = MobileStartupProbe::new().unwrap();
    probe.set_page(9);
    probe.set_core_running(true);
    let targets = Rc::new(RefCell::new(
        std::collections::BTreeMap::<String, Vec<f32>>::new(),
    ));
    let observed = targets.clone();
    probe
        .global::<UiGeometry>()
        .on_measured(move |key, bounds| {
            observed
                .borrow_mut()
                .insert(key.to_string(), bounds.iter().collect());
        });
    let requests = Rc::new(RefCell::new(Vec::new()));
    let captured = requests.clone();
    probe
        .global::<AndroidActions>()
        .on_request(move |intent, _| captured.borrow_mut().push(intent));
    probe.global::<UiGeometry>().set_enabled(true);
    probe.show().unwrap();
    for (width, height) in [(393, 852), (800, 1280), (1280, 800)] {
        adapter.set_size(slint::PhysicalSize::new(width, height));
        for available in [false, true, false] {
            probe
                .global::<AndroidData>()
                .set_update_available(available);
            advance(&clock, Duration::from_millis(300));
            frame(&adapter, width as usize, height as usize);
            let geometry = probe.global::<UiGeometry>();
            geometry.set_revision(geometry.get_revision().wrapping_add(1));
            advance(&clock, Duration::ZERO);
            frame(&adapter, width as usize, height as usize);
            let bounds = targets.borrow()["preview-application-routing"].clone();
            assert!(bounds[2] > 0. && bounds[3] >= 48.);
            let position =
                LogicalPosition::new(bounds[0] + bounds[2] / 2., bounds[1] + bounds[3] / 2.);
            assert!(position.x >= 0. && position.x < width as f32);
            assert!(position.y >= 0. && position.y < height as f32);
            requests.borrow_mut().clear();
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
                probe.window().dispatch_event(event);
            }
            advance(&clock, Duration::from_millis(50));
            assert_eq!(
                requests.borrow().as_slice(),
                &[AndroidIntent::ApplicationRouting]
            );
        }
    }
    probe.hide().unwrap();
}

#[test]
fn mobile_update_banner_survives_notice_expiry_and_respects_protection() {
    let (adapter, clock) = headless(393, 852);
    let probe = MobileNoticeProbe::new().unwrap();
    probe.set_update_available(true);
    probe.set_update_status("发现新版本 0.6.8".into());
    probe.show().unwrap();
    for (width, height) in [(393, 852), (800, 1280), (852, 393)] {
        adapter.set_size(slint::PhysicalSize::new(width, height));
        for page in [0, 1, 2, 9] {
            probe.set_page(page);
            frame(&adapter, width as usize, height as usize);
            assert!(probe.get_update_height() >= 48.);
        }
        probe.invoke_notify("已完成检查".into());
        advance(&clock, Duration::from_millis(5100));
        frame(&adapter, width as usize, height as usize);
        assert!(probe.get_notice().is_empty());
        assert!(probe.get_update_height() >= 48.);
        probe.set_protected(true);
        frame(&adapter, width as usize, height as usize);
        assert_eq!(probe.get_update_height(), 0.);
        probe.set_protected(false);
        probe.set_protection_pending(true);
        frame(&adapter, width as usize, height as usize);
        assert_eq!(probe.get_update_height(), 0.);
        probe.set_protection_pending(false);
        probe.set_update_downloading(true);
        probe.set_update_progress(0.5);
        frame(&adapter, width as usize, height as usize);
        assert!(probe.get_update_height() >= 48.);
        probe.set_update_downloading(false);
    }
    probe.set_update_available(false);
    frame(&adapter, 852, 393);
    assert_eq!(probe.get_update_height(), 0.);
    probe.hide().unwrap();
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
fn mobile_pages_start_cold_without_geometry_observers() {
    let (adapter, clock) = headless(393, 852);
    for use_ios in [false, true] {
        let startup = MobileStartupProbe::new().unwrap();
        startup.set_use_ios(use_ios);
        let observations = Rc::new(Cell::new(0));
        let reported = observations.clone();
        startup
            .global::<UiGeometry>()
            .on_measured(move |_, _| reported.set(reported.get() + 1));
        assert!(!startup.global::<UiGeometry>().get_enabled());
        startup.show().unwrap();
        for (width, height, scale) in [(393, 852, 1.), (320, 640, 2.), (1100, 740, 1.)] {
            adapter.set_size(slint::PhysicalSize::new(width, height));
            startup.set_text_scale(scale);
            for core_running in [false, true] {
                startup.set_core_running(core_running);
                for page in 0..=10 {
                    startup.set_page(page);
                    advance(&clock, Duration::from_millis(250));
                    frame(&adapter, width as usize, height as usize);
                }
            }
        }
        assert_eq!(observations.get(), 0);
        startup.set_page(0);
        startup.global::<UiGeometry>().set_enabled(true);
        advance(&clock, Duration::from_millis(250));
        frame(&adapter, 1100, 740);
        assert!(observations.get() > 0);
        startup.hide().unwrap();
    }
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
    profiles.global::<UiGeometry>().set_enabled(true);
    let events = Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    profiles
        .global::<MobileActions>()
        .on_request(move |intent, first, second| {
            recorded.borrow_mut().push((intent, first, second))
        });
    profiles.show().unwrap();
    let render = || {
        targets.borrow_mut().clear();
        let geometry = profiles.global::<UiGeometry>();
        geometry.set_revision(geometry.get_revision() + 1);
        let size = profiles.window().size();
        frame(&adapter, size.width as usize, size.height as usize);
    };
    advance(&clock, Duration::from_secs(1));
    render();
    let mut renamed = profile_items.row_data(0).unwrap();
    renamed.name = "Renamed imported file".into();
    profile_items.set_row_data(0, renamed);
    profile_items.push(ProfileItem {
        id: "temporary".into(),
        name: "Temporary remote subscription".into(),
        ..Default::default()
    });
    advance(&clock, Duration::from_secs(1));
    render();
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
    render();
    profile_items.set_vec(restored);
    advance(&clock, Duration::from_secs(1));
    render();
    let bounds = |key: &str| targets.borrow()[key].clone();
    for width in [320, 740] {
        adapter.set_size(slint::PhysicalSize::new(width, 1100));
        advance(&clock, Duration::from_secs(1));
        render();
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
        render();
    };
    assert!((bounds("profile:imported")[1] - bounds("profile:builtin")[1]).abs() < 1.);
    for profile in ["imported", "builtin"] {
        for action in ["update", "edit", "delete"] {
            assert!(
                !targets
                    .borrow()
                    .contains_key(&format!("profile-{action}:{profile}")),
                "{action} must stay inside the subscription details"
            );
        }
    }
    click("profile-use:imported");
    assert!(events.borrow().is_empty());
    assert_eq!(profiles.get_detail(), 0);
    click("profile-detail:imported");
    click("profile-update:imported");
    assert!(events.borrow().is_empty());
    click("profile-edit:imported");
    assert_eq!(profiles.get_detail(), 3);
    assert_eq!(profiles.get_selected_id(), "imported");
    profiles.set_detail(0);
    profiles.set_selected_id("".into());
    click("profile-detail:imported");
    click("profiles-view");
    assert!(profiles.get_list_view());
    let mut renamed = profile_items.row_data(0).unwrap();
    renamed.name = "A very long subscription name ".repeat(8).into();
    profile_items.set_row_data(0, renamed);
    for width in [320, 740] {
        adapter.set_size(slint::PhysicalSize::new(width, 1100));
        advance(&clock, Duration::from_secs(1));
        render();
        let first = bounds("profile:imported");
        let second = bounds("profile:builtin");
        assert!(first[0] >= 0. && first[0] + first[2] <= width as f32 + 1.);
        assert!((first[0] - second[0]).abs() < 1.);
        assert!(second[1] >= first[1] + first[3]);
        assert!(
            first[3] < 110.,
            "{width}px mobile list rows must stay compact: {first:?}"
        );
        let use_action = bounds("profile-use:imported");
        let detail_action = bounds("profile-detail:imported");
        assert!(detail_action[0] >= use_action[0] + use_action[2]);
        assert!(detail_action[0] + detail_action[2] <= first[0] + first[2]);
        assert!(use_action[3] >= 48. && detail_action[3] >= 48.);
    }
    let imported = bounds("profile:imported");
    let builtin = bounds("profile:builtin");
    assert!(
        imported[3] < 110.,
        "mobile list rows must stay compact: {imported:?}"
    );
    assert!((imported[0] - builtin[0]).abs() < 1.);
    assert!(builtin[1] >= imported[1] + imported[3]);
    click("profile-detail:imported");
    assert!(targets.borrow().contains_key("profile-delete:imported"));
    click("profile-detail:builtin");
    assert!(!targets.borrow().contains_key("profile-edit:imported"));
    assert!(!targets.borrow().contains_key("profile-update:imported"));
    assert!(targets.borrow().contains_key("profile-delete:builtin"));
    for action in ["update", "edit", "delete"] {
        click(&format!("profile-{action}:builtin"));
    }
    assert_eq!(profiles.get_detail(), 0);
    assert!(profiles.get_selected_id().is_empty());
    assert!(events.borrow().is_empty());
    click("profile-detail:imported");
    click("profile-delete:imported");
    assert_eq!(profiles.get_detail(), 4);
    assert_eq!(profiles.get_selected_id(), "imported");
    profiles.set_detail(0);
    profiles.set_selected_id("".into());
    click("profile-detail:imported");
    profile_items.push(ProfileItem {
        id: "remote".into(),
        name: "Remote subscription".into(),
        source: "https://example.com/subscription.yaml".into(),
        ..Default::default()
    });
    advance(&clock, Duration::from_secs(1));
    render();
    assert!(!targets.borrow().contains_key("profile-update:remote"));
    click("profile-detail:remote");
    click("profile-update:remote");
    assert_eq!(
        events.borrow().as_slice(),
        &[(MobileIntent::UpdateProfile, "remote".into(), "".into())]
    );
    events.borrow_mut().clear();
    for protected in [true, false] {
        profiles.set_protected(protected);
        profiles.set_busy(!protected);
        render();
        for action in ["update", "edit", "delete"] {
            click(&format!("profile-{action}:remote"));
        }
        assert!(events.borrow().is_empty());
        assert_eq!(profiles.get_detail(), 0);
        assert!(profiles.get_selected_id().is_empty());
    }
    profiles.set_busy(false);
    click("profile-edit:remote");
    assert_eq!(profiles.get_detail(), 3);
    assert_eq!(profiles.get_selected_id(), "remote");
    profiles.hide().unwrap();
}

#[test]
fn mobile_accent_picker_remains_usable_while_protected() {
    let (adapter, clock) = headless(393, 852);
    let probe = MobileAccentProbe::new().unwrap();
    let targets = Rc::new(RefCell::new(
        std::collections::BTreeMap::<String, Vec<f32>>::new(),
    ));
    let observed = targets.clone();
    probe
        .global::<UiGeometry>()
        .on_measured(move |key, bounds| {
            observed
                .borrow_mut()
                .insert(key.to_string(), bounds.iter().collect());
        });
    probe.global::<UiGeometry>().set_enabled(true);
    let events = Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    probe
        .global::<MobileActions>()
        .on_request(move |intent, subject, _| {
            recorded.borrow_mut().push((intent, subject.to_string()));
        });
    probe.show().unwrap();
    let render = || {
        advance(&clock, Duration::from_millis(300));
        let size = probe.window().size();
        frame(&adapter, size.width as usize, size.height as usize);
        targets.borrow_mut().clear();
        let geometry = probe.global::<UiGeometry>();
        geometry.set_revision(geometry.get_revision().wrapping_add(1));
        advance(&clock, Duration::ZERO);
        frame(&adapter, size.width as usize, size.height as usize);
    };
    probe.set_modal(true);
    probe.set_detail(11);
    for (protected, pending) in [(false, false), (false, true), (true, false), (true, true)] {
        probe.set_protected(protected);
        probe.set_protection_pending(pending);
        events.borrow_mut().clear();
        render();
        let swatch = targets.borrow()["accent-swatch:2"].clone();
        assert!(swatch[2] >= 48. && swatch[3] >= 48.);
        let position = LogicalPosition::new(swatch[0] + swatch[2] / 2., swatch[1] + swatch[3] / 2.);
        probe.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        advance(&clock, Duration::from_millis(50));
        probe.window().dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
        render();
        assert_eq!(
            events.borrow().as_slice(),
            &[(MobileIntent::Accent, "2".to_owned())],
            "protected={protected}, pending={pending}"
        );
    }
}
