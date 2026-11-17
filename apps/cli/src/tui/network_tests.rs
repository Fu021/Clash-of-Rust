use crate::tui::{
    input::Key,
    model::{Action, Page, State, demo_poll},
    network, render,
};
use clash_client_core::network_tools::{Entry, Options, Report, Tool};
use unicode_width::UnicodeWidthStr;

fn state(tool: Tool) -> State {
    let mut state = State::new(false);
    state.page = Page::Network;
    state.release_page();
    state.network_tool = tool.index() as usize;
    state.query = "example.test".into();
    assert!(matches!(state.key(Key::Char('o')), Action::CancelNetwork));
    state
}

#[test]
fn editors_offer_only_the_current_tools_applicable_parameters() {
    for (tool, expected) in [
        (Tool::Website, vec![]),
        (Tool::Dns, vec!["type"]),
        (Tool::Phase, vec!["seconds"]),
        (Tool::Family, vec!["seconds"]),
        (Tool::Tls, vec!["seconds"]),
        (Tool::Stability, vec!["count", "interval-ms", "seconds"]),
        (Tool::Udp, vec!["seconds"]),
        (Tool::Ping, vec!["count"]),
        (Tool::Trace, vec!["max-hops"]),
        (Tool::Download, vec!["max-bytes", "seconds"]),
        (Tool::Tcp, vec!["seconds"]),
        (Tool::Http, vec!["method", "seconds"]),
    ] {
        let mut state = state(tool);
        let form = state.form.as_ref().unwrap();
        assert_eq!(
            form.fields
                .iter()
                .filter_map(|field| field.option)
                .collect::<Vec<_>>(),
            expected,
            "{tool:?}"
        );
        assert_eq!(form.fields[0].value, "example.test");
        network::apply(&mut state).unwrap();
        assert!(state.form.is_none());
        assert!(matches!(
            state.key(Key::Enter),
            Action::CheckNetwork(selected, target, options)
                if selected == tool && target == "example.test"
                    && options.seconds == 10 && options.count == 5
        ));
    }
}

#[test]
fn applying_parameters_keeps_them_local_and_navigation_restores_defaults() {
    let mut state = state(Tool::Http);
    state.network_results.push("旧结果".into());
    let form = state.form.as_mut().unwrap();
    form.fields[0].value = "https://example.test/path?q=1".into();
    form.selected = 1;
    assert!(form.cycle(false));
    form.fields[2].value = "3".into();
    network::apply(&mut state).unwrap();
    assert!(state.network_results.is_empty());
    assert!(
        matches!(state.key(Key::Enter), Action::CheckNetwork(Tool::Http, target, options)
        if target == "https://example.test/path?q=1" && options.method == "GET"
            && options.seconds == 3)
    );
    state.key(Key::Char('o'));
    assert_eq!(state.form.as_ref().unwrap().fields[1].value, "GET");
    state.key(Key::Escape);
    assert_eq!(state.network_options.method, "GET");
    state.key(Key::Char('['));
    assert_eq!(state.network_options.method, "HEAD");
    assert_eq!(state.network_options.seconds, 10);
    state.network_options.seconds = 2;
    state.key(Key::Char('1'));
    state.release_page();
    assert_eq!(state.network_options.seconds, 10);
    assert!(state.form.is_none());
}

#[test]
fn confirming_a_tool_choice_keeps_the_value_without_starting_a_probe() {
    let mut state = state(Tool::Dns);
    state.key(Key::Down);
    state.key(Key::Right);
    assert_eq!(state.form.as_ref().unwrap().fields[1].value, "A");
    assert!(matches!(state.key(Key::Enter), Action::None));
    assert!(!state.entering_text());
    assert_eq!(state.form.as_ref().unwrap().fields[1].value, "A");
    assert!(state.form.is_some());
    network::apply(&mut state).unwrap();
    assert_eq!(state.network_options.record_type, "A");
    assert_eq!(state.query, "example.test");
}

#[test]
fn invalid_parameters_preserve_both_applied_values_and_the_current_draft() {
    for invalid in ["", "not-a-number", "0", "31", "-1", "18446744073709551616"] {
        let mut state = state(Tool::Http);
        state.network_options.seconds = 4;
        state.network_results.push("保留结果".into());
        state.form.as_mut().unwrap().fields[2].value = invalid.into();
        assert!(network::apply(&mut state).is_err(), "{invalid}");
        assert_eq!(state.network_options.seconds, 4);
        assert_eq!(state.query, "example.test");
        assert_eq!(state.network_results, ["保留结果"]);
        assert_eq!(state.form.as_ref().unwrap().fields[2].value, invalid);
    }
}

