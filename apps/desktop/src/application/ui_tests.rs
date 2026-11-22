//! Exercise production component callbacks, model projection and application state.
use super::*;
use clash_client_ui::{
    AppWindow, Intent, IpStatusText, NetworkTools, UiActions, UiData, UiGeometry,
    models::ViewModels,
};
use slint::{ComponentHandle, Model};

#[test]
fn network_report_is_replaced_and_late_results_cannot_reappear_after_navigation() {
    let report = || network_tools::Report {
        schema: 1,
        tool: network_tools::Tool::Website,
        target: "https://example.test".into(),
        elapsed_ms: 1,
        status: "complete",
        entries: [
            "系统 DNS",
            "内核 DNS",
            "本机代理",
            "HTTPS",
            "HTTP",
            "采用协议",
            "实际线路",
        ]
        .into_iter()
        .map(|label| network_tools::Entry {
            label: label.into(),
            detail: "示例响应".into(),
            success: Some(true),
            code: "ok",
            detail_english: None,
        })
        .collect(),
    };
    let mut app = fixture(Page::Tests);
    app.test_results = vec!["旧记录".into()];
    app.probing = true;
    let generation = app.network_test_generation;
    let _ = app.update(Message::ProbeDone(generation, Ok(report())));
    assert_eq!(app.test_results.len(), 9);
    assert_eq!(app.test_results[0], "检查状态 · 完成 · 1 ms");
    assert!(!app.test_results.iter().any(|line| line.contains("旧记录")));
    let _ = app.update(Message::Navigate(Page::Home));
    assert!(app.test_results.is_empty());
    let _ = app.update(Message::Navigate(Page::Tests));
    let _ = app.update(Message::ProbeDone(generation, Ok(report())));
    assert!(app.test_results.is_empty());
    app.test_results = vec!["新结果".into()];
    app.probing = true;
    let _ = app.update(Message::NetworkTool(network_tools::Tool::Tls.index()));
    assert!(!app.probing);
    assert!(app.test_results.is_empty());
    let _ = app.update(Message::ProbeDone(generation, Ok(report())));
    assert!(app.test_results.is_empty());
    app.probing = true;
    let _ = app.update(Message::ProbeCancel);
    assert!(!app.probing);
    assert_eq!(app.test_results, ["检查已取消"]);
}

#[test]
fn english_network_report_displays_translated_status_and_preserves_raw_evidence() {
    let mut app = fixture(Page::Tests);
    app.presentation_language = Language::English;
    let generation = app.network_test_generation;
    let report = network_tools::Report {
        schema: 1,
        tool: network_tools::Tool::Dns,
        target: "https://example.test".into(),
        elapsed_ms: 42,
        status: "partial",
        entries: vec![network_tools::Entry {
            label: "系统 DNS".into(),
            detail: "系统解析失败或超时".into(),
            success: Some(false),
            code: "dns_failed",
            detail_english: None,
        }],
    };
    let _ = app.update(Message::ProbeDone(generation, Ok(report)));
    assert_eq!(app.test_results[0], "Status · Partial · 42 ms");
    assert_eq!(
        app.test_results[2],
        "System DNS · System DNS failed or timed out"
    );
    assert_eq!(app.network_codes, ["dns_failed"]);
}

#[test]
fn network_report_follows_language_changes_from_another_client() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let mut app = fixture(Page::Tests);
    app.settings.language = LanguagePreference::SimplifiedChinese;
    app.presentation_language = Language::SimplifiedChinese;
    let report = network_tools::Report {
        schema: 1,
        tool: network_tools::Tool::Dns,
        target: "https://example.test".into(),
        elapsed_ms: 42,
        status: "partial",
        entries: vec![network_tools::Entry {
            label: "系统 DNS".into(),
            detail: "保留原始值".into(),
            success: Some(false),
            code: "dns_failed",
            detail_english: None,
        }],
    };
    let generation = app.network_test_generation;
    let _ = app.update(Message::ProbeDone(generation, Ok(report)));
    assert_eq!(app.test_results[0], "检查状态 · 部分完成 · 42 ms");
    app.present_slint(&data, &mut models);
    assert_eq!(
        data.get_network_results().row_data(0).unwrap(),
        "检查状态 · 部分完成 · 42 ms"
    );
    let mut english = synchronized_reply(&app);
    english.settings.language = LanguagePreference::English;
    let _ = app.update(Message::Finished(Box::new(english)));
    assert_eq!(app.test_results[0], "Status · Partial · 42 ms");
    assert_eq!(app.test_results[2], "System DNS · 保留原始值");
    assert_eq!(app.network_codes, ["dns_failed"]);
    app.present_slint(&data, &mut models);
    assert_eq!(
        data.get_network_results().row_data(0).unwrap(),
        "Status · Partial · 42 ms"
    );
    assert_eq!(
        data.get_network_results().row_data(2).unwrap(),
        "System DNS · 保留原始值"
    );
    let mut chinese = synchronized_reply(&app);
    chinese.settings.language = LanguagePreference::SimplifiedChinese;
    let _ = app.update(Message::Finished(Box::new(chinese)));
    assert_eq!(app.test_results[0], "检查状态 · 部分完成 · 42 ms");
    assert_eq!(app.test_results[2], "系统 DNS · 保留原始值");

    let _ = app.update(Message::ProbeCancel);
    let mut english = synchronized_reply(&app);
    english.settings.language = LanguagePreference::English;
    let _ = app.update(Message::Finished(Box::new(english)));
    assert_eq!(app.test_results, ["Check cancelled"]);
    assert_eq!(app.network_codes, ["cancelled"]);

    let _ = app.update(Message::Probe(String::new()));
    let generation = app.network_test_generation;
    let _ = app.update(Message::ProbeDone(generation, Err("原始错误".into())));
    let mut chinese = synchronized_reply(&app);
    chinese.settings.language = LanguagePreference::SimplifiedChinese;
    let _ = app.update(Message::Finished(Box::new(chinese)));
    assert_eq!(app.test_results, ["检查失败 · 原始错误"]);
    assert!(app.network_codes.is_empty());

    app.claude.protection_pending = Some(true);
    let _ = app.update(Message::Probe(String::new()));
    assert_eq!(app.test_results, ["Claude 保护切换中，请稍后重试"]);
    let mut english = synchronized_reply(&app);
    english.settings.language = LanguagePreference::English;
    let _ = app.update(Message::Finished(Box::new(english)));
    assert_eq!(
        app.test_results,
        ["Claude protection is changing; try again shortly"]
    );
    assert!(app.network_codes.is_empty());
    let _ = app.update(Message::Navigate(Page::Home));
    let _ = app.update(Message::Navigate(Page::Tests));
    assert!(app.test_results.is_empty());
    assert!(app.network_result.is_none());
}

#[test]
fn english_update_results_localize_projection_without_changing_failure_state() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let mut app = fixture(Page::Settings);
    app.presentation_language = Language::English;
    let failure = "更新失败：安装授权已取消；本次未停止原后台";
    app.update_outcome(failure.into(), true);
    app.present_slint(&data, &mut models);
    let expected = "Update failed: Installation authorization cancelled; existing background process not stopped";
    assert_eq!(data.get_notice(), expected);
    assert_eq!(data.get_update_status(), expected);
    assert_eq!(app.pending_update_failure.as_deref(), Some(failure));
    assert_eq!(app.updates.failure.as_deref(), Some(failure));
    let windows_failure = "更新失败：安装未完成：文件仍被占用（resources/GeoIP.dat），请处理后重试";
    app.update_outcome(windows_failure.into(), true);
    app.present_slint(&data, &mut models);
    let windows_expected = "Update failed: Installation incomplete: File is still in use (resources/GeoIP.dat); resolve the issue and retry";
    assert_eq!(data.get_notice(), windows_expected);
    assert_eq!(data.get_update_status(), windows_expected);
    assert_eq!(app.pending_update_failure.as_deref(), Some(windows_failure));
    let download_failure = "内核代理：安装包 SHA256 校验失败，请重试；系统代理：安装包 SHA256 校验失败，请重试；内核直连：安装包 SHA256 校验失败，请重试（已尝试可用代理和直连）";
    app.fail_update(download_failure.into());
    app.present_slint(&data, &mut models);
    let download_expected = "Update failed: Core proxy: Package SHA256 verification failed; retry; System proxy: Package SHA256 verification failed; retry; Core direct: Package SHA256 verification failed; retry (tried available proxies and direct connections)";
    assert_eq!(data.get_notice(), download_expected);
    assert_eq!(data.get_update_status(), download_expected);
    assert_eq!(
        app.pending_update_failure.as_deref(),
        Some(format!("更新失败：{download_failure}").as_str())
    );
    let _ = app.update(Message::DismissNotice);
    assert!(app.pending_update_failure.is_none());
    assert!(app.updates.failure.is_none());
    app.updates.outcome = Some("已更新至 0.6.7；后台和内核已启动".into());
    app.present_slint(&data, &mut models);
    assert_eq!(
        data.get_update_status(),
        "Updated to 0.6.7; background process and core started"
    );
    app.updates.outcome = None;
    app.updates.finish(Ok(None));
    app.present_slint(&data, &mut models);
    assert!(data.get_update_status().ends_with("No updates found"));
    assert!(app.updates.status.ends_with("未发现新版本"));
    assert!(app.updates.begin());
    let check_id = app.updates.check_id;
    let reason = "检查更新失败（已尝试 4 次）：校验文件格式无效: 東京";
    let _ = app.update(Message::UpdateChecked(check_id, Err(reason.into())));
    app.present_slint(&data, &mut models);
    let expected = "Update check failed after 4 attempts: The checksum file is invalid: 東京";
    assert_eq!(data.get_notice(), expected);
    assert!(data.get_update_status().ends_with(expected));
    assert!(app.updates.status.contains(reason));
}

#[test]
fn network_tools_remain_reachable_after_scrolling_other_pages() {
    use slint::platform::WindowEvent;

    let adapter = ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let groups: clash_desktop_protocol::List<_> = (0..100)
        .map(|index| clash_desktop_protocol::GroupItem {
            id: format!("group-{index}").into(),
            title: format!("Group {index}").into(),
            ..Default::default()
        })
        .collect::<Vec<_>>()
        .into();
    let profiles: clash_desktop_protocol::List<_> = (0..100)
        .map(|index| clash_desktop_protocol::ProfileItem {
            id: format!("profile-{index}").into(),
            name: format!("Profile {index}").into(),
            ..Default::default()
        })
        .collect::<Vec<_>>()
        .into();
    let frame = |page, text_scale| clash_desktop_protocol::Frame {
        page,
        text_scale,
        ready: true,
        running: true,
        groups: if page == 1 {
            groups.clone()
        } else {
            Default::default()
        },
        profiles: if page == 2 {
            profiles.clone()
        } else {
            Default::default()
        },
        ..Default::default()
    };
    let events = pointer_events(&window);
    window.show().unwrap();
    for (width, height, text_scale) in [(800, 450, 100), (950, 700, 100), (1280, 800, 130)] {
        adapter.set_size(slint::PhysicalSize::new(width, height));
        for page in [1, 2] {
            let mut previous_network = frame(6, text_scale);
            previous_network.network_results = (0..100)
                .map(|index| format!("Previous diagnostic result {index}").into())
                .collect::<Vec<_>>()
                .into();
            clash_desktop_ui::apply(&previous_network, &data, &mut models);
            data.set_scroll_network(0.);
            motion_frame(&window, 0);
            let viewport = target_bounds("network-detail-viewport");
            window
                .window()
                .dispatch_event(WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(
                        viewport[0] + viewport[2] / 2.,
                        viewport[1] + viewport[3] / 2.,
                    ),
                    delta_x: 0.,
                    delta_y: -100_000.,
                });
            motion_frame(&window, 50);
            assert!(
                data.get_scroll_network() < 0.,
                "{width}x{height}: previous network results must scroll"
            );

            clash_desktop_ui::apply(&frame(page, text_scale), &data, &mut models);
            motion_frame(&window, 0);
            let content = target_bounds("content");
            window
                .window()
                .dispatch_event(WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(
                        content[0] + content[2] / 2.,
                        content[1] + content[3] / 2.,
                    ),
                    delta_x: 0.,
                    delta_y: -100_000.,
                });
            motion_frame(&window, 50);
            let offset = if page == 1 {
                data.get_scroll_proxies()
            } else {
                data.get_scroll_profiles()
            };
            assert!(offset < 0., "{width}x{height} page {page}: {offset}");

            clash_desktop_ui::apply(&frame(6, text_scale), &data, &mut models);
            TARGETS.with(|targets| targets.borrow_mut().clear());
            motion_frame(&window, 0);
            let geometry = window.global::<UiGeometry>();
            geometry.set_revision(geometry.get_revision().wrapping_add(1));
            motion_frame(&window, 5);
            let content = target_bounds("content");
            let first = target_bounds("network-tool:0");
            let viewport = target_bounds("network-detail-viewport");
            let action = target_bounds("button:检查");
            assert!(
                action[1] >= viewport[1] && action[1] + action[3] <= viewport[1] + viewport[3],
                "{width}x{height} from page {page}: action {action:?}, viewport {viewport:?}"
            );
            assert_eq!(
                data.get_scroll_network(),
                0.,
                "{width}x{height} from page {page}: stale detail offset"
            );
            assert!(
                first[1] >= content[1] && first[1] + first[3] <= content[1] + content[3],
                "{width}x{height} from page {page}: tool {first:?}, content {content:?}"
            );
            events.borrow_mut().clear();
            click_target(&window, "network-tool:0");
            assert_eq!(
                events.borrow().last(),
                Some(&(Intent::NetworkTool, "0".into(), "".into()))
            );
            window
                .window()
                .dispatch_event(WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(
                        first[0] + first[2] / 2.,
                        first[1] + first[3] / 2.,
                    ),
                    delta_x: 0.,
                    delta_y: -100_000.,
                });
            motion_frame(&window, 50);
            let last = target_bounds("network-tool:11");
            assert!(
                last[1] >= content[1] && last[1] + last[3] <= content[1] + content[3],
                "{width}x{height} from page {page}: tool {last:?}, content {content:?}"
            );
            events.borrow_mut().clear();
            click_target(&window, "network-tool:11");
            assert_eq!(
                events.borrow().last(),
                Some(&(Intent::NetworkTool, "11".into(), "".into()))
            );
            clash_desktop_ui::apply(&frame(page, text_scale), &data, &mut models);
            motion_frame(&window, 0);
            clash_desktop_ui::apply(&frame(6, text_scale), &data, &mut models);
            motion_frame(&window, 0);
            let reopened = target_bounds("network-tool:0");
            assert!(
                reopened[1] >= content[1] && reopened[1] + reopened[3] <= content[1] + content[3],
                "{width}x{height} return from page {page}: tool {reopened:?}, content {content:?}"
            );
        }
    }
    window.hide().unwrap();
}

