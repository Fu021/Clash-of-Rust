//! Exercise production component callbacks, model projection and application state.
use super::*;
use clash_client_ui::{AppWindow, Intent, UiActions, UiData, models::ViewModels};
use slint::{ComponentHandle, Model};

#[test]
fn closing_a_foreground_window_while_the_tray_starts_waits_before_exiting() {
    let mut app = fixture(Page::Home);
    app.visible = true;
    app.tray_startup = Some(tray::Startup::new(Duration::from_millis(1)));
    let id = runtime::window::Id::unique();

    let _ = app.update(Message::Window((
        id,
        runtime::window::Event::CloseRequested,
    )));
    assert!(app.visible);
    assert!(!app.exiting);
    assert_eq!(app.tray_close_requested, Some(id));
    assert!(app.notice_visible);

    let _ = app.update(Message::ShowWindow(Some(id)));
    assert!(app.tray_close_requested.is_none());
    let _ = app.update(Message::Window((
        id,
        runtime::window::Event::CloseRequested,
    )));
    assert_eq!(app.tray_close_requested, Some(id));
    let _ = app.update(Message::Window((
        id,
        runtime::window::Event::CloseRequested,
    )));
    assert!(app.exiting);
    assert!(app.tray_close_requested.is_none());
}

#[test]
fn dropping_rules_uses_identities_in_the_full_draft_and_ignores_stale_rows() {
    let mut app = fixture(Page::Rules);
    let document = rule_manager::Document::from_raw(
        "fixture",
        false,
        "proxies: []\nrules: ['DOMAIN,github.one,DIRECT', 'DOMAIN,hidden.test,DIRECT', 'DOMAIN,github.two,DIRECT', 'MATCH,DIRECT']\n",
    ).unwrap();
    app.rule_state
        .load(Arc::new(document), Arc::new(ProfileRules::default()));
    let before: Vec<_> = app
        .rule_state
        .rows
        .iter()
        .map(|row| row.id.clone())
        .collect();
    assert!(before.len() >= 3);
    let _ = app.ui_intent(Intent::RuleFilter, "3", "");
    let _ = app.ui_intent(Intent::Query, "github", "");
    let _ = app.ui_intent(Intent::RuleDrop, &before[0], &before[2]);
    let after: Vec<_> = app
        .rule_state
        .rows
        .iter()
        .map(|row| row.id.clone())
        .collect();
    assert_eq!(after[2], before[0]);
    assert_eq!(app.rule_state.selected.as_deref(), Some(before[0].as_str()));
    assert_eq!(app.rule_state.saved.order, Vec::<String>::new());
    assert!(app.rule_state.dirty());
    let draft = app.rule_state.draft.clone();
    let _ = app.ui_intent(Intent::RuleDrop, &before[0], "removed-target");
    let _ = app.ui_intent(Intent::RuleDrop, "removed-source", &before[1]);
    assert_eq!(app.rule_state.draft, draft);
    app.working = true;
    let _ = app.ui_intent(Intent::RuleDrop, &before[0], &before[1]);
    assert_eq!(app.rule_state.draft, draft);
}

#[test]
fn desktop_rule_handle_edges_lift_the_real_row_and_follow_the_pointer() {
    use slint::platform::{PointerEventButton, WindowEvent, software_renderer::RepaintBufferType};
    let adapter = ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Rules);
    let document = rule_manager::Document::from_raw(
        "fixture",
        false,
        "proxies: []\nrules: ['DOMAIN,first.test,DIRECT', 'DOMAIN,second.test,DIRECT', 'DOMAIN,third.test,DIRECT', 'MATCH,DIRECT']\n",
    )
    .unwrap();
    app.rule_state
        .load(Arc::new(document), Arc::new(ProfileRules::default()));
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    data.set_appearance(1);
    data.set_accent(0);
    let events = pointer_events(&window);
    window.show().unwrap();
    let mut pixels = vec![slint::Rgb8Pixel::default(); 950 * 700];
    window.window().request_redraw();
    assert!(adapter.draw_if_needed(|renderer| {
        renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
        renderer.render(&mut pixels, 950);
        renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
        renderer.render(&mut pixels, 950);
    }));
    let mut border_y = || {
        window.window().request_redraw();
        assert!(adapter.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 950);
            let mut full = vec![slint::Rgb8Pixel::default(); 950 * 700];
            renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
            renderer.render(&mut full, 950);
            assert!(pixels == full, "the drag preview must repaint correctly");
            renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
            renderer.render(&mut pixels, 950);
        }));
        (300..600)
            .find(|&y| {
                (350..600).all(|x| pixels[y * 950 + x] == slint::Rgb8Pixel::new(15, 118, 110))
            })
            .expect("the lifted row must render a full-width accent border")
    };
    for x in [225., 247.] {
        let start = slint::LogicalPosition::new(x, 355.);
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: start });
        window.window().dispatch_event(WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        });
        let top = border_y();
        let end = slint::LogicalPosition::new(x, 435.);
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: end });
        assert_eq!(border_y(), top + 80, "the preview must follow the pointer");
        window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position: end,
                button: PointerEventButton::Left,
            });
        assert!(
            events
                .borrow_mut()
                .iter()
                .any(|event| event.0 == Intent::RuleDrop),
            "the full rules page must dispatch the reorder"
        );
        events.borrow_mut().clear();
    }
    window.hide().unwrap();
}

#[test]
fn rule_editor_round_trip_and_cancel_preserve_the_draft_through_ui_intents() {
    let mut app = fixture(Page::Rules);
    let _ = app.ui_intent(Intent::RuleAdd, "", "");
    let _ = app.ui_intent(Intent::RuleTextMode, "true", "");
    let _ = app.ui_intent(Intent::RuleTextMode, "false", "");
    assert!(!app.rule_state.editor.as_ref().unwrap().text_mode);
    assert!(app.rule_state.editor.as_ref().unwrap().error.is_empty());
    let _ = app.ui_intent(Intent::RulePayload, "example.test", "");
    let _ = app.ui_intent(Intent::CancelConfirm, "", "");
    assert!(app.confirmation.is_some());
    let _ = app.ui_intent(Intent::CancelConfirm, "", "");
    assert_eq!(
        app.rule_state.editor.as_ref().unwrap().payload,
        "example.test"
    );
    let _ = app.ui_intent(Intent::Confirm, "", "");
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
    app.present_slint(&data, &mut models);
    let first = data.get_connections();
    assert_eq!(first.row_count(), 3);
    app.present_slint(&data, &mut models);
    assert_eq!(
        first,
        data.get_connections(),
        "polling must preserve model identity"
    );
    let _ = app.ui_intent(Intent::ConnectionSort, "download-rate", "");
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_connections().row_data(0).unwrap().id, "2");
    let _ = app.ui_intent(Intent::ConnectionSort, "download-rate", "");
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_connections().row_data(0).unwrap().id, "3");
    app.visible = false;
    app.present_slint(&data, &mut models);
    assert_eq!(
        first.row_count(),
        0,
        "hidden window must release projected rows"
    );
    app.visible = true;
    app.page = Page::Settings;
    app.present_slint(&data, &mut models);
    assert_eq!(first.row_count(), 0);
}

