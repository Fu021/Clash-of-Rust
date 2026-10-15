//! Exercise production component callbacks, model projection and application state.
use super::*;
use clash_client_ui::{AppWindow, Intent, UiActions, UiData, models::ViewModels};
use slint::{ComponentHandle, Model};

#[test]
fn rule_editor_round_trip_and_cancel_preserve_the_draft_through_ui_intents() {
    let mut app = fixture(Page::Rules);
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
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Connections);
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
    let mut app = fixture(Page::Profiles);
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
fn clicking_a_node_selects_it_but_its_latency_button_only_tests_it() {
    use clash_client_ui::{GroupItem, NodeItem, NodePair};
    ui_testing::initialize(950, 700);
    let app = fixture(Page::Proxies);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present(&data, &mut models);
    models.set_groups(vec![(
        GroupItem {
            id: "Proxy".into(),
            title: "Proxy".into(),
            expanded: true,
            ..Default::default()
        },
        vec![NodePair {
            left: NodeItem {
                group: "Proxy".into(),
                name: "node-a".into(),
                kind: "AnyTLS".into(),
                valid: true,
                ..Default::default()
            },
            ..Default::default()
        }],
    )]);
    let events = std::rc::Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    window
        .global::<UiActions>()
        .on_dispatch(move |intent, a, b| recorded.borrow_mut().push((intent, a, b)));
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    let click = |x, y| ui_testing::click(window.window(), x, y);
    click(507.0, 298.0);
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::NodeDelay, "node-a".into(), "".into())],
        "the first button click must run the action, not only focus it"
    );
    events.borrow_mut().clear();
    click(250.0, 252.0);
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::SelectNode, "Proxy".into(), "node-a".into())],
        "clicking the card must select the node in its own group"
    );
    events.borrow_mut().clear();
    click(507.0, 298.0);
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::NodeDelay, "node-a".into(), "".into())],
        "the latency button must consume the click without selecting the node"
    );
    window.hide().unwrap();
}

#[test]
fn pointer_keyboard_escape_and_modal_input_use_the_production_callbacks() {
    use slint::platform::{Key, WindowEvent};
    ui_testing::initialize(950, 700);
    let app = fixture(Page::Home);
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
    let click = |x, y| ui_testing::click(window.window(), x, y);
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

// Small fixtures used only by the interaction tests above. Visual previews
// inject their own data into ui/preview.slint without compiling the desktop.
fn fixture(page: Page) -> App {
    let mut app = App::initial_state(Err(anyhow::anyhow!("isolated UI test")), None);
    app.page = page;
    app.snapshot.running = true;
    app.core_failure = None;
    app.notice_visible = false;
    app.updates.checking = false;
    app.settings.active_profile = Some("fixture".into());
    app.profiles.push(Profile {
        id: "fixture".into(),
        name: "Test subscription".into(),
        source: String::new(),
        updated: 0,
        usage: None,
    });
    if page == Page::Rules {
        let doc = rule_manager::Document::from_raw("fixture", false,
            "proxies: []\nproxy-groups: [{name: Proxy, type: select, proxies: [DIRECT]}]\nrules: ['MATCH,Proxy']\n").unwrap();
        app.rule_state
            .load(Arc::new(doc), Arc::new(ProfileRules::default()));
    } else if page == Page::Connections {
        app.snapshot.connections = serde_json::from_value(serde_json::json!({"connections": [
            {"id":"1","metadata":{"host":"one.test"},"upload":0,"download":262144},
            {"id":"2","metadata":{"host":"two.test"},"upload":0,"download":2097152},
            {"id":"3","metadata":{"host":"three.test"},"upload":0,"download":0}
        ]}))
        .unwrap();
        let mut previous = app.snapshot.connections.connections.clone();
        for connection in &mut previous {
            connection.download = 0;
        }
        connection::sample_rates(
            &mut app.snapshot.connections.connections,
            &previous,
            Duration::from_secs(1),
        );
    }
    app
}