#[test]
fn remounted_subscription_list_clamps_a_stale_scroll_position() {
    use slint::platform::WindowEvent;

    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    pointer_events(&window);
    let profiles = (0..100)
        .map(|index| clash_desktop_protocol::ProfileItem {
            id: format!("profile-{index}").into(),
            name: format!("Profile {index}").into(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let mut profile_frame = clash_desktop_protocol::Frame {
        page: 2,
        text_scale: 100,
        ready: true,
        profiles: profiles.clone().into(),
        ..Default::default()
    };
    clash_desktop_ui::apply(&profile_frame, &data, &mut models);
    window.show().unwrap();
    motion_frame(&window, 0);
    let content = target_bounds("content");
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(
                content[0] + content[2] / 2.,
                content[1] + content[3] / 2.,
            ),
            delta_x: 0.,
            delta_y: -100_000.,
        });
    motion_frame(&window, 50);
    assert!(data.get_scroll_profiles() < 0.);
    let network = clash_desktop_protocol::Frame {
        page: 6,
        text_scale: 100,
        ..Default::default()
    };
    clash_desktop_ui::apply(&network, &data, &mut models);
    motion_frame(&window, 0);
    profile_frame.profiles = profiles[..12].to_vec().into();
    clash_desktop_ui::apply(&profile_frame, &data, &mut models);
    TARGETS.with(|targets| targets.borrow_mut().clear());
    motion_frame(&window, 0);
    let geometry = window.global::<UiGeometry>();
    geometry.set_revision(geometry.get_revision().wrapping_add(1));
    motion_frame(&window, 5);
    let last = target_bounds("profile:profile-11");
    let content = target_bounds("content");
    assert!(
        last[1] >= content[1] && last[1] + last[3] <= content[1] + content[3],
        "last profile {last:?} is outside {content:?}"
    );
    assert!(data.get_scroll_profiles() < 0.);
    clash_desktop_ui::apply(&network, &data, &mut models);
    motion_frame(&window, 0);
    profile_frame.profiles = vec![profiles[0].clone()].into();
    clash_desktop_ui::apply(&profile_frame, &data, &mut models);
    TARGETS.with(|targets| targets.borrow_mut().clear());
    motion_frame(&window, 0);
    let geometry = window.global::<UiGeometry>();
    geometry.set_revision(geometry.get_revision().wrapping_add(1));
    motion_frame(&window, 5);
    let profile = target_bounds("profile:profile-0");
    let content = target_bounds("content");
    assert!(
        profile[1] >= content[1] && profile[1] + profile[3] <= content[1] + content[3],
        "profile {profile:?} is outside {content:?}"
    );
    assert_eq!(data.get_scroll_profiles(), 0.);
    window.hide().unwrap();
}

#[test]
fn remounted_long_pages_preserve_valid_scroll_positions() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    pointer_events(&window);
    let mut network = clash_desktop_protocol::Frame {
        page: 6,
        text_scale: 100,
        ready: true,
        ..Default::default()
    };
    network.network_results = (0..100)
        .map(|index| format!("Diagnostic result {index}").into())
        .collect::<Vec<_>>()
        .into();
    let profiles = clash_desktop_protocol::Frame {
        page: 2,
        text_scale: 100,
        ready: true,
        profiles: (0..100)
            .map(|index| clash_desktop_protocol::ProfileItem {
                id: format!("profile-{index}").into(),
                name: format!("Profile {index}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>()
            .into(),
        ..Default::default()
    };
    let home = clash_desktop_protocol::Frame {
        text_scale: 100,
        ready: true,
        ..Default::default()
    };
    window.show().unwrap();
    for (page, network_page) in [(&network, true), (&profiles, false)] {
        clash_desktop_ui::apply(page, &data, &mut models);
        motion_frame(&window, 5);
        if network_page {
            data.set_scroll_network(-100.);
        } else {
            data.set_scroll_profiles(-100.);
        }
        motion_frame(&window, 5);
        let offset = if network_page {
            data.get_scroll_network()
        } else {
            data.get_scroll_profiles()
        };
        assert_eq!(offset, -100.);
        clash_desktop_ui::apply(&home, &data, &mut models);
        motion_frame(&window, 5);
        clash_desktop_ui::apply(page, &data, &mut models);
        motion_frame(&window, 5);
        let restored = if network_page {
            data.get_scroll_network()
        } else {
            data.get_scroll_profiles()
        };
        assert_eq!(restored, offset, "page {}", page.page);
    }
    window.hide().unwrap();
}

#[test]
fn window_controls_remain_available_during_subscription_and_proxy_updates() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    pointer_events(&window);
    let closed = std::rc::Rc::new(std::cell::Cell::new(0));
    let requests = closed.clone();
    window.window().on_close_requested(move || {
        requests.set(requests.get() + 1);
        slint::CloseRequestResponse::KeepWindowShown
    });
    window.window().set_maximized(false);
    window.window().set_minimized(false);
    window.show().unwrap();
    let profiles: clash_desktop_protocol::List<_> = ["original", "temporary"]
        .into_iter()
        .map(|name| clash_desktop_protocol::ProfileItem {
            id: name.into(),
            name: name.into(),
            source: "https://example.test/subscription".into(),
            ..Default::default()
        })
        .collect::<Vec<_>>()
        .into();
    let nodes = (0..4)
        .map(|index| clash_desktop_protocol::NodePair {
            left: clash_desktop_protocol::NodeItem {
                name: format!("fixture-node-{index}").into(),
                valid: true,
                ..Default::default()
            },
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let groups: clash_desktop_protocol::List<_> = (0..4)
        .map(|index| clash_desktop_protocol::GroupItem {
            id: format!("group-{index}").into(),
            title: format!("Group {index}").into(),
            expanded: true,
            nodes: nodes.clone().into(),
            ..Default::default()
        })
        .collect::<Vec<_>>()
        .into();
    for (index, (page, working, modal)) in [
        (2, true, 0),
        (1, false, 0),
        (2, true, 1),
        (1, true, 0),
        (2, false, 0),
        (1, false, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let frame = clash_desktop_protocol::Frame {
            page,
            working,
            modal,
            text_scale: 100,
            ready: true,
            running: true,
            profiles: if page == 2 {
                profiles.clone()
            } else {
                Default::default()
            },
            groups: if page == 1 {
                groups.clone()
            } else {
                Default::default()
            },
            ..Default::default()
        };
        clash_desktop_ui::apply(&frame, &data, &mut models);
        motion_frame(&window, 0);
        click_target(&window, "window-minimize");
        assert!(
            window.window().is_minimized(),
            "minimize during update {index}"
        );
        window.window().set_minimized(false);
        click_target(&window, "window-maximize");
        assert!(
            window.window().is_maximized(),
            "maximize during update {index}"
        );
        window.window().set_maximized(false);
        click_target(&window, "window-close");
        assert_eq!(closed.get(), index + 1, "close during update {index}");
    }
    window.hide().unwrap();
}

#[test]
fn home_overview_reports_host_exit_dns_and_each_service_without_overclaiming() {
    use clash_of_rust::overview::{Health, Status, TARGETS};
    ui_testing::initialize(950, 700);
    let ip = |text: &str| text.parse::<std::net::IpAddr>().unwrap();
    let mut app = fixture(Page::Home);
    app.visible = true;
    let _ = app.overview_tick();
    assert!(app.overview.busy());
    let _ = app.overview_tick();
    let generation = 1;
    let _ = app.update(Message::OverviewHost(
        generation,
        Ok(platform::HostInfo {
            addresses: vec![ip("192.168.1.23")],
            dns: vec![ip("223.5.5.5")],
        }),
    ));
    let _ = app.update(Message::OverviewDns(
        generation,
        Ok((vec!["198.18.0.9".into()], 12)),
    ));
    let _ = app.update(Message::OverviewExit(
        generation,
        Ok(ip_check::CheckResult {
            state: ip_check::State::Identified,
            summary: "203.0.113.2".into(),
            country: "🇩🇪 德国".into(),
            millis: Some(188),
            authentication_check_skipped: false,
            detail: String::new(),
            target_exit: None,
        }),
    ));
    for (index, (health, status, millis)) in [
        (Health::Ok, 204, 142),
        (Health::Reachable, 403, 264),
        (Health::Error, 502, 50),
    ]
    .into_iter()
    .enumerate()
    {
        let _ = app.update(Message::OverviewService(
            generation,
            index,
            Ok(Status {
                health,
                status,
                millis,
            }),
        ));
    }
    let _ = app.update(Message::OverviewService(
        generation,
        3,
        Err("无法连接".into()),
    ));
    let _ = app.update(Message::OverviewService(
        0,
        4,
        Ok(Status {
            health: Health::Ok,
            status: 200,
            millis: 1,
        }),
    ));
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let network = data.get_overview_network();
    let row = |index| network.row_data(index).unwrap();
    assert_eq!(
        (row(0).value.as_str(), row(0).detail.as_str()),
        ("192.168.1.23", "仅 IPv4")
    );
    assert_eq!(row(1).value.as_str(), "203.0.113.2");
    assert!(row(1).flagged);
    assert_eq!(row(1).flag.as_str(), "🇩🇪");
    assert_eq!(row(1).detail.as_str(), "德国");
    assert_eq!((row(1).delay.as_str(), row(1).delay_tone), ("188 ms", 1));
    assert_eq!(row(2).value.as_str(), "223.5.5.5");
    assert!(row(3).detail.contains("Fake-IP"));
    let services = data.get_overview_services();
    let item = |index| services.row_data(index).unwrap();
    assert_eq!((item(0).tone, item(0).value.as_str()), (1, "142 ms"));
    assert_eq!(
        (item(1).tone, item(1).detail.as_str()),
        (2, "HTTP 403 · 可达")
    );
    assert_eq!(item(1).delay_tone, 1);
    assert_eq!((item(2).tone, item(2).value.as_str()), (3, "服务异常"));
    assert_eq!((item(3).tone, item(3).value.as_str()), (3, "无法连接"));
    assert_eq!((item(4).tone, item(4).value.as_str()), (4, "检测中…"));
    assert!(data.get_overview_busy());

    let _ = app.ui_intent(Intent::OverviewPrivacy, "", "");
    app.present_slint(&data, &mut models);
    assert!(data.get_privacy_hidden());
    let hidden = data.get_overview_network();
    for (index, real) in [(0, "192.168.1.23"), (1, "203.0.113.2"), (2, "223.5.5.5")] {
        let row = hidden.row_data(index).unwrap();
        assert!(row.fogged);
        assert_ne!(row.value.as_str(), real);
        assert_eq!(row.value.len(), real.len());
        assert!(row.value.parse::<std::net::IpAddr>().is_ok());
    }
    let first = hidden.row_data(0).unwrap().value;
    app.present_slint(&data, &mut models);
    assert_eq!(
        data.get_overview_network().row_data(0).unwrap().value,
        first
    );
    assert!(hidden.row_data(3).unwrap().fogged);
    assert!(hidden.row_data(3).unwrap().value.contains("github.com"));
    assert!(!hidden.row_data(3).unwrap().value.contains("198.18.0.9"));
    let _ = app.ui_intent(Intent::OverviewPrivacy, "", "");
    app.present_slint(&data, &mut models);
    assert!(!data.get_privacy_hidden());
    assert_eq!(
        data.get_overview_network()
            .row_data(0)
            .unwrap()
            .value
            .as_str(),
        "192.168.1.23"
    );
    for index in 4..TARGETS.len() {
        let _ = app.update(Message::OverviewService(
            generation,
            index,
            Err("超时".into()),
        ));
    }
    assert!(!app.overview.busy());
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_overview_status().as_str(), "可访问 2 / 8");
    let _ = app.overview_tick();
    assert!(!app.overview.busy());
    app.snapshot.running = false;
    app.present_slint(&data, &mut models);
    assert_eq!(
        data.get_overview_services()
            .row_data(0)
            .unwrap()
            .value
            .as_str(),
        "未检测"
    );
    assert_eq!(
        data.get_overview_network()
            .row_data(1)
            .unwrap()
            .detail
            .as_str(),
        "启动内核后检测"
    );
}

#[test]
fn leaving_home_aborts_overview_and_late_results_do_not_repopulate_it() {
    let mut app = fixture(Page::Home);
    app.visible = true;
    let _ = app.overview_tick();
    assert!(app.overview.busy());
    let _ = app.update(Message::Navigate(Page::Profiles));
    assert!(!app.overview.busy());
    let _ = app.update(Message::OverviewDns(1, Ok((vec!["198.18.0.9".into()], 12))));
    assert!(app.overview.dns.is_none());
    let _ = app.update(Message::OverviewRefresh);
    assert!(!app.overview.busy());
    app.page = Page::Home;
    app.visible = false;
    let _ = app.update(Message::OverviewRefresh);
    assert!(!app.overview.busy());
    app.visible = true;
    app.working = true;
    let _ = app.update(Message::OverviewRefresh);
    assert!(!app.overview.busy());
}

#[test]
fn home_mode_choices_have_visible_hit_targets_and_dispatch_each_action() {
    ui_testing::initialize(950, 700);
    let app = fixture(Page::Home);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let events = pointer_events(&window);
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    for (title, expected, value) in [
        ("规则", Intent::Mode, "0"),
        ("全局", Intent::Mode, "1"),
        ("直连", Intent::Mode, "2"),
        ("系统代理", Intent::ProxyMode, "0"),
        ("TUN", Intent::ProxyMode, "1"),
        ("关闭", Intent::ProxyMode, "2"),
    ] {
        let key = format!("button:{title}");
        let bounds = target_bounds(&key);
        assert!(
            bounds[2] >= 40.0 && bounds[3] >= 28.0,
            "{title}: {bounds:?}"
        );
        events.borrow_mut().clear();
        click_target(&window, &key);
        let recorded = events.borrow();
        assert_eq!(recorded.len(), 1, "{title}: {recorded:?}");
        assert_eq!(recorded[0].0, expected);
        assert_eq!(recorded[0].1.as_str(), value);
    }
    window.hide().unwrap();
}

#[test]
fn startup_install_failure_reaches_the_visible_recoverable_notice_and_close_action() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Home);
    let failure = "更新失败：安装程序退出（代码 68）；请检查安装目录后重试";
    app.prepare_initialization(Some((failure.into(), true)));
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let events = pointer_events(&window);
    track_targets(&window);
    app.present_slint(&data, &mut models);
    window.show().unwrap();
    motion_frame(&window, 250);
    assert!(window.get_notice_mounted());
    assert_eq!(data.get_notice(), failure);
    assert!(data.get_notice_error() && !data.get_notice_critical());
    click_target(&window, "notice-message");
    assert!(events.borrow().is_empty());
    let coordinates: Vec<f32> = window.get_notice_close_target().iter().collect();
    assert!(coordinates[2] > 0.0 && coordinates[3] > 0.0);
    ui_testing::click(
        window.window(),
        coordinates[0] + coordinates[2] / 2.0,
        coordinates[1] + coordinates[3] / 2.0,
    );
    assert_eq!(events.borrow().last().unwrap().0, Intent::DismissNotice);
    let _ = app.ui_intent(Intent::DismissNotice, "", "");
    app.present_slint(&data, &mut models);
    assert!(data.get_notice().is_empty());
    window.hide().unwrap();
}

#[test]
fn errors_require_an_explicit_close_and_success_messages_allow_body_clicks() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let events = pointer_events(&window);
    track_targets(&window);
    window.show().unwrap();
    for (error, critical) in [(false, false), (true, false), (true, true)] {
        data.set_notice("测试通知，错误详情不会因点击正文丢失".into());
        data.set_notice_error(error);
        data.set_notice_critical(critical);
        let _ = window.window().take_snapshot().unwrap();
        events.borrow_mut().clear();
        if error {
            click_target(&window, "notice-message");
            assert!(events.borrow().is_empty());
        }
        let target = window.get_notice_close_target();
        let coordinates: Vec<f32> = target.iter().collect();
        ui_testing::click(
            window.window(),
            coordinates[0] + coordinates[2] / 2.0,
            coordinates[1] + coordinates[3] / 2.0,
        );
        assert_eq!(events.borrow().len(), 1);
        assert_eq!(events.borrow()[0].0, Intent::DismissNotice);
        data.set_notice("".into());
    }
}

#[test]
fn long_subscription_names_preserve_visible_actions_and_the_full_model() {
    let name = format!(
        "🚀 超长订阅配置名称{}",
        "VeryLongUnbrokenName0123456789".repeat(12)
    );
    for (width, height, scale) in [(800, 450, 100), (950, 700, 130)] {
        ui_testing::initialize(width, height);
        let mut app = fixture(Page::Profiles);
        app.profiles[0].name = name.clone();
        app.profiles[0].source = "https://example.invalid/subscription".into();
        let window = AppWindow::new().unwrap();
        let data = window.global::<UiData>();
        let mut models = ViewModels::default();
        models.attach(&data);
        app.present_slint(&data, &mut models);
        data.set_text_scale(scale);
        let events = pointer_events(&window);
        window.show().unwrap();
        let _ = window.window().take_snapshot().unwrap();
        let mut previous_right = 0.0;
        for action in ["update", "edit", "detail"] {
            let bounds = target_bounds(&format!("profile-{action}:fixture"));
            assert!(bounds[0] >= previous_right);
            assert!(bounds[0] + bounds[2] <= width as f32);
            assert!(bounds[1] >= 0.0 && bounds[1] + bounds[3] <= height as f32);
            previous_right = bounds[0] + bounds[2];
        }
        assert_eq!(models.profiles.row_data(0).unwrap().name.as_str(), name);
        click_target(&window, "profile-update:fixture");
        assert_eq!(events.borrow()[0].0, Intent::UpdateProfile);
        assert_eq!(events.borrow()[0].1.as_str(), "fixture");
        window.hide().unwrap();
    }
}

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
fn rule_warnings_follow_the_desktop_language_without_changing_rule_data() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let mut app = fixture(Page::Rules);
    app.profiles[0].name = "机场订阅 · 原名".into();
    let document = rule_manager::Document::from_raw(
        "fixture",
        false,
        "proxies: []\nrules: ['DOMAIN,duplicate.test,DIRECT', 'MATCH,DIRECT', 'DOMAIN,hidden.test,DIRECT']\n",
    )
    .unwrap();
    let policy = "店铺“香港”；東京";
    let mut order = vec!["s:removed-rule".into()];
    order.extend(document.original.iter().map(|row| row.id.clone()));
    let draft = Arc::new(ProfileRules {
        custom: vec![
            rule_manager::CustomRule {
                id: "duplicate".into(),
                rule: "DOMAIN,duplicate.test,DIRECT".into(),
                enabled: true,
            },
            rule_manager::CustomRule {
                id: "missing-policy".into(),
                rule: format!("DOMAIN,policy.test,{policy}"),
                enabled: true,
            },
        ],
        order,
        ..Default::default()
    });
    let mut reply = synchronized_reply(&app);
    reply.settings.language = LanguagePreference::SimplifiedChinese;
    reply
        .settings
        .rule_profiles
        .insert("fixture".into(), draft.clone());
    reply.rule_document = Some(Arc::new(document));
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert_eq!(app.rule_state.warnings.len(), 4);
    let unknown = "订阅更新后有供应商提示；请保留原文";
    app.rule_state.warnings.push(unknown.into());
    let raw = app.rule_state.warnings.clone();
    let chinese = [
        "有 1 条自定义规则已包含在订阅或其他规则中，保留原规则，不重复加入。",
        "订阅更新后有 1 条原规则已删除或更改；旧排序引用已跳过，可恢复订阅顺序重新整理。",
        "MATCH 之后有 1 条规则不会被匹配，请检查顺序。",
        "订阅中未找到策略“店铺“香港”；東京”，请确认名称或改选策略；应用时会再次校验。",
        unknown,
    ]
    .join("\n");
    let english = [
        "Skipped duplicate custom rules already present in the subscription or other rules (1).",
        "Skipped stale ordering references (1) to subscription rules that were removed or changed. Restore the subscription order to reorder the rules.",
        "Rules after MATCH will never be evaluated (1 affected). Check the rule order.",
        "Policy “店铺“香港”；東京” was not found in the subscription. Check its name or choose another policy. It will be validated again when you apply the rules.",
        unknown,
    ]
    .join("\n");

    for (language, expected) in [
        (LanguagePreference::SimplifiedChinese, &chinese),
        (LanguagePreference::English, &english),
        (LanguagePreference::SimplifiedChinese, &chinese),
    ] {
        let mut reply = synchronized_reply(&app);
        reply.settings.language = language;
        let _ = app.update(Message::Finished(Box::new(reply)));
        app.present_slint(&data, &mut models);
        assert_eq!(data.get_rule_warning().as_str(), expected);
        assert_eq!(data.get_rule_profile(), "机场订阅 · 原名");
        assert_eq!(app.rule_state.warnings, raw);
        assert!(Arc::ptr_eq(&app.rule_state.draft, &draft));
    }
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
        let accent = window.global::<clash_client_ui::Theme>().get_accent();
        let accent = slint::Rgb8Pixel::new(accent.red(), accent.green(), accent.blue());
        (0..700)
            .find(|&y| (350..600).all(|x| pixels[y * 950 + x] == accent))
            .expect("the lifted row must render a full-width accent border")
    };
    let bounds = target_bounds("rule-drag");
    let distance = bounds[3].round() as usize;
    for horizontal in [1.0, bounds[2] - 1.0] {
        let start =
            slint::LogicalPosition::new(bounds[0] + horizontal, bounds[1] + bounds[3] / 2.0);
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: start });
        window.window().dispatch_event(WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        });
        let top = border_y();
        let end = slint::LogicalPosition::new(start.x, start.y + distance as f32);
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: end });
        assert_eq!(
            border_y(),
            top + distance,
            "the preview must follow the pointer"
        );
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
    let mut frame_number = 0;
    let mut header = || {
        frame_number += 1;
        assert!(adapter.draw_if_needed(|renderer| {
            renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
            renderer.render(&mut pixels, 950);
            let mut reference = vec![slint::Rgb8Pixel::default(); 950 * 700];
            renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
            renderer.render(&mut reference, 950);
            if let Some(index) = pixels.iter().zip(&reference).position(|(a, b)| a != b) {
                let differences: Vec<_> = pixels
                    .iter()
                    .zip(&reference)
                    .enumerate()
                    .filter(|(_, (actual, expected))| actual != expected)
                    .collect();
                let max_delta = differences
                    .iter()
                    .map(|(_, (actual, expected))| {
                        actual.r.abs_diff(expected.r)
                            .max(actual.g.abs_diff(expected.g))
                            .max(actual.b.abs_diff(expected.b))
                    })
                    .max()
                    .unwrap();
                panic!(
                    "frame {frame_number}: {} pixels differ, largest channel delta {max_delta}; first at ({}, {}): {:?} versus {:?}",
                    differences.len(),
                    index % 950,
                    index / 950,
                    pixels[index],
                    reference[index]
                );
            }
            renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
            // Changing buffer modes clears Slint's damage cache. Warm it now,
            // so the next click really renders incrementally into this buffer.
            renderer.render(&mut pixels, 950);
        }));
        let bounds = target_bounds("connection-header");
        let left = bounds[0].round() as usize;
        let right = (bounds[0] + bounds[2]).round() as usize;
        pixels
            .as_chunks::<950>()
            .0
            .iter()
            .skip(bounds[1].round() as usize)
            .take(bounds[3].round() as usize)
            .flat_map(|row| row[left..right].iter().copied())
            .collect::<Vec<_>>()
    };
    let before = header();
    let original = data
        .get_connections()
        .iter()
        .map(|row| row.id)
        .collect::<Vec<_>>();
    for (field, first_descending) in [
        ("download-rate", true),
        ("download-total", true),
        ("upload-rate", true),
        ("upload-total", true),
        ("rule", false),
        ("chain", false),
        ("name", false),
        ("start", true),
    ] {
        let mut frames = Vec::new();
        for click in 0..3 {
            click_target(&window, &format!("connection-sort:{field}"));
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
        for field in ["upload-rate", "chain", "name", "download-rate"] {
            rapid_target(&window, &format!("connection-sort:{field}"));
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
    app.snapshot.connections.connections[0].metadata.process = "test-browser".into();
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let _ = app.ui_intent(Intent::ConnectionDetail, "1", "");
    app.present_slint(&data, &mut models);
    let _events = pointer_events(&window);
    window.show().unwrap();
    motion_frame(&window, 0);
    let close = target_bounds("dialog-dismiss");
    for key in ["dialog-copy", "dialog-close-connection"] {
        let bounds = target_bounds(key);
        assert_eq!(bounds[2], close[2], "{key} must use the same button width");
        assert_eq!(bounds[3], close[3], "{key} must use the same button height");
    }
    let details = data.get_connection_details();
    assert_eq!(details.row_count(), 15);
    for row in details.iter() {
        assert!(!row.value.is_empty());
        assert!(
            data.get_modal_copy()
                .contains(&format!("{}：{}", row.label, row.value))
        );
    }
    assert!(data.get_modal_copy().contains("下载速度：256.0 KiB/s"));
    assert!(data.get_modal_copy().contains("域名：one.test"));
    assert!(data.get_modal_copy().contains("嗅探域名：—"));
    assert!(data.get_modal_copy().contains("目标 IP：—"));
    assert!(
        details
            .iter()
            .any(|row| row.label == "进程" && row.value == "test-browser")
    );
    assert!(data.get_modal_copy().contains("进程：test-browser"));
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
    window.hide().unwrap();
}

#[test]
fn english_connection_detail_actions_fit_a_scaled_window() {
    ui_testing::initialize(800, 450);
    let mut app = fixture(Page::Connections);
    app.presentation_language = Language::English;
    app.settings.text_scale = 130;
    let window = AppWindow::new().unwrap();
    slint::select_bundled_translation("en").unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let _ = app.ui_intent(Intent::ConnectionDetail, "1", "");
    app.present_slint(&data, &mut models);
    let _events = pointer_events(&window);
    window.show().unwrap();
    motion_frame(&window, 250);
    let close = target_bounds("dialog-dismiss");
    for key in ["dialog-copy", "dialog-close-connection"] {
        let bounds = target_bounds(key);
        assert_eq!(bounds[2], close[2], "{key} must use the same button width");
        assert!(
            bounds[0] >= 0. && bounds[0] + bounds[2] <= 800.,
            "{key}: {bounds:?}"
        );
        assert!(
            bounds[1] >= 0. && bounds[1] + bounds[3] <= 450.,
            "{key}: {bounds:?}"
        );
    }
    window.hide().unwrap();
    slint::select_bundled_translation("").unwrap();
}

#[test]
fn ip_response_times_preserve_availability_and_use_independent_latency_tones() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Websites);
    app.site_show_results = true;
    for millis in [128, 420, 925] {
        app.site_results.push(Some(Ok(ip_check::CheckResult {
            state: ip_check::State::Confirmed,
            summary: "Confirmed service".into(),
            country: String::new(),
            detail: String::new(),
            millis: Some(millis),
            authentication_check_skipped: false,
            target_exit: None,
        })));
    }
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    for (index, (delay, tone)) in [("128 ms", 1), ("420 ms", 2), ("925 ms", 3)]
        .into_iter()
        .enumerate()
    {
        let row = data.get_site_results().row_data(index).unwrap();
        assert_eq!(row.delay, delay);
        assert_eq!(row.delay_tone, tone);
        assert_eq!(row.tone, 1);
        assert!(!row.source.contains("ms"));
    }
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
    track_targets(&window);
    window
        .global::<UiActions>()
        .on_dispatch(move |intent, a, b| recorded.borrow_mut().push((intent, a, b)));
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    click_target(&window, "node-detail:node-a");
    assert!(
        events.borrow().is_empty(),
        "details must not select or test a node"
    );
    let _ = window.window().take_snapshot().unwrap();
    click_target(&window, "node-detail:node-a");
    let _ = window.window().take_snapshot().unwrap();
    click_target(&window, "node-delay:node-a");
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::NodeDelay, "node-a".into(), "".into())],
        "the first button click must run the action, not only focus it"
    );
    events.borrow_mut().clear();
    click_target_at(&window, "node:node-a", 0.15, 0.2);
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::SelectNode, "Proxy".into(), "node-a".into())],
        "clicking the card must select the node in its own group"
    );
    events.borrow_mut().clear();
    data.set_protected(true);
    let _ = window.window().take_snapshot().unwrap();
    click_target_at(&window, "node:node-a", 0.15, 0.2);
    assert!(
        events.borrow().is_empty(),
        "protected nodes must not dispatch a selection"
    );
    data.set_protected(false);
    let _ = window.window().take_snapshot().unwrap();
    events.borrow_mut().clear();
    click_target(&window, "group:Proxy");
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::ToggleGroup, "Proxy".into(), "".into())]
    );
    events.borrow_mut().clear();
    click_target(&window, "node-delay:node-a");
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::NodeDelay, "node-a".into(), "".into())],
        "the latency button must consume the click without selecting the node"
    );
    window.hide().unwrap();
}