#[test]
fn connection_sort_clicks_redraw_the_header_immediately_and_third_click_restores_rows() {
    use slint::platform::software_renderer::RepaintBufferType;
    let adapter = ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Connections);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let events = pointer_events(&window);
    window.show().unwrap();
    let mut pixels = vec![slint::Rgb8Pixel::default(); 950 * 700];
    let mut header = || {
        assert!(adapter.draw_if_needed(|renderer| {
            renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
            renderer.render(&mut pixels, 950);
            let mut reference = vec![slint::Rgb8Pixel::default(); 950 * 700];
            renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
            renderer.render(&mut reference, 950);
            assert!(
                pixels == reference,
                "partial repaint differs from a fresh frame at pixel {:?}",
                pixels.iter().zip(&reference).position(|(a, b)| a != b)
            );
            renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
            // Changing buffer modes clears Slint's damage cache. Warm it now,
            // so the next click really renders incrementally into this buffer.
            renderer.render(&mut pixels, 950);
        }));
        pixels
            .as_chunks::<950>()
            .0
            .iter()
            .skip(156)
            .take(52)
            .flat_map(|row| row[200..926].iter().copied())
            .collect::<Vec<_>>()
    };
    let before = header();
    let original = data
        .get_connections()
        .iter()
        .map(|row| row.id)
        .collect::<Vec<_>>();
    for (x, y, field, first_descending) in [
        (780., 174., "download-rate", true),
        (780., 197., "download-total", true),
        (670., 174., "upload-rate", true),
        (670., 197., "upload-total", true),
        (480., 174., "rule", false),
        (480., 197., "chain", false),
        (260., 174., "name", false),
        (260., 197., "start", true),
    ] {
        let mut frames = Vec::new();
        for click in 0..3 {
            ui_testing::click(window.window(), x, y);
            let event = events
                .borrow_mut()
                .pop()
                .expect("heading must dispatch one sort intent");
            assert_eq!(event.0, Intent::ConnectionSort);
            assert_eq!(event.1, field);
            let _ = app.ui_intent(event.0, &event.1, &event.2);
            app.present_slint(&data, &mut models);
            assert_eq!(data.get_page(), Page::Connections as i32);
            assert_eq!(
                data.get_connection_sort(),
                if click == 2 { "" } else { field }
            );
            assert_eq!(
                data.get_descending(),
                match click {
                    0 => first_descending,
                    1 => !first_descending,
                    _ => false,
                },
                "wrong direction for {field} on click {click}"
            );
            frames.push(header());
        }
        assert_ne!(
            frames[0], before,
            "first click must render an arrow without navigation"
        );
        assert_ne!(
            frames[1], frames[0],
            "second click must reverse the arrow immediately"
        );
        assert_eq!(frames[2], before, "third click must remove the indicator");
        assert_eq!(
            data.get_connections()
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            original
        );
    }
    // Record the normal down-arrow frame, then return to unsorted before a
    // burst of real pointer events with no rendered frame between callbacks.
    let _ = app.ui_intent(Intent::ConnectionSort, "download-rate", "");
    app.present_slint(&data, &mut models);
    let expected = header();
    for _ in 0..2 {
        let _ = app.ui_intent(Intent::ConnectionSort, "download-rate", "");
    }
    app.present_slint(&data, &mut models);
    assert_eq!(header(), before);
    for _ in 0..4 {
        for (x, y) in [(670., 174.), (480., 197.), (260., 174.), (780., 174.)] {
            ui_testing::rapid_click(window.window(), x, y);
        }
    }
    let pending = std::mem::take(&mut *events.borrow_mut());
    assert_eq!(pending.len(), 16, "rapid clicks must each dispatch once");
    for (index, (intent, a, b)) in pending.into_iter().enumerate() {
        let _ = app.ui_intent(intent, &a, &b);
        if index % 3 == 0 {
            // A completed background connection refresh must not overwrite
            // newer sort intent while the queued UI commands are being drained.
            let mut snapshot = app.snapshot.clone();
            snapshot.connections.connections[0].download += 4096;
            let _ = app.update(Message::Finished(Box::new(Reply {
                view_revision: 0,
                remote: false,
                profile_edit: false,
                profile_import: false,
                profile_updated: false,
                scope: Scope::Connections,
                start_requested: false,
                settings: app.settings.clone(),
                profiles: None,
                snapshot: Some(snapshot),
                rule_document: None,
                rules_applied: false,
                notice: Ok(String::new()),
                running: true,
                geo_status: String::new(),
                autostart: false,
                protection_changed: false,
                tray_nodes: None,
            })));
        }
        app.present_slint(&data, &mut models);
    }
    assert_eq!(data.get_connection_sort(), "download-rate");
    assert!(data.get_descending());
    assert_eq!(
        header(),
        expected,
        "burst clicks must paint the final arrow"
    );
    window.hide().unwrap();
}

#[test]
fn connection_detail_exposes_all_fields_and_copies_the_same_values() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Connections);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let _ = app.ui_intent(Intent::ConnectionDetail, "1", "");
    app.present_slint(&data, &mut models);
    let details = data.get_connection_details();
    assert_eq!(details.row_count(), 14);
    for row in details.iter() {
        assert!(!row.value.is_empty());
        assert!(
            data.get_modal_copy()
                .contains(&format!("{}：{}", row.label, row.value))
        );
    }
    assert!(data.get_modal_copy().contains("下载速度：256.0 KiB/s"));
    assert!(data.get_modal_copy().contains("嗅探域名：one.test"));
    assert!(data.get_modal_copy().contains("目标 IP：—"));
    let _ = app.update(Message::Finished(Box::new(Reply {
        view_revision: 0,
        remote: false,
        profile_edit: false,
        profile_import: false,
        profile_updated: false,
        scope: Scope::Connections,
        start_requested: false,
        settings: app.settings.clone(),
        profiles: None,
        snapshot: Some(Snapshot {
            running: true,
            ..Snapshot::default()
        }),
        rule_document: None,
        rules_applied: false,
        notice: Ok(String::new()),
        running: true,
        geo_status: String::new(),
        autostart: false,
        protection_changed: false,
        tray_nodes: None,
    })));
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_connection_details().row_count(), 0);
    assert!(data.get_modal_copy().is_empty());
    assert!(data.get_modal_description().contains("已结束"));
}

#[test]
fn imported_file_reaches_dialog_and_cancel_or_exit_requires_confirmation() {
    let mut app = fixture(Page::Profiles);
    app.profile_name.clear();
    app.profile_source.clear();
    let _ = app.ui_intent(Intent::AddProfile, "", "");
    let _ = app.ui_intent(Intent::ProfileSource, "/home/user/subscription.yaml", "");
    let _ = app.ui_intent(Intent::CancelConfirm, "", "");
    assert!(app.confirmation.is_some());
    let _ = app.ui_intent(Intent::CancelConfirm, "", "");
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
    app.present_slint(&data, &mut models);
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
    data.set_protected(true);
    let _ = window.window().take_snapshot().unwrap();
    click(250.0, 252.0);
    assert!(
        events.borrow().is_empty(),
        "protected nodes must not dispatch a selection"
    );
    data.set_protected(false);
    let _ = window.window().take_snapshot().unwrap();
    events.borrow_mut().clear();
    click(260.0, 205.0);
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::ToggleGroup, "Proxy".into(), "".into())]
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
    app.present_slint(&data, &mut models);
    let events = std::rc::Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    window
        .global::<UiActions>()
        .on_dispatch(move |intent, a, b| recorded.borrow_mut().push((intent, a, b)));
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    let click = |x, y| ui_testing::click(window.window(), x, y);
    click(28.0, 36.0);
    assert_eq!(
        events.borrow().last().unwrap().0,
        Intent::SystemNotification
    );
    events.borrow_mut().clear();
    data.set_notification_busy(true);
    click(28.0, 36.0);
    assert!(
        events.borrow().is_empty(),
        "only one native notification may be active"
    );
    data.set_notification_busy(false);
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
    click(28.0, 36.0);
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
pub(super) fn fixture(page: Page) -> App {
    let mut app = App::initial_state(Err(anyhow::anyhow!("isolated UI test")), None);
    app.page = page;
    app.notification_prepared = true;
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

type UiEventLog = std::rc::Rc<RefCell<Vec<(Intent, slint::SharedString, slint::SharedString)>>>;
fn pointer_events(window: &AppWindow) -> UiEventLog {
    // Existing interaction assertions intentionally inspect the settled UI.
    // Dedicated motion tests below enable animations and advance the clock.
    window
        .global::<clash_client_ui::Motion>()
        .set_enabled(false);
    let events = std::rc::Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    window
        .global::<UiActions>()
        .on_dispatch(move |intent, a, b| recorded.borrow_mut().push((intent, a, b)));
    events
}

#[test]
fn node_sort_popup_dispatches_each_clicked_option_and_reorders_real_models() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Proxies);
    app.busy = true; // retain effects for inspection without starting a real core
    app.expanded.insert("Proxy".into());
    app.snapshot.proxies = serde_json::from_value(serde_json::json!({"proxies": {
        "Proxy": {"type":"Selector", "now":"node2", "all":["node10","node2","node1"]},
        "node10": {"type":"AnyTLS", "history":[{"delay":20}]},
        "node2": {"type":"AnyTLS", "history":[{"delay":10}]},
        "node1": {"type":"AnyTLS", "history":[{"delay":30}]}
    }}))
    .unwrap();
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    assert_eq!(
        data.get_node_sort(),
        2,
        "new preferences default to name order"
    );
    let events = pointer_events(&window);
    window.show().unwrap();
    for (index, names) in [
        (0, ["node2", "node10", "node1"]),
        (1, ["node1", "node10", "node2"]),
        (2, ["node1", "node2", "node10"]),
    ] {
        let _ = window.window().take_snapshot().unwrap();
        ui_testing::click(window.window(), 830., 124.);
        let _ = window.window().take_snapshot().unwrap();
        ui_testing::click(window.window(), 830., 176. + index as f32 * 44.);
        let events = events.borrow_mut().drain(..).collect::<Vec<_>>();
        assert_eq!(
            events.len(),
            1,
            "one pointer selection must emit one intent"
        );
        assert_eq!(
            events[0],
            (Intent::NodeSort, index.to_string().into(), "".into())
        );
        let _ = app.ui_intent(events[0].0, &events[0].1, &events[0].2);
        app.present_slint(&data, &mut models);
        let group = data.get_groups().row_data(0).unwrap();
        let first = group.nodes.row_data(0).unwrap();
        let second = group.nodes.row_data(1).unwrap();
        assert_eq!(
            [
                first.left.name.as_str(),
                first.right.name.as_str(),
                second.left.name.as_str()
            ],
            names
        );
        assert!(
            matches!(app.queued_actions.pop_back(), Some(Action::NodeSort(mode)) if mode == NodeSort::ALL[index])
        );
    }
    window.hide().unwrap();
}

