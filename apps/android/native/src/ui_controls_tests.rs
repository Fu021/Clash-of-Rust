use crate::test_ui::{
    AndroidData, AndroidWindow, MobileActions, MobileData, MobileIntent, MobileView, SelectOption,
    UiGeometry,
};
use slint::{
    ComponentHandle, LogicalPosition, Model, PhysicalSize, Rgb8Pixel,
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

const WIDTH: u32 = 393;
const HEIGHT: u32 = 852;

struct Headless {
    adapter: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.adapter.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}

type Requests = Vec<(MobileIntent, String, String)>;

struct AndroidControls {
    window: AndroidWindow,
    adapter: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
    targets: Rc<RefCell<BTreeMap<String, Vec<f32>>>>,
    requests: Rc<RefCell<Requests>>,
}

impl AndroidControls {
    fn new(page: i32) -> Self {
        let adapter = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(Headless {
            adapter: adapter.clone(),
            clock: clock.clone(),
        }))
        .unwrap();
        adapter.set_size(PhysicalSize::new(WIDTH, HEIGHT));
        let window = AndroidWindow::new().unwrap();
        window.set_current_page(page);
        let targets = Rc::new(RefCell::new(BTreeMap::new()));
        let observed = targets.clone();
        let geometry = window.global::<UiGeometry>();
        geometry.on_measured(move |key, bounds| {
            observed
                .borrow_mut()
                .insert(key.to_string(), bounds.iter().collect());
        });
        geometry.set_enabled(true);
        let requests = Rc::new(RefCell::new(Vec::new()));
        let recorded = requests.clone();
        window
            .global::<MobileActions>()
            .on_request(move |intent, first, second| {
                recorded
                    .borrow_mut()
                    .push((intent, first.to_string(), second.to_string()));
            });
        window.show().unwrap();
        Self {
            window,
            adapter,
            clock,
            targets,
            requests,
        }
    }

    fn advance(&self, milliseconds: u64) {
        let deadline = self.clock.get() + Duration::from_millis(milliseconds);
        slint::platform::update_timers_and_animations();
        while self.clock.get() < deadline {
            self.clock
                .set((self.clock.get() + Duration::from_millis(10)).min(deadline));
            slint::platform::update_timers_and_animations();
        }
    }

    fn render(&self) {
        self.advance(300);
        self.targets.borrow_mut().clear();
        let geometry = self.window.global::<UiGeometry>();
        geometry.set_revision(geometry.get_revision().wrapping_add(1));
        self.advance(0);
        self.window.window().request_redraw();
        let mut pixels = vec![Rgb8Pixel::default(); (WIDTH * HEIGHT) as usize];
        assert!(self.adapter.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, WIDTH as usize);
        }));
    }

    fn click(&self, key: &str, popup: Option<&str>) {
        let targets = self.targets.borrow();
        let bounds = &targets[key];
        let (offset_x, offset_y) = popup.map_or((0., 0.), |key| {
            let bounds = &targets[key];
            (
                bounds[0].clamp(0., (WIDTH as f32 - bounds[2]).max(0.)),
                bounds[1].clamp(0., (HEIGHT as f32 - bounds[3]).max(0.)),
            )
        });
        let position = LogicalPosition::new(
            offset_x + bounds[0] + bounds[2] / 2.,
            offset_y + bounds[1] + bounds[3] / 2.,
        );
        assert!(
            (0. ..WIDTH as f32).contains(&position.x) && (0. ..HEIGHT as f32).contains(&position.y),
            "{key} is outside the window: {position:?}"
        );
        drop(targets);
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
        self.advance(50);
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        self.render();
    }
}

#[test]
fn android_rules_select_subscription_id_when_display_names_match() {
    let controls = AndroidControls::new(4);
    let data = controls.window.global::<MobileData>();
    data.set_rule_profiles(
        Rc::new(slint::VecModel::from(vec![
            SelectOption {
                id: "first-profile".into(),
                label: "Shared name".into(),
            },
            SelectOption {
                id: "second-profile".into(),
                label: "Shared name".into(),
            },
        ]))
        .into(),
    );
    data.set_rule_profile("first-profile".into());
    data.set_rule_profile_label("Shared name".into());
    controls.render();
    controls.click("rule-profile", None);
    controls.requests.borrow_mut().clear();
    controls.click("option:rule-profile/1", Some("popup:rule-profile"));
    assert_eq!(
        controls.requests.borrow().as_slice(),
        &[(
            MobileIntent::RuleSelect,
            "second-profile".into(),
            "profile".into()
        )]
    );
}

#[test]
fn android_profile_editor_locks_url_and_save_during_file_selection() {
    let controls = AndroidControls::new(2);
    controls.window.set_current_detail(3);
    let data = controls.window.global::<MobileData>();
    let view = controls.window.global::<MobileView>();
    view.set_profile_name("Local test".into());
    data.set_profile_file_pending(true);
    controls.render();
    controls.requests.borrow_mut().clear();
    controls.click("profile-source", None);
    assert!(!view.get_profile_source_focused());
    controls.click("profile-save", None);
    controls.click("profile-pick-file", None);
    assert!(controls.requests.borrow().is_empty());

    data.set_profile_file_pending(false);
    data.set_profile_file_selected(true);
    controls.render();
    controls.click("profile-source", None);
    assert!(!view.get_profile_source_focused());
    controls.click("profile-save", None);
    controls.click("profile-clear-file", None);
    assert_eq!(
        controls.requests.borrow().as_slice(),
        &[
            (MobileIntent::SaveProfile, "Local test".into(), "".into()),
            (MobileIntent::ProfileClearFile, "".into(), "".into()),
        ]
    );

    data.set_profile_file_selected(false);
    controls.render();
    controls.click("profile-source", None);
    assert!(view.get_profile_source_focused());
}

#[test]
fn android_ipv6_toggle_emits_no_request_while_platform_locks_it() {
    let controls = AndroidControls::new(9);
    let android = controls.window.global::<AndroidData>();
    controls.window.global::<MobileData>().set_ipv6(true);
    android.set_ipv6_locked(true);
    controls.render();
    let view = controls.window.global::<MobileView>();
    let bounds = controls.targets.borrow()["ipv6-toggle"].clone();
    let center = bounds[1] + bounds[3] / 2.;
    let min_scroll =
        (view.get_settings_viewport_height() - view.get_settings_content_height()).min(0.);
    view.set_settings_scroll_y(
        (view.get_settings_scroll_y() + HEIGHT as f32 / 2. - center).clamp(min_scroll, 0.),
    );
    controls.render();
    controls.requests.borrow_mut().clear();
    controls.click("ipv6-toggle", None);
    assert!(controls.requests.borrow().is_empty());

    android.set_ipv6_locked(false);
    controls.render();
    controls.click("ipv6-toggle", None);
    assert_eq!(
        controls.requests.borrow().as_slice(),
        &[(MobileIntent::Ipv6, "false".into(), "".into())]
    );
}