#[test]
fn resizing_a_node_grid_preserves_odd_tail_nodes_and_their_own_actions() {
    use clash_client_ui::{GroupItem, NodeItem, NodePair};
    let surface = ui_testing::initialize(1600, 900);
    let app = fixture(Page::Proxies);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let nodes: Vec<_> = (0..5)
        .map(|index| NodeItem {
            group: "Proxy".into(),
            name: format!("node-{index}").into(),
            kind: "VLESS".into(),
            valid: true,
            ..Default::default()
        })
        .collect();
    models.set_groups(vec![(
        GroupItem {
            id: "Proxy".into(),
            title: "Proxy".into(),
            expanded: true,
            ..Default::default()
        },
        nodes
            .chunks(2)
            .map(|pair| NodePair {
                left: pair[0].clone(),
                right: pair.get(1).cloned().unwrap_or_default(),
            })
            .collect(),
    )]);
    let events = pointer_events(&window);
    window.show().unwrap();
    window.global::<clash_client_ui::Motion>().set_enabled(true);
    motion_frame(&window, 250);
    let geometry = window.global::<UiGeometry>();
    geometry.set_enabled(false);
    motion_frame(&window, 0);
    TARGETS.with(|targets| targets.borrow_mut().clear());
    geometry.set_revision(geometry.get_revision().wrapping_add(1));
    motion_frame(&window, 0);
    assert!(TARGETS.with(|targets| targets.borrow().is_empty()));
    geometry.set_enabled(true);
    geometry.set_revision(geometry.get_revision().wrapping_add(1));
    motion_frame(&window, 0);
    assert!(target_bounds("node:node-0")[2] > 0.);
    assert!(target_bounds("node-delay:node-0")[2] > 0.);
    for (width, height) in [
        (1600, 900),
        (950, 700),
        (875, 700),
        (865, 700),
        (885, 700),
        (940, 700),
        (1200, 700),
    ] {
        surface.set_size(slint::PhysicalSize::new(width, height));
        motion_frame(&window, 0);
        let expected_sidebar = if width < 880 { 60. } else { 216. };
        let content = target_bounds("content");
        assert!(
            (content[0] - expected_sidebar - 24.).abs() < 1.,
            "{width}x{height}: content must retain its page padding after sidebar reflow: {content:?}"
        );
        let initial_bounds: Vec<_> = (0..5)
            .map(|index| target_bounds(&format!("node:node-{index}")))
            .collect();
        motion_frame(&window, 250);
        assert!(
            !window.window().has_active_animations(),
            "{width}x{height}: resizing must not leave an animation running"
        );
        for (index, bounds) in initial_bounds.iter().enumerate() {
            assert_eq!(
                target_bounds(&format!("node:node-{index}")),
                *bounds,
                "{width}x{height}: node geometry must follow the resize without a delayed transition"
            );
            events.borrow_mut().clear();
            click_target_at(&window, &format!("node:node-{index}"), 0.15, 0.2);
            assert_eq!(
                events.borrow().as_slice(),
                &[(
                    Intent::SelectNode,
                    "Proxy".into(),
                    format!("node-{index}").into()
                )],
                "{width}x{height}: each visible card must retain its original node"
            );
        }
    }
    click_target(&window, "nodes-view");
    motion_frame(&window, 250);
    assert!(
        window
            .global::<clash_client_ui::PageViews>()
            .get_nodes_list()
    );
    let first = target_bounds("node:node-0");
    let second = target_bounds("node:node-1");
    assert!(
        first[3] < 52.,
        "node list rows must stay compact: {first:?}"
    );
    let delay = target_bounds("node-delay:node-0");
    let detail = target_bounds("node-detail:node-0");
    assert!(
        delay[0] < detail[0],
        "latency must precede the detail action"
    );
    assert!((first[0] - second[0]).abs() < 1.);
    assert!(second[1] >= first[1] + first[3]);
    events.borrow_mut().clear();
    click_target(&window, "node-delay:node-1");
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::NodeDelay, "node-1".into(), "".into())]
    );
    click_target(&window, "nodes-view");
    motion_frame(&window, 250);
    assert!(
        !window
            .global::<clash_client_ui::PageViews>()
            .get_nodes_list()
    );
    window.hide().unwrap();
}

#[test]
fn sidebar_toggle_keeps_manual_choice_across_breakpoints_and_respects_reduced_motion() {
    let surface = ui_testing::initialize(950, 700);
    let app = fixture(Page::Home);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let _events = pointer_events(&window);
    window.show().unwrap();
    motion_frame(&window, 0);
    let expanded = target_bounds("content");
    click_target(&window, "sidebar-toggle");
    motion_frame(&window, 0);
    assert!(window.get_sidebar_collapsed());
    assert!(window.get_compact_sidebar());
    let collapsed = target_bounds("content");
    assert!((expanded[0] - collapsed[0] - 156.).abs() < 1.);
    surface.set_size(slint::PhysicalSize::new(800, 450));
    motion_frame(&window, 0);
    assert!(window.get_compact_sidebar());
    assert!(window.get_sidebar_collapsed());
    surface.set_size(slint::PhysicalSize::new(950, 700));
    motion_frame(&window, 0);
    assert!(window.get_sidebar_collapsed());
    click_target(&window, "sidebar-toggle");
    motion_frame(&window, 0);
    assert!(!window.get_compact_sidebar());
    let motion = window.global::<clash_client_ui::Motion>();
    motion.set_enabled(true);
    motion_frame(&window, 250);
    click_target(&window, "sidebar-toggle");
    motion_frame(&window, 60);
    let moving = target_bounds("content")[0];
    assert!(moving > collapsed[0] && moving < expanded[0]);
    motion_frame(&window, 250);
    assert!(!window.window().has_active_animations());
    motion.set_reduced_motion(true);
    click_target(&window, "sidebar-toggle");
    motion_frame(&window, 0);
    assert!(!window.get_sidebar_collapsed());
    assert!((target_bounds("content")[0] - expanded[0]).abs() < 1.);
    assert_eq!(data.get_page(), Page::Home as i32);
    window.hide().unwrap();
}

#[test]
fn subscription_search_filters_names_and_sources_without_replacing_the_model() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Profiles);
    let mut backup = app.profiles[0].clone();
    backup.id = "backup".into();
    backup.name = "备用线路".into();
    backup.source = "https://backup.example.test/config".into();
    app.profiles.push(backup);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let rows = data.get_profiles();
    assert_eq!(rows.row_count(), 2);
    for query in ["备用", "BACKUP.EXAMPLE.TEST"] {
        app.query = query.into();
        app.present_slint(&data, &mut models);
        assert_eq!(rows, data.get_profiles());
        assert_eq!(rows.row_count(), 1);
        assert_eq!(rows.row_data(0).unwrap().id, "backup");
    }
    app.query = "no matching subscription".into();
    app.present_slint(&data, &mut models);
    assert_eq!(rows.row_count(), 0);
    app.query.clear();
    app.present_slint(&data, &mut models);
    assert_eq!(rows.row_count(), 2);
}

#[test]
fn subscription_usage_changes_language_without_altering_metadata() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Profiles);
    let usage = "upload=1048576; download=2147483648; total=3221225472; expire=0";
    app.profiles[0].source = "https://example.test/subscription".into();
    app.profiles[0].usage = Some(usage.into());
    app.settings.language = LanguagePreference::SimplifiedChinese;
    app.presentation_language = Language::SimplifiedChinese;
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let rows = data.get_profiles();
    assert_eq!(rows.row_data(0).unwrap().detail, "已用 2/3 GiB · 不限期");
    for (language, expected) in [
        (LanguagePreference::English, "Used 2/3 GiB · No expiry"),
        (
            LanguagePreference::SimplifiedChinese,
            "已用 2/3 GiB · 不限期",
        ),
    ] {
        let mut reply = synchronized_reply(&app);
        reply.settings.language = language;
        let _ = app.update(Message::Finished(Box::new(reply)));
        app.present_slint(&data, &mut models);
        assert_eq!(rows, data.get_profiles());
        assert_eq!(rows.row_data(0).unwrap().detail, expected);
        assert_eq!(app.profiles[0].usage.as_deref(), Some(usage));
    }
}

#[test]
fn subscription_forms_use_the_available_width_without_truncating_stored_input() {
    let surface = ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let _events = pointer_events(&window);
    let name = "A long subscription name with emoji 🚀 and multiple words";
    let source = "https://example.test/subscription/config.yaml?client=desktop&format=mihomo";
    data.set_profile_name(name.into());
    data.set_profile_source(source.into());
    data.set_edit_name(name.into());
    data.set_edit_source(source.into());
    data.set_modal_title("订阅".into());
    data.set_modal_accept("保存".into());
    window.show().unwrap();
    for width in [800, 950, 1600] {
        surface.set_size(slint::PhysicalSize::new(width, 700));
        for kind in [2, 3] {
            data.set_modal(kind);
            motion_frame(&window, 0);
            let name_bounds = target_bounds("field:profile-edit-name");
            let source_bounds = target_bounds("field:profile-edit-source");
            assert!(
                name_bounds[2] >= 500.,
                "{width}px, modal {kind}: {name_bounds:?}"
            );
            assert_eq!(name_bounds[0], source_bounds[0]);
            assert_eq!(name_bounds[2], source_bounds[2]);
            assert!(name_bounds[0] >= 0.);
            assert!(name_bounds[0] + name_bounds[2] <= width as f32);
            assert!(source_bounds[1] >= name_bounds[1] + name_bounds[3]);
        }
    }
    assert_eq!(data.get_profile_name(), name);
    assert_eq!(data.get_profile_source(), source);
    assert_eq!(data.get_edit_name(), name);
    assert_eq!(data.get_edit_source(), source);
    window.hide().unwrap();
}