#[test]
fn a_discarded_software_surface_is_fully_repainted_after_restore() {
    use slint::platform::software_renderer::RepaintBufferType;
    let adapter = ui_testing::initialize(950, 700);
    let app = fixture(Page::Settings);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    window.show().unwrap();
    let mut before = vec![slint::Rgb8Pixel::default(); 950 * 700];
    assert!(adapter.draw_if_needed(|renderer| {
        renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
        renderer.render(&mut before, 950);
    }));
    let mut restored = vec![slint::Rgb8Pixel::default(); 950 * 700];
    // DWM/compositor invalidation is independent of changes to the UI models.
    bindings::invalidate_window_for_test(window.window());
    assert!(adapter.draw_if_needed(|renderer| {
        renderer.render(&mut restored, 950);
        renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
    }));
    assert_eq!(
        before, restored,
        "restored surface must contain the unchanged full window"
    );
    window.hide().unwrap();
}

#[test]
fn appearance_remains_interactive_while_geo_owns_the_engine_and_late_reply_cannot_undo_it() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Settings);
    app.busy = true;
    app.working = true;
    app.updates.started = true;
    let old_settings = app.settings.clone();
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let events = pointer_events(&window);
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 840., 233.);
    assert_eq!(events.borrow_mut().pop().unwrap().0, Intent::AccentPicker);
    let _ = app.ui_intent(Intent::AccentPicker, "", "");
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_modal(), 12);
    assert!(data.get_modal_can_accept());
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 475., 314.); // blue swatch
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 710., 460.); // complete while Geo is still busy
    let chosen = events
        .borrow_mut()
        .pop()
        .expect("busy Geo must not disable appearance confirmation");
    assert_eq!(chosen.0, Intent::AccentColor);
    assert_eq!(chosen.1, "1");
    let _ = app.ui_intent(chosen.0, &chosen.1, &chosen.2);
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_accent(), 1);
    for color in [2, 3, 5] {
        let _ = app.ui_intent(Intent::AccentColor, &color.to_string(), "");
    }
    assert_eq!(app.queued_actions.len(), 1, "rapid changes must coalesce");
    let _ = app.update(Message::Finished(Box::new(Reply {
        view_revision: 0,
        remote: false,
        profile_edit: false,
        profile_import: false,
        profile_updated: false,
        scope: Scope::Other,
        start_requested: false,
        settings: old_settings,
        profiles: None,
        rule_document: None,
        rules_applied: false,
        snapshot: None,
        notice: Ok("Geo 数据已更新".into()),
        running: true,
        geo_status: "更新完成".into(),
        autostart: false,
        protection_changed: false,
        tray_nodes: None,
    })));
    app.present_slint(&data, &mut models);
    assert_eq!(
        data.get_accent(),
        5,
        "old engine reply must preserve the most recent color"
    );
    window.hide().unwrap();
}

#[test]
fn font_popup_changes_the_saved_request_and_rendered_font_at_fractional_scale() {
    use clash_client_ui::Theme;
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Settings);
    app.busy = true;
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let events = pointer_events(&window);
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    let before = window.global::<Theme>().get_body_size();
    ui_testing::click(window.window(), 860., 290.);
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 860., 518.); // fifth option: 125%
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::TextScale, "125%".into(), "".into())]
    );
    let _ = app.ui_intent(Intent::TextScale, "125%", "");
    assert!(matches!(
        app.queued_actions.pop_back(),
        Some(Action::TextScale(125))
    ));
    app.settings.text_scale = 125;
    app.present_slint(&data, &mut models);
    slint::platform::update_timers_and_animations();
    let _ = window.window().take_snapshot().unwrap();
    assert!((window.global::<Theme>().get_body_size() - before * 1.25).abs() < 0.01);
    // Restoring 100% must shrink the same rendered controls again.
    app.settings.text_scale = 100;
    app.present_slint(&data, &mut models);
    slint::platform::update_timers_and_animations();
    let _ = window.window().take_snapshot().unwrap();
    assert!((window.global::<Theme>().get_body_size() - before).abs() < 0.01);
    window.hide().unwrap();
}

#[test]
fn rule_editor_popups_deliver_type_and_policy_inside_a_modal() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Rules);
    let _ = app.ui_intent(Intent::RuleAdd, "", "");
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let events = pointer_events(&window);
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 500., 266.);
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 500., 406.); // DOMAIN-KEYWORD, nonzero index
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::RuleKind, "DOMAIN-KEYWORD".into(), "".into())]
    );
    let event = events.borrow_mut().pop().unwrap();
    let _ = app.ui_intent(event.0, &event.1, &event.2);
    app.present_slint(&data, &mut models);
    let _ = window.window().take_snapshot().unwrap();
    assert_eq!(data.get_rule_kind(), "DOMAIN-KEYWORD");
    let policy = data.get_rule_policies().row_data(1).unwrap();
    ui_testing::click(window.window(), 500., 370.);
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 500., 466.);
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::RulePolicy, policy.clone(), "".into())]
    );
    let event = events.borrow_mut().pop().unwrap();
    let _ = app.ui_intent(event.0, &event.1, &event.2);
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_rule_policy(), policy);
    window.hide().unwrap();
}

