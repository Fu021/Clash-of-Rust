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
        if *key == "fake_ip_exclusions" {
            assert!(state.fake_ip.take().is_some());
            assert!(state.form.is_none());
            continue;
        }
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
        assert!(!state.items().contains(&excluded), "{excluded}");
    }
    state.query = "node_sort".into();
    assert_eq!(state.items(), ["tui_node_sort"]);
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
fn command_forms_label_options_required_by_the_shared_parser() {
    for spec in form::commands() {
        let form = Form::new(spec);
        for (key, _) in spec.required_options() {
            let field = form
                .fields
                .iter()
                .find(|field| field.option == Some(*key))
                .unwrap();
            assert!(field.label.contains("必填"), "{} --{key}", spec.path);
            assert!(form.args(&context()).is_err(), "{} --{key}", spec.path);
        }
    }
}

#[test]
fn english_forms_localize_labels_and_validation_without_changing_arguments() {
    use clash_client_core::config::Language;

    let spec = form::commands()
        .find(|spec| spec.path == "profile update")
        .unwrap();
    let mut form = Form::new_in(spec, Language::English);
    let all = form
        .fields
        .iter_mut()
        .find(|field| field.option == Some("all"))
        .unwrap();
    assert!(all.label.contains("true to enable"));
    all.value = "maybe".into();
    assert!(
        form.args(&context())
            .err()
            .unwrap()
            .to_string()
            .contains("must be true")
    );
    form.fields
        .iter_mut()
        .find(|field| field.option == Some("all"))
        .unwrap()
        .value = "true".into();
    assert!(form.args(&context()).unwrap().flag("all"));
}

#[test]
fn profile_changes_release_the_previous_sources_notice_without_an_open_draft() {
    let mut state = State::new(false);
    state.page = Page::Rules;
    let mut poll = demo_poll();
    poll.meta.settings.active_profile = Some("included-profile".into());
    state.accept(Some(poll));
    state.message = Some("订阅已包含此规则，无需重复添加".into());
    let mut next = demo_poll();
    next.meta.settings.active_profile = Some("plain-profile".into());
    state.accept(Some(next));
    assert!(state.message.is_none());
    assert!(state.error.is_none());
    assert!(state.form.is_none());
}