#[test]
fn profile_views_keep_local_actions_available_and_builtin_actions_disabled() {
    ui_testing::initialize(1600, 1000);
    let mut app = fixture(Page::Profiles);
    app.profiles[0].source = "/fixture/imported.yaml".into();
    let mut remote = app.profiles[0].clone();
    remote.id = "remote".into();
    remote.name = "Remote subscription".into();
    remote.source = "https://example.test/subscription".into();
    let mut builtin = app.profiles[0].clone();
    builtin.id = "builtin".into();
    builtin.name = "默认直连".into();
    builtin.source = "/fixture/default.yaml".into();
    builtin.builtin = true;
    app.profiles.extend([remote, builtin]);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let imported = data
        .get_profiles()
        .iter()
        .find(|profile| profile.id == "fixture")
        .unwrap();
    assert!(imported.local);
    assert!(!imported.builtin);
    let events = pointer_events(&window);
    window.show().unwrap();
    motion_frame(&window, 0);
    let first = target_bounds("profile:fixture");
    let second = target_bounds("profile:remote");
    assert!((first[1] - second[1]).abs() < 1.);
    assert!(second[0] >= first[0] + first[2]);
    for suffix in ["use", "update", "edit", "detail"] {
        for id in ["fixture", "remote", "builtin"] {
            let _ = target_bounds(&format!("profile-{suffix}:{id}"));
        }
    }
    for key in [
        "profile-use:fixture",
        "profile-update:fixture",
        "profile-update:builtin",
        "profile-edit:builtin",
    ] {
        click_target(&window, key);
    }
    assert!(events.borrow().is_empty());
    click_target(&window, "profile-edit:fixture");
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::EditProfile, "fixture".into(), "".into())]
    );
    events.borrow_mut().clear();
    click_target(&window, "profiles-view");
    motion_frame(&window, 0);
    assert!(
        window
            .global::<clash_client_ui::PageViews>()
            .get_profiles_list()
    );
    let first = target_bounds("profile:fixture");
    let second = target_bounds("profile:remote");
    assert!(
        first[3] < 60.,
        "profile list rows must stay compact: {first:?}"
    );
    assert!((first[0] - second[0]).abs() < 1.);
    assert!(second[1] >= first[1] + first[3]);
    click_target(&window, "profile-edit:fixture");
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::EditProfile, "fixture".into(), "".into())]
    );
    let _ = app.ui_intent(Intent::EditProfile, "fixture", "");
    assert_eq!(app.profile_editor.as_ref().unwrap().id, "fixture");
    assert_eq!(
        app.profile_editor.as_ref().unwrap().source,
        "/fixture/imported.yaml"
    );
    let _ = app.ui_intent(Intent::EditProfile, "builtin", "");
    assert_eq!(app.profile_editor.as_ref().unwrap().id, "fixture");
    let _ = app.ui_intent(Intent::EditName, "Renamed local file", "");
    app.busy = true;
    let _ = app.ui_intent(Intent::SaveProfile, "", "");
    assert!(app.profile_editor.as_ref().unwrap().error.is_empty());
    assert!(app.queued_actions.iter().any(|action| matches!(
        action,
        Action::EditProfile(id, name, source)
            if id == "fixture" && name == "Renamed local file" && source == "/fixture/imported.yaml"
    )));
    events.borrow_mut().clear();
    click_target(&window, "profile-detail:fixture");
    motion_frame(&window, 0);
    click_target(&window, "profile-delete:fixture");
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::DeleteProfile, "fixture".into(), "".into())]
    );
    events.borrow_mut().clear();
    track_targets(&window);
    motion_frame(&window, 0);
    click_target(&window, "profile-detail:builtin");
    motion_frame(&window, 0);
    let _ = target_bounds("profile-delete:builtin");
    click_target(&window, "profile-delete:builtin");
    click_target(&window, "profile-edit:builtin");
    assert!(events.borrow().is_empty());
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
    track_targets(&window);
    window
        .global::<UiActions>()
        .on_dispatch(move |intent, a, b| recorded.borrow_mut().push((intent, a, b)));
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    click_target(&window, "notification-icon");
    assert_eq!(
        events.borrow().last().unwrap().0,
        Intent::SystemNotification
    );
    events.borrow_mut().clear();
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Return.into(),
    });
    assert_eq!(
        events.borrow().last().unwrap().0,
        Intent::SystemNotification
    );
    events.borrow_mut().clear();
    data.set_notification_busy(true);
    click_target(&window, "notification-icon");
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Return.into(),
    });
    assert!(
        events.borrow().is_empty(),
        "only one native notification may be active"
    );
    data.set_notification_busy(false);
    click_target(&window, "nav:代理");
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
    click_target(&window, "notification-icon");
    click_target(&window, "nav:代理");
    assert!(
        events.borrow().is_empty(),
        "modal overlay must block background actions"
    );
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    assert_eq!(events.borrow().last().unwrap().0, Intent::Escape);
    data.set_modal(0);
    data.set_notice("检查更新失败：测试网络不可用".into());
    data.set_notice_error(true);
    data.set_notice_critical(false);
    let _ = window.window().take_snapshot().unwrap();
    events.borrow_mut().clear();
    click_target(&window, "nav:代理");
    assert_eq!(
        events.borrow().last().unwrap().0,
        Intent::Navigate,
        "a recoverable failure must not block navigation"
    );
    data.set_notice("内核无法启动：测试权限不足".into());
    data.set_notice_critical(true);
    let _ = window.window().take_snapshot().unwrap();
    events.borrow_mut().clear();
    click_target(&window, "nav:代理");
    click_target(&window, "notification-icon");
    assert!(
        events.borrow().is_empty(),
        "the centered error must block navigation behind it"
    );
    data.set_modal(2);
    data.set_edit_name("Unsaved subscription".into());
    data.set_edit_source("https://example.invalid/draft".into());
    let _ = window.window().take_snapshot().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: "x".into() });
    assert_eq!(data.get_edit_name(), "Unsaved subscription");
    assert_eq!(data.get_modal(), 2);
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    assert_eq!(
        events.borrow().last().unwrap().0,
        Intent::DismissNotice,
        "Escape must dismiss the error before closing the underlying editor"
    );
    assert_eq!(data.get_edit_source(), "https://example.invalid/draft");
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
        builtin: false,
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
thread_local! {
    static TARGETS: RefCell<std::collections::HashMap<String, [f32; 4]>> = RefCell::new(std::collections::HashMap::new());
}

fn track_targets(window: &AppWindow) {
    TARGETS.with(|targets| targets.borrow_mut().clear());
    window.global::<UiGeometry>().set_enabled(true);
    window.global::<UiGeometry>().on_measured(|key, bounds| {
        let bounds: Vec<f32> = bounds.iter().collect();
        if let [horizontal, vertical, width, height] = bounds.as_slice()
            && *width > 0.0
            && *height > 0.0
        {
            TARGETS.with(|targets| {
                targets
                    .borrow_mut()
                    .insert(key.to_string(), [*horizontal, *vertical, *width, *height]);
            });
        }
    });
    let geometry = window.global::<UiGeometry>();
    geometry.set_revision(geometry.get_revision().wrapping_add(1));
}

fn target_bounds(key: &str) -> [f32; 4] {
    TARGETS.with(|targets| {
        *targets
            .borrow()
            .get(key)
            .unwrap_or_else(|| panic!("missing rendered target: {key}"))
    })
}

fn click_target(window: &AppWindow, key: &str) {
    click_target_at(window, key, 0.5, 0.5);
}

fn click_target_at(window: &AppWindow, key: &str, horizontal: f32, vertical: f32) {
    let bounds = target_bounds(key);
    ui_testing::click(
        window.window(),
        bounds[0] + bounds[2] * horizontal,
        bounds[1] + bounds[3] * vertical,
    );
}

fn click_popup_target(window: &AppWindow, trigger: &str, key: &str) {
    let popup = target_bounds(&format!("popup:{trigger}"));
    let option = target_bounds(key);
    let scale = window.window().scale_factor();
    let size = window.window().size();
    let width = size.width as f32 / scale;
    let height = size.height as f32 / scale;
    let popup_x = popup[0].clamp(0.0, (width - popup[2]).max(0.0));
    let popup_y = popup[1].clamp(0.0, (height - popup[3]).max(0.0));
    ui_testing::click(
        window.window(),
        popup_x + option[0] + option[2] / 2.0,
        popup_y + option[1] + option[3] / 2.0,
    );
}

fn rapid_target(window: &AppWindow, key: &str) {
    let bounds = target_bounds(key);
    ui_testing::rapid_click(
        window.window(),
        bounds[0] + bounds[2] / 2.0,
        bounds[1] + bounds[3] / 2.0,
    );
}

fn pointer_events(window: &AppWindow) -> UiEventLog {
    track_targets(window);
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
fn custom_window_controls_remain_available_during_errors_and_preserve_close_policy() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let events = pointer_events(&window);
    let closed = std::rc::Rc::new(std::cell::Cell::new(0));
    let requests = closed.clone();
    window.window().on_close_requested(move || {
        requests.set(requests.get() + 1);
        slint::CloseRequestResponse::KeepWindowShown
    });
    window.window().set_maximized(false);
    window.window().set_minimized(false);
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    click_target(&window, "window-maximize");
    assert!(window.window().is_maximized());
    click_target(&window, "window-maximize");
    assert!(!window.window().is_maximized());
    ui_testing::advance(1000);
    click_target(&window, "window-drag");
    click_target(&window, "window-drag");
    assert!(window.window().is_maximized());
    window.window().set_maximized(false);
    data.set_modal(1);
    data.set_edit_name("unfinished subscription".into());
    data.set_notice("内核启动失败".into());
    data.set_notice_error(true);
    data.set_notice_critical(true);
    let _ = window.window().take_snapshot().unwrap();
    click_target(&window, "window-minimize");
    assert!(window.window().is_minimized());
    window.window().set_minimized(false);
    click_target(&window, "window-maximize");
    assert!(window.window().is_maximized());
    click_target(&window, "window-close");
    assert_eq!(closed.get(), 1);
    assert!(window.window().is_visible());
    assert_eq!(data.get_edit_name(), "unfinished subscription");
    assert_eq!(data.get_notice(), "内核启动失败");
    assert!(events.borrow().is_empty());
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Escape.into(),
        });
    assert_eq!(events.borrow().last().unwrap().0, Intent::DismissNotice);
    window.window().set_maximized(false);
    window.hide().unwrap();
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
        click_target(&window, "node-sort");
        let _ = window.window().take_snapshot().unwrap();
        click_popup_target(&window, "node-sort", &format!("option:node-sort/{index}"));
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
    click_target(&window, "settings-section:2");
    let _ = window.window().take_snapshot().unwrap();
    click_target(&window, "accent-inline:1");
    let chosen = events
        .borrow_mut()
        .pop()
        .expect("busy Geo must not disable inline appearance controls");
    assert_eq!(chosen.0, Intent::AccentColor);
    assert_eq!(chosen.1, "1");
    assert_eq!(data.get_modal(), 0);
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
    click_target(&window, "settings-section:2");
    let _ = window.window().take_snapshot().unwrap();
    let before = window.global::<Theme>().get_body_size();
    click_target(&window, "text-scale");
    let _ = window.window().take_snapshot().unwrap();
    click_popup_target(&window, "text-scale", "option:text-scale/4");
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
fn shortcut_dialog_edits_the_draft_and_keeps_disabled_choices_after_document_release() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Rules);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let events = pointer_events(&window);
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    click_target(&window, "rule-shortcuts-open");
    let _ = window.window().take_snapshot().unwrap();
    assert!(window.get_rule_shortcuts_open());
    click_target(&window, "rule-shortcut:0");
    let event = events.borrow_mut().pop().unwrap();
    assert_eq!(event, (Intent::RuleToggle, "0".into(), "true".into()));
    let _ = app.ui_intent(event.0, &event.1, &event.2);
    app.present_slint(&data, &mut models);
    assert!(app.rule_state.draft.enabled[0]);
    assert!(!app.rule_state.saved.enabled[0]);
    assert!(data.get_rule_dirty());
    let _ = window.window().take_snapshot().unwrap();
    click_target(&window, "rule-shortcuts-close");
    assert!(!window.get_rule_shortcuts_open());
    assert!(app.rule_state.dirty());
    assert!(events.borrow().is_empty());
    app.rule_state.release_document();
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_shortcuts().row_count(), 5);
    assert!(data.get_shortcut_available().iter().all(|value| !value));
    assert!(data.get_shortcut_values().iter().all(|value| !value));
    assert_eq!(data.get_rule_policies().row_count(), 0);
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
    let kind_index = data
        .get_rule_types()
        .iter()
        .position(|kind| kind == "DOMAIN-KEYWORD")
        .unwrap();
    click_target(&window, "rule-kind");
    let _ = window.window().take_snapshot().unwrap();
    click_popup_target(
        &window,
        "rule-kind",
        &format!("option:rule-kind/{kind_index}"),
    );
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
    click_target(&window, "rule-policy");
    let _ = window.window().take_snapshot().unwrap();
    click_popup_target(&window, "rule-policy", "option:rule-policy/1");
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
        (
            Intent::Language,
            "en",
            "",
            Action::SetLanguage(LanguagePreference::English),
        ),
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
fn desktop_language_menu_dispatches_stable_tags_in_both_catalogs() {
    ui_testing::initialize(800, 450);
    let window = AppWindow::new().unwrap();
    clash_desktop_ui::install_region_names(&window);
    let data = window.global::<UiData>();
    data.set_page(9);
    data.set_ready(true);
    data.set_protected(true);
    let mut models = ViewModels::default();
    models.attach(&data);
    track_targets(&window);
    let events = pointer_events(&window);
    window.show().unwrap();

    for (locale, title, network_title, status_label, option, expected) in [
        (
            "en",
            "Settings",
            "Website check",
            "Available",
            "option:language/1",
            "zh-CN",
        ),
        (
            "",
            "设置",
            "网站检查",
            "确认可用",
            "option:language/2",
            "en",
        ),
    ] {
        slint::select_bundled_translation(locale).unwrap();
        let original = if locale == "en" {
            LanguagePreference::English
        } else {
            LanguagePreference::SimplifiedChinese
        };
        data.set_language(original.tag().into());
        data.set_ready(true);
        data.set_protected(true);
        assert_eq!(window.get_page_title(), title);
        assert_eq!(
            window
                .global::<NetworkTools>()
                .get_titles()
                .row_data(0)
                .unwrap(),
            network_title
        );
        assert_eq!(
            window
                .global::<IpStatusText>()
                .invoke_label("确认可用".into()),
            status_label
        );
        assert_eq!(
            window.global::<IpStatusText>().invoke_label("全球".into()),
            if locale == "en" { "Global" } else { "全球" }
        );
        let _ = window.window().take_snapshot().unwrap();
        click_target(&window, "language");
        let _ = window.window().take_snapshot().unwrap();
        click_popup_target(&window, "language", option);
        assert_eq!(
            events.borrow_mut().pop(),
            Some((Intent::Language, expected.into(), "".into()))
        );
        let mut app = fixture(Page::Settings);
        app.settings.language = original;
        app.presentation_language = original.resolve_desktop();
        app.profile_name = "Unsaved draft".into();
        let mut reply = synchronized_reply(&app);
        reply.settings.language = LanguagePreference::from_tag(expected).unwrap();
        let _ = app.update(Message::Finished(Box::new(reply)));
        assert_eq!(app.profile_name, "Unsaved draft");
        assert_eq!(app.page, Page::Settings);
        assert!(app.snapshot.running);
        let projection = clash_desktop_protocol::UiData::default();
        let mut projected = clash_desktop_protocol::models::ViewModels::default();
        projected.attach(&projection);
        app.present(&projection, &mut projected);
        let frame = projection.snapshot();
        assert!(!clash_desktop_ui::apply_if_current(&frame, &data, &mut models, 1, 2).unwrap());
        assert_eq!(window.get_page_title(), title);
        assert!(clash_desktop_ui::apply_if_current(&frame, &data, &mut models, 2, 2).unwrap());
        assert_eq!(
            window.get_page_title(),
            if expected == "en" {
                "Settings"
            } else {
                "设置"
            }
        );
        assert_eq!(
            window.global::<IpStatusText>().invoke_label("德国".into()),
            if expected == "en" {
                "Germany"
            } else {
                "德国"
            }
        );
        if let Some(prefix) = std::env::var_os("COR_LIVE_LANGUAGE_PREVIEW") {
            data.set_ready(true);
            let image = window.window().take_snapshot().unwrap();
            let size = window.window().size();
            let mut bytes = format!("P6\n{} {}\n255\n", size.width, size.height).into_bytes();
            for pixel in image.as_bytes().as_chunks::<4>().0 {
                bytes.extend_from_slice(&pixel[..3]);
            }
            std::fs::write(
                format!("{}-{expected}.ppm", prefix.to_string_lossy()),
                bytes,
            )
            .unwrap();
        }
    }
    let mut app = fixture(Page::Settings);
    app.settings.language = LanguagePreference::English;
    app.presentation_language = Language::English;
    app.profile_name = "Unsaved draft".into();
    let mut failed = synchronized_reply(&app);
    failed.notice = Err("Settings save failed".into());
    let _ = app.update(Message::Finished(Box::new(failed)));
    assert_eq!(app.presentation_language, Language::English);
    assert_eq!(app.settings.language, LanguagePreference::English);
    assert_eq!(app.profile_name, "Unsaved draft");
    let mut reply = synchronized_reply(&app);
    reply.settings.language = LanguagePreference::System;
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert_eq!(
        app.presentation_language,
        LanguagePreference::System.resolve_desktop()
    );
    assert_eq!(app.profile_name, "Unsaved draft");
    data.set_language("en".into());
    slint::select_bundled_translation("en").unwrap();
    let projection = clash_desktop_protocol::UiData::default();
    let mut projected = clash_desktop_protocol::models::ViewModels::default();
    projected.attach(&projection);
    app.present(&projection, &mut projected);
    assert!(
        clash_desktop_ui::apply_if_current(&projection.snapshot(), &data, &mut models, 3, 3)
            .unwrap()
    );
    assert_eq!(
        window.get_page_title(),
        app.presentation_language.text("设置", "Settings")
    );
    window.hide().unwrap();
}

#[test]
fn english_sample_actions_fit_and_dispatch_in_a_scaled_dialog() {
    ui_testing::initialize(800, 450);
    let window = AppWindow::new().unwrap();
    slint::select_bundled_translation("en").unwrap();
    let data = window.global::<UiData>();
    data.set_text_scale(130);
    data.set_modal(8);
    data.set_browser_session(true);
    window
        .global::<clash_client_ui::Motion>()
        .set_enabled(false);
    track_targets(&window);
    let events = pointer_events(&window);
    window.show().unwrap();
    motion_frame(&window, 250);
    for (key, intent) in [
        ("button:Copy script", Intent::ClaudeCopyScript),
        ("button:Import clipboard", Intent::ClaudeImport),
        ("button:Close callback", Intent::ClaudeCloseSession),
        ("button:Open sample page", Intent::ClaudeBrowser),
    ] {
        let bounds = target_bounds(key);
        assert!(bounds[0] >= 0. && bounds[0] + bounds[2] <= 800.);
        assert!(bounds[1] >= 0. && bounds[1] + bounds[3] <= 450.);
        click_target(&window, key);
        assert_eq!(events.borrow_mut().pop().unwrap().0, intent);
    }
    window.hide().unwrap();
    slint::select_bundled_translation("").unwrap();
}

#[test]
fn ip_report_uses_emoji_in_summary_rows_and_details() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Websites);
    app.site_results.push(Some(Ok(ip_check::CheckResult {
        state: ip_check::State::Identified,
        summary: "203.0.113.2".into(),
        country: "🇩🇪 德国".into(),
        millis: Some(4),
        authentication_check_skipped: false,
        detail: String::new(),
        target_exit: None,
    })));
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    assert!(data.get_site_exit_flagged());
    assert_eq!(data.get_site_exit_country(), "德国");
    assert_eq!(data.get_site_exit_flag(), "🇩🇪");
    assert!(!data.get_site_summary().contains('🇩'));
    let _ = app.ui_intent(Intent::SiteDetail, "exit-ip", "");
    app.present_slint(&data, &mut models);
    assert!(data.get_modal_flagged());
    assert_eq!(data.get_modal_flag(), "🇩🇪");
    assert_eq!(data.get_modal_country(), "德国");
    assert!(!data.get_modal_description().contains('🇩'));
}