#[test]
fn setting_and_toolbar_intents_preserve_their_values_in_the_command_queue() {
    let cases = [
        (
            Intent::Appearance,
            "1",
            "",
            Action::Appearance(ThemeMode::Light),
        ),
        (Intent::TextScale, "125%", "", Action::TextScale(125)),
        (Intent::TraySpeed, "true", "", Action::TraySpeed(true)),
        (Intent::DnsOverride, "false", "", Action::DnsOverride(false)),
        (Intent::FakeIp, "false", "", Action::FakeIp(false)),
        (Intent::Ipv6, "true", "", Action::Ipv6(true)),
        (Intent::Udp, "false", "", Action::Udp(false)),
        (Intent::Autostart, "true", "", Action::Autostart(true)),
        (
            Intent::ProxyMode,
            "1",
            "",
            Action::ProxyMode(ProxyMode::Tun),
        ),
        (Intent::Mode, "2", "", Action::Mode("direct".into())),
        (
            Intent::SelectNode,
            "Proxy",
            "node2",
            Action::Select("Proxy".into(), "node2".into()),
        ),
        (
            Intent::CloseConnection,
            "123",
            "",
            Action::Close("123".into()),
        ),
        (Intent::CloseAll, "", "", Action::CloseAll),
        (Intent::ClearLogs, "", "", Action::ClearLogs),
        (Intent::ExportLogs, "", "", Action::ExportLogs),
        (
            Intent::ActivateProfile,
            "fixture",
            "",
            Action::Activate("fixture".into()),
        ),
        (
            Intent::UpdateProfile,
            "fixture",
            "",
            Action::UpdateProfile("fixture".into()),
        ),
        (
            Intent::NodeDelay,
            "node2",
            "",
            Action::Delay("node2".into()),
        ),
    ];
    for (intent, a, b, expected) in cases {
        let mut app = fixture(Page::Settings);
        app.busy = true;
        let _ = app.ui_intent(intent, a, b);
        assert_eq!(
            format!("{:?}", app.queued_actions.pop_back().unwrap()),
            format!("{expected:?}")
        );
    }
    let mut app = fixture(Page::Settings);
    app.busy = true;
    let _ = app.ui_intent(Intent::MixedPort, "7899", "");
    let _ = app.ui_intent(Intent::SavePorts, "", "");
    assert!(matches!(
        app.queued_actions.pop_back(),
        Some(Action::SavePorts(7899))
    ));
    let _ = app.ui_intent(Intent::MixedPort, "invalid", "");
    let _ = app.ui_intent(Intent::SavePorts, "", "");
    assert!(app.queued_actions.is_empty());
    let mut app = fixture(Page::Settings);
    app.busy = true;
    app.settings.fake_ip = true;
    app.settings.fake_ip_exclusions = vec!["existing.test".into()];
    app.fake_ip_input = "+.example.test, ds.example.test".into();
    let _ = app.ui_intent(Intent::FakeIpAdd, "", "");
    assert!(matches!(
        app.queued_actions.pop_back(),
        Some(Action::FakeIpAdd(values))
            if values == vec![
                "+.example.test".to_owned(),
                "ds.example.test".to_owned()
            ]
    ));
    let mut app = fixture(Page::Settings);
    app.busy = true;
    app.settings.fake_ip_exclusions = vec!["existing.test".into(), "another.test".into()];
    let _ = app.ui_intent(Intent::FakeIpRemove, "existing.test", "");
    assert!(matches!(
        app.queued_actions.pop_back(),
        Some(Action::FakeIpRemove(domain)) if domain == "existing.test"
    ));
    let mut app = fixture(Page::Settings);
    let _ = app.ui_intent(Intent::FakeIpEditor, "true", "");
    assert!(app.fake_ip_editor);
    let _ = app.ui_intent(Intent::FakeIpEditor, "false", "");
    assert!(!app.fake_ip_editor);
    let mut app = fixture(Page::Settings);
    app.busy = true;
    let _ = app.ui_intent(Intent::IntervalInput, "15", "");
    let _ = app.ui_intent(Intent::SaveInterval, "", "");
    assert!(matches!(
        app.queued_actions.pop_back(),
        Some(Action::DelayInterval(15))
    ));
}

#[test]
fn ip_report_uses_bitmaps_in_summary_rows_and_details() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Websites);
    app.site_results.push(Some(Ok(ip_check::CheckResult {
        state: ip_check::State::Identified,
        summary: "203.0.113.2".into(),
        country: "🇩🇪 德国".into(),
        millis: 4,
        detail: String::new(),
    })));
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    assert!(data.get_site_exit_flagged());
    assert_eq!(data.get_site_exit_country(), "德国");
    assert_eq!(data.get_site_exit_flag().size().width, 72);
    assert!(!data.get_site_summary().contains('🇩'));
    let _ = app.ui_intent(Intent::SiteDetail, "exit-ip", "");
    app.present_slint(&data, &mut models);
    assert!(data.get_modal_flagged());
    assert_eq!(data.get_modal_country(), "德国");
    assert!(!data.get_modal_description().contains('🇩'));
}

#[test]
fn recursive_dns_flags_follow_the_country_without_changing_risk_or_other_signals() {
    use clash_of_rust::claude_check::{Environment, Evidence, Group, Report, Signal};

    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Claude);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    for (code, label, flagged) in [
        ("DE", "德国", true),
        ("US", "美国", true),
        ("HK", "香港", false),
        ("MO", "澳门", false),
        ("TW", "台湾", false),
        ("", "", false),
        ("ZZ", "", false),
    ] {
        let mut report = Report::new(Environment::Core, "fixture");
        let mut dns = Signal::new(
            Group::Dns,
            "实际递归 DNS",
            "original observation",
            "fixture",
            Evidence::Measured,
            "country evidence",
        );
        dns.country = code.into();
        dns.ip = "2001:db8:1234:5678:abcd:ef01:2345:6789".into();
        dns.risk = true;
        report.signals.push(dns);
        let mut other = Signal::new(
            Group::Exit,
            "出口",
            "unchanged exit",
            "fixture",
            Evidence::Measured,
            "",
        );
        other.country = "DE".into();
        other.ip = "203.0.113.2".into();
        report.signals.push(other);
        app.claude.reports[0] = Some(report);
        app.present_slint(&data, &mut models);
        let rows = data.get_claude_results();
        let dns = rows.row_data(0).unwrap();
        assert_eq!(dns.country, label, "{code}");
        assert_eq!(dns.flagged, flagged, "{code}");
        assert_eq!(dns.flag.size().width > 0, flagged, "{code}");
        assert_eq!(dns.tone, 3, "{code}");
        assert_eq!(
            dns.value,
            if label.is_empty() {
                "original observation"
            } else {
                "2001:db8:1234:5678:abcd:ef01:2345:6789"
            },
            "{code}"
        );
        let exit = rows.row_data(1).unwrap();
        assert!(exit.country.is_empty());
        assert!(!exit.flagged);
        assert_eq!(exit.value, "unchanged exit");
    }
}

#[test]
fn profile_and_rule_source_popups_preserve_ids_and_filters() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Rules);
    app.busy = true;
    app.profiles.push(Profile {
        id: "second".into(),
        name: "Second subscription".into(),
        source: String::new(),
        updated: 0,
        usage: None,
    });
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let events = pointer_events(&window);
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 400., 124.);
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 400., 220.);
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::RuleProfile, "second".into(), "".into())]
    );
    let event = events.borrow_mut().pop().unwrap();
    let _ = app.ui_intent(event.0, &event.1, &event.2);
    assert!(matches!(app.queued_actions.pop_back(), Some(Action::LoadRules(id)) if id == "second"));
    ui_testing::click(window.window(), 750., 260.);
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 750., 356.);
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::RuleFilter, "1".into(), "".into())]
    );
    let event = events.borrow_mut().pop().unwrap();
    let _ = app.ui_intent(event.0, &event.1, &event.2);
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_rule_source(), "自定义");
    window.hide().unwrap();
}

#[test]
fn ip_category_status_and_region_popups_drive_report_filters() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Websites);
    app.site_show_results = true;
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let events = pointer_events(&window);
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    let category = data.get_site_category_options().row_data(1).unwrap();
    let status = data.get_site_status_options().row_data(3).unwrap();
    for (x, index, intent, value) in [
        (300., 1, Intent::SiteCategory, category),
        (510., 3, Intent::SiteStatus, status),
        (730., 1, Intent::SiteRegion, "未识别地区".into()),
    ] {
        ui_testing::click(window.window(), x, 305.);
        let _ = window.window().take_snapshot().unwrap();
        ui_testing::click(window.window(), x, 357. + index as f32 * 44.);
        assert_eq!(
            events.borrow().as_slice(),
            &[(intent, value.clone(), "".into())]
        );
        let event = events.borrow_mut().pop().unwrap();
        let _ = app.ui_intent(event.0, &event.1, &event.2);
        app.present_slint(&data, &mut models);
        let _ = window.window().take_snapshot().unwrap();
        let selected = match intent {
            Intent::SiteCategory => data.get_site_category(),
            Intent::SiteStatus => data.get_site_status(),
            _ => data.get_site_region(),
        };
        assert_eq!(selected, value);
    }
    window.hide().unwrap();
}

