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