#[test]
fn ip_report_shows_each_target_exit_and_its_measurement_state() {
    let preview = std::env::var_os("COR_IP_DESKTOP_PREVIEW");
    ui_testing::initialize(1100, if preview.is_some() { 1800 } else { 900 });
    let mut app = fixture(Page::Websites);
    app.site_has_run = true;
    app.site_show_results = true;
    app.site_category = CategoryFilter::Group("AI");
    app.site_results
        .resize_with(ip_check::services().len(), || None);
    app.site_results[0] = Some(Ok(ip_check::CheckResult {
        state: ip_check::State::Identified,
        summary: "203.0.113.42".into(),
        country: "香港".into(),
        millis: Some(4),
        authentication_check_skipped: false,
        detail: String::new(),
        target_exit: None,
    }));
    let index = |id| {
        ip_check::services()
            .iter()
            .position(|service| service.id == id)
            .unwrap()
    };
    let claude = ip_check::CheckResult {
        state: ip_check::State::Reachable,
        summary: "网页可达".into(),
        country: String::new(),
        millis: Some(12),
        authentication_check_skipped: false,
        detail: String::new(),
        target_exit: Some(ip_check::TargetExit {
            host: "claude.ai".into(),
            ip: "203.0.113.12".parse().unwrap(),
            country: Some("香港".into()),
        }),
    };
    let chatgpt = ip_check::CheckResult {
        state: ip_check::State::Confirmed,
        summary: "确认可用".into(),
        country: "美国".into(),
        millis: Some(218),
        authentication_check_skipped: false,
        detail: String::new(),
        target_exit: Some(ip_check::TargetExit {
            host: "chatgpt.com".into(),
            ip: "203.0.113.13".parse().unwrap(),
            country: Some("美国".into()),
        }),
    };
    app.site_results[index("AIUnlockTest_Claude")] = Some(Ok(claude.clone()));
    app.site_results[index("MediaUnlockTest_ChatGPT")] = Some(Ok(chatgpt.clone()));
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let summary = data.get_site_summary();
    assert!(summary.contains("Cloudflare 探针出口：203.0.113.42"));
    let rows = data.get_site_results().iter().collect::<Vec<_>>();
    let row = |id: &str| rows.iter().find(|row| row.id == id).unwrap();
    assert_eq!(row("AIUnlockTest_Claude").value, "仅网页可达");
    assert_eq!(row("MediaUnlockTest_ChatGPT").value, "确认可用");
    assert_eq!(
        row("AIUnlockTest_Claude").detail,
        "同域出口：203.0.113.12 · 香港"
    );
    assert_eq!(
        row("MediaUnlockTest_ChatGPT").detail,
        "同域出口：203.0.113.13 · 美国"
    );
    assert_eq!(
        row("AIUnlockTest_Gemini_location").detail,
        "同域出口：待检测"
    );
    app.presentation_language = Language::English;
    app.present_slint(&data, &mut models);
    assert!(data.get_site_summary().contains("Cloudflare probe exit:"));
    let available = data.get_site_counts().row_data(0).unwrap();
    assert_eq!(available.id, "确认可用");
    assert!(available.label.starts_with("Available "));
    let english_rows = data.get_site_results().iter().collect::<Vec<_>>();
    let english_claude = english_rows
        .iter()
        .find(|row| row.id == "AIUnlockTest_Claude")
        .unwrap();
    assert_eq!(english_claude.status_label, "仅网页可达");
    assert_eq!(english_claude.value, "Website reachable");
    assert_eq!(
        english_claude.detail,
        "Same-domain exit: 203.0.113.12 · Hong Kong SAR China"
    );
    app.presentation_language = Language::SimplifiedChinese;
    app.site_results[index("AIUnlockTest_Claude")] = None;
    app.site_results[index("MediaUnlockTest_ChatGPT")] = Some(Err("请求失败".into()));
    app.present_slint(&data, &mut models);
    let rows = data.get_site_results().iter().collect::<Vec<_>>();
    let row = |id: &str| rows.iter().find(|row| row.id == id).unwrap();
    assert_eq!(row("AIUnlockTest_Claude").detail, "同域出口：待检测");
    assert_eq!(row("MediaUnlockTest_ChatGPT").detail, "同域出口：未确认");
    assert_eq!(row("AIUnlockTest_Claude").status_label, "未检测");
    assert_eq!(row("MediaUnlockTest_ChatGPT").status_label, "超时或失败");
    let netflix = index("MediaUnlockTest_Netflix");
    app.site_results[netflix] = Some(Ok(ip_check::CheckResult {
        state: ip_check::State::Confirmed,
        summary: "确认可用".into(),
        country: "美国".into(),
        millis: Some(150),
        authentication_check_skipped: false,
        detail: String::new(),
        target_exit: Some(ip_check::TargetExit {
            host: "www.netflix.com".into(),
            ip: "203.0.113.14".parse().unwrap(),
            country: Some("美国".into()),
        }),
    }));
    app.site_category = CategoryFilter::Group("全球");
    app.present_slint(&data, &mut models);
    let rows = data.get_site_results().iter().collect::<Vec<_>>();
    assert_eq!(
        rows.iter()
            .find(|row| row.id == "MediaUnlockTest_Netflix")
            .unwrap()
            .detail,
        "同域出口：203.0.113.14 · 美国"
    );
    app.site_category = CategoryFilter::Group("AI");
    if let Some(path) = preview {
        app.site_results[index("AIUnlockTest_Claude")] = Some(Ok(claude));
        app.site_results[index("MediaUnlockTest_ChatGPT")] = Some(Ok(chatgpt));
        app.present_slint(&data, &mut models);
        window.show().unwrap();
        motion_frame(&window, 250);
        let image = window.window().take_snapshot().unwrap();
        let size = window.window().size();
        let mut bytes = format!("P6\n{} {}\n255\n", size.width, size.height).into_bytes();
        for pixel in image.as_bytes().as_chunks::<4>().0 {
            bytes.extend_from_slice(&pixel[..3]);
        }
        std::fs::write(path, bytes).unwrap();
        window.hide().unwrap();
    }
}

#[test]
fn ip_report_localizes_details_and_preserves_raw_evidence_in_both_languages() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Websites);
    app.site_show_results = true;
    let (index, service) = ip_check::services()
        .iter()
        .enumerate()
        .find(|(_, service)| service.id == "MediaUnlockTest_DisneyPlus")
        .unwrap();
    app.site_category = CategoryFilter::Group(&service.group);
    app.site_results.resize_with(index + 1, || None);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);

    let evidence = "Originals Only (上游原文)";
    let detail = format!("原生地区检测结果：{evidence}");
    app.site_results[index] = Some(Ok(ip_check::CheckResult {
        state: ip_check::State::Partial,
        summary: "仅自制内容".into(),
        country: "🇯🇵 日本".into(),
        millis: Some(0),
        authentication_check_skipped: false,
        detail,
        target_exit: None,
    }));
    let _ = app.ui_intent(Intent::SiteDetail, &service.id, "");

    for (language, country, summary, row_value, prefix) in [
        (
            Language::SimplifiedChinese,
            "日本",
            "仅自制内容",
            "部分可用 · 仅自制内容",
            "原生地区检测结果：",
        ),
        (
            Language::English,
            "Japan",
            "Originals only",
            "Partially available · Originals only",
            "Native region check result: ",
        ),
    ] {
        app.presentation_language = language;
        app.present_slint(&data, &mut models);
        let row = data
            .get_site_results()
            .iter()
            .find(|row| row.id == service.id.as_str())
            .unwrap();
        let description = format!("{summary}\n0 ms\n\n{prefix}{evidence}");
        assert_eq!(data.get_modal(), 11);
        assert_eq!(row.value, row_value);
        assert_eq!(data.get_modal_country(), country);
        assert_eq!(data.get_modal_flag(), "🇯🇵");
        assert_eq!(data.get_modal_description(), description);
        assert_eq!(data.get_modal_copy(), format!("{country}\n{description}"));
    }

    let Ok(result) = app.site_results[index].as_mut().unwrap() else {
        unreachable!();
    };
    result.state = ip_check::State::Unknown;
    result.summary = "平台自定义摘要".into();
    result.detail = "原始说明：提到原生地区检测结果，但不是固定前缀".into();
    for language in [Language::SimplifiedChinese, Language::English] {
        app.presentation_language = language;
        app.present_slint(&data, &mut models);
        assert_eq!(
            data.get_modal_description(),
            "平台自定义摘要\n0 ms\n\n原始说明：提到原生地区检测结果，但不是固定前缀"
        );
    }
}

#[test]
fn ip_report_projects_skipped_authentication_and_real_zero_in_both_languages() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Websites);
    app.site_show_results = true;
    let service = ip_check::services()
        .iter()
        .find(|service| service.id == "MediaUnlockTest_DisneyPlus")
        .unwrap();
    app.site_category = CategoryFilter::Group(&service.group);
    app.site_status = StatusFilter::Status(Status::Unknown);
    let index = ip_check::services()
        .iter()
        .position(|entry| entry.id == service.id)
        .unwrap();
    app.site_results
        .resize_with(ip_check::services().len(), || None);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let _ = app.ui_intent(Intent::SiteDetail, &service.id, "");
    for (language, status, duration, reason, exit) in [
        (
            Language::SimplifiedChinese,
            "结果未确认",
            "无响应测量",
            "未执行认证检测",
            "同域出口：",
        ),
        (
            Language::English,
            "Unconfirmed",
            "No response measurement",
            "Authentication check was not run",
            "Same-domain exit: ",
        ),
    ] {
        app.presentation_language = language;
        for skipped in [true, false] {
            app.site_results[index] = Some(Ok(ip_check::CheckResult {
                state: ip_check::State::Unknown,
                summary: "fixture".into(),
                country: "未提供".into(),
                millis: Some(0),
                authentication_check_skipped: skipped,
                detail: "fixture evidence".into(),
                target_exit: None,
            }));
            app.present_slint(&data, &mut models);
            let row = data
                .get_site_results()
                .iter()
                .find(|row| row.id == service.id.as_str())
                .unwrap();
            let detail = data.get_modal_description();
            let category = data
                .get_categories()
                .iter()
                .flat_map(|pair| [pair.left, pair.right])
                .find(|category| category.id == service.group.as_str())
                .unwrap();
            let category_total = ip_check::services()
                .iter()
                .filter(|item| item.group == service.group)
                .count();
            assert_eq!(category.count, format!("{category_total}"));
            let pending = category_total - 1;
            assert_eq!(
                category.detail,
                match language {
                    Language::SimplifiedChinese => format!("结果未确认 1 · 未检测 {pending}"),
                    Language::English => format!("Unconfirmed 1 · Not checked {pending}"),
                }
            );
            let country = if skipped && language == Language::English {
                "Not provided"
            } else {
                "未提供"
            };
            assert_eq!(row.source, country);
            assert_eq!(data.get_modal_country(), country);
            assert!(row.detail.contains(exit));
            if skipped {
                assert_eq!(row.value, status);
                assert_eq!(detail.lines().next(), Some(status));
                assert_eq!(data.get_modal_copy(), format!("{country}\n{detail}"));
                assert!(row.delay.is_empty());
                assert_eq!(row.delay_tone, 4);
                assert!(row.detail.contains(duration));
                assert!(detail.contains(duration) && detail.contains(reason));
                assert!(!detail.contains("0 ms"));
            } else {
                assert_eq!(row.delay, "0 ms");
                assert_eq!(row.delay_tone, 1);
                assert!(detail.contains("0 ms") && detail.contains("fixture evidence"));
                assert!(!row.detail.contains(duration));
            }
        }
    }
}

#[test]
fn ip_report_uses_the_china_flag_for_hong_kong_macao_and_taiwan() {
    ui_testing::initialize(950, 700);
    for (country, expected) in [
        ("🇭🇰 香港", "香港"),
        ("🇲🇴 澳门", "澳门"),
        ("🇹🇼 台湾", "台湾"),
    ] {
        let mut app = fixture(Page::Websites);
        app.site_results.push(Some(Ok(ip_check::CheckResult {
            state: ip_check::State::Identified,
            summary: "203.0.113.2".into(),
            country: country.into(),
            millis: Some(4),
            authentication_check_skipped: false,
            detail: String::new(),
            target_exit: None,
        })));
        let window = AppWindow::new().unwrap();
        let data = window.global::<UiData>();
        let mut models = ViewModels::default();
        models.attach(&data);
        app.present_slint(&data, &mut models);
        assert!(data.get_site_exit_flagged(), "{country}");
        assert_eq!(data.get_site_exit_country(), expected, "{country}");
        assert_eq!(data.get_site_exit_flag(), "🇨🇳", "{country}");
        let _ = app.ui_intent(Intent::SiteDetail, "exit-ip", "");
        app.present_slint(&data, &mut models);
        assert!(data.get_modal_flagged(), "{country}");
        assert_eq!(data.get_modal_country(), expected, "{country}");
        assert_eq!(data.get_modal_flag(), "🇨🇳", "{country}");
    }
}

#[test]
fn node_names_reach_all_views_without_rewriting_prefixes_or_emoji() {
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Proxies);
    let group_name = "🚀 策略组 ❤️";
    app.expanded.insert(group_name.into());
    let nodes = [
        "🇺🇸 美国 01",
        "日本 01",
        "🇩🇪 德国 01",
        "🇭🇰 香港 01",
        "澳门 01",
        "TW-台湾 01",
        "DIRECT",
        "🚀 🇯🇵 日本 02",
        "  😀 US-美国 02 ❤️",
        "👩🏽‍💻 工作线路",
        "🇽🇽 原始名称",
        "🇩🇪",
    ];
    app.snapshot.proxies = serde_json::from_value(serde_json::json!({"proxies": {
        "🚀 策略组 ❤️": {"type":"Selector", "now":nodes[0], "all":nodes}
    }}))
    .unwrap();
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    for name in nodes {
        app.snapshot.active_node = Some(name.into());
        app.snapshot
            .proxies
            .proxies
            .get_mut(group_name)
            .unwrap()
            .now = name.into();
        app.present_slint(&data, &mut models);
        assert_eq!(data.get_node(), name);
        let group = data.get_groups().row_data(0).unwrap();
        assert_eq!(group.id, group_name);
        assert_eq!(group.title, group_name);
        assert_eq!(group.current, name);
        let node = group
            .nodes
            .iter()
            .flat_map(|pair| [pair.left, pair.right])
            .find(|node| node.name == name)
            .unwrap();
        assert_eq!(node.group, group_name);
        assert_eq!(node.name, name);
        assert!(node.selected, "{name}");
        assert_eq!(app.snapshot.active_node.as_deref(), Some(name));
    }
}

#[test]
fn claude_report_rows_keep_titles_and_labeled_explanations() {
    use clash_of_rust::claude_check::{Environment, Group, Report, Route, Signal};

    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Claude);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let mut report = Report::new(Environment::Core, "fixture");
    let mut signal = Signal::unknown(Group::Dns, "temporary", "DNS fixture", "未取得结果");
    signal.title = " ".into();
    signal.value.clear();
    signal.advice = "稍后重新检测".into();
    report.signals.push(signal);
    report.routes.push(Route {
        chain: "节点 A".into(),
        rule: "DOMAIN".into(),
        time: "2026-11-12T08:00:00Z".into(),
        outcome: " ".into(),
        ..Default::default()
    });
    report.routes.push(Route {
        host: "direct.example".into(),
        chain: "DIRECT".into(),
        outcome: "内核已选路".into(),
        ..Default::default()
    });
    app.claude.reports[0] = Some(report);
    app.present_slint(&data, &mut models);
    let rows = data.get_claude_results();
    assert_eq!(rows.iter().filter(|row| row.tone == 0).count(), 5);
    let route = rows.iter().find(|row| row.title == "连接检查").unwrap();
    assert_eq!(route.title, "连接检查");
    assert_eq!(route.value, "仅记录选路");
    assert!(route.detail.contains("线路：节点 A"));
    assert!(route.detail.contains("时间：2026-11-12T08:00:00Z"));
    assert_eq!(route.source, "内核连接记录");
    let direct = rows
        .iter()
        .find(|row| row.title == "direct.example")
        .unwrap();
    assert_eq!(direct.value, "异常线路 · 内核已选路");
    assert_eq!(direct.tone, 3);
    let signal = rows
        .iter()
        .find(|row| row.title == "DNS 地区" && row.tone != 0)
        .unwrap();
    assert_eq!(signal.title, "DNS 地区");
    assert_eq!(signal.value, "未确认");
    assert_eq!(signal.detail, "说明：未取得结果\n建议：稍后重新检测");
}

#[test]
fn english_claude_report_keeps_original_keys_and_evidence() {
    use clash_of_rust::claude_check::{Environment, Evidence, Group, Report, Route, Signal};

    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Claude);
    app.presentation_language = Language::English;
    let mut report = Report::new(Environment::Core, "fixture");
    report.signals.push(Signal::new(
        Group::Quality,
        "出口 IP 质量 · 203.0.113.12",
        "机房或托管网络：是 · 2 项待确认",
        "IPQuery、IPLogs",
        Evidence::Inferred,
        "已标记\n机房或托管网络：IPQuery 是、IPLogs 否",
    ));
    report.routes = ["api.anthropic.com", "claude.ai"]
        .into_iter()
        .map(|host| Route {
            host: host.into(),
            chain: "Node A".into(),
            rule: "DOMAIN".into(),
            ..Default::default()
        })
        .collect();
    let original = serde_json::to_value(&report).unwrap();
    app.claude.reports[0] = Some(report);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let rows = data.get_claude_results();
    let quality = rows
        .iter()
        .find(|row| row.title == "Exit IP quality · 203.0.113.12")
        .unwrap();
    assert_eq!(quality.id, "出口与 IP 属性");
    assert_eq!(quality.value, "Datacenter or hosting: Yes · 2 unconfirmed");
    assert!(quality.detail.contains("IPQuery: Yes, IPLogs: No"));
    let routes = rows
        .iter()
        .find(|row| row.title == "Claude domain routes")
        .unwrap();
    assert_eq!(routes.id, "Claude 连接与选路");
    assert_eq!(routes.value, "2 domains · Route recorded");
    assert!(routes.detail.contains("api.anthropic.com"));
    assert!(routes.detail.contains("claude.ai"));
    assert_eq!(
        serde_json::to_value(app.claude.reports[0].as_ref().unwrap()).unwrap(),
        original
    );
    app.settings.language = LanguagePreference::SimplifiedChinese;
    app.present_slint(&data, &mut models);
    assert!(
        data.get_claude_results()
            .iter()
            .any(|row| row.title == "Claude domain routes")
    );
}