#[test]
fn log_rows_open_full_text_copy_without_opening_and_filter_by_source_level() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Logs);
    let long = format!("[warning] DNS timeout {}", "long.example.test ".repeat(80));
    app.snapshot.logs = vec![
        "[info] error counter reset".into(),
        long.clone().into(),
        "[error] failed".into(),
    ];
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_logs().row_data(2).unwrap().level, "INFO");
    let events = pointer_events(&window);
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 876., 196.);
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::Copy, "[error] failed".into(), "".into())]
    );
    events.borrow_mut().clear();
    ui_testing::click(window.window(), 400., 252.);
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::LogDetail, long.clone().into(), "".into())]
    );
    let _ = app.ui_intent(Intent::LogDetail, &long, "");
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_modal(), 13);
    assert_eq!(data.get_modal_description(), long.as_str());
    assert_eq!(data.get_modal_copy(), long.as_str());
    let _ = app.ui_intent(Intent::CancelConfirm, "", "");
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_modal(), 0);
    events.borrow_mut().clear();
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 708., 124.);
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 708., 352.);
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::LogLevel, "WARN".into(), "".into())]
    );
    let _ = app.ui_intent(Intent::Query, "timeout", "");
    app.list_offset = 120;
    let _ = app.ui_intent(Intent::LogLevel, "WARN", "");
    app.present_slint(&data, &mut models);
    assert_eq!(app.list_offset, 0);
    assert_eq!(data.get_logs().row_count(), 1);
    assert_eq!(data.get_logs().row_data(0).unwrap().message, long.as_str());
    window.hide().unwrap();
}

#[test]
fn a_new_log_detail_resets_the_previous_dialog_scroll_and_shows_what_it_copies() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Logs);
    let window = AppWindow::new().unwrap();
    window
        .global::<clash_client_ui::Motion>()
        .set_enabled(false);
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let short = "[info] short diagnostic fixture";
    let _ = app.ui_intent(Intent::LogDetail, short, "");
    app.present_slint(&data, &mut models);
    let events = pointer_events(&window);
    window.show().unwrap();
    let reference = motion_frame(&window, 0);
    let long = format!("[warning] {}", "long.example.test ".repeat(600));
    let _ = app.ui_intent(Intent::LogDetail, &long, "");
    app.present_slint(&data, &mut models);
    motion_frame(&window, 0);
    data.set_scroll_dialog(-1000.);
    motion_frame(&window, 0);
    let _ = app.ui_intent(Intent::CancelConfirm, "", "");
    app.present_slint(&data, &mut models);
    motion_frame(&window, 0);
    let _ = app.ui_intent(Intent::LogDetail, short, "");
    app.present_slint(&data, &mut models);
    let reopened = motion_frame(&window, 0);
    assert_eq!(data.get_scroll_dialog(), 0.);
    assert_eq!(
        reference, reopened,
        "the short log must remain visibly identical to a fresh detail"
    );
    ui_testing::click(window.window(), 194., 392.);
    assert_eq!(
        events.borrow().last(),
        Some(&(Intent::Copy, short.into(), "".into()))
    );
    window.hide().unwrap();
}

#[test]
fn list_navigation_resets_scroll_but_background_refresh_preserves_reading_position() {
    use clash_client_ui::LogItem;
    ui_testing::initialize(950, 700);
    let app = fixture(Page::Logs);
    let window = AppWindow::new().unwrap();
    window
        .global::<clash_client_ui::Motion>()
        .set_enabled(false);
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let row = LogItem {
        level: "INFO".into(),
        message: "fixture log".into(),
        ..Default::default()
    };
    let rows = std::rc::Rc::new(slint::VecModel::from(vec![row.clone(); 60]));
    data.set_logs(rows.clone().into());
    window.show().unwrap();
    motion_frame(&window, 0);
    for change in 0..3 {
        data.set_scroll_logs(-200.);
        motion_frame(&window, 0);
        assert_eq!(data.get_scroll_logs(), -200.);
        rows.set_row_data(
            0,
            LogItem {
                message: "refreshed log".into(),
                ..row.clone()
            },
        );
        motion_frame(&window, 0);
        assert_eq!(
            data.get_scroll_logs(),
            -200.,
            "polling must not jump to the top"
        );
        match change {
            0 => data.set_page_offset(60),
            1 => data.set_query("fixture".into()),
            _ => data.set_log_level("WARN".into()),
        }
        motion_frame(&window, 0);
        assert_eq!(data.get_scroll_logs(), 0.);
    }
    data.set_scroll_logs(-1000.);
    motion_frame(&window, 0);
    rows.set_vec(vec![row]);
    motion_frame(&window, 0);
    assert_eq!(
        data.get_scroll_logs(),
        0.,
        "a shorter list must remain visible"
    );
    window.hide().unwrap();
}

#[test]
fn claude_risk_pages_and_categories_start_at_the_top() {
    use clash_client_ui::ReportItem;
    ui_testing::initialize(950, 700);
    let app = fixture(Page::Claude);
    let window = AppWindow::new().unwrap();
    window
        .global::<clash_client_ui::Motion>()
        .set_enabled(false);
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    data.set_claude_filter(1);
    data.set_claude_results(
        std::rc::Rc::new(slint::VecModel::from(vec![
            ReportItem {
                title: "Fixture risk".into(),
                value: "Risk details".into(),
                detail: "Detailed diagnostic evidence".into(),
                tone: 3,
                ..Default::default()
            };
            30
        ]))
        .into(),
    );
    window.show().unwrap();
    motion_frame(&window, 0);
    data.set_scroll_claude(-300.);
    motion_frame(&window, 0);
    assert_eq!(data.get_scroll_claude(), -300.);
    data.set_claude_result_page(1);
    motion_frame(&window, 0);
    assert_eq!(data.get_scroll_claude(), 0.);
    data.set_scroll_claude(-300.);
    motion_frame(&window, 0);
    data.set_claude_filter(2);
    motion_frame(&window, 0);
    assert_eq!(data.get_scroll_claude(), 0.);
    window.hide().unwrap();
}

fn motion_frame(window: &AppWindow, milliseconds: u64) -> Vec<u8> {
    // Evaluate newly retargeted properties before moving the test clock.
    slint::platform::update_timers_and_animations();
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::advance(milliseconds);
    window.window().take_snapshot().unwrap().as_bytes().to_vec()
}

#[test]
fn motion_overlays_reverse_on_reopen_release_content_and_leave_no_idle_clock() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let motion = window.global::<clash_client_ui::Motion>();
    motion.set_enabled(true);
    data.set_dark(true);
    window.show().unwrap();
    let initial = motion_frame(&window, 0);
    // Saved dark appearance is already present in the first rendered frame.
    let canvas = (690 * 950 + 940) * 4;
    assert_eq!(&initial[canvas..canvas + 3], &[0x11, 0x19, 0x23]);
    motion_frame(&window, 250);
    assert!(
        motion.get_initialized(),
        "startup must enable the motion scope"
    );
    assert!(motion.get_enabled());
    assert!(!window.window().has_active_animations());

    for _ in 0..3 {
        data.set_modal_title("确认操作".into());
        data.set_modal_description("用于验证真实弹窗的退出与重新打开。".into());
        data.set_modal(1);
        motion_frame(&window, 2);
        assert!(window.get_modal_mounted());
        motion_frame(&window, 250);
        data.set_modal(0);
        motion_frame(&window, 30);
        assert!(
            window.get_modal_mounted(),
            "closing content must survive the fade"
        );
        data.set_modal(1);
        motion_frame(&window, 250);
        assert!(
            window.get_modal_mounted(),
            "reopen must cancel the old disposal timer"
        );
        data.set_modal(0);
        motion_frame(&window, 250);
        assert!(
            !window.get_modal_mounted(),
            "closed content must be released"
        );
    }

    data.set_notice("配置已更新".into());
    data.set_notice_revision(1);
    motion_frame(&window, 2);
    motion_frame(&window, 250);
    assert!(window.get_notice_mounted());
    data.set_notice("".into());
    motion_frame(&window, 30);
    assert!(window.get_notice_mounted());
    data.set_notice("新通知".into());
    data.set_notice_revision(2);
    motion_frame(&window, 250);
    assert!(window.get_notice_mounted());
    data.set_notice("".into());
    motion_frame(&window, 250);
    assert!(!window.get_notice_mounted());
    motion_frame(&window, 1200);
    assert!(!window.window().has_active_animations());
    assert!(
        slint::platform::duration_until_next_timer_update().is_none(),
        "one-shot motion timers must stop instead of ticking forever"
    );
    window.hide().unwrap();
}

