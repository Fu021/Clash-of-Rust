//! Exercise production component callbacks, model projection and application state.
use super::*;
use clash_client_ui::{AppWindow, Intent, UiActions, UiData, models::ViewModels};
use slint::{ComponentHandle, Model};

fn wsl_report(
    action: clash_client_core::wsl::Action,
    generation: u64,
) -> clash_client_core::wsl::Report {
    clash_client_core::wsl::Report::parse(&serde_json::json!({
        "schema_version": 1, "operation_id": uuid::Uuid::new_v4().to_string(),
        "generation": generation, "action": action, "outcome": "succeeded",
        "protection": "verified", "phase": "connected_unverified", "backend": "Tailscale",
        "message": "Operation completed. Packet-level, exit-location, and reboot verification remain separate.",
        "distributions": [
            {"name":"Ubuntu", "state":"running", "coverage":"not_enrolled"},
            {"name":"docker-desktop", "state":"stopped", "coverage":"blocked"}
        ],
        "checks": [
            {"id":"tailscale_connected", "result":"PASS", "detail":""},
            {"id":"german_exit", "result":"PASS", "detail":""},
            {"id":"ipv4_only", "result":"PASS", "detail":""},
            {"id":"cold_boot", "result":"NOT_RUN", "detail":""}
        ], "logs": ["token=must-never-reach-ui"]
    }).to_string()).unwrap()
}

#[test]
fn wsl_projection_separates_transport_exit_protection_and_invalidates_on_edit() {
    use clash_client_core::wsl::Action as W;
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Wsl);
    app.wsl.draft.backend = clash_client_core::wsl::Backend::Tailscale;
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let ticket = app
        .wsl
        .evidence
        .begin(app.wsl.draft.request(W::Test, false))
        .unwrap();
    let _ = app.update(Message::Wsl(wsl::Event::Finished(
        ticket,
        Box::new(Ok(wsl_report(W::Test, u64::MAX))),
        String::new(),
    )));
    app.present(&data, &mut models);
    assert_eq!(data.get_page(), 10);
    assert_eq!(data.get_wsl_generation(), u64::MAX.to_string().as_str());
    assert_eq!(data.get_wsl_tunnel(), "已验证连接");
    assert_eq!(data.get_wsl_exit(), "DE · IPv4 已验证");
    assert_eq!(data.get_wsl_protection(), "blocked");
    assert!(data.get_wsl_functional().starts_with("Blocked"));
    assert_eq!(data.get_wsl_distributions().row_count(), 2);
    assert_eq!(
        data.get_wsl_distributions().row_data(1).unwrap().coverage,
        "blocked"
    );
    assert_eq!(data.get_wsl_logs().row_data(0).unwrap(), "[redacted]");
    let original = data.get_wsl_distributions();
    app.present(&data, &mut models);
    assert_eq!(original, data.get_wsl_distributions());

    let _ = app.intent(Intent::WslInput, "backend", "WireGuard");
    app.present(&data, &mut models);
    assert_eq!(data.get_wsl_backend(), "WireGuard");
    assert_eq!(data.get_wsl_distributions().row_count(), 0);
    assert_eq!(data.get_wsl_checks().row_count(), 0);
    assert_eq!(data.get_wsl_logs().row_count(), 0);
    assert!(data.get_wsl_tunnel().starts_with("unknown"));
    assert!(data.get_wsl_exit().starts_with("unknown"));
    assert_eq!(data.get_wsl_protection(), "blocked");
    assert_eq!(data.get_wsl_status(), "not_run");
}

