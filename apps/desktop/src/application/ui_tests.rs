//! Render the compiled production tree, and exercise its application boundary.
use super::*;
use clash_client_ui::{AppWindow, Intent, UiActions, UiData, models::ViewModels};
use slint::{ComponentHandle, Model};

#[test]
fn all_pages_and_dialogs_render_without_a_core_or_display_server() {
    let adapter = clash_client_ui::testing::initialize(950, 700);
    for (width, height, scale) in [(950, 700, 100), (800, 450, 125)] {
        for scene in [
            "home",
            "home-failure",
            "home-downloading",
            "proxies-ascending",
            "profiles",
            "connections",
            "connections-details",
            "rules",
            "rules-editor",
            "rules-discard",
            "rules-switch-confirm",
            "profiles-edit",
            "profiles-delete-active",
            "profiles-discard",
            "logs",
            "diagnostics",
            "ip-summary",
            "ip-empty",
            "ip-running",
            "ip-ai",
            "ip-details",
            "claude",
            "claude-risk",
            "claude-protection-guide",
            "settings",
        ] {
            let mut app = fixtures::fixture(scene);
            app.settings.text_scale = scale;
            let window = AppWindow::new().unwrap();
            adapter.set_size(slint::PhysicalSize::new(width, height));
            let mut models = ViewModels::default();
            let data = window.global::<UiData>();
            models.attach(&data);
            app.present(&data, &mut models);
            window.show().unwrap();
            let pixels = window.window().take_snapshot().unwrap();
            assert_eq!(
                (pixels.width(), pixels.height()),
                (width, height),
                "{scene}"
            );
            let mut colors = std::collections::HashSet::new();
            for pixel in pixels.as_bytes().as_chunks::<4>().0.iter().step_by(64) {
                colors.insert(*pixel);
            }
            assert!(colors.len() > 16, "blank render for {scene}");
            window.hide().unwrap();
        }
    }
}

#[test]
fn rule_editor_round_trip_and_cancel_preserve_the_draft_through_ui_intents() {
    let mut app = fixtures::fixture("rules");
    let _ = app.intent(Intent::RuleAdd, "", "");
    let _ = app.intent(Intent::RuleTextMode, "true", "");
    let _ = app.intent(Intent::RuleTextMode, "false", "");
    assert!(!app.rule_state.editor.as_ref().unwrap().text_mode);
    assert!(app.rule_state.editor.as_ref().unwrap().error.is_empty());
    let _ = app.intent(Intent::RulePayload, "example.test", "");
    let _ = app.intent(Intent::CancelConfirm, "", "");
    assert!(app.confirmation.is_some());
    let _ = app.intent(Intent::CancelConfirm, "", "");
    assert_eq!(
        app.rule_state.editor.as_ref().unwrap().payload,
        "example.test"
    );
    let _ = app.intent(Intent::Confirm, "", "");
    assert!(app.rule_state.editor.is_none());
    assert!(app.rule_state.dirty());
}

#[test]
fn projection_reuses_models_and_releases_hidden_and_inactive_lists() {
    clash_client_ui::testing::initialize(950, 700);
    let mut app = fixtures::fixture("connections");
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present(&data, &mut models);
    let first = data.get_connections();
    assert_eq!(first.row_count(), 3);
    app.present(&data, &mut models);
    assert_eq!(
        first,
        data.get_connections(),
        "polling must preserve model identity"
    );
    let _ = app.intent(Intent::ConnectionSort, "download-rate", "");
    app.present(&data, &mut models);
    assert_eq!(data.get_connections().row_data(0).unwrap().id, "2");
    let _ = app.intent(Intent::ConnectionSort, "download-rate", "");
    app.present(&data, &mut models);
    assert_eq!(data.get_connections().row_data(0).unwrap().id, "3");
    app.visible = false;
    app.present(&data, &mut models);
    assert_eq!(
        first.row_count(),
        0,
        "hidden window must release projected rows"
    );
    app.visible = true;
    app.page = Page::Settings;
    app.present(&data, &mut models);
    assert_eq!(first.row_count(), 0);
}

#[test]
fn imported_file_reaches_dialog_and_cancel_or_exit_requires_confirmation() {
    let mut app = fixtures::fixture("profiles");
    app.profile_name.clear();
    app.profile_source.clear();
    let _ = app.intent(Intent::AddProfile, "", "");
    let _ = app.intent(Intent::ProfileSource, "/home/user/subscription.yaml", "");
    let _ = app.intent(Intent::CancelConfirm, "", "");
    assert!(app.confirmation.is_some());
    let _ = app.intent(Intent::CancelConfirm, "", "");
    assert!(app.import_open);
    let _ = app.request_exit();
    assert!(app.confirmation.is_some());
    assert!(!app.exiting);
}

#[test]
fn pointer_keyboard_escape_and_modal_input_use_the_production_callbacks() {
    use slint::platform::{Key, PointerEventButton, WindowEvent};
    clash_client_ui::testing::initialize(950, 700);
    let app = fixtures::fixture("home");
    let window = AppWindow::new().unwrap();
    let mut models = ViewModels::default();
    let data = window.global::<UiData>();
    models.attach(&data);
    app.present(&data, &mut models);
    let events = std::rc::Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    window
        .global::<UiActions>()
        .on_dispatch(move |intent, a, b| recorded.borrow_mut().push((intent, a, b)));
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    let click = |x, y| {
        let position = slint::LogicalPosition::new(x, y);
        window.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
    };
    click(65.0, 156.0);
    assert_eq!(events.borrow().last().unwrap().0, Intent::Navigate);
    assert_eq!(events.borrow().last().unwrap().1, "1");
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    assert_eq!(
        events.borrow().last().unwrap().0,
        Intent::Escape,
        "Escape must bubble from focused controls"
    );
    events.borrow_mut().clear();
    data.set_modal(1);
    data.set_modal_title("确认删除？".into());
    data.set_modal_description("不会改变真实配置".into());
    data.set_modal_accept("删除".into());
    data.set_modal_can_accept(true);
    let _ = window.window().take_snapshot().unwrap();
    click(65.0, 156.0);
    assert!(
        events.borrow().is_empty(),
        "modal overlay must block background actions"
    );
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    assert_eq!(events.borrow().last().unwrap().0, Intent::Escape);
    window.hide().unwrap();
}
