use crate::tui::{
    form::{self, Form},
    input::Key,
    model::{Action, Page, State, Target, demo_poll},
    panels, render,
};

fn context() -> crate::args::Args {
    crate::args::parse(
        [
            "ui",
            "--data-dir",
            "/tmp/test-only",
            "--resources",
            "/tmp/resources",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .unwrap()
}

#[test]
fn automatic_member_changes_do_not_invalidate_the_parent_selector() {
    let mut poll = demo_poll();
    let target = Target {
        profile: None,
        group: "PROXY".into(),
        node: "AUTO".into(),
    };
    target.validate(&poll).unwrap();
    poll.snapshot.proxies.proxies.get_mut("AUTO").unwrap().now = "🇯🇵 日本 01".into();
    target.validate(&poll).unwrap();
    let automatic = Target {
        profile: None,
        group: "AUTO".into(),
        node: "🇯🇵 日本 01".into(),
    };
    let error = automatic.validate(&poll).unwrap_err().to_string();
    assert!(error.contains("自动策略组"));
    assert!(!error.contains("变化"));
    poll.snapshot
        .proxies
        .proxies
        .get_mut("PROXY")
        .unwrap()
        .all
        .retain(|node| node.as_str() != "AUTO");
    assert!(
        target
            .validate(&poll)
            .unwrap_err()
            .to_string()
            .contains("已不在")
    );
}

#[test]
fn automatic_groups_do_not_offer_invalid_manual_actions() {
    let mut state = State::new(true);
    state.page = Page::Nodes;
    state.group = Some("AUTO".into());
    state.key(Key::Enter);
    assert!(state.confirmation.is_none());
    assert!(state.message.is_none());
}

#[test]
fn node_sort_cycles_preserve_selection_and_show_only_the_saved_mode() {
    use clash_client_core::proxy_order::NodeSort;

    let mut state = State::new(true);
    state.page = Page::Nodes;
    state.group = Some("PROXY".into());
    assert_eq!(
        state.items(),
        ["DIRECT", "🇺🇸 美国 02", "🇯🇵 日本 01", "AUTO"]
    );
    state.selected = 1;
    assert!(matches!(state.key(Key::Char('s')), Action::None));
    assert_eq!(
        state.poll.as_ref().unwrap().meta.settings.tui_node_sort,
        NodeSort::LatencyDescending
    );
    assert_eq!(state.items()[1], "🇯🇵 日本 01");
    assert_eq!(state.items()[state.selected], "🇺🇸 美国 02");
    state.key(Key::Char('s'));
    assert_eq!(
        state.poll.as_ref().unwrap().meta.settings.tui_node_sort,
        NodeSort::Name
    );
    assert_eq!(state.items()[state.selected], "🇺🇸 美国 02");
    assert!(
        render::frame(&state, 80, 24)
            .iter()
            .any(|line| line.text.contains("排序：名称排序"))
    );
    state.key(Key::Char('s'));
    assert_eq!(state.items()[state.selected], "🇺🇸 美国 02");
    state.key(Key::Char('/'));
    state.key(Key::Paste("美国".into()));
    assert_eq!(state.items()[state.selected], "🇺🇸 美国 02");
    state.key(Key::Clear);
    state.key(Key::Escape);
    assert_eq!(state.items()[state.selected], "🇺🇸 美国 02");
    let mut next = demo_poll();
    next.snapshot
        .proxies
        .proxies
        .get_mut("🇯🇵 日本 01")
        .unwrap()
        .delay = Some(10);
    state.accept(Some(next));
    assert_eq!(state.items()[state.selected], "🇺🇸 美国 02");
    assert_eq!(state.items()[1], "🇯🇵 日本 01");

    let mut connected = State::new(false);
    connected.accept(Some(demo_poll()));
    connected.page = Page::Nodes;
    assert!(matches!(
        connected.key(Key::Char('s')),
        Action::SetNodeSort(NodeSort::LatencyDescending)
    ));
    assert_eq!(
        connected.poll.as_ref().unwrap().meta.settings.tui_node_sort,
        NodeSort::LatencyAscending
    );
    connected.group = Some("PROXY".into());
    connected.selected = 1;
    let mut saved = demo_poll();
    saved.meta.settings.tui_node_sort = NodeSort::Name;
    connected.accept(Some(saved));
    assert_eq!(connected.items()[connected.selected], "🇺🇸 美国 02");
    assert!(matches!(
        connected.key(Key::Char('s')),
        Action::SetNodeSort(NodeSort::LatencyAscending)
    ));
    let frame = render::frame(&connected, 60, 18);
    assert!(frame.iter().any(|line| line.text.contains("[s 切换排序]")));
    connected.key(Key::Char('/'));
    connected.key(Key::Char('s'));
    connected.key(Key::Char('n'));
    assert_eq!(connected.query, "sn");
    assert!(connected.items().is_empty());
    connected.key(Key::Escape);
    connected.key(Key::Back);
    assert!(connected.query.is_empty());
    assert!(connected.group.is_none());
    connected.failure("offline".into());
    assert!(matches!(connected.key(Key::Char('s')), Action::None));
    assert_eq!(
        connected.poll.as_ref().unwrap().meta.settings.tui_node_sort,
        NodeSort::Name
    );
}

#[test]
fn settings_and_operations_follow_the_shared_command_catalogue() {
    let mut state = State::new(true);
    state.page = Page::Settings;
    state.poll.as_mut().unwrap().meta.profiles = vec![clash_client_core::config::Profile {
        id: "test-profile".into(),
        name: "测试订阅".into(),
        source: "https://example.test/sub".into(),
        updated: 0,
        usage: None,
        builtin: false,
    }];
    state.poll.as_mut().unwrap().meta.settings.active_profile = Some("test-profile".into());
    assert_eq!(state.items(), crate::tui::settings::KEYS);
    for key in crate::tui::settings::KEYS {
        state.query = (*key).into();
        state.selected = state.items().iter().position(|item| item == key).unwrap();
        state.key(Key::Enter);
        let form = state.form.take().unwrap();
        assert_eq!(form.setting.as_deref(), Some(*key));
        let args = form.args(&context()).unwrap();
        assert!(
            crate::args::COMMANDS
                .iter()
                .any(|spec| spec.path == args.path)
        );
        if args.path == "config set" {
            assert!(clash_client_core::config::MUTABLE_SETTINGS.contains(key));
            assert_eq!(form.fields[0].value, *key);
        }
    }
    for excluded in [
        "theme_mode",
        "accent_color",
        "text_scale",
        "tray_speed",
        "dark",
        "node_sort",
    ] {
        state.query = excluded.into();
        assert!(state.items().is_empty(), "{excluded}");
    }
    state.query.clear();
    state.page = Page::Tools;
    let items = state.items();
    for spec in crate::args::COMMANDS {
        assert_eq!(
            items.contains(&spec.path),
            !matches!(spec.path, "run" | "ui") && !spec.retired(),
            "{}",
            spec.path
        );
    }
    for spec in form::commands() {
        let form = Form::new(spec);
        assert_eq!(
            form.fields
                .iter()
                .filter(|field| field.option.is_none())
                .count(),
            spec.max,
            "{}",
            spec.path
        );
        assert_eq!(
            form.fields
                .iter()
                .filter_map(|field| field.option)
                .collect::<Vec<_>>(),
            spec.options.split_whitespace().collect::<Vec<_>>(),
            "{}",
            spec.path
        );
    }
}

#[test]
fn form_values_preserve_unicode_literals_and_original_account_without_shell_parsing() {
    let spec = form::commands()
        .find(|spec| spec.path == "node select")
        .unwrap();
    let mut form = Form::new(spec);
    form.fields[0].value = "PROXY".into();
    form.fields[1].value = "--🇰🇷 韩国 $(literal) 'quoted'".into();
    let args = form.args(&context()).unwrap();
    assert_eq!(args.path, "node select");
    assert_eq!(args.positional, ["PROXY", "--🇰🇷 韩国 $(literal) 'quoted'"]);
    assert_eq!(args.get("data-dir"), Some("/tmp/test-only"));
    assert_eq!(args.get("resources"), Some("/tmp/resources"));
    assert!(!args.flag("yes"));
    form.fields[0].value.clear();
    assert!(form.args(&context()).is_err());
}

#[test]
fn forms_reject_missing_required_values_bad_flags_and_incomplete_export_paths() {
    let mut form = Form::new(
        form::commands()
            .find(|spec| spec.path == "logs export")
            .unwrap(),
    );
    assert!(form.args(&context()).is_err());
    form.fields[0].value = "/tmp/log with spaces.json".into();
    let args = form.args(&context()).unwrap();
    assert_eq!(args.get("output"), Some("/tmp/log with spaces.json"));
    let mut form = Form::new(
        form::commands()
            .find(|spec| spec.path == "profile update")
            .unwrap(),
    );
    let all = form
        .fields
        .iter_mut()
        .find(|field| field.option == Some("all"))
        .unwrap();
    all.value = "maybe".into();
    assert!(form.args(&context()).is_err());
    form.fields
        .iter_mut()
        .find(|field| field.option == Some("all"))
        .unwrap()
        .value = "true".into();
    assert!(form.args(&context()).unwrap().flag("all"));
    form.fields[0].value = "profile".into();
    assert!(form.args(&context()).is_err());
}

#[test]
fn setting_edit_cancel_confirm_demo_and_input_bounds_are_independent_of_shortcuts() {
    let mut state = State::new(true);
    state.page = Page::Settings;
    state.query = "fake_ip_exclusions".into();
    state.key(Key::Enter);
    state.key(Key::Enter);
    assert!(state.entering_text());
    state.key(Key::Char('q'));
    state.key(Key::Char('6'));
    assert!(state.page == Page::Settings);
    state.key(Key::Escape);
    assert_eq!(state.form.as_ref().unwrap().fields[1].value, "[]");
    state.key(Key::Enter);
    state.key(Key::Clear);
    assert!(state.form.as_ref().unwrap().draft.is_empty());
    state.key(Key::Paste("日".repeat(10000)));
    assert!(state.form.as_ref().unwrap().draft.len() <= 8192);
    state.key(Key::Escape);
    state.form.as_mut().unwrap().fields[1].value = "example.test".into();
    state.form.as_mut().unwrap().selected = 2;
    assert!(matches!(state.key(Key::Enter), Action::ReviewForm));
    state.execution = Some(state.form.as_ref().unwrap().args(&context()).unwrap());
    state.key(Key::Escape);
    assert!(state.execution.is_none());
    state.execution = Some(state.form.as_ref().unwrap().args(&context()).unwrap());
    assert!(matches!(state.key(Key::Enter), Action::None));
    assert!(state.message.as_deref().unwrap().contains("演示模式"));
    state.release_page();
    assert!(state.form.is_none());
    assert!(state.execution.is_none());
}

#[test]
fn connection_details_keep_identity_when_a_connection_disappears() {
    let mut state = State::new(true);
    state.page = Page::Connections;
    state.key(Key::Enter);
    assert_eq!(state.connection.as_deref(), Some("demo-1"));
    assert!(
        panels::connection(&state, 80, 24)
            .iter()
            .any(|(text, _)| text.contains("AUTO → 🇺🇸"))
    );
    state.key(Key::Char('x'));
    let form = state.form.take().unwrap();
    assert_eq!(form.spec.path, "connection close");
    assert_eq!(form.args(&context()).unwrap().positional, ["demo-1"]);
    state
        .poll
        .as_mut()
        .unwrap()
        .snapshot
        .connections
        .connections
        .remove(0);
    let lines = panels::connection(&state, 80, 24);
    assert!(lines.iter().any(|(text, _)| text.contains("已关闭")));
    assert!(!lines.iter().any(|(text, _)| text.contains("example.com")));
    state.key(Key::Escape);
    assert!(state.connection.is_none());
}

#[test]
fn settings_secrets_and_large_rule_collections_do_not_leak_into_frames() {
    let mut state = State::new(true);
    state.page = Page::Settings;
    state.poll.as_mut().unwrap().meta.settings.secret = "must-not-render-token".into();
    let text = render::frame(&state, 120, 35)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("must-not-render-token"));
    assert!(!state.items().contains(&"secret"));
    state.form = Some(Form::setting("secret", &serde_json::json!("")));
    state.form.as_mut().unwrap().fields[1].value = "new-hidden-token".into();
    state.form.as_mut().unwrap().selected = 0;
    state.form.as_mut().unwrap().begin_edit();
    assert!(!state.form.as_ref().unwrap().editing);
    let text = render::frame(&state, 120, 35)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("new-hidden-token"));
    for key in clash_client_core::config::READABLE_SETTINGS {
        if !matches!(*key, "theme_mode" | "active_profile") {
            assert!(
                !form::setting_value(&state.poll.as_ref().unwrap().meta.settings, key).is_null(),
                "{key}"
            );
        }
    }
}