#[test]
fn cancelling_field_and_form_edits_keeps_the_previous_parameters() {
    let mut state = state(Tool::Dns);
    state.key(Key::Enter);
    state.key(Key::Clear);
    state.key(Key::Paste("discard.test".into()));
    state.key(Key::Escape);
    assert_eq!(state.form.as_ref().unwrap().fields[0].value, "example.test");
    state.key(Key::Down);
    state.key(Key::Right);
    assert_eq!(state.form.as_ref().unwrap().fields[1].value, "A");
    state.key(Key::Escape);
    assert!(state.form.is_none());
    assert_eq!(state.network_options.record_type, "ADDR");
    assert_eq!(state.query, "example.test");
}

#[test]
fn download_limits_reject_overflow_and_keep_the_smallest_supported_size() {
    let mut state = state(Tool::Download);
    state.form.as_mut().unwrap().fields[1].value = u64::MAX.to_string();
    assert!(network::apply(&mut state).is_err());
    state.form.as_mut().unwrap().fields[1].value = "1".into();
    network::apply(&mut state).unwrap();
    assert_eq!(state.network_options.max_bytes, 1024);
    assert_eq!(state.network_options.seconds, Options::default().seconds);
}

#[test]
fn profile_refresh_preserves_network_drafts_but_mismatched_tools_are_rejected() {
    let mut state = state(Tool::Stability);
    state.key(Key::Enter);
    state.key(Key::Clear);
    state.key(Key::Paste("draft.test".into()));
    let mut poll = demo_poll();
    poll.meta.settings.active_profile = Some("new-profile".into());
    state.accept(Some(poll));
    assert_eq!(state.form.as_ref().unwrap().draft, "draft.test");
    state.key(Key::Enter);
    state.network_tool = Tool::Ping.index() as usize;
    assert!(network::apply(&mut state).is_err());
    assert_eq!(state.query, "example.test");
    assert!(state.form.is_some());
}

#[test]
fn wrapped_results_reach_the_last_line_on_phone_and_tiny_screens() {
    for (columns, height) in [(36, 16), (40, 20), (55, 16), (80, 24), (40, 2)] {
        let mut state = state(Tool::Trace);
        state.key(Key::Escape);
        let report = format!("路由证据 {} REPORT_END", "长地址与说明".repeat(48));
        state.network_results = vec![report.clone()];
        let width = render::content_width(columns, height);
        let rows = network::rows(&state, width);
        assert_eq!(
            rows.iter().map(|row| row.text.as_str()).collect::<String>(),
            report
        );
        assert!(rows.iter().all(|row| row.text.width() <= width));
        for _ in 0..rows.len() + 3 {
            network::scroll(&mut state, width, true);
        }
        assert_eq!(state.selected, rows.len() - 1);
        let frame = render::frame(&state, columns, height);
        assert!(
            frame
                .iter()
                .any(|line| line.text.trim() == rows.last().unwrap().text.trim())
        );
        if height >= 3 {
            assert!(frame.iter().any(|line| line.text.contains("网络")));
        }
        for _ in 0..rows.len() + 3 {
            network::scroll(&mut state, width, false);
        }
        assert_eq!(state.selected, 0);
    }
}

#[test]
fn resize_preserves_the_result_anchor_and_keeps_the_last_line_reachable() {
    let mut state = state(Tool::Tls);
    state.key(Key::Escape);
    state.network_results = vec![
        "前一项".into(),
        format!("证书 {}", "域名与信任链证据".repeat(60)),
        "LAST_REPORT_ROW".into(),
    ];
    let old_width = 78;
    let new_width = 35;
    state.selected = 3;
    let old = network::rows(&state, old_width);
    let anchor = old[1..state.selected]
        .iter()
        .map(|row| row.text.chars().count())
        .sum::<usize>();
    network::resize(&mut state, old_width, new_width);
    let new = network::rows(&state, new_width);
    let start = new[1..state.selected]
        .iter()
        .map(|row| row.text.chars().count())
        .sum::<usize>();
    assert!(start <= anchor);
    assert!(start + new[state.selected].text.chars().count() > anchor);
    for _ in 0..new.len() {
        network::scroll(&mut state, new_width, true);
    }
    assert!(
        render::frame(&state, 36, 16)
            .iter()
            .any(|line| line.text.contains("LAST_REPORT_ROW"))
    );
    network::resize(&mut state, new_width, old_width);
    assert_eq!(state.selected, network::rows(&state, old_width).len() - 1);
}