#[test]
fn motion_does_not_delay_connection_sort_arrow_or_third_click_reset() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Connections);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let events = pointer_events(&window);
    window.global::<clash_client_ui::Motion>().set_enabled(true);
    window.show().unwrap();
    motion_frame(&window, 250);
    let before = motion_frame(&window, 0);
    for click in 0..3 {
        ui_testing::rapid_click(window.window(), 780., 174.);
        let (intent, a, b) = events.borrow_mut().pop().unwrap();
        assert_eq!(intent, Intent::ConnectionSort);
        let _ = app.ui_intent(intent, &a, &b);
        app.present_slint(&data, &mut models);
        let immediately = motion_frame(&window, 0);
        let header = |pixels: &[u8]| {
            pixels
                .as_chunks::<{ 950 * 4 }>()
                .0
                .iter()
                .skip(156)
                .take(52)
                .flat_map(|row| row[740 * 4..820 * 4].iter().copied())
                .collect::<Vec<_>>()
        };
        if click < 2 {
            assert_ne!(
                header(&immediately),
                header(&before),
                "the direction must render in the same frame as sorting"
            );
        }
        assert_eq!(
            data.get_connection_sort(),
            if click == 2 { "" } else { "download-rate" }
        );
    }
    motion_frame(&window, 250);
    assert!(!window.window().has_active_animations());
    window.hide().unwrap();
}

#[test]
fn copy_feedback_waits_for_matching_native_ack_and_resets_without_resizing() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Logs);
    app.snapshot.logs = vec!["[info] copy fixture".into()];
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let events = pointer_events(&window);
    window.global::<clash_client_ui::Motion>().set_enabled(true);
    window.show().unwrap();
    motion_frame(&window, 250);
    ui_testing::rapid_click(window.window(), 876., 196.);
    let (_, value, _) = events.borrow_mut().pop().unwrap();
    let waiting = motion_frame(&window, 250);
    data.set_copy_value("another request".into());
    data.set_copy_revision(1);
    assert_eq!(
        waiting,
        motion_frame(&window, 250),
        "unrelated or failed writes must not display success"
    );
    ui_testing::rapid_click(window.window(), 876., 196.);
    data.set_copy_value(value);
    data.set_copy_revision(2);
    motion_frame(&window, 0);
    let copied = motion_frame(&window, 250);
    assert_ne!(waiting, copied);
    motion_frame(&window, 1100);
    let restored = motion_frame(&window, 250);
    assert_eq!(
        waiting, restored,
        "copy feedback must restore both the label and geometry"
    );
    assert!(!window.window().has_active_animations());
    window.hide().unwrap();
}

#[test]
fn motion_group_collapse_is_persisted_before_exit_and_navigation_drops_nodes() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Proxies);
    app.expanded.insert("Proxy".into());
    app.snapshot.proxies = serde_json::from_value(serde_json::json!({"proxies": {
        "Proxy": {"type":"Selector", "now":"one", "all":["one"]},
        "one": {"type":"AnyTLS", "history":[{"delay":20}]}
    }}))
    .unwrap();
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let events = pointer_events(&window);
    window.global::<clash_client_ui::Motion>().set_enabled(true);
    window.show().unwrap();
    motion_frame(&window, 250);
    for expanded in [false, true, false] {
        ui_testing::rapid_click(window.window(), 400., 185.);
        let (intent, a, b) = events
            .borrow_mut()
            .pop()
            .expect("expansion changes immediately");
        assert_eq!(intent, Intent::ToggleGroup);
        assert!(events.borrow().is_empty());
        let _ = app.ui_intent(intent, &a, &b);
        app.present_slint(&data, &mut models);
        assert_eq!(app.expanded.contains("Proxy"), expanded);
        assert_eq!(
            data.get_groups().row_data(0).unwrap().nodes.row_count(),
            usize::from(expanded)
        );
        motion_frame(&window, 30);
    }
    // Leave before the final collapse finishes: its state must not be lost.
    let _ = app.ui_intent(Intent::Navigate, "0", "");
    app.present_slint(&data, &mut models);
    motion_frame(&window, 250);
    assert_eq!(models.groups.row_count(), 0);
    assert!(!app.expanded.contains("Proxy"));
    assert!(
        events.borrow().is_empty(),
        "no delayed group command after leaving"
    );
    window.hide().unwrap();
}

#[test]
fn notification_recovery_waits_for_drafts_and_clears_when_permission_is_granted() {
    let mut app = fixture(Page::Home);
    app.import_open = true;
    app.profile_name = "unfinished subscription".into();
    let denied = platform::notification::Preparation {
        allowed: false,
        offer_settings: true,
    };
    let _ = app.update(Message::NotificationPrepared(Ok(denied)));
    assert!(app.confirmation.is_none());
    assert!(app.notification_guide_pending);
    app.import_open = false;
    app.offer_notification_settings();
    assert!(matches!(
        &app.confirmation.as_ref().unwrap().pending,
        desktop_view::Pending::NotificationSettings
    ));
    assert_eq!(app.profile_name, "unfinished subscription");
    let allowed = platform::notification::Preparation {
        allowed: true,
        offer_settings: false,
    };
    let _ = app.update(Message::NotificationPrepared(Ok(allowed)));
    assert!(app.confirmation.is_none());
    assert!(!app.notification_guide_pending);
    assert!(!app.notification_checking);
    app.notification_busy = true;
    let _ = app.update(Message::NotificationPermissionDenied);
    assert!(!app.notification_busy);
    assert!(app.confirmation.is_some());
}

#[test]
fn recreated_window_models_project_live_proxy_state_instead_of_saved_preferences() {
    let mut app = fixture(Page::Home);
    app.snapshot.running = true;
    app.settings.proxy_mode = ProxyMode::Off;
    for (system, tun, expected) in [(true, false, 0), (false, true, 1), (false, false, 2)] {
        app.snapshot.system_proxy = system;
        app.snapshot.tun = tun;
        app.snapshot.retain_scope(Scope::Other);
        for _ in 0..3 {
            let data = clash_desktop_protocol::UiData::default();
            let mut models = clash_desktop_protocol::models::ViewModels::default();
            models.attach(&data);
            app.present(&data, &mut models);
            assert_eq!(data.get_proxy_mode(), expected);
            assert_eq!(app.snapshot.system_proxy, system);
            assert_eq!(app.snapshot.tun, tun);
            assert_eq!(app.settings.proxy_mode, ProxyMode::Off);
        }
    }
}

#[test]
fn reopening_a_maximized_window_does_not_force_a_normal_size_request() {
    let mut app = fixture(Page::Home);
    let normal = app.window_state.size();
    app.window_opening = true;
    let _ = app.update(Message::Window((
        runtime::window::Id::unique(),
        runtime::window::Event::Opened {
            preserve_native_state: true,
        },
    )));
    assert!(!app.window_opening);
    assert!(!app.size_pending);
    assert_eq!(app.window_state.size(), normal);
}

impl App {
    fn present_slint(
        &self,
        data: &clash_client_ui::UiData<'_>,
        models: &mut clash_client_ui::models::ViewModels,
    ) {
        let projection = clash_desktop_protocol::UiData::default();
        let mut projected = clash_desktop_protocol::models::ViewModels::default();
        projected.attach(&projection);
        self.present(&projection, &mut projected);
        clash_desktop_ui::apply(&projection.snapshot(), data, models);
    }
    fn ui_intent(&mut self, intent: clash_client_ui::Intent, a: &str, b: &str) -> Task<Message> {
        self.intent(clash_desktop_ui::intent(intent), a, b)
    }
}