#[test]
fn polling_preserves_global_setting_drafts_and_invalidates_profile_operations() {
    let mut state = State::new(true);
    state.page = Page::Settings;
    state.query = "fake_ip_exclusions".into();
    state.key(Key::Enter);
    state.form.as_mut().unwrap().fields[1].value = "draft.test".into();
    let mut poll = demo_poll();
    poll.snapshot.proxies.proxies.get_mut("AUTO").unwrap().now = "🇯🇵 日本 01".into();
    state.accept(Some(poll));
    assert_eq!(state.form.as_ref().unwrap().fields[1].value, "draft.test");
    state.execution = Some(state.form.as_ref().unwrap().args(&context()).unwrap());
    let mut changed = demo_poll();
    changed.meta.settings.active_profile = Some("another-profile".into());
    state.accept(Some(changed));
    assert_eq!(state.form.as_ref().unwrap().fields[1].value, "draft.test");
    assert!(state.execution.is_some());
    state.form = Some(Form::new(
        form::commands()
            .find(|spec| spec.path == "profile edit")
            .unwrap(),
    ));
    state.accept(Some(demo_poll()));
    assert!(state.form.is_none());
    assert!(state.execution.is_none());
}

#[test]
fn every_management_form_fits_supported_terminal_sizes_and_keeps_execution_visible() {
    use unicode_width::UnicodeWidthStr;
    for spec in form::commands() {
        for (columns, rows) in [
            (40, 1),
            (40, 2),
            (32, 8),
            (40, 10),
            (55, 16),
            (60, 18),
            (80, 24),
            (120, 40),
        ] {
            let mut state = State::new(true);
            state.page = Page::Tools;
            let mut form = Form::new(spec);
            form.selected = form.fields.len();
            state.form = Some(form);
            let frame = render::frame(&state, columns, rows);
            assert!(frame.len() <= rows, "{}", spec.path);
            assert!(
                frame.iter().any(|line| line.text.contains("检查并执行")),
                "{}",
                spec.path
            );
            assert!(frame.iter().all(|line| line.text.width() < columns));
        }
    }
}