#[test]
fn wsl_busy_inputs_and_stale_callbacks_do_not_disturb_the_active_operation() {
    use clash_client_core::wsl::Action as W;
    let mut app = fixture(Page::Wsl);
    let first = app
        .wsl
        .evidence
        .begin(app.wsl.draft.request(W::Status, false))
        .unwrap();
    let original = app.wsl.draft.clone();
    let _ = app.intent(Intent::WslInput, "profile-path", r"C:\Private\changed.json");
    assert_eq!(app.wsl.draft, original);
    let _ = app.update(Message::Wsl(wsl::Event::Finished(
        first.clone(),
        Box::new(Ok(wsl_report(W::Status, 10))),
        String::new(),
    )));
    let current = app
        .wsl
        .evidence
        .begin(app.wsl.draft.request(W::Status, false))
        .unwrap();
    let _ = app.update(Message::Wsl(wsl::Event::Finished(
        first,
        Box::new(Ok(wsl_report(W::Status, 9))),
        "stale command".into(),
    )));
    assert!(app.wsl.evidence.busy());
    assert!(app.wsl.evidence.report().is_none());
    let _ = app.update(Message::Wsl(wsl::Event::Finished(
        current,
        Box::new(Ok(wsl_report(W::Status, 11))),
        String::new(),
    )));
    assert!(!app.wsl.evidence.busy());
    assert_eq!(app.wsl.evidence.service_generation(), Some(11));
}

#[test]
fn wsl_restore_requires_a_separate_confirmation_and_cancel_keeps_policy_untouched() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Wsl);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let _ = app.intent(Intent::Navigate, "10", "");
    assert_eq!(app.page, Page::Wsl);
    let _ = app.intent(Intent::WslAction, "Rollback", "restore-confirm");
    assert!(matches!(
        app.confirmation.as_ref().unwrap().pending,
        desktop_view::Pending::WslRestore(_)
    ));
    assert!(!app.wsl.evidence.busy());
    app.present(&data, &mut models);
    assert!(data.get_modal_title().contains("撤销本轮保护"));
    assert!(data.get_modal_danger());
    let original = app.wsl.draft.clone();
    let _ = app.intent(Intent::WslInput, "backend", "WireGuard");
    assert_eq!(
        app.wsl.draft, original,
        "confirmation freezes its exact inputs"
    );
    let _ = app.intent(Intent::CancelConfirm, "", "");
    assert!(app.confirmation.is_none());
    assert!(!app.wsl.evidence.busy());
}

#[test]
fn fork_rejects_official_update_discovery_and_installation_callbacks() {
    let mut app = fixture(Page::Settings);
    let _ = app.check_update();
    assert!(!app.updates.checking);
    assert!(app.updates.status.contains("定制版"));
    let available = clash_of_rust::update::Available {
        version: "99.0.0".into(),
        url: clash_of_rust::update::RELEASES_URL.into(),
        preview: false,
        package: None,
    };
    let _ = app.update(Message::UpdateChecked(Ok(Some(available.clone()))));
    assert!(app.updates.available.is_none());
    app.updates.available = Some(available);
    let _ = app.update(Message::InstallUpdate);
    assert!(app.updates.available.is_none());
    assert!(!app.updates.busy());
}