#[test]
fn claude_route_section_keeps_each_domain_and_failure() {
    use clash_of_rust::claude_check::{Environment, Evidence, Group, Report, Route, Signal};

    let preview = std::env::var_os("COR_CLAUDE_REPORT_PREVIEW");
    ui_testing::initialize(1100, if preview.is_some() { 1200 } else { 1000 });
    let mut app = fixture(Page::Claude);
    let mut report = Report::new(Environment::Core, "fixture");
    report.routes = ["api.anthropic.com", "claude.ai", "claude.com"]
        .into_iter()
        .map(|host| Route {
            host: host.into(),
            rule: "DOMAIN".into(),
            chain: "演示节点".into(),
            time: "08:00".into(),
            ..Default::default()
        })
        .collect();
    report.routes.push(Route {
        host: "failed.example".into(),
        outcome: "连接失败".into(),
        ..Default::default()
    });
    report.signals.push(Signal::new(
        Group::Quality,
        "出口 IP 质量 · 203.0.113.12",
        "机房或托管网络来源不一致 · 2 项待确认 · 1 个来源查询未完成",
        "IPQuery、IPLogs、Layer3 Intel",
        Evidence::Inferred,
        "来源不一致\n机房或托管网络：IPQuery 是、IPLogs 否\n\n已有结果未标记\nVPN 地址：IPQuery 否、IPLogs 否\n\n尚未确认\n爬虫地址：未提供\n\n查询未完成\nLayer3 Intel：请求超时",
    ));
    for (group, title, value) in [
        (Group::Exit, "Claude 出口", "美国 · 203.0.113.12"),
        (Group::Dns, "递归 DNS", "美国"),
        (Group::Connectivity, "Claude 连通性", "连接成功"),
    ] {
        report.signals.push(Signal::new(
            group,
            title,
            value,
            "演示数据",
            Evidence::Measured,
            "仅用于报告预览",
        ));
    }
    app.claude.reports[0] = Some(report);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_claude_normal(), "3");
    assert_eq!(data.get_claude_risk(), "1");
    assert_eq!(data.get_claude_clues(), "4");
    let rows = data.get_claude_results();
    let header = rows
        .iter()
        .find(|row| row.tone == 0 && row.title == "Claude 连接与选路")
        .unwrap();
    assert_eq!(header.detail, "1 项需关注");
    assert!(header.value.contains("failed.example"));
    assert!(header.value.contains("3 个域名 · 仅记录选路"));
    let grouped = rows
        .iter()
        .find(|row| row.title == "Claude 域名选路" && row.tone == 2)
        .unwrap();
    assert_eq!(grouped.value, "3 个域名 · 仅记录选路");
    for expected in [
        "api.anthropic.com",
        "claude.ai",
        "claude.com",
        "规则：DOMAIN",
        "线路：演示节点",
        "时间：08:00",
    ] {
        assert!(grouped.detail.contains(expected), "{expected}");
    }
    assert!(
        rows.iter()
            .any(|row| row.title == "failed.example" && row.tone == 3)
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.title == "出口与 IP 属性" && row.tone == 0)
            .unwrap()
            .detail,
        "1 项来源不一致"
    );
    if preview.is_some() {
        let report = app.claude.reports[0].as_mut().unwrap();
        for signal in &mut report.signals {
            match signal.group {
                Group::Quality => {
                    signal.value = "机房或托管网络：是 · 2 项待确认 · 1 个来源查询未完成".into();
                    signal.detail = "已标记\n机房或托管网络：IPQuery 是、IPLogs 否\n\n尚未确认\n爬虫地址：未提供\n\n查询未完成\nLayer3 Intel：请求超时".into();
                }
                Group::Exit | Group::Dns => {
                    signal.title = if signal.group == Group::Exit {
                        "Claude 网页实际出口"
                    } else {
                        "实际递归 DNS"
                    }
                    .into();
                    signal.value = "203.0.113.12 · 美国".into();
                    signal.ip = "203.0.113.12".into();
                    signal.country = "US".into();
                }
                Group::Connectivity => {
                    signal.title = "claude.ai 连通性".into();
                    signal.value = "响应 3/3 · 中位 42 ms · HTTP 200".into();
                }
                _ => {}
            }
        }
        let mut guard = Report::new(Environment::Desktop, "演示数据");
        let route = Route {
            host: "api.anthropic.com".into(),
            chain: "DIRECT".into(),
            outcome: "异常连接已关闭".into(),
            ..Default::default()
        };
        let snapshot = clash_of_rust::claude_check::guard::Snapshot {
            total: 8,
            blocked: 1,
            connected: true,
            routes: [route.clone()].into(),
            ..Default::default()
        };
        guard
            .signals
            .push(clash_of_rust::claude_check::guard::report_signal(&snapshot));
        guard.routes.push(route);
        app.claude.reports[2] = Some(guard);
        app.claude.reports[0]
            .as_mut()
            .unwrap()
            .signals
            .push(Signal::new(
                Group::Browser,
                "语言偏好",
                "zh-CN、en-US",
                "演示数据",
                Evidence::Measured,
                "按浏览器返回的优先顺序排列。",
            ));
        app.present_slint(&data, &mut models);
    }
    let events = pointer_events(&window);
    window.show().unwrap();
    motion_frame(&window, 250);
    let capture = |stage: &str| {
        if let Some(prefix) = &preview {
            let image = window.window().take_snapshot().unwrap();
            let size = window.window().size();
            let mut bytes = format!("P6\n{} {}\n255\n", size.width, size.height).into_bytes();
            for pixel in image.as_bytes().as_chunks::<4>().0 {
                bytes.extend_from_slice(&pixel[..3]);
            }
            std::fs::write(format!("{}-{stage}.ppm", prefix.to_string_lossy()), bytes).unwrap();
        }
    };
    capture("overview");
    if preview.is_some() {
        app.presentation_language = Language::English;
        slint::select_bundled_translation("en").unwrap();
        app.present_slint(&data, &mut models);
        motion_frame(&window, 50);
        capture("english");
        app.presentation_language = Language::SimplifiedChinese;
        slint::select_bundled_translation("").unwrap();
        app.present_slint(&data, &mut models);
        window
            .window()
            .set_size(slint::PhysicalSize::new(1100, 2200));
        motion_frame(&window, 50);
    }
    click_target(&window, "claude-block:Claude 连接与选路");
    motion_frame(&window, 50);
    assert_eq!(
        events.borrow().last(),
        Some(&(Intent::ClaudeFilter, "Claude 连接与选路".into(), "".into()))
    );
    let _ = app.claude_event(claude_view::Event::ResultsFilter(Some(1)));
    app.present_slint(&data, &mut models);
    motion_frame(&window, 50);
    let routes = data.get_claude_results();
    let grouped_index = routes
        .iter()
        .position(|row| row.title == "Claude 域名选路")
        .unwrap();
    assert!(
        routes
            .iter()
            .any(|row| row.title == "failed.example" && row.tone == 3)
    );
    let grouped_key = format!("claude-result:{grouped_index}");
    let grouped_height = target_bounds(&grouped_key)[3];
    click_target(&window, &format!("claude-result-detail:{grouped_index}"));
    motion_frame(&window, 50);
    assert!(target_bounds(&grouped_key)[3] > grouped_height);
    capture("details");
    let _ = app.claude_event(claude_view::Event::ResultsFilter(Some(2)));
    app.present_slint(&data, &mut models);
    let exit = data.get_claude_results();
    assert!(
        exit.iter()
            .any(|row| row.detail.contains("IPQuery 是、IPLogs 否"))
    );
    assert!(!exit.iter().any(|row| row.title == "failed.example"));
    window.hide().unwrap();
}

#[test]
fn claude_guard_block_keeps_status_and_interceptions_together() {
    use clash_of_rust::claude_check::{Environment, Evidence, Report, Route, guard};

    ui_testing::initialize(1100, 1000);
    let mut app = fixture(Page::Claude);
    let mut report = Report::new(Environment::Core, "fixture");
    report.routes = ["api.anthropic.com", "claude.ai"]
        .into_iter()
        .map(|host| Route {
            host: host.into(),
            chain: "演示节点".into(),
            ..Default::default()
        })
        .collect();
    report.routes.push(Route {
        host: "observed.example".into(),
        chain: "DIRECT".into(),
        outcome: "内核已选路".into(),
        ..Default::default()
    });
    app.claude.reports[0] = Some(report);
    let mut snapshot = guard::Snapshot {
        total: 8,
        blocked: 1,
        alerts: 2,
        connected: true,
        error: "异常连接关闭失败".into(),
        routes: vec![
            Route {
                host: "closed.example".into(),
                chain: "DIRECT".into(),
                outcome: "异常连接已关闭".into(),
                ..Default::default()
            },
            Route {
                host: "failed.example".into(),
                chain: "DIRECT".into(),
                outcome: "异常连接关闭失败".into(),
                ..Default::default()
            },
        ]
        .into(),
        ..Default::default()
    };
    app.claude
        .guard_report(&snapshot, Language::SimplifiedChinese);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let rows = data.get_claude_results();
    let guard_header = rows
        .iter()
        .find(|row| row.tone == 0 && row.title == guard::REPORT_SECTION)
        .unwrap();
    assert_eq!(guard_header.block_items, 3);
    assert!(guard_header.value.contains("异常连接关闭失败"));
    assert!(guard_header.value.contains("已关闭 1 条异常连接"));
    for (host, outcome) in [
        ("closed.example", "异常连接已关闭"),
        ("failed.example", "异常连接关闭失败"),
    ] {
        let row = rows.iter().find(|row| row.title == host).unwrap();
        assert_eq!(row.id, guard::REPORT_SECTION);
        assert_eq!(row.value, outcome);
        assert_eq!(row.source, "实时拦截 · 内核连接记录");
        assert_eq!(row.tone, 3);
    }
    let observed = rows
        .iter()
        .find(|row| row.title == "observed.example")
        .unwrap();
    assert_eq!(observed.id, "Claude 连接与选路");
    track_targets(&window);
    window.show().unwrap();
    motion_frame(&window, 250);
    let _ = app.claude_event(claude_view::Event::ResultsFilter(Some(0)));
    app.present_slint(&data, &mut models);
    motion_frame(&window, 50);
    let signal_index = data
        .get_claude_results()
        .iter()
        .position(|row| row.title == guard::REPORT_SECTION)
        .unwrap();
    let item_key = format!("claude-result:{signal_index}");
    let collapsed_height = target_bounds(&item_key)[3];
    click_target(&window, &format!("claude-result-detail:{signal_index}"));
    motion_frame(&window, 50);
    assert!(target_bounds(&item_key)[3] > collapsed_height);
    snapshot.total += 1;
    app.claude
        .guard_report(&snapshot, Language::SimplifiedChinese);
    app.present_slint(&data, &mut models);
    motion_frame(&window, 50);
    assert!(
        target_bounds(&item_key)[3] > collapsed_height,
        "refreshing live protection counts closed the expanded explanation"
    );
    app.claude.clear_guard();
    let _ = app.claude_event(claude_view::Event::ResultsFilter(None));
    app.present_slint(&data, &mut models);
    let rows = data.get_claude_results();
    let guard_header = rows
        .iter()
        .find(|row| row.tone == 0 && row.title == guard::REPORT_SECTION)
        .unwrap();
    assert!(guard_header.value.contains("上次保护记录"));
    assert!(rows.iter().any(|row| row.title == "closed.example"));
    let signal = app.claude.reports[2]
        .as_ref()
        .unwrap()
        .signals
        .iter()
        .find(|signal| signal.title == guard::REPORT_SECTION)
        .unwrap();
    assert_eq!(signal.evidence, Evidence::Unknown);
    window.hide().unwrap();
}

#[test]
fn claude_summary_keeps_quality_disagreements_visible_with_many_risks() {
    use clash_of_rust::claude_check::{Environment, Evidence, Group, Report, Signal};

    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Claude);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let mut report = Report::new(Environment::Core, "fixture");
    report.signals.extend((1..=7).map(|index| {
        let mut signal = Signal::new(
            Group::Dns,
            &format!("DNS 风险 {index}"),
            "解析器地区不同",
            "DNS fixture",
            Evidence::Measured,
            "",
        );
        signal.risk = true;
        signal
    }));
    report.signals.push(Signal::new(
        Group::Quality,
        "出口 IP 质量",
        "机房或托管网络来源不一致",
        "IPQuery、IPLogs",
        Evidence::Inferred,
        "IPQuery 是、IPLogs 否",
    ));
    app.claude.reports[0] = Some(report);
    app.present_slint(&data, &mut models);

    let rows = data.get_claude_results();
    assert_eq!(rows.iter().filter(|row| row.tone == 0).count(), 5);
    assert_eq!(data.get_claude_risk(), "7");
    assert_eq!(data.get_claude_clues(), "1");
    let disagreement = rows
        .iter()
        .find(|row| row.tone == 2 && row.value.contains("来源不一致"))
        .expect("the overview includes the source disagreement");
    assert_eq!(disagreement.tone, 2);
    assert_eq!(disagreement.source, "IPQuery、IPLogs");
    assert!(disagreement.detail.contains("IPQuery 是、IPLogs 否"));

    let report = app.claude.reports[0].as_mut().unwrap();
    report.signals.extend((2..=7).map(|index| {
        Signal::new(
            Group::Quality,
            &format!("出口 IP 质量 {index}"),
            "机房或托管网络来源不一致",
            "IPQuery、IPLogs",
            Evidence::Inferred,
            "",
        )
    }));
    app.present_slint(&data, &mut models);
    let rows = data.get_claude_results();
    assert_eq!(rows.iter().filter(|row| row.tone == 0).count(), 5);
    assert!(
        rows.iter().any(|row| row.tone == 0 && row.delay_tone == 3),
        "the overview also includes risks when many sources disagree"
    );
    assert!(
        rows.iter()
            .any(|row| row.tone == 0 && row.value.contains("来源不一致"))
    );
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
    for (code, label, emoji) in [
        ("DE", "德国", "🇩🇪"),
        ("US", "美国", "🇺🇸"),
        ("JP", "日本", "🇯🇵"),
        ("HK", "香港", "🇨🇳"),
        ("MO", "澳门", "🇨🇳"),
        ("TW", "台湾", "🇨🇳"),
        ("", "", ""),
        ("ZZ", "", ""),
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
        let dns = rows
            .iter()
            .find(|row| row.title == "实际递归 DNS" && row.tone != 0)
            .unwrap();
        assert_eq!(dns.country, label, "{code}");
        assert_eq!(dns.flagged, !emoji.is_empty(), "{code}");
        assert_eq!(dns.flag, emoji, "{code}");
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
        let exit = rows
            .iter()
            .find(|row| row.title == "出口" && row.tone != 0)
            .unwrap();
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
        builtin: false,
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
    click_target(&window, "rule-profile");
    let _ = window.window().take_snapshot().unwrap();
    click_popup_target(&window, "rule-profile", "option:rule-profile/1");
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::RuleProfile, "second".into(), "".into())]
    );
    let event = events.borrow_mut().pop().unwrap();
    let _ = app.ui_intent(event.0, &event.1, &event.2);
    assert!(matches!(app.queued_actions.pop_back(), Some(Action::LoadRules(id)) if id == "second"));
    click_target(&window, "rule-source");
    let _ = window.window().take_snapshot().unwrap();
    click_popup_target(&window, "rule-source", "option:rule-source/1");
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
    slint::select_bundled_translation("en").unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    app.present_slint(&data, &mut models);
    let events = pointer_events(&window);
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    let category = data.get_site_category_options().row_data(1).unwrap();
    let status = data.get_site_status_options().row_data(3).unwrap();
    for (key, index, intent, value) in [
        ("site-category", 1, Intent::SiteCategory, category),
        ("site-status", 3, Intent::SiteStatus, status),
        ("site-region", 1, Intent::SiteRegion, "未识别地区".into()),
    ] {
        click_target(&window, key);
        let _ = window.window().take_snapshot().unwrap();
        click_popup_target(&window, key, &format!("option:{key}/{index}"));
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
    slint::select_bundled_translation("").unwrap();
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
    click_target(&window, "copy-log:[error] failed");
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::Copy, "[error] failed".into(), "".into())]
    );
    events.borrow_mut().clear();
    click_target(&window, &format!("log:{long}"));
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
    click_target(&window, "log-level");
    let _ = window.window().take_snapshot().unwrap();
    click_popup_target(&window, "log-level", "option:log-level/4");
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
    events.borrow_mut().clear();
    let _ = window.window().take_snapshot().unwrap();
    click_target(&window, "button:导出诊断");
    assert_eq!(
        events.borrow().as_slice(),
        &[(Intent::ExportDiagnostics, "".into(), "".into())]
    );
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
    click_target(&window, "dialog-copy");
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
    let color = window.global::<clash_client_ui::Theme>().get_canvas();
    assert_eq!(
        &initial[canvas..canvas + 3],
        &[color.red(), color.green(), color.blue()]
    );
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
        rapid_target(&window, "connection-sort:download-rate");
        let (intent, a, b) = events.borrow_mut().pop().unwrap();
        assert_eq!(intent, Intent::ConnectionSort);
        let _ = app.ui_intent(intent, &a, &b);
        app.present_slint(&data, &mut models);
        let immediately = motion_frame(&window, 0);
        let header = |pixels: &[u8]| {
            let bounds = target_bounds("connection-sort:download-rate");
            let left = bounds[0].round() as usize * 4;
            let right = (bounds[0] + bounds[2]).round() as usize * 4;
            pixels
                .as_chunks::<{ 950 * 4 }>()
                .0
                .iter()
                .skip(bounds[1].round() as usize)
                .take(bounds[3].round() as usize)
                .flat_map(|row| row[left..right].iter().copied())
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
    rapid_target(&window, "copy-log:[info] copy fixture");
    let (_, value, _) = events.borrow_mut().pop().unwrap();
    let waiting = motion_frame(&window, 250);
    data.set_copy_value("another request".into());
    data.set_copy_revision(1);
    assert_eq!(
        waiting,
        motion_frame(&window, 250),
        "unrelated or failed writes must not display success"
    );
    rapid_target(&window, "copy-log:[info] copy fixture");
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
        rapid_target(&window, "group:Proxy");
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
fn external_tun_alert_survives_navigation_and_clears_after_proxy_state_changes() {
    let mut app = fixture(Page::Home);
    app.network_watch.proxy(Some(ProxyMode::Off));
    let revision = app.network_watch.begin().unwrap();
    let _ = app.update(Message::HostRoutes(
        revision,
        serde_json::json!({
            "scope":"current_network_namespace",
            "ipv4":{"state":"observed","kind":"tun","interface":"Meta"},
            "tun_holders":{"interfaces":{"Meta":[{"uid":0,"pid":42}]}}
        }),
    ));
    for page in [Page::Home, Page::Proxies, Page::Settings] {
        app.page = page;
        let data = clash_desktop_protocol::UiData::default();
        let mut models = clash_desktop_protocol::models::ViewModels::default();
        models.attach(&data);
        app.present(&data, &mut models);
        assert!(data.get_network_alert().as_str().contains("root · PID 42"));
    }
    app.network_watch.proxy(Some(ProxyMode::Tun));
    let data = clash_desktop_protocol::UiData::default();
    let mut models = clash_desktop_protocol::models::ViewModels::default();
    models.attach(&data);
    app.present(&data, &mut models);
    assert!(data.get_network_alert().as_str().is_empty());
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
    assert!(!clash_desktop_ui::apply_if_current(&frame, &data, &mut models, 2, 3).unwrap());
    assert_eq!(data.get_edit_name(), "newer draft");
    assert_eq!(data.get_rule_kind(), "DOMAIN-SUFFIX");
    let frame = clash_desktop_protocol::Frame {
        edit_name: "newer draft".into(),
        rule_kind: "DOMAIN-SUFFIX".into(),
        ..frame
    };
    assert!(clash_desktop_ui::apply_if_current(&frame, &data, &mut models, 3, 3).unwrap());
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
fn english_desktop_validation_and_errors_follow_language() {
    let mut app = fixture(Page::Settings);
    app.presentation_language = Language::English;
    app.mixed_port = "0".into();
    let _ = app.update(Message::SavePorts);
    assert!(
        app.notice
            .contains("Port must be an integer from 1 to 65535")
    );

    app.update_interval_input = "10081".into();
    let _ = app.update(Message::SaveUpdateInterval);
    assert!(app.notice.contains("0 disables automatic update checks"));

    app.settings.dns_override = false;
    let _ = app.update(Message::FakeIpAdd);
    assert_eq!(app.fake_ip_error, "Enable DNS override first");

    let _ = app.update(Message::ProbeCancel);
    assert_eq!(app.test_results, ["Check cancelled"]);
}

#[test]
fn english_shared_errors_are_projected_without_changing_drafts_or_evidence() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let mut app = fixture(Page::Rules);
    app.presentation_language = Language::English;
    let _ = app.ui_intent(Intent::RuleAdd, "", "");
    let _ = app.ui_intent(Intent::RuleTextMode, "true", "");
    let _ = app.ui_intent(Intent::RuleRaw, "DOMAIN,,DIRECT", "");
    let _ = app.ui_intent(Intent::RuleSave, "", "");
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_modal_error(), "Rule content cannot be empty");
    let editor = app.rule_state.editor.as_ref().unwrap();
    assert_eq!(editor.raw, "DOMAIN,,DIRECT");
    assert_eq!(editor.error, "规则内容不能为空");

    let complex = "AND,((DOMAIN,a.test),(NETWORK,TCP)),REJECT";
    let _ = app.ui_intent(Intent::RuleRaw, complex, "");
    let _ = app.ui_intent(Intent::RuleTextMode, "false", "");
    app.present_slint(&data, &mut models);
    assert_eq!(
        data.get_modal_error(),
        "Use the text editor for logical and other complex rules."
    );
    let editor = app.rule_state.editor.as_ref().unwrap();
    assert!(editor.text_mode);
    assert_eq!(editor.raw, complex);
    assert_eq!(editor.error, "逻辑规则等复杂类型请使用规则文本编辑。");

    app.presentation_language = Language::SimplifiedChinese;
    app.present_slint(&data, &mut models);
    assert_eq!(
        data.get_modal_error(),
        "逻辑规则等复杂类型请使用规则文本编辑。"
    );
    app.presentation_language = Language::English;
    app.rule_state.editor = None;
    let failure = "订阅更新失败，已保留原配置与运行状态：YAML 配置格式错误: 東京；provider";
    app.show_notice(failure.into(), true);
    app.present_slint(&data, &mut models);
    assert_eq!(
        data.get_notice(),
        "Subscription update failed; the previous configuration and running state were retained: Invalid YAML configuration: 東京；provider"
    );
    assert_eq!(app.notice, failure);
}