#[test]
fn renderer_rejects_stale_frames_without_losing_newer_input_or_dropdown_state() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    data.set_edit_name("newer draft".into());
    data.set_rule_kind("DOMAIN-SUFFIX".into());
    let frame = clash_desktop_protocol::Frame {
        edit_name: "old draft".into(),
        rule_kind: "DOMAIN".into(),
        ..Default::default()
    };
    assert!(!clash_desktop_ui::apply_if_current(
        &frame,
        &data,
        &mut models,
        2,
        3
    ));
    assert_eq!(data.get_edit_name(), "newer draft");
    assert_eq!(data.get_rule_kind(), "DOMAIN-SUFFIX");
    let frame = clash_desktop_protocol::Frame {
        edit_name: "newer draft".into(),
        rule_kind: "DOMAIN-SUFFIX".into(),
        ..frame
    };
    assert!(clash_desktop_ui::apply_if_current(
        &frame,
        &data,
        &mut models,
        3,
        3
    ));
    assert_eq!(data.get_edit_name(), "newer draft");
}

fn synchronized_reply(app: &App) -> Reply {
    Reply {
        view_revision: app.view_revision.wrapping_add(1),
        remote: true,
        profile_edit: false,
        profile_import: false,
        profile_updated: false,
        scope: app.page.scope(),
        start_requested: false,
        settings: app.settings.clone(),
        profiles: Some(app.profiles.clone()),
        snapshot: Some(app.snapshot.clone()),
        rule_document: None,
        rules_applied: false,
        notice: Ok(String::new()),
        running: app.snapshot.running,
        geo_status: app.geo_status.clone(),
        autostart: false,
        protection_changed: false,
        tray_nodes: None,
    }
}

#[test]
fn failed_subscription_update_keeps_measurements_and_reports_until_a_successful_retry() {
    use clash_of_rust::claude_check::{Environment, Report};
    let mut app = fixture(Page::Profiles);
    app.updates.started = true;
    app.node_delays.insert("Test".into(), 42);
    app.site_results.push(Some(Ok(ip_check::CheckResult {
        state: ip_check::State::Identified,
        summary: "203.0.113.2".into(),
        country: String::new(),
        millis: 4,
        detail: String::new(),
    })));
    app.claude.reports[0] = Some(Report::new(Environment::Core, "saved report"));
    let before = serde_json::to_value(&app.profiles).unwrap();
    let generation = app.delay_generation;
    let _ = app.ui_intent(Intent::UpdateProfile, "fixture", "");
    assert_eq!(app.node_delays.get("Test"), Some(&42));
    assert_eq!(app.site_results.len(), 1);
    assert!(app.claude.reports[0].is_some());

    let mut failed = synchronized_reply(&app);
    failed.view_revision = app.view_revision;
    failed.notice = Err("订阅更新失败，已保留原配置与运行状态".into());
    let _ = app.update(Message::Finished(Box::new(failed)));
    assert!(!app.busy && !app.working);
    assert!(app.error && app.notice.contains("保留原配置"));
    assert!(app.snapshot.running && app.core_failure.is_none());
    assert_eq!(serde_json::to_value(&app.profiles).unwrap(), before);
    assert_eq!(app.node_delays.get("Test"), Some(&42));
    assert_eq!(app.delay_generation, generation);
    assert_eq!(app.site_results.len(), 1);
    assert!(app.claude.reports[0].is_some());

    let mut success = synchronized_reply(&app);
    success.profile_updated = true;
    success.notice = Ok("订阅已更新".into());
    let _ = app.update(Message::Finished(Box::new(success)));
    assert!(app.node_delays.is_empty() && app.site_results.is_empty());
    assert!(app.claude.reports.iter().all(Option::is_none));
    assert!(!app.error && app.snapshot.running);
}

#[test]
fn background_subscription_update_invalidates_network_results_without_overwriting_rule_drafts() {
    use clash_of_rust::claude_check::{Environment, Report};
    let mut app = fixture(Page::Rules);
    let _ = app.ui_intent(Intent::RuleAdd, "", "");
    let _ = app.ui_intent(Intent::RulePayload, "unsaved.test", "");
    let draft = app.rule_state.draft.clone();
    let payload = app.rule_state.editor.as_ref().unwrap().payload.clone();
    app.node_delays.insert("Test".into(), 42);
    app.claude.reports[0] = Some(Report::new(Environment::Core, "old report"));
    let generation = app.delay_generation;
    let mut reply = synchronized_reply(&app);
    reply.profiles.as_mut().unwrap()[0].updated += 60;
    assert!(
        !reply.profile_updated,
        "background refresh has no GUI action"
    );
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert!(app.node_delays.is_empty());
    assert!(app.claude.reports.iter().all(Option::is_none));
    assert_ne!(app.delay_generation, generation);
    assert_eq!(app.rule_state.draft, draft);
    assert_eq!(app.rule_state.editor.as_ref().unwrap().payload, payload);
}

#[test]
fn console_changes_refresh_profiles_and_clean_inputs_but_preserve_typed_drafts() {
    let mut app = fixture(Page::Settings);
    let mut reply = synchronized_reply(&app);
    reply.settings.mixed_port = 12345;
    reply.settings.delay_interval_minutes = 17;
    reply.settings.subscription_update_interval_minutes = 720;
    reply.profiles.as_mut().unwrap()[0].name = "Renamed by CLI".into();
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert_eq!(app.mixed_port, "12345");
    assert_eq!(app.interval_input, "17");
    assert_eq!(app.subscription_interval_input, "720");
    assert_eq!(app.profiles[0].name, "Renamed by CLI");
    assert!(app.backend_remote);

    app.mixed_port = "12346".into();
    app.interval_input = "25".into();
    app.subscription_interval_input = "90".into();
    let mut reply = synchronized_reply(&app);
    reply.settings.mixed_port = 12347;
    reply.settings.delay_interval_minutes = 30;
    reply.settings.subscription_update_interval_minutes = 180;
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert_eq!(app.mixed_port, "12346");
    assert_eq!(app.interval_input, "25");
    assert_eq!(app.subscription_interval_input, "90");
    assert_eq!(app.settings.mixed_port, 12347);
}

#[test]
fn protection_locks_rule_controls_and_proxy_mode_without_a_warning_or_draft_change() {
    let mut app = fixture(Page::Rules);
    app.claude.protection_pending = Some(true);
    let draft = app.rule_state.draft.clone();
    for intent in [
        Intent::RuleAdd,
        Intent::RuleApply,
        Intent::RuleDiscard,
        Intent::RuleSelect,
        Intent::RuleProfile,
        Intent::RuleCollapse,
        Intent::Query,
        Intent::ListPage,
    ] {
        let _ = app.ui_intent(intent, "blocked", "");
    }
    assert_eq!(app.rule_state.draft, draft);
    assert!(app.query.is_empty());
    assert!(!app.rule_state.has_editor());
    assert!(!app.notice_visible);
    app.page = Page::Home;
    for mode in ["0", "1", "2"] {
        let _ = app.ui_intent(Intent::ProxyMode, mode, "");
    }
    app.page = Page::Settings;
    let settings = serde_json::to_value(&app.settings).unwrap();
    for intent in [
        Intent::DnsOverride,
        Intent::FakeIp,
        Intent::FakeIpAdd,
        Intent::FakeIpRemove,
        Intent::FakeIpRestoreDefaults,
        Intent::Ipv6,
        Intent::Udp,
        Intent::MixedPort,
        Intent::SavePorts,
        Intent::Mode,
    ] {
        let _ = app.ui_intent(intent, "false", "blocked.test");
    }
    assert!(!app.notice_visible);
    assert!(app.confirmation.is_none());
    assert_eq!(serde_json::to_value(&app.settings).unwrap(), settings);
    app.claude.protection_pending = None;
    app.page = Page::Rules;
    let _ = app.ui_intent(Intent::Query, "available", "");
    assert_eq!(app.query, "available");
}

