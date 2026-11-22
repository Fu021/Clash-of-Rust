use crate::tui::{
    input::Key,
    model::{Action, Page, State, demo_poll},
    network, render, settings,
};
use clash_client_core::config::{Language, LanguagePreference};
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
fn english_network_controls_keep_stable_options_and_validate_the_draft() {
    let mut state = State::new(false);
    state.language = Language::English;
    state.page = Page::Network;
    state.network_tool = Tool::Dns.index() as usize;
    state.query = "example.test".into();
    let page = render::frame(&state, 100, 30)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(page.contains("DNS lookup"));
    assert!(page.contains("Target: example.test"));
    assert!(page.contains("Record ADDR"));

    assert!(matches!(state.key(Key::Char('o')), Action::CancelNetwork));
    let form = state.form.as_ref().unwrap();
    assert_eq!(form.fields[0].label, "Domain");
    assert_eq!(form.fields[1].label, "Record type");
    state.form.as_mut().unwrap().fields[0].value.clear();
    let error = network::apply(&mut state).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Target must contain 1–256 bytes")
    );
    assert_eq!(state.query, "example.test");
    let form = state.form.as_mut().unwrap();
    form.fields[0].value = "new.example.test".into();
    form.fields[1].value = "AAAA".into();
    network::apply(&mut state).unwrap();
    assert_eq!(state.query, "new.example.test");
    assert_eq!(state.network_options.record_type, "AAAA");
    assert!(state.message.as_deref().unwrap().contains("press Enter"));
    state.network_tool = Tool::Http.index() as usize;
    assert!(matches!(state.key(Key::Char('o')), Action::CancelNetwork));
    state.form.as_mut().unwrap().fields[2].value = "0".into();
    assert_eq!(
        network::apply(&mut state).unwrap_err().to_string(),
        "Timeout must be between 1 and 30 seconds"
    );
    assert_eq!(state.network_options.seconds, Options::default().seconds);
    assert_eq!(state.form.as_ref().unwrap().fields[2].value, "0");
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
    state.key(Key::Left);
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
    assert_eq!(state.network_tool, Tool::Dns.index() as usize);
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
            assert!(frame.iter().any(|line| line.text.contains("工具")));
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
            detail_english: None,
        }],
    }
}

#[tokio::test]
async fn network_failure_follows_external_language_changes_and_preserves_the_error_chain() {
    use LanguagePreference::{English, SimplifiedChinese};

    let external = "上游响应：示例原文；keep punctuation";
    let timeout = tokio::time::timeout(std::time::Duration::ZERO, std::future::pending::<()>())
        .await
        .unwrap_err();
    let timeout_detail = timeout.to_string();
    for (error, chinese, english, detail) in [
        (
            anyhow::anyhow!("{external}").context("订阅下载失败"),
            "订阅下载失败",
            "Subscription download failed",
            external,
        ),
        (
            anyhow::anyhow!("{external}").context(crate::backend::Unavailable(
                "The running backend is temporarily unreachable; preserve the diagnostic state"
                    .into(),
            )),
            "存活的后台暂时不可达，请先保留排障现场",
            "The running backend is temporarily unreachable; preserve the diagnostic state",
            external,
        ),
        (
            anyhow::Error::new(timeout).context("读取当前用户后台状态超过 5 秒"),
            "读取当前用户后台状态超过 5 秒",
            "Reading the current user's backend status exceeded 5 seconds",
            &timeout_detail,
        ),
    ] {
        let mut state = state(Tool::Http);
        state.key(Key::Escape);
        let mut args = crate::args::parse(["ui"].into_iter().map(str::to_owned)).unwrap();
        let original = format!("{error:#}");
        state.network_outcome = Some(Err(error));
        network::refresh_result(&mut state);
        for (preference, label, context, changed) in [
            (SimplifiedChinese, "检查失败", chinese, false),
            (English, "Check failed", english, true),
            (SimplifiedChinese, "检查失败", chinese, true),
        ] {
            let mut poll = demo_poll();
            poll.meta.settings.language = preference;
            state.accept(Some(poll));
            assert_eq!(
                settings::sync_language(&mut state, &mut args, true, 158),
                changed
            );
            let expected = format!("{label} · {context}: {detail}");
            assert_eq!(state.network_results, [expected.as_str()]);
            assert!(
                render::frame(&state, 180, 30)
                    .iter()
                    .any(|line| line.text.contains(&expected))
            );
            let error = state
                .network_outcome
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap_err();
            assert_eq!(format!("{error:#}"), original);
            assert_eq!(error.root_cause().to_string(), detail);
            assert!(state.network_report().is_none());
            network::scroll(&mut state, 24, true);
        }
        state.key(Key::Char('e'));
        assert!(state.form.is_none());
        state.key(Key::Escape);
        let mut poll = demo_poll();
        poll.meta.settings.language = English;
        state.accept(Some(poll));
        assert!(settings::sync_language(&mut state, &mut args, true, 158));
        assert!(state.network_outcome.is_none());
        assert!(state.network_results.is_empty());
    }
}