#[test]
fn profile_edit_closes_on_success_independently_of_presentation_language() {
    let mut app = fixture(Page::Profiles);
    app.presentation_language = Language::English;
    app.updates.started = true;
    let _ = app.update(Message::EditProfile("fixture".into()));
    assert!(app.profile_editor.is_some());

    let mut failure = synchronized_reply(&app);
    failure.profile_edit = true;
    failure.notice = Err("Permission denied".into());
    let _ = app.update(Message::Finished(Box::new(failure)));
    assert_eq!(
        app.profile_editor.as_ref().unwrap().error,
        "Permission denied"
    );

    let mut success = synchronized_reply(&app);
    success.profile_edit = true;
    success.notice = Ok("Subscription saved".into());
    let _ = app.update(Message::Finished(Box::new(success)));
    assert!(app.profile_editor.is_none());
}

#[test]
fn english_desktop_dialogs_show_edit_validation_and_protection_effects() {
    ui_testing::initialize(800, 450);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let mut app = fixture(Page::Profiles);
    app.presentation_language = Language::English;

    let _ = app.update(Message::EditProfile("fixture".into()));
    let _ = app.update(Message::EditName(String::new()));
    let _ = app.update(Message::SaveProfileEdit);
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_modal_title(), "Edit subscription");
    assert_eq!(data.get_modal_accept(), "Save");
    assert_eq!(
        data.get_modal_error(),
        "The configuration name and source are required."
    );

    app.profile_editor = None;
    app.claude.guide = Some(claude_view::Guide::Protection);
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_modal_title(), "Enable Claude protection?");
    let description = data.get_modal_description().to_string();
    for expected in [
        "strict routing",
        "encrypted DNS",
        "DIRECT",
        "other apps",
        "restored when protection is disabled",
    ] {
        assert!(description.contains(expected), "{description}");
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
        millis: Some(4),
        authentication_check_skipped: false,
        detail: String::new(),
        target_exit: None,
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
    failed.notice = Err(
        "订阅更新失败，已保留原配置与运行状态：订阅格式不兼容，请选择 Mihomo/Clash Meta YAML 订阅"
            .into(),
    );
    let _ = app.update(Message::Finished(Box::new(failed)));
    assert!(!app.busy && !app.working);
    assert!(app.error && app.notice.contains("保留原配置"));
    assert!(app.notice_visible && app.notice.contains("Mihomo/Clash Meta YAML"));
    assert!(app.snapshot.running && app.core_failure.is_none());
    assert_eq!(serde_json::to_value(&app.profiles).unwrap(), before);
    assert_eq!(app.node_delays.get("Test"), Some(&42));
    assert_eq!(app.delay_generation, generation);
    assert_eq!(app.site_results.len(), 1);
    assert!(app.claude.reports[0].is_some());

    let mut background = synchronized_reply(&app);
    background.notice = Ok("检查完成".into());
    let _ = app.update(Message::Finished(Box::new(background)));
    assert!(app.error && app.notice.contains("保留原配置"));
    assert!(!app.notice_critical && app.snapshot.running);
    let _ = app.ui_intent(Intent::DismissNotice, "", "");
    assert!(!app.notice_visible);

    let mut success = synchronized_reply(&app);
    success.profile_updated = true;
    success.notice = Ok("订阅已更新".into());
    let _ = app.update(Message::Finished(Box::new(success)));
    assert!(app.node_delays.is_empty() && app.site_results.is_empty());
    assert!(app.claude.reports.iter().all(Option::is_none));
    assert!(!app.error && app.snapshot.running);
}

#[test]
fn scheduled_subscription_format_failure_notifies_once_without_marking_the_core_unhealthy() {
    use clash_client_core::subscription::UpdateFailure;

    let mut app = fixture(Page::Profiles);
    app.updates.started = true;
    app.node_delays.insert("saved node".into(), 42);
    let failure = UpdateFailure {
        revision: 1,
        message: "订阅格式不兼容，请选择 Mihomo/Clash Meta YAML 订阅".into(),
    };
    let mut reply = synchronized_reply(&app);
    reply.view_revision = app.view_revision;
    reply.snapshot.as_mut().unwrap().subscription_failure = Some(failure.clone());
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert!(app.notice_visible && app.error);
    assert_eq!(app.notice, failure.message);
    assert!(app.snapshot.running && app.core_failure.is_none());
    assert_eq!(app.node_delays.get("saved node"), Some(&42));
    let revision = app.notice_revision;

    app.dismiss_notice();
    let mut reply = synchronized_reply(&app);
    reply.view_revision = app.view_revision;
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert!(!app.notice_visible);
    assert_eq!(app.notice_revision, revision.wrapping_add(1));

    let mut reply = synchronized_reply(&app);
    reply.view_revision = app.view_revision;
    reply.snapshot.as_mut().unwrap().subscription_failure = Some(UpdateFailure {
        revision: 2,
        ..failure
    });
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert!(app.notice_visible && app.error);
}

#[test]
fn geo_download_survives_unrelated_refresh_and_only_success_clears_results() {
    let mut app = fixture(Page::Settings);
    app.geo_updating = true;
    app.node_delays.insert("Test".into(), 42);
    let mut refresh = synchronized_reply(&app);
    refresh.view_revision = app.view_revision;
    let _ = app.update(Message::Finished(Box::new(refresh)));
    assert!(app.geo_updating);

    let _ = app.update(Message::GeoFinished(Err("下载失败".into())));
    assert!(!app.geo_updating);
    assert_eq!(app.node_delays.get("Test"), Some(&42));
    assert!(app.error && app.notice.contains("下载失败"));

    let mut refresh = synchronized_reply(&app);
    refresh.notice = Ok("检查完成".into());
    let _ = app.update(Message::Finished(Box::new(refresh)));
    assert!(app.error && app.notice.contains("下载失败"));
    assert!(!app.notice_critical && app.snapshot.running);
    let _ = app.ui_intent(Intent::DismissNotice, "", "");
    assert!(!app.notice_visible);

    app.geo_updating = true;
    let _ = app.update(Message::GeoFinished(Ok(())));
    assert!(!app.geo_updating);
    assert!(app.node_delays.is_empty());
    assert!(!app.error && app.notice.contains("已全部更新"));
}

#[test]
fn geo_failure_follows_the_current_language_and_keeps_external_details() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let mut models = ViewModels::default();
    models.attach(&data);
    let mut app = fixture(Page::Settings);
    app.presentation_language = Language::English;
    let _ = app.update(Message::GeoFinished(Err(
        "Geo 更新失败，已恢复旧数据: 東京；provider".into(),
    )));
    app.present_slint(&data, &mut models);
    assert_eq!(
        data.get_notice(),
        "Geo update failed: Geo update failed; previous data restored: 東京；provider"
    );
    assert!(app.error && app.snapshot.running);
    app.presentation_language = Language::SimplifiedChinese;
    app.present_slint(&data, &mut models);
    assert_eq!(
        data.get_notice(),
        "Geo 更新失败：Geo 更新失败，已恢复旧数据: 東京；provider"
    );
}