#[test]
fn a_locked_rules_page_still_accepts_the_global_exit_confirmation() {
    let mut app = fixture(Page::Rules);
    let _ = app.ui_intent(Intent::RuleAdd, "", "");
    let _ = app.ui_intent(Intent::RulePayload, "unsaved.test", "");
    app.claude.protection_pending = Some(true);
    let _ = app.ui_intent(Intent::Quit, "", "");
    assert!(matches!(
        app.confirmation
            .as_ref()
            .map(|confirmation| &confirmation.pending),
        Some(desktop_view::Pending::Exit)
    ));
    assert!(!app.exiting);
    let _ = app.ui_intent(Intent::Confirm, "", "");
    assert!(app.exiting);
}

#[test]
fn entering_proxies_pins_and_expands_the_actual_group_without_reopening_a_manual_collapse() {
    let mut app = fixture(Page::Home);
    let _ = app.ui_intent(Intent::Navigate, "1", "");
    assert_eq!(app.page, Page::Proxies);
    assert!(app.focus_active_group);
    let mut reply = synchronized_reply(&app);
    reply.scope = Scope::Proxies;
    reply.snapshot = Some(Snapshot {
        running: true,
        mode: "rule".into(),
        active_group: Some("ZZ Active".into()),
        proxies: serde_json::from_value(serde_json::json!({"proxies": {
            "AA Unused": {"type":"Selector", "now":"Leaf", "all":["Leaf"]},
            "ZZ Active": {"type":"Selector", "now":"Leaf", "all":["Leaf"]},
            "Leaf": {"type":"Trojan"}
        }}))
        .unwrap(),
        ..Default::default()
    });
    let _ = app.update(Message::Finished(Box::new(reply)));
    let (_, groups) = app.proxy_rows();
    assert_eq!(groups[0].0.id.as_str(), "ZZ Active");
    assert!(groups[0].0.expanded);
    assert_eq!(groups[1].0.id.as_str(), "AA Unused");
    app.expanded.remove("ZZ Active");
    let mut reply = synchronized_reply(&app);
    reply.scope = Scope::Proxies;
    reply.snapshot = Some(app.snapshot.clone());
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert!(!app.proxy_rows().1[0].0.expanded);
    app.query = "Unused".into();
    assert_eq!(app.proxy_rows().1[0].0.id.as_str(), "AA Unused");
    app.snapshot.active_group = None;
    app.query.clear();
    assert_eq!(app.proxy_rows().1[0].0.id.as_str(), "AA Unused");
}

#[test]
fn console_rule_changes_update_clean_views_without_discarding_an_open_editor() {
    let mut app = fixture(Page::Rules);
    let mut reply = synchronized_reply(&app);
    let mut rules = ProfileRules::default();
    rules.custom.push(rule_manager::CustomRule {
        id: uuid::Uuid::new_v4().to_string(),
        rule: "DOMAIN,from-cli.test,DIRECT".into(),
        enabled: true,
    });
    reply
        .settings
        .rule_profiles
        .insert("fixture".into(), Arc::new(rules));
    reply.rule_document = app.rule_state.document.clone();
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert!(
        app.rule_state
            .draft
            .custom
            .iter()
            .any(|rule| rule.rule.contains("from-cli.test"))
    );
    let _ = app.ui_intent(Intent::RuleAdd, "", "");
    let _ = app.ui_intent(Intent::RulePayload, "unsaved.test", "");
    let mut reply = synchronized_reply(&app);
    reply.profiles.as_mut().unwrap().clear();
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert_eq!(
        app.rule_state.editor.as_ref().unwrap().payload,
        "unsaved.test"
    );
    assert_eq!(
        app.rule_state.document.as_ref().unwrap().profile_id,
        "fixture"
    );
    assert!(app.notice.contains("草稿"));
}

#[test]
fn fake_ip_projection_follows_dns_override_and_exposes_the_exclusion_page() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Settings);
    app.settings.dns_override = false;
    app.settings.fake_ip = true;
    app.settings.fake_ip_exclusions = vec![
        "+.example.test".into(),
        "ds.example.test".into(),
        "localhost".into(),
    ];
    app.settings.fake_ip_disabled_defaults = vec!["+.tailscale.com".into()];
    app.fake_ip_editor = true;
    app.fake_ip_input = "next.example.test".into();
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    assert!(!data.get_fake_ip());
    assert!(data.get_fake_ip_editor());
    assert_eq!(data.get_fake_ip_input().to_string(), "next.example.test");
    let rows = data.get_fake_ip_exclusions();
    assert_eq!(
        rows.row_count(),
        clash_of_rust::config::DEFAULT_FAKE_IP_EXCLUSIONS.len() + 1
    );
    assert_eq!(rows.row_data(0).unwrap().domain, "+.example.test");
    assert_eq!(rows.row_data(0).unwrap().source, "用户自定义");
    assert_eq!(rows.row_data(1).unwrap().domain, "ds.example.test");
    assert_eq!(rows.row_data(2).unwrap().source, "用户自定义、DNS 覆盖");
    assert_eq!(
        rows.iter()
            .find(|row| row.domain == "+.lan")
            .unwrap()
            .source,
        "DNS 覆盖"
    );
    assert!(rows.iter().all(|row| row.domain != "+.tailscale.com"));
    assert_eq!(data.get_fake_ip_removed_defaults(), 1);

    app.settings.dns_override = true;
    app.present_slint(&data, &mut models);
    assert!(data.get_fake_ip());
    app.busy = true;
    let _ = app.ui_intent(Intent::DnsOverride, "false", "");
    assert!(matches!(
        app.queued_actions.pop_back(),
        Some(Action::DnsOverride(false))
    ));
    app.fake_ip_editor = false;
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_fake_ip_exclusions().row_count(), 0);
    app.fake_ip_editor = true;
    app.visible = false;
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_fake_ip_exclusions().row_count(), 0);
}

#[test]
fn fake_ip_add_validates_the_input_before_queueing_a_save() {
    let mut app = fixture(Page::Settings);
    app.busy = true;
    app.fake_ip_input = "bad name".into();
    let _ = app.ui_intent(Intent::FakeIpAdd, "", "");
    assert!(app.fake_ip_error.contains("只能包含"));
    assert!(app.queued_actions.is_empty());

    let _ = app.ui_intent(Intent::FakeIpInput, "+.ok.test", "");
    assert!(app.fake_ip_error.is_empty());
    let _ = app.ui_intent(Intent::FakeIpAdd, "", "");
    assert!(app.fake_ip_error.is_empty());
    assert!(matches!(
        app.queued_actions.pop_back(),
        Some(Action::FakeIpAdd(values))
            if values == vec!["+.ok.test".to_owned()]
    ));

    app.fake_ip_input.clear();
    let _ = app.ui_intent(Intent::FakeIpAdd, "", "");
    assert!(app.fake_ip_error.contains("请输入"));

    app.fake_ip_input = "+.TAILSCALE.COM".into();
    let _ = app.ui_intent(Intent::FakeIpAdd, "", "");
    assert!(app.fake_ip_error.contains("已存在"));
    assert!(app.queued_actions.is_empty());
}

#[test]
fn fake_ip_remove_and_restore_use_stable_domains_and_keep_dns_override_required() {
    let mut app = fixture(Page::Settings);
    app.busy = true;
    let _ = app.ui_intent(Intent::FakeIpRemove, "+.tailscale.com", "");
    assert!(matches!(
        app.queued_actions.pop_back(),
        Some(Action::FakeIpRemove(domain)) if domain == "+.tailscale.com"
    ));
    let _ = app.ui_intent(Intent::FakeIpRemove, "0", "");
    assert!(app.queued_actions.is_empty());
    assert!(app.fake_ip_error.contains("未找到"));

    app.settings.fake_ip_disabled_defaults = vec!["+.tailscale.com".into()];
    let _ = app.ui_intent(Intent::FakeIpRestoreDefaults, "", "");
    assert!(app.fake_ip_error.is_empty());
    assert!(matches!(
        app.queued_actions.pop_back(),
        Some(Action::FakeIpRestoreDefaults)
    ));
    app.settings.dns_override = false;
    for (intent, domain) in [
        (Intent::FakeIpRemove, "localhost"),
        (Intent::FakeIpRestoreDefaults, ""),
    ] {
        let _ = app.ui_intent(intent, domain, "");
        assert!(app.fake_ip_error.contains("请先开启"));
        assert!(app.queued_actions.is_empty());
    }
}