fn completed_report() -> Report {
    Report {
        schema: 1,
        tool: Tool::Http,
        target: "https://example.test".into(),
        elapsed_ms: 123,
        status: "failed",
        entries: vec![Entry {
            label: "TLS".into(),
            detail: "证书过期".into(),
            success: Some(false),
            code: "tls_certificate_expired",
        }],
    }
}

#[test]
fn export_saves_the_existing_report_with_codes_status_and_measured_time() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("network.json");
    let mut state = state(Tool::Http);
    state.key(Key::Escape);
    state.network_report = Some(completed_report());
    state.key(Key::Char('e'));
    state.form.as_mut().unwrap().fields[0].value = path.display().to_string();
    network::prepare_export(&state).unwrap().save().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["schema"], 1);
    assert_eq!(value["command"], "network http");
    assert_eq!(
        value["data"],
        serde_json::to_value(state.network_report.as_ref().unwrap()).unwrap()
    );
    assert_eq!(value["data"]["status"], "failed");
    assert_eq!(value["data"]["elapsed_ms"], 123);
    assert_eq!(
        value["data"]["entries"][0]["code"],
        "tls_certificate_expired"
    );
    assert!(state.poll.is_none());
}

#[test]
fn export_failure_keeps_the_report_and_path_and_navigation_releases_the_report() {
    let directory = tempfile::tempdir().unwrap();
    let mut state = state(Tool::Http);
    state.key(Key::Escape);
    state.network_report = Some(completed_report());
    state.key(Key::Char('e'));
    state.form.as_mut().unwrap().fields[0].value = directory.path().display().to_string();
    assert!(network::prepare_export(&state).unwrap().save().is_err());
    assert_eq!(
        state.form.as_ref().unwrap().fields[0].value,
        directory.path().display().to_string()
    );
    assert_eq!(state.network_report.as_ref().unwrap().elapsed_ms, 123);
    state.key(Key::Escape);
    state.key(Key::Char('['));
    assert!(state.network_report.is_none());
}

#[test]
fn empty_and_demo_results_cannot_export_or_create_files() {
    let mut state = state(Tool::Http);
    state.key(Key::Escape);
    state.key(Key::Char('e'));
    assert!(state.form.is_none());
    assert!(state.message.as_ref().unwrap().contains("先完成一次检查"));
    state.demo = true;
    state.network_report = Some(completed_report());
    state.key(Key::Char('e'));
    assert!(state.form.is_none());
    assert!(state.message.as_ref().unwrap().contains("未导出"));
    state.form = Some(network::export_editor());
    assert!(network::prepare_export(&state).is_err());
}

#[test]
fn phone_parameter_fields_wrap_and_keep_the_input_tail_visible_after_resize() {
    let mut state = state(Tool::Http);
    let target = format!("https://example.test/{}TARGET_END", "abcd".repeat(20));
    state.form.as_mut().unwrap().fields[0].value = target.clone();
    state.key(Key::Enter);
    let frame = render::frame(&state, 36, 20);
    let selected = frame
        .iter()
        .filter(|line| line.tone == render::Tone::Selected)
        .flat_map(|line| {
            line.text
                .chars()
                .filter(|character| !character.is_whitespace())
        })
        .collect::<String>();
    assert!(selected.contains(&target));
    assert!(!selected.contains('…'));
    for (columns, rows) in [(36, 16), (40, 10), (55, 16), (100, 30)] {
        let frame = render::frame(&state, columns, rows);
        let selected = frame
            .iter()
            .filter(|line| line.tone == render::Tone::Selected)
            .flat_map(|line| {
                line.text
                    .chars()
                    .filter(|character| !character.is_whitespace())
            })
            .collect::<String>();
        assert!(selected.contains("TARGET_END▏"));
        assert!(frame.iter().any(|line| line.text.contains("网络")));
    }
}

#[test]
fn long_generic_form_edits_show_the_tail_without_revealing_secret_fields() {
    let mut state = state(Tool::Http);
    state.form.as_mut().unwrap().draft = format!("{}INPUT_END", "abcdef".repeat(600));
    state.form.as_mut().unwrap().editing = true;
    let frame = render::frame(&state, 40, 10);
    assert!(frame.iter().any(|line| line.text.contains("INPUT_END")));
    state.form.as_mut().unwrap().fields[0].secret = true;
    let frame = render::frame(&state, 40, 10);
    assert!(!frame.iter().any(|line| line.text.contains("INPUT_END")));
    assert!(frame.iter().any(|line| line.text.contains("••••••")));
}