#[test]
fn geo_update_from_another_client_clears_stale_results() {
    let mut app = fixture(Page::Settings);
    app.geo_status = "old Geo".into();
    app.node_delays.insert("Test".into(), 42);
    let mut reply = synchronized_reply(&app);
    reply.geo_status = "new Geo".into();
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert!(app.node_delays.is_empty());
    assert_eq!(app.geo_status, "new Geo");
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
fn update_interval_saves_zero_and_keeps_invalid_or_failed_drafts() {
    let mut app = fixture(Page::Settings);
    app.busy = true;
    for value in ["0", " 30 ", "10080"] {
        let _ = app.ui_intent(Intent::UpdateIntervalInput, value, "");
        let _ = app.ui_intent(Intent::SaveUpdateInterval, "", "");
        assert!(matches!(
            app.queued_actions.pop_back(),
            Some(Action::UpdateInterval(minutes)) if minutes == value.trim().parse::<u32>().unwrap()
        ));
    }
    for value in ["", "-1", "1.5", "10081", "酒店里点炒饭"] {
        let _ = app.ui_intent(Intent::UpdateIntervalInput, value, "");
        let _ = app.ui_intent(Intent::SaveUpdateInterval, "", "");
        assert!(app.queued_actions.is_empty());
        assert_eq!(app.update_interval_input, value);
        assert!(app.error);
    }
    let _ = app.ui_intent(Intent::UpdateIntervalInput, "0", "");
    let mut failed = synchronized_reply(&app);
    failed.notice = Err("设置目录不可写".into());
    let _ = app.update(Message::Finished(Box::new(failed)));
    assert_eq!(app.update_interval_input, "0");
    assert_eq!(app.settings.update_check_interval_minutes, 360);
    assert!(app.notice.contains("设置目录不可写"));
}

#[test]
fn disabling_automatic_update_checks_keeps_manual_checks_available() {
    let mut app = fixture(Page::Settings);
    app.settings.update_check_interval_minutes = 0;
    let _ = app.auto_check_update();
    assert!(!app.updates.started);
    let reply = synchronized_reply(&app);
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert!(!app.updates.started);
    let _ = app.ui_intent(Intent::CheckUpdate, "", "");
    assert!(app.updates.checking);
    let check_id = app.updates.check_id;
    let _ = app.ui_intent(Intent::CheckUpdate, "", "");
    assert_eq!(app.updates.check_id, check_id);

    let mut enabled = fixture(Page::Settings);
    enabled.claude.protection_pending = Some(true);
    let _ = enabled.auto_check_update();
    assert!(!enabled.updates.started);
    enabled.claude.protection_pending = None;
    let _ = enabled.auto_check_update();
    assert!(enabled.updates.checking);
}

#[test]
fn automatic_update_checks_are_not_starved_by_read_only_refreshes() {
    let mut app = fixture(Page::Settings);
    app.settings.update_check_interval_minutes = 5;
    app.updates.started = true;
    app.update_schedule
        .checked(std::time::Instant::now() - Duration::from_secs(300));
    app.busy = true;
    app.working = false;

    let _ = app.auto_check_update();
    assert!(app.updates.checking);
    let check_id = app.updates.check_id;
    let _ = app.auto_check_update();
    assert_eq!(app.updates.check_id, check_id);

    let _ = app.update(Message::UpdateChecked(
        check_id,
        Err("GitHub temporarily unreachable".into()),
    ));
    assert!(
        app.updates
            .status
            .contains("GitHub temporarily unreachable")
    );
    assert!(app.notice_visible && app.notice.contains("GitHub temporarily unreachable"));
    let _ = app.auto_check_update();
    assert_eq!(app.updates.check_id, check_id);

    app.update_schedule
        .checked(std::time::Instant::now() - Duration::from_secs(300));
    let _ = app.auto_check_update();
    assert_eq!(app.updates.check_id, check_id + 1);
    let _ = app.update(Message::UpdateChecked(app.updates.check_id, Ok(None)));
    assert!(app.updates.status.contains("未发现新版本"));
    assert!(
        !app.updates
            .status
            .contains("GitHub temporarily unreachable")
    );
    assert!(!app.notice_visible);
}

#[cfg(target_os = "linux")]
#[test]
fn automatic_update_checks_follow_the_host_tick_and_refresh_order() {
    let directory = tempfile::tempdir().unwrap();
    let resources = directory.path().join("resources");
    std::fs::create_dir(&resources).unwrap();
    std::fs::write(resources.join("mihomo"), b"fixture").unwrap();
    std::fs::write(
        resources.join("default.yaml"),
        b"proxies: []\nrules: [MATCH,DIRECT]\n",
    )
    .unwrap();
    let files = clash_of_rust::assets::GEO_FILES
        .iter()
        .map(|name| {
            let path = resources.join(name);
            std::fs::write(&path, b"fixture").unwrap();
            let (sha256, size) = clash_of_rust::assets::hash_file(&path).unwrap();
            clash_of_rust::assets::GeoFile {
                name: (*name).into(),
                source: String::new(),
                sha256,
                size,
            }
        })
        .collect();
    let manifest = clash_of_rust::assets::GeoManifest {
        version: "fixture".into(),
        updated: 0,
        installed_at: None,
        files,
    };
    std::fs::write(
        resources.join(clash_of_rust::assets::MANIFEST),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let store = Store::at(directory.path().join("data")).unwrap();
    let engine = Engine::with_resources(store, resources).unwrap();
    let mut app = fixture(Page::Settings);
    app.engine = Some(Arc::new(Mutex::new(engine)));
    app.visible = true;
    app.settings.update_check_interval_minutes = 5;
    app.updates.started = true;
    app.update_schedule
        .checked(std::time::Instant::now() - Duration::from_secs(300));

    let _ = app.update(Message::Tick);
    assert!(app.busy && !app.working);
    let _ = app.auto_check_update();
    assert!(app.updates.checking);
}

#[test]
fn automatic_update_checks_wait_for_mutating_or_queued_actions() {
    let mut app = fixture(Page::Settings);
    app.settings.update_check_interval_minutes = 5;
    app.busy = true;
    app.working = true;
    let _ = app.auto_check_update();
    assert!(!app.updates.started);

    app.working = false;
    app.queued_actions.push_back(Action::UpdateInterval(10));
    let _ = app.auto_check_update();
    assert!(!app.updates.started);

    app.queued_actions.clear();
    let _ = app.auto_check_update();
    assert!(app.updates.checking);
}

#[test]
fn automatic_update_checks_coalesce_while_a_slow_check_is_running() {
    let mut app = fixture(Page::Settings);
    app.settings.update_check_interval_minutes = 1;
    let _ = app.auto_check_update();
    let check_id = app.updates.check_id;
    assert!(app.updates.checking);
    app.update_schedule
        .checked(std::time::Instant::now() - Duration::from_secs(60));
    let _ = app.auto_check_update();
    assert_eq!(app.updates.check_id, check_id);

    let _ = app.update(Message::UpdateChecked(check_id, Err("断网".into())));
    let _ = app.auto_check_update();
    assert!(app.updates.checking);
    assert_eq!(app.updates.check_id, check_id + 1);
    let _ = app.auto_check_update();
    assert_eq!(app.updates.check_id, check_id + 1);
}

#[test]
fn automatic_update_success_does_not_dismiss_a_newer_unrelated_failure() {
    let mut app = fixture(Page::Settings);
    let _ = app.auto_check_update();
    let failed = app.updates.check_id;
    let _ = app.update(Message::UpdateChecked(failed, Err("断网".into())));
    assert!(app.notice_visible && app.notice.contains("断网"));

    app.update_schedule
        .checked(std::time::Instant::now() - Duration::from_secs(360 * 60));
    let _ = app.auto_check_update();
    let retry = app.updates.check_id;
    app.show_critical("内核故障".into());
    let _ = app.update(Message::UpdateChecked(retry, Ok(None)));
    assert!(app.notice_visible && app.notice_critical);
    assert_eq!(app.notice, "内核故障");
    assert!(app.updates.status.contains("未发现新版本"));
}

#[test]
fn console_changes_refresh_profiles_and_clean_inputs_but_preserve_typed_drafts() {
    let mut app = fixture(Page::Settings);
    let mut reply = synchronized_reply(&app);
    reply.settings.mixed_port = 12345;
    reply.settings.delay_interval_minutes = 17;
    reply.settings.subscription_update_interval_minutes = 720;
    reply.settings.update_check_interval_minutes = 60;
    reply.profiles.as_mut().unwrap()[0].name = "Renamed by CLI".into();
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert_eq!(app.mixed_port, "12345");
    assert_eq!(app.interval_input, "17");
    assert_eq!(app.subscription_interval_input, "720");
    assert_eq!(app.update_interval_input, "60");
    assert_eq!(app.profiles[0].name, "Renamed by CLI");
    assert!(app.backend_remote);

    app.mixed_port = "12346".into();
    app.interval_input = "25".into();
    app.subscription_interval_input = "90".into();
    app.update_interval_input = "0".into();
    let mut reply = synchronized_reply(&app);
    reply.settings.mixed_port = 12347;
    reply.settings.delay_interval_minutes = 30;
    reply.settings.subscription_update_interval_minutes = 180;
    reply.settings.update_check_interval_minutes = 120;
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert_eq!(app.mixed_port, "12346");
    assert_eq!(app.interval_input, "25");
    assert_eq!(app.subscription_interval_input, "90");
    assert_eq!(app.update_interval_input, "0");
    assert_eq!(app.settings.mixed_port, 12347);
}

#[test]
fn protection_locks_rule_controls_and_proxy_mode_without_a_warning_or_draft_change() {
    let mut app = fixture(Page::Rules);
    app.claude.protection_pending = Some(true);
    let draft = app.rule_state.draft.clone();
    for intent in [Intent::RuleAdd, Intent::RuleApply, Intent::RuleDiscard] {
        let _ = app.ui_intent(intent, "blocked", "");
    }
    assert_eq!(app.rule_state.draft, draft);
    assert!(app.query.is_empty());
    assert!(!app.rule_state.has_editor());
    assert!(!app.notice_visible);
    app.page = Page::Home;
    let update_check = app.updates.check_id;
    let _ = app.ui_intent(Intent::CheckUpdate, "", "");
    let _ = app.ui_intent(Intent::UpdateGeo, "", "");
    let _ = app.ui_intent(Intent::InstallUpdate, "", "");
    let _ = app.ui_intent(Intent::UpdatePrereleases, "true", "");
    assert_eq!(app.updates.check_id, update_check);
    assert!(!app.updates.checking && !app.geo_updating);
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
        Intent::SubscriptionIntervalInput,
        Intent::SaveSubscriptionInterval,
        Intent::UpdateIntervalInput,
        Intent::SaveUpdateInterval,
    ] {
        let _ = app.ui_intent(intent, "false", "blocked.test");
    }
    assert!(!app.notice_visible);
    assert!(app.confirmation.is_none());
    assert_eq!(serde_json::to_value(&app.settings).unwrap(), settings);
    app.busy = true;
    let _ = app.ui_intent(Intent::AccentColor, "1", "");
    assert!(matches!(app.queued_actions.back(), Some(Action::Accent(_))));
    let _ = app.ui_intent(Intent::ClearLogs, "", "");
    assert!(matches!(app.queued_actions.back(), Some(Action::ClearLogs)));
    app.page = Page::Rules;
    let _ = app.ui_intent(Intent::Query, "visible during protection", "");
    assert_eq!(app.query, "visible during protection");
    assert_eq!(app.rule_state.draft, draft);
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
fn protection_rejects_confirmations_that_would_save_or_delete() {
    let mut app = fixture(Page::Rules);
    let _ = app.ui_intent(Intent::RuleAdd, "", "");
    let _ = app.ui_intent(Intent::RulePayload, "unsaved.test", "");
    app.page = Page::Home;
    app.claude.protection_pending = Some(true);
    let draft = app.rule_state.draft.clone();
    assert!(app.rule_state.has_editor());
    let _ = app.ui_intent(Intent::Confirm, "", "");
    assert!(app.rule_state.has_editor());
    assert_eq!(app.rule_state.draft, draft);

    app.confirmation = Some(desktop_view::Confirmation {
        title: "删除订阅？".into(),
        description: "该订阅将被删除。".into(),
        accept: "删除订阅",
        pending: desktop_view::Pending::Delete("fixture".into()),
        ready_at: None,
    });
    let profiles = app.profiles.len();
    let _ = app.ui_intent(Intent::Confirm, "", "");
    assert!(app.confirmation.is_some());
    assert_eq!(app.profiles.len(), profiles);
}

#[test]
fn a_stopped_protected_core_exposes_a_retry_without_disabling_protection() {
    use clash_client_core::protection::{Previous, Protection};
    ui_testing::initialize(950, 700);
    let mut app = fixture(Page::Home);
    app.snapshot.running = false;
    app.settings.claude_protection = Some(Arc::new(Protection::new(
        "fixture".into(),
        "node".into(),
        Previous {
            mode: "rule".into(),
            proxy_mode: ProxyMode::Off,
            selectors: vec![],
        },
    )));
    let settings = serde_json::to_value(&app.settings).unwrap();
    let data = clash_desktop_protocol::UiData::default();
    let mut models = clash_desktop_protocol::models::ViewModels::default();
    app.present(&data, &mut models);
    assert!(data.get_protected() && data.get_retry_core());
    app.claude.protection_pending = Some(true);
    assert!(!app.ready_to_retry());
    let _ = app.ui_intent(Intent::RestartCore, "", "");
    assert!(!app.busy);
    app.claude.protection_pending = None;
    let _ = app.ui_intent(Intent::RestartCore, "", "");
    assert!(app.busy && app.working);
    assert_eq!(serde_json::to_value(&app.settings).unwrap(), settings);
    let _ = app.ui_intent(Intent::RestartCore, "", "");
    assert!(app.queued_actions.is_empty());
    app.busy = false;
    app.working = false;
    app.snapshot.running = true;
    let _ = app.ui_intent(Intent::RestartCore, "", "");
    assert!(!app.busy && app.queued_actions.is_empty());
    assert_eq!(serde_json::to_value(&app.settings).unwrap(), settings);
}

#[test]
fn every_page_projects_protection_readiness_and_discards_unconfirmed_success() {
    use clash_client_core::protection::{Previous, Protection, status};
    let mut app = fixture(Page::Home);
    app.settings.claude_protection = Some(Arc::new(Protection::new(
        "fixture".into(),
        "node".into(),
        Previous {
            mode: "rule".into(),
            proxy_mode: ProxyMode::Off,
            selectors: vec![],
        },
    )));
    for page in Page::ALL {
        app.page = page;
        for (guard, expected) in [
            (Some((true, "")), status::State::Active),
            (None, status::State::Checking),
            (Some((false, "DNS 接管未就绪")), status::State::Error),
        ] {
            app.snapshot.protection_status =
                status::Status::observe(true, false, true, true, guard);
            app.snapshot.retain_scope(page.scope());
            let data = clash_desktop_protocol::UiData::default();
            let mut models = clash_desktop_protocol::models::ViewModels::default();
            app.present(&data, &mut models);
            assert_eq!(data.get_protection_health(), expected as i32);
        }
    }
    app.page = Page::Home;
    let failure = "连接已处理，但系统通知失败：notification disabled";
    app.snapshot.protection_status =
        status::Status::observe(true, false, true, true, Some((true, failure)));
    app.presentation_language = clash_client_core::config::Language::English;
    let data = clash_desktop_protocol::UiData::default();
    app.present(
        &data,
        &mut clash_desktop_protocol::models::ViewModels::default(),
    );
    assert_eq!(
        data.get_protection_message(),
        "Connection handled; system notification failed: notification disabled"
    );
    assert_eq!(app.snapshot.protection_status.message, failure);
    app.presentation_language = clash_client_core::config::Language::SimplifiedChinese;
    app.present(
        &data,
        &mut clash_desktop_protocol::models::ViewModels::default(),
    );
    assert_eq!(data.get_protection_message(), failure);
    let mut reply = synchronized_reply(&app);
    reply.snapshot = None;
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert_eq!(
        app.snapshot.protection_status.state,
        status::State::Checking
    );
    app.settings.claude_protection = None;
    let data = clash_desktop_protocol::UiData::default();
    app.present(
        &data,
        &mut clash_desktop_protocol::models::ViewModels::default(),
    );
    assert_eq!(data.get_protection_health(), status::State::Off as i32);
    app.page = Page::Home;
    for changing in [true, false] {
        app.claude.protection_pending = changing.then_some(true);
        app.present(
            &data,
            &mut clash_desktop_protocol::models::ViewModels::default(),
        );
        assert_eq!(data.get_protection_pending(), changing);
        assert_eq!(
            data.get_protection_health(),
            if changing {
                status::State::Checking
            } else {
                status::State::Off
            } as i32
        );
    }
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
        active_group: None,
        proxies: serde_json::from_value(serde_json::json!({"proxies": {
            "AA Unused": {"type":"Selector", "now":"Leaf", "all":["Leaf"]},
            "ZZ Active": {"type":"Selector", "now":"Leaf", "all":["Leaf"]},
            "Leaf": {"type":"Trojan"}
        }}))
        .unwrap(),
        ..Default::default()
    });
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert!(app.focus_active_group);
    assert!(!app.proxy_rows().1.iter().any(|(group, _)| group.expanded));
    let mut reply = synchronized_reply(&app);
    reply.scope = Scope::Proxies;
    let mut snapshot = app.snapshot.clone();
    snapshot.active_group = Some("ZZ Active".into());
    reply.snapshot = Some(snapshot);
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert!(!app.focus_active_group);
    let (_, groups) = app.proxy_rows();
    assert_eq!(groups[0].0.id.as_str(), "ZZ Active");
    assert!(groups[0].0.expanded);
    assert_eq!(groups[1].0.id.as_str(), "AA Unused");
    let _ = app.ui_intent(Intent::ToggleGroup, "ZZ Active", "");
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
fn delayed_active_group_respects_manual_navigation_and_direct_mode() {
    for (intent, value, extra) in [
        (Intent::ToggleGroup, "Manual", ""),
        (Intent::Query, "Manual", ""),
        (Intent::ListPage, "4", ""),
        (Intent::GroupPage, "Manual", "true"),
        (Intent::GroupPagination, "true", ""),
        (Intent::Navigate, "0", ""),
    ] {
        let mut app = fixture(Page::Home);
        app.busy = true;
        let _ = app.ui_intent(Intent::Navigate, "1", "");
        assert!(app.focus_active_group);
        let _ = app.ui_intent(intent, value, extra);
        assert!(!app.focus_active_group);
    }
    let mut app = fixture(Page::Home);
    let _ = app.ui_intent(Intent::Navigate, "1", "");
    let mut reply = synchronized_reply(&app);
    reply.scope = Scope::Proxies;
    reply.snapshot = Some(Snapshot {
        running: true,
        mode: "direct".into(),
        ..Default::default()
    });
    let _ = app.update(Message::Finished(Box::new(reply)));
    assert!(!app.focus_active_group);
}

#[test]
fn proxy_groups_default_to_all_and_keep_independent_node_pagination() {
    let mut app = fixture(Page::Proxies);
    let mut proxies = serde_json::Map::new();
    let names: Vec<_> = (0..125).map(|index| format!("Node {index:03}")).collect();
    for index in 0..9 {
        proxies.insert(
            format!("Group {index}"),
            serde_json::json!({"type":"Selector","now":names[0],"all":names}),
        );
    }
    app.snapshot.proxies = serde_json::from_value(serde_json::json!({"proxies":proxies})).unwrap();
    app.snapshot.active_group = Some("Group 8".into());
    app.expanded.insert("Group 8".into());
    let (total, groups) = app.proxy_rows();
    assert_eq!(total, 9);
    assert_eq!(groups.len(), 9);
    assert_eq!(groups[0].0.id, "Group 8");
    assert!(groups[0].0.next);
    assert_eq!(groups[0].1.len(), PAGE_SIZE / 2);
    assert!(groups[1..].iter().all(|(_, nodes)| nodes.is_empty()));
    let _ = app.ui_intent(Intent::GroupPage, "Group 8", "true");
    assert!(app.proxy_rows().1[0].0.previous);
    app.settings.proxy_groups_paginated = true;
    let (_, first_page) = app.proxy_rows();
    assert_eq!(first_page.len(), 8);
    assert_eq!(first_page.last().unwrap().0.id, "Group 6");
    app.list_offset = GROUP_PAGE_SIZE;
    let (_, last_page) = app.proxy_rows();
    assert_eq!(last_page.len(), 1);
    assert_eq!(last_page[0].0.id, "Group 7");
    app.settings.proxy_groups_paginated = false;
    assert_eq!(app.proxy_rows().1.len(), 9);
    app.query = "Group 7".into();
    assert_eq!(app.proxy_rows().1[0].0.id, "Group 7");
    app.query.clear();
    let _ = app.ui_intent(Intent::Navigate, "0", "");
    assert!(app.proxy_order.borrow().is_empty());
}

#[test]
fn group_pagination_button_dispatches_while_protected_and_tracks_saved_state() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    data.set_page(1);
    data.set_protected(true);
    track_targets(&window);
    let events = std::rc::Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    window
        .global::<UiActions>()
        .on_dispatch(move |intent, a, b| recorded.borrow_mut().push((intent, a, b)));
    window.show().unwrap();
    let mut app = fixture(Page::Proxies);
    app.busy = true;
    app.claude.protection_pending = Some(true);
    data.set_page_offset(GROUP_PAGE_SIZE as i32);
    data.set_previous(true);
    data.set_next(true);
    for (saved, requested) in [(false, "true"), (true, "false")] {
        data.set_groups_paginated(saved);
        let _ = window.window().take_snapshot().unwrap();
        click_target(&window, "group-pagination");
        assert_eq!(
            events.borrow_mut().pop(),
            Some((Intent::GroupPagination, requested.into(), "".into()))
        );
        let _ = app.ui_intent(Intent::GroupPagination, requested, "");
        assert!(matches!(
            app.queued_actions.pop_back(),
            Some(Action::GroupPagination(value)) if value != saved
        ));
    }
    for (target, expected) in [
        ("group-pagination-previous", "0"),
        ("group-pagination-next", "16"),
    ] {
        click_target(&window, target);
        assert_eq!(
            events.borrow_mut().pop(),
            Some((Intent::ListPage, expected.into(), "".into()))
        );
    }
    window.hide().unwrap();
}

#[test]
fn all_proxy_groups_are_scrollable_without_mounting_every_group() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    data.set_page(1);
    let mut models = ViewModels::default();
    models.attach(&data);
    models.set_groups(
        (0..500)
            .map(|index| {
                (
                    clash_client_ui::GroupItem {
                        id: format!("group-{index}").into(),
                        title: format!("Group {index}").into(),
                        ..Default::default()
                    },
                    Vec::new(),
                )
            })
            .collect(),
    );
    track_targets(&window);
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    let mounted = TARGETS.with(|targets| {
        targets
            .borrow()
            .keys()
            .filter(|key| key.starts_with("group:"))
            .count()
    });
    assert!((1..100).contains(&mounted), "{mounted} mounted groups");
    assert_eq!(data.get_groups().row_count(), 500);
    data.set_scroll_proxies(-100_000.);
    let _ = window.window().take_snapshot().unwrap();
    let bounds = target_bounds("group:group-499");
    assert!(bounds[1] >= 0. && bounds[1] + bounds[3] <= 700.);
    window.hide().unwrap();
}

#[test]
fn subscription_pagination_preserves_access_to_the_last_profile_in_both_views() {
    ui_testing::initialize(950, 700);
    let window = AppWindow::new().unwrap();
    let data = window.global::<UiData>();
    let views = window.global::<clash_client_ui::PageViews>();
    let mut app = fixture(Page::Profiles);
    let template = app.profiles[0].clone();
    app.profiles = (0..240)
        .map(|index| {
            let mut profile = template.clone();
            profile.id = format!("profile-{index}");
            profile.name = format!("Profile {index}");
            profile
        })
        .collect();
    let mut models = ViewModels::default();
    models.attach(&data);
    let events = pointer_events(&window);
    window.show().unwrap();
    app.present_slint(&data, &mut models);
    motion_frame(&window, 0);
    assert_eq!(data.get_profiles().row_count(), 240);
    assert!(!data.get_next());
    click_target(&window, "profiles-pagination");
    assert_eq!(
        events.borrow_mut().pop(),
        Some((Intent::ProfilePagination, "true".into(), "".into()))
    );

    app.settings.profiles_paginated = true;
    app.present_slint(&data, &mut models);
    motion_frame(&window, 0);
    assert_eq!(data.get_profiles().row_count(), PAGE_SIZE);
    assert!(data.get_next());
    click_target(&window, "profiles-pagination");
    assert_eq!(
        events.borrow_mut().pop(),
        Some((Intent::ProfilePagination, "false".into(), "".into()))
    );

    app.settings.profiles_paginated = false;
    app.list_offset = PAGE_SIZE;
    app.present_slint(&data, &mut models);
    assert_eq!(data.get_page_offset(), 0);
    assert_eq!(data.get_profiles().row_count(), 240);
    assert!(!data.get_next());
    for list_view in [false, true] {
        TARGETS.with(|targets| targets.borrow_mut().clear());
        let geometry = window.global::<UiGeometry>();
        geometry.set_revision(geometry.get_revision().wrapping_add(1));
        views.set_profiles_list(list_view);
        data.set_scroll_profiles(0.);
        motion_frame(&window, 0);
        let first = target_bounds("profile:profile-0");
        let footer = target_bounds("profiles-pagination");
        assert!(first[1] + first[3] <= footer[1], "{first:?}, {footer:?}");
        click_target(&window, "profile-use:profile-0");
        assert_eq!(
            events.borrow_mut().pop(),
            Some((Intent::ActivateProfile, "profile-0".into(), "".into()))
        );
        let mounted = TARGETS.with(|targets| {
            targets
                .borrow()
                .keys()
                .filter(|key| key.starts_with("profile:"))
                .count()
        });
        assert!((1..100).contains(&mounted), "{mounted} mounted profiles");
        data.set_scroll_profiles(-100_000.);
        motion_frame(&window, 0);
        let bounds = target_bounds("profile:profile-239");
        assert!(bounds[1] >= 0. && bounds[1] + bounds[3] <= footer[1]);
    }
    window.hide().unwrap();
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