#[test]
fn wsl_native_buttons_dispatch_preview_and_busy_state_keeps_navigation_available() {
    ui_testing::initialize(950, 700);
    let app = fixture(Page::Wsl);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present(&data, &mut models);
    // Exercise the same Windows-only page on every CI renderer without running
    // any helper. Only production callback values are recorded by this window.
    data.set_wsl_supported(true);
    let events = pointer_events(&window);
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    let mut doctor_y = None;
    // Text metrics vary across platforms; probe the small first-row region,
    // not the full window, and never execute any recorded network intent.
    for y in (144..216).step_by(8) {
        events.borrow_mut().clear();
        ui_testing::click(window.window(), 250.0, y as f32);
        if events.borrow().iter().any(|(intent, action, mode)| {
            *intent == Intent::WslAction && action == "Doctor" && mode == "preview"
        }) {
            doctor_y = Some(y as f32);
            break;
        }
    }
    let doctor_y = doctor_y.expect("native Doctor button must emit the read-only preview intent");
    events.borrow_mut().clear();
    data.set_wsl_busy(true);
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 250.0, doctor_y);
    assert!(
        events.borrow().is_empty(),
        "busy controls must suppress duplicate operations"
    );
    ui_testing::click(window.window(), 65.0, 156.0);
    assert_eq!(events.borrow().last().unwrap().0, Intent::Navigate);
    window.hide().unwrap();
}

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
fn connection_sort_clicks_redraw_the_header_immediately_and_third_click_restores_rows() {
    use slint::platform::software_renderer::RepaintBufferType;
    let adapter = ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Connections);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present(&data, &mut models);
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
            let _ = app.intent(event.0, &event.1, &event.2);
            app.present(&data, &mut models);
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
    let _ = app.intent(Intent::ConnectionSort, "download-rate", "");
    app.present(&data, &mut models);
    let expected = header();
    for _ in 0..2 {
        let _ = app.intent(Intent::ConnectionSort, "download-rate", "");
    }
    app.present(&data, &mut models);
    assert_eq!(header(), before);
    for _ in 0..4 {
        for (x, y) in [(670., 174.), (480., 197.), (260., 174.), (780., 174.)] {
            ui_testing::rapid_click(window.window(), x, y);
        }
    }
    let pending = std::mem::take(&mut *events.borrow_mut());
    assert_eq!(pending.len(), 16, "rapid clicks must each dispatch once");
    for (index, (intent, a, b)) in pending.into_iter().enumerate() {
        let _ = app.intent(intent, &a, &b);
        if index % 3 == 0 {
            // A completed background connection refresh must not overwrite
            // newer sort intent while the queued UI commands are being drained.
            let mut snapshot = app.snapshot.clone();
            snapshot.connections.connections[0].download += 4096;
            let _ = app.update(Message::Finished(Box::new(Reply {
                profile_edit: false,
                profile_import: false,
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
        app.present(&data, &mut models);
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
    let _ = app.intent(Intent::ConnectionDetail, "1", "");
    app.present(&data, &mut models);
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
        profile_edit: false,
        profile_import: false,
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
    app.present(&data, &mut models);
    assert_eq!(data.get_connection_details().row_count(), 0);
    assert!(data.get_modal_copy().is_empty());
    assert!(data.get_modal_description().contains("已结束"));
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
    app.present(&data, &mut models);
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

type UiEventLog = std::rc::Rc<RefCell<Vec<(Intent, slint::SharedString, slint::SharedString)>>>;
fn pointer_events(window: &AppWindow) -> UiEventLog {
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
    app.present(&data, &mut models);
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
        let _ = app.intent(events[0].0, &events[0].1, &events[0].2);
        app.present(&data, &mut models);
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
    app.present(&data, &mut models);
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
    app.present(&data, &mut models);
    let events = pointer_events(&window);
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 840., 233.);
    assert_eq!(events.borrow_mut().pop().unwrap().0, Intent::AccentPicker);
    let _ = app.intent(Intent::AccentPicker, "", "");
    app.present(&data, &mut models);
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
    let _ = app.intent(chosen.0, &chosen.1, &chosen.2);
    app.present(&data, &mut models);
    assert_eq!(data.get_accent(), 1);
    for color in [2, 3, 5] {
        let _ = app.intent(Intent::AccentColor, &color.to_string(), "");
    }
    assert_eq!(app.queued_actions.len(), 1, "rapid changes must coalesce");
    let _ = app.update(Message::Finished(Box::new(Reply {
        profile_edit: false,
        profile_import: false,
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
    app.present(&data, &mut models);
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
    app.present(&data, &mut models);
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
    let _ = app.intent(Intent::TextScale, "125%", "");
    assert!(matches!(
        app.queued_actions.pop_back(),
        Some(Action::TextScale(125))
    ));
    app.settings.text_scale = 125;
    app.present(&data, &mut models);
    slint::platform::update_timers_and_animations();
    let _ = window.window().take_snapshot().unwrap();
    assert!((window.global::<Theme>().get_body_size() - before * 1.25).abs() < 0.01);
    // Restoring 100% must shrink the same rendered controls again.
    app.settings.text_scale = 100;
    app.present(&data, &mut models);
    slint::platform::update_timers_and_animations();
    let _ = window.window().take_snapshot().unwrap();
    assert!((window.global::<Theme>().get_body_size() - before).abs() < 0.01);
    window.hide().unwrap();
}

#[test]
fn rule_editor_popups_deliver_type_and_policy_inside_a_modal() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Rules);
    let _ = app.intent(Intent::RuleAdd, "", "");
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present(&data, &mut models);
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
    let _ = app.intent(event.0, &event.1, &event.2);
    app.present(&data, &mut models);
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
    let _ = app.intent(event.0, &event.1, &event.2);
    app.present(&data, &mut models);
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
        let _ = app.intent(intent, a, b);
        assert_eq!(
            format!("{:?}", app.queued_actions.pop_back().unwrap()),
            format!("{expected:?}")
        );
    }
    let mut app = fixture(Page::Settings);
    app.busy = true;
    let _ = app.intent(Intent::ControllerPort, "9092", "");
    let _ = app.intent(Intent::MixedPort, "7899", "");
    let _ = app.intent(Intent::SavePorts, "", "");
    assert!(matches!(
        app.queued_actions.pop_back(),
        Some(Action::SavePorts(9092, 7899))
    ));
    let _ = app.intent(Intent::MixedPort, "invalid", "");
    let _ = app.intent(Intent::SavePorts, "", "");
    assert!(app.queued_actions.is_empty());
    let _ = app.intent(Intent::IntervalInput, "15", "");
    let _ = app.intent(Intent::SaveInterval, "", "");
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
    app.present(&data, &mut models);
    assert!(data.get_site_exit_flagged());
    assert_eq!(data.get_site_exit_country(), "德国");
    assert_eq!(data.get_site_exit_flag().size().width, 72);
    assert!(!data.get_site_summary().contains('🇩'));
    let _ = app.intent(Intent::SiteDetail, "exit-ip", "");
    app.present(&data, &mut models);
    assert!(data.get_modal_flagged());
    assert_eq!(data.get_modal_country(), "德国");
    assert!(!data.get_modal_description().contains('🇩'));
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
    app.present(&data, &mut models);
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
    let _ = app.intent(event.0, &event.1, &event.2);
    assert!(matches!(app.queued_actions.pop_back(), Some(Action::LoadRules(id)) if id == "second"));
    ui_testing::click(window.window(), 750., 260.);
    let _ = window.window().take_snapshot().unwrap();
    ui_testing::click(window.window(), 750., 356.);
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::RuleFilter, "1".into(), "".into())]
    );
    let event = events.borrow_mut().pop().unwrap();
    let _ = app.intent(event.0, &event.1, &event.2);
    app.present(&data, &mut models);
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
    app.present(&data, &mut models);
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
        let _ = app.intent(event.0, &event.1, &event.2);
        app.present(&data, &mut models);
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
    app.present(&data, &mut models);
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
    let _ = app.intent(Intent::LogDetail, &long, "");
    app.present(&data, &mut models);
    assert_eq!(data.get_modal(), 13);
    assert_eq!(data.get_modal_description(), long.as_str());
    assert_eq!(data.get_modal_copy(), long.as_str());
    let _ = app.intent(Intent::CancelConfirm, "", "");
    app.present(&data, &mut models);
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
    let _ = app.intent(Intent::Query, "timeout", "");
    app.list_offset = 120;
    let _ = app.intent(Intent::LogLevel, "WARN", "");
    app.present(&data, &mut models);
    assert_eq!(app.list_offset, 0);
    assert_eq!(data.get_logs().row_count(), 1);
    assert_eq!(data.get_logs().row_data(0).unwrap().message, long.as_str());
    window.hide().unwrap();
}