#[test]
fn desktop_operation_rows_keep_the_complete_command_name_visible() {
    let mut state = State::new(true);
    state.page = Page::Tools;
    for spec in form::commands() {
        state.query = spec.path.into();
        state.selected = state
            .items()
            .iter()
            .position(|path| *path == spec.path)
            .unwrap();
        for (columns, rows) in [(80, 24), (100, 30), (120, 40)] {
            let frame = render::frame(&state, columns, rows);
            let selected = frame
                .iter()
                .find(|line| line.tone == render::Tone::Selected)
                .unwrap();
            assert!(
                selected.text.trim_end().ends_with(spec.path),
                "{} at {columns}×{rows}: {}",
                spec.path,
                selected.text
            );
        }
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
    state.query = "mixed_port".into();
    state.key(Key::Enter);
    state.key(Key::Enter);
    assert!(state.entering_text());
    state.key(Key::Char('q'));
    state.key(Key::Char('8'));
    assert!(state.page == Page::Settings);
    state.key(Key::Escape);
    assert_eq!(
        state.form.as_ref().unwrap().fields[1].value,
        state
            .poll
            .as_ref()
            .unwrap()
            .meta
            .settings
            .mixed_port
            .to_string()
    );
    state.key(Key::Enter);
    state.key(Key::Clear);
    assert!(state.form.as_ref().unwrap().draft.is_empty());
    state.key(Key::Paste("日".repeat(10000)));
    assert!(state.form.as_ref().unwrap().draft.len() <= 8192);
    state.key(Key::Escape);
    state.form.as_mut().unwrap().fields[1].value = "12345".into();
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
fn english_node_and_connection_pages_keep_names_and_live_identity() {
    let mut state = State::new(true);
    state.language = clash_client_core::config::Language::English;
    state.page = Page::Nodes;
    state.group = Some("PROXY".into());
    let nodes = render::frame(&state, 120, 30)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(nodes.contains("[s change sort] Sort:"), "{nodes}");
    assert!(nodes.contains("🇺🇸 美国 02"), "{nodes}");

    state.page = Page::Connections;
    state.group = None;
    state.selected = 0;
    state.key(Key::Enter);
    let details = panels::connection(&state, 120, 30)
        .into_iter()
        .map(|(text, _)| text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(details.contains("Connection details"), "{details}");
    assert!(
        details.contains("Proxy chain: PROXY → AUTO → 🇺🇸"),
        "{details}"
    );
    assert!(details.contains("Connection ID: demo-1"), "{details}");

    state
        .poll
        .as_mut()
        .unwrap()
        .snapshot
        .connections
        .connections
        .clear();
    let missing = panels::connection(&state, 120, 30)
        .into_iter()
        .map(|(text, _)| text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        missing.contains("Connection demo-1 has closed"),
        "{missing}"
    );
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
    state.form = Some(Form::setting(
        "secret",
        &serde_json::json!(""),
        state.language,
    ));
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
    state.query = "mixed_port".into();
    state.key(Key::Enter);
    state.form.as_mut().unwrap().fields[1].value = "12345".into();
    let mut poll = demo_poll();
    poll.snapshot.proxies.proxies.get_mut("AUTO").unwrap().now = "🇯🇵 日本 01".into();
    state.accept(Some(poll));
    assert_eq!(state.form.as_ref().unwrap().fields[1].value, "12345");
    state.execution = Some(state.form.as_ref().unwrap().args(&context()).unwrap());
    let mut changed = demo_poll();
    changed.meta.settings.active_profile = Some("another-profile".into());
    state.accept(Some(changed));
    assert_eq!(state.form.as_ref().unwrap().fields[1].value, "12345");
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

#[test]
fn wrapped_operation_rows_preserve_descriptions_commands_and_keyboard_selection() {
    let compact = |text: &str| {
        text.chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>()
    };
    let mut state = State::new(true);
    state.page = Page::Tools;
    let count = state.items().len();
    for index in [0, count / 2, count - 1] {
        state.selected = index;
        let path = state.items()[index].to_owned();
        let spec = form::commands().find(|spec| spec.path == path).unwrap();
        for columns in [36, 40, 60, 120] {
            let frame = render::frame(&state, columns, 24);
            let selected = frame
                .iter()
                .filter(|line| line.tone == render::Tone::Selected)
                .map(|line| line.text.as_str())
                .collect::<String>();
            let selected = compact(&selected);
            assert!(
                selected.contains(&compact(spec.description)),
                "{columns}: {selected}"
            );
            assert!(selected.contains(&compact(&path)), "{columns}: {selected}");
            assert!(!selected.contains('…'), "{columns}: {selected}");
        }
        state.key(Key::Enter);
        assert_eq!(state.form.as_ref().unwrap().spec.path, path);
        state.key(Key::Escape);
    }
}

#[test]
fn profile_list_localizes_only_the_builtin_default_name() {
    use clash_client_core::config::{Language, Profile};

    let mut state = State::new(true);
    state.page = Page::Profiles;
    let poll = state.poll.as_mut().unwrap();
    poll.meta.settings.active_profile = Some("builtin".into());
    poll.meta.profiles = [
        ("builtin", "默认直连", "/resources/default.yaml", true),
        ("imported", "默认直连", "/imports/default.yaml", false),
        ("remote", "旅行节点", "https://example.test/sub", false),
        ("renamed", "自定义直连", "/resources/default.yaml", true),
    ]
    .into_iter()
    .map(|(id, name, source, builtin)| Profile {
        id: id.into(),
        name: name.into(),
        source: source.into(),
        updated: 0,
        usage: None,
        builtin,
    })
    .collect();

    for (language, expected) in [
        (
            Language::English,
            [
                "[Current] Default direct · Built-in",
                "默认直连 · Local",
                "旅行节点 · Remote",
                "自定义直连 · Built-in",
            ],
        ),
        (
            Language::SimplifiedChinese,
            [
                "[当前] 默认直连 · 内置",
                "默认直连 · 本地",
                "旅行节点 · 远程",
                "自定义直连 · 内置",
            ],
        ),
    ] {
        state.language = language;
        let frame = render::frame(&state, 120, 30);
        for text in expected {
            assert!(frame.iter().any(|line| line.text.contains(text)), "{text}");
        }
        assert_eq!(state.items(), ["builtin", "imported", "remote", "renamed"]);
        state.key(Key::Enter);
        let form = state.form.as_ref().unwrap();
        assert_eq!(form.fields[0].value, "builtin");
        assert_eq!(
            form.visible_value(0),
            language.text("默认直连", "Default direct")
        );
        state.key(Key::Right);
        let form = state.form.as_ref().unwrap();
        assert_eq!(form.fields[0].value, "imported");
        assert_eq!(form.visible_value(0), "默认直连");
        state.key(Key::Escape);
    }
    state.language = Language::English;
    state.key(Key::Char('/'));
    state.key(Key::Paste("默认直连".into()));
    assert_eq!(state.items(), ["builtin", "imported"]);
    state.key(Key::Clear);
    state.key(Key::Paste("dEfAuLt DiReCt".into()));
    assert_eq!(state.items(), ["builtin"]);
    state.language = Language::SimplifiedChinese;
    assert!(state.items().is_empty());
    state.language = Language::English;
    state.key(Key::Clear);
    state.key(Key::Paste("IMPORTED".into()));
    assert_eq!(state.items(), ["imported"]);
}

#[test]
fn profile_list_keeps_localized_usage_and_expiry_visible_beside_long_names() {
    use chrono::{DateTime, Local};
    use clash_client_core::config::{Language, LanguagePreference, Profile};
    use unicode_width::UnicodeWidthStr;

    let mut state = State::new(true);
    state.page = Page::Profiles;
    let long_name = "跨地域远程线路".repeat(7);
    let expiry_timestamp = 1813147200;
    let expiry_date = DateTime::from_timestamp(expiry_timestamp, 0)
        .unwrap()
        .with_timezone(&Local)
        .format("%Y-%m-%d");
    let no_expiry = "upload=1048576; download=2147483648; total=3221225472; expire=0";
    let with_expiry = no_expiry.replace("expire=0", &format!("expire={expiry_timestamp}"));
    let profiles: Vec<_> = [
        ("旅行节点", Some(no_expiry.into())),
        ("限期订阅", Some(format!("expire={expiry_timestamp}"))),
        ("备用订阅", None),
        (long_name.as_str(), Some(with_expiry)),
    ]
    .into_iter()
    .map(|(name, usage)| Profile {
        id: name.into(),
        name: name.into(),
        source: "https://example.test/sub".into(),
        updated: 0,
        usage,
        builtin: false,
    })
    .collect();
    state.poll.as_mut().unwrap().meta.profiles = profiles.clone();
    let mut args = crate::args::parse(["ui"].map(str::to_owned)).unwrap();

    for (preference, language) in [
        (LanguagePreference::English, Language::English),
        (
            LanguagePreference::SimplifiedChinese,
            Language::SimplifiedChinese,
        ),
        (LanguagePreference::English, Language::English),
    ] {
        state.language_preference = preference;
        crate::tui::settings::sync_language(&mut state, &mut args, true, 31);
        assert_eq!(state.language, language);
        let remote = language.text("远程", "Remote");
        let expected = [
            language
                .text("已用 2/3 GiB · 不限期", "Used 2/3 GiB · No expiry")
                .into(),
            format!("{} {expiry_date}", language.text("到期", "Expires")),
            language
                .text("未提供用量与到期", "Usage and expiry unavailable")
                .into(),
        ];
        for columns in [32, 120] {
            let frame = render::frame(&state, columns, 30);
            assert_eq!(frame.len(), 30);
            assert!(frame.iter().all(|line| line.text.width() < columns));
            for (profile, summary) in profiles.iter().zip(&expected) {
                let prefix = format!("{} · {remote}", profile.name);
                let row = if columns == 120 {
                    format!("{prefix} · {summary}")
                } else {
                    prefix
                };
                assert!(
                    frame
                        .iter()
                        .any(|line| line.text.trim_end().ends_with(&row)),
                    "{columns}: {row}"
                );
                if columns == 32 {
                    assert!(
                        frame.iter().any(|line| line.text.trim_end() == summary),
                        "{columns}: {summary}"
                    );
                }
            }
        }
        state.selected = profiles.len() - 1;
        for (columns, rows) in [(32, 5), (40, 24), (120, 30)] {
            let frame = render::frame(&state, columns, rows);
            assert_eq!(frame.len(), rows);
            assert!(frame.iter().all(|line| line.text.width() < columns));
            let selected = frame
                .iter()
                .filter(|line| line.tone == render::Tone::Selected)
                .map(|line| line.text.trim())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(selected.starts_with("› 跨地域"), "{selected}");
            assert!(
                selected.contains(language.text("已用 2/3 GiB", "Used 2/3 GiB")),
                "{selected}"
            );
            assert!(selected.contains(&expected[1]), "{selected}");
            assert_eq!(selected.contains(&long_name), columns == 120, "{selected}");
        }
        state.key(Key::Enter);
        let form = state.form.as_ref().unwrap();
        assert_eq!(form.fields[0].value, long_name);
        assert_eq!(form.visible_value(0), long_name);
        state.key(Key::Escape);
        state.selected = 0;
    }
    assert_eq!(state.poll.as_ref().unwrap().meta.profiles, profiles);
}

#[test]
fn short_profile_list_keeps_partial_usage_expiry_and_update_visible() {
    use clash_client_core::config::{Language, Profile};

    let mut state = State::new(true);
    state.page = Page::Profiles;
    let usage = "upload=1048576; total=3221225472; expire=1813147200";
    let updated = 1795392000;
    let name = "Selected 跨地域远程线路".repeat(7);
    state.poll.as_mut().unwrap().meta.profiles = vec![
        Profile {
            id: "other".into(),
            name: "Other".into(),
            source: "https://example.test/other".into(),
            updated: 0,
            usage: None,
            builtin: false,
        },
        Profile {
            id: "selected".into(),
            name: name.clone(),
            source: "https://example.test/selected".into(),
            updated,
            usage: Some(usage.into()),
            builtin: false,
        },
    ];
    state.selected = 1;

    for language in [Language::English, Language::SimplifiedChinese] {
        state.language = language;
        for columns in [32, 44] {
            let frame = render::frame(&state, columns, 5);
            let selected = frame
                .iter()
                .filter(|line| line.tone == render::Tone::Selected)
                .map(|line| line.text.trim())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(selected.contains("Selected"), "{columns}: {selected}");
            let updated_prefix = language.text("更新", "Updated");
            let summary = clash_client_core::subscription::summary(Some(usage), updated, language);
            for part in summary
                .split(" · ")
                .filter(|part| !part.starts_with(updated_prefix))
            {
                assert!(
                    selected.contains(part),
                    "{columns}: {part} missing from {selected}"
                );
            }
        }
    }
}

#[test]
fn very_short_profile_list_prioritizes_expiry_over_update_after_language_switch() {
    use clash_client_core::config::{Language, LanguagePreference, Profile};
    use unicode_width::UnicodeWidthStr;

    let mut state = State::new(true);
    state.page = Page::Profiles;
    let name = "旅行🌍Travel".repeat(8);
    let usage = "expire=1813147200";
    let profiles = vec![Profile {
        id: "travel".into(),
        name: name.clone(),
        source: "https://example.test/travel".into(),
        updated: 1795392000,
        usage: Some(usage.into()),
        builtin: false,
    }];
    state.poll.as_mut().unwrap().meta.profiles = profiles.clone();
    let mut args = crate::args::parse(["ui"].map(str::to_owned)).unwrap();

    for (preference, language) in [
        (LanguagePreference::English, Language::English),
        (
            LanguagePreference::SimplifiedChinese,
            Language::SimplifiedChinese,
        ),
        (LanguagePreference::English, Language::English),
    ] {
        state.language_preference = preference;
        crate::tui::settings::sync_language(&mut state, &mut args, true, 31);
        assert_eq!(state.language, language);
        assert_eq!(state.items()[state.selected], "travel");
        let expiry = clash_client_core::subscription::summary(Some(usage), 0, language);
        let frame = render::frame(&state, 32, 4);
        let selected = frame
            .iter()
            .filter(|line| line.tone == render::Tone::Selected)
            .map(|line| line.text.trim_end())
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 2);
        assert!(selected[0].starts_with("› 旅行🌍Travel"), "{selected:?}");
        assert_eq!(selected[1], expiry);
        assert!(frame.iter().all(|line| line.text.width() < 32));
    }
    state.poll.as_mut().unwrap().meta.settings.active_profile = Some("travel".into());
    let expiry = clash_client_core::subscription::summary(Some(usage), 0, Language::English);
    for (columns, title, date) in [
        (13, "› 旅行", expiry.strip_prefix("Expires ").unwrap()),
        (24, "› [Current] 旅行", expiry.as_str()),
    ] {
        let frame = render::frame(&state, columns, 4);
        assert!(
            frame.iter().any(|line| {
                line.tone == render::Tone::Selected && line.text.starts_with(title)
            })
        );
        assert!(
            frame.iter().any(|line| {
                line.tone == render::Tone::Selected && line.text.trim_end() == date
            })
        );
    }
    state.key(Key::Enter);
    let form = state.form.as_ref().unwrap();
    assert_eq!(form.fields[0].value, "travel");
    assert_eq!(form.visible_value(0), name);
    assert_eq!(state.poll.as_ref().unwrap().meta.profiles, profiles);
}

#[test]
fn profile_scroll_never_displays_expiry_without_its_subscription_title() {
    use clash_client_core::config::{Language, Profile};

    let mut state = State::new(true);
    state.page = Page::Profiles;
    state.language = Language::English;
    let usage = "upload=1048576; download=2147483648; total=3221225472; expire=1813147200";
    state.poll.as_mut().unwrap().meta.profiles = [
        ("first", "First", "/tmp/first.yaml", None),
        (
            "remote",
            "Travel",
            "https://example.test/travel",
            Some(usage),
        ),
        ("last", "Last", "/tmp/last.yaml", None),
    ]
    .into_iter()
    .map(|(id, name, source, usage)| Profile {
        id: id.into(),
        name: name.into(),
        source: source.into(),
        updated: 0,
        usage: usage.map(str::to_owned),
        builtin: false,
    })
    .collect();
    let frame_text = |state: &State, rows| {
        render::frame(state, 32, rows)
            .into_iter()
            .map(|line| line.text.trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    };
    state.key(Key::Up);
    assert_eq!(state.items()[state.selected], "first");
    state.key(Key::Down);
    let remote = frame_text(&state, 8);
    assert!(remote.contains("› Travel · Remote"), "{remote}");
    assert!(remote.contains("Expires "), "{remote}");
    state.key(Key::Down);
    state.key(Key::Down);
    assert_eq!(state.items()[state.selected], "last");
    let cramped = frame_text(&state, 8);
    assert!(cramped.contains("› Last · Local"), "{cramped}");
    assert!(
        !cramped.contains("Expires ") || cramped.contains("Travel · Remote"),
        "{cramped}"
    );
    let taller = frame_text(&state, 16);
    assert!(taller.contains("Travel · Remote"), "{taller}");
    assert!(taller.contains("Expires "), "{taller}");
    state.key(Key::Up);
    assert_eq!(state.items()[state.selected], "remote");
    assert_eq!(frame_text(&state, 8), remote);
}

#[test]
fn long_profile_node_and_connection_names_wrap_and_keep_the_original_target() {
    use unicode_width::UnicodeWidthStr;
    let compact = |text: &str| {
        text.chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>()
    };
    let name = "跨地域远程线路与完整节点名称".repeat(4);
    for page in [Page::Profiles, Page::Nodes, Page::Connections] {
        let mut state = State::new(true);
        state.page = page;
        let poll = state.poll.as_mut().unwrap();
        match page {
            Page::Profiles => {
                poll.meta.profiles = (0..50)
                    .map(|index| clash_client_core::config::Profile {
                        id: format!("profile-{index}"),
                        name: format!("{index:03} {name}"),
                        source: "/tmp/example.yaml".into(),
                        updated: 0,
                        usage: None,
                        builtin: false,
                    })
                    .collect();
                state.selected = 49;
            }
            Page::Nodes => {
                state.group = Some("PROXY".into());
                poll.snapshot.proxies.proxies.get_mut("PROXY").unwrap().all =
                    vec![name.as_str().into()];
            }
            Page::Connections => {
                poll.snapshot.connections.connections[0].metadata.sniff_host = name.as_str().into();
            }
            _ => unreachable!(),
        }
        for (columns, rows) in [(36, 24), (40, 24), (60, 24), (120, 40), (40, 24)] {
            let frame = render::frame(&state, columns, rows);
            let selected = frame
                .iter()
                .filter(|line| line.tone == render::Tone::Selected)
                .map(|line| line.text.as_str())
                .collect::<String>();
            assert!(
                compact(&selected).contains(&compact(&name)),
                "{columns}: {selected}"
            );
            assert!(!selected.contains('…'), "{columns}: {selected}");
            assert_eq!(frame.len(), rows);
            assert!(frame.iter().all(|line| line.text.width() < columns));
        }
        state.key(Key::Enter);
        match page {
            Page::Profiles => {
                assert_eq!(state.form.as_ref().unwrap().fields[0].value, "profile-49")
            }
            Page::Nodes => assert_eq!(state.confirmation.as_ref().unwrap().node, name),
            Page::Connections => assert_eq!(state.connection.as_deref(), Some("demo-1")),
            _ => unreachable!(),
        }
    }
}

#[test]
fn empty_profile_and_shortcut_guidance_wraps_in_phone_frames() {
    for (page, expected) in [
        (Page::Profiles, "没有匹配的订阅；清除搜索或在操作页添加。"),
        (Page::Rules, "尚未选择订阅；按 3 打开订阅页"),
    ] {
        let mut state = State::new(true);
        state.page = page;
        state.shortcuts_open = page == Page::Rules;
        for columns in [36, 40, 60] {
            let text = render::frame(&state, columns, 24)
                .iter()
                .map(|line| line.text.trim())
                .collect::<String>();
            assert!(text.contains(expected), "{columns}: {text}");
            assert!(!text.contains('…'), "{columns}: {text}");
        }
    }
}

#[test]
fn english_profile_and_shortcut_guidance_remains_readable_at_narrow_widths() {
    for (page, expected) in [
        (
            Page::Profiles,
            "No matching subscriptions; clear the search or add one under Actions.",
        ),
        (
            Page::Rules,
            "Select a subscription; press 3 to open subscriptions",
        ),
    ] {
        let mut state = State::new(true);
        state.language = clash_client_core::config::Language::English;
        state.page = page;
        state.shortcuts_open = page == Page::Rules;
        let compact = |text: &str| {
            text.chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>()
        };
        for columns in [36, 40, 80] {
            let text = render::frame(&state, columns, 24)
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>();
            assert!(
                compact(&text).contains(&compact(expected)),
                "{columns}: {text}"
            );
        }
    }
    let mut state = State::new(false);
    state.language = clash_client_core::config::Language::English;
    assert_eq!(
        crate::tui::catalog::open_shortcut(&state, "shortcut_ads")
            .err()
            .unwrap()
            .to_string(),
        "Backend disconnected"
    );
}