#[test]
fn external_language_changes_preserve_the_network_report_and_open_export_draft() {
    let mut state = state(Tool::Http);
    state.key(Key::Escape);
    let mut args = crate::args::parse(["ui"].into_iter().map(str::to_owned)).unwrap();
    let report = completed_report();
    let original = serde_json::to_value(&report).unwrap();
    state.network_outcome = Some(Ok(report));
    network::refresh_result(&mut state);
    assert!(state.network_results[0].starts_with("检查状态 · 失败"));

    let path = "示例：未保存报告.json";
    state.key(Key::Char('e'));
    state.key(Key::Enter);
    state.key(Key::Clear);
    state.key(Key::Paste(path.into()));
    let mut poll = demo_poll();
    poll.meta.settings.language = LanguagePreference::English;
    state.accept(Some(poll));
    assert!(!settings::sync_language(&mut state, &mut args, true, 158));
    assert_eq!(state.language, Language::SimplifiedChinese);
    assert_eq!(state.form.as_ref().unwrap().draft, path);
    assert!(state.form.as_ref().unwrap().editing);
    assert!(state.network_results[0].starts_with("检查状态 · 失败"));

    state.key(Key::Enter);
    assert_eq!(state.form.as_ref().unwrap().fields[0].value, path);
    assert!(!settings::sync_language(&mut state, &mut args, true, 158));
    state.key(Key::Escape);
    assert!(settings::sync_language(&mut state, &mut args, true, 158));
    assert!(state.network_results[0].starts_with("Status · Failed"));
    assert_eq!(
        serde_json::to_value(state.network_report().unwrap()).unwrap(),
        original
    );

    let mut poll = demo_poll();
    poll.meta.settings.language = LanguagePreference::SimplifiedChinese;
    state.accept(Some(poll));
    assert!(settings::sync_language(&mut state, &mut args, true, 158));
    assert!(state.network_results[0].starts_with("检查状态 · 失败"));
    assert_eq!(
        serde_json::to_value(state.network_report().unwrap()).unwrap(),
        original
    );
}

#[test]
fn language_switch_keeps_the_same_network_entry_at_the_top() {
    let mut state = state(Tool::Http);
    state.key(Key::Escape);
    let mut args = crate::args::parse(["ui"].into_iter().map(str::to_owned)).unwrap();
    let mut report = completed_report();
    report.entries[0].detail_english = Some("Preserve the diagnostic details. ".repeat(8));
    report.entries.push(Entry {
        label: "EVIDENCE".into(),
        detail: "external".into(),
        success: None,
        code: "information",
        detail_english: None,
    });
    state.network_outcome = Some(Ok(report));
    network::refresh_result(&mut state);
    let width = 31;
    let anchor = "EVIDENCE · external";
    state.selected = network::rows(&state, width)
        .iter()
        .position(|row| row.text == anchor)
        .unwrap();
    let chinese_offset = state.selected;
    for preference in [
        LanguagePreference::English,
        LanguagePreference::SimplifiedChinese,
    ] {
        state.language_preference = preference;
        assert!(settings::sync_language(&mut state, &mut args, true, width));
        assert_eq!(network::rows(&state, width)[state.selected].text, anchor);
        if preference == LanguagePreference::English {
            assert!(state.selected > chinese_offset);
        } else {
            assert_eq!(state.selected, chinese_offset);
        }
    }
}

#[test]
fn export_saves_the_existing_report_with_codes_status_and_measured_time() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("network.json");
    let mut state = state(Tool::Http);
    state.key(Key::Escape);
    state.network_outcome = Some(Ok(completed_report()));
    state.key(Key::Char('e'));
    state.form.as_mut().unwrap().fields[0].value = path.display().to_string();
    network::prepare_export(&state).unwrap().save().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["schema"], 1);
    assert_eq!(value["command"], "network http");
    assert_eq!(
        value["data"],
        serde_json::to_value(state.network_report().unwrap()).unwrap()
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
    let destination = directory.path().join("用户路径：保存文件失败");
    std::fs::create_dir(&destination).unwrap();
    let mut state = state(Tool::Http);
    state.key(Key::Escape);
    state.network_outcome = Some(Ok(completed_report()));
    state.key(Key::Char('e'));
    let path = destination.display().to_string();
    state.form.as_mut().unwrap().fields[0].value = path.clone();
    let error = network::prepare_export(&state).unwrap().save().unwrap_err();
    let system_detail = error.root_cause().to_string();
    assert_eq!(
        network::export_failure(&error, Language::English),
        format!(
            "Export failed; path and report retained: Could not save the file: {path}: {system_detail}"
        )
    );
    assert_eq!(
        network::export_failure(&error, Language::SimplifiedChinese),
        format!("导出失败，路径和报告保留：保存文件失败：{path}: {system_detail}")
    );
    assert_eq!(state.form.as_ref().unwrap().fields[0].value, path);
    assert_eq!(state.network_report().unwrap().elapsed_ms, 123);
    state.key(Key::Escape);
    state.key(Key::Char('['));
    assert!(state.network_report().is_none());
}

#[test]
fn empty_and_demo_results_cannot_export_or_create_files() {
    let mut state = state(Tool::Http);
    state.key(Key::Escape);
    state.key(Key::Char('e'));
    assert!(state.form.is_none());
    assert!(state.message.as_ref().unwrap().contains("先完成一次检查"));
    state.demo = true;
    state.network_outcome = Some(Ok(completed_report()));
    state.key(Key::Char('e'));
    assert!(state.form.is_none());
    assert!(state.message.as_ref().unwrap().contains("未导出"));
    state.form = Some(network::export_editor(state.language));
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
        assert!(frame.iter().any(|line| line.text.contains("工具")));
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
