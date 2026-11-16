use crate::tui::{
    form::Form,
    input::Key,
    model::{Action, Page, State, demo_poll},
    render, settings,
};

fn args() -> crate::args::Args {
    crate::args::parse(["ui"].into_iter().map(str::to_owned)).unwrap()
}

#[test]
fn native_choices_wait_for_confirmation_and_backend_state_before_showing_saved_values() {
    let mut state = State::new(false);
    state.accept(Some(demo_poll()));
    state.page = Page::Settings;
    state.query = "ipv6".into();
    let original = state.poll.as_ref().unwrap().meta.settings.ipv6;
    state.key(Key::Enter);
    state.key(Key::Right);
    assert_eq!(
        state.form.as_ref().unwrap().fields[1].value,
        (!original).to_string()
    );
    assert_eq!(state.poll.as_ref().unwrap().meta.settings.ipv6, original);
    state.key(Key::Down);
    assert_eq!(state.form.as_ref().unwrap().selected, 2);
    state.key(Key::Up);
    assert_eq!(state.form.as_ref().unwrap().selected, 1);
    state.key(Key::Up);
    assert_eq!(state.form.as_ref().unwrap().selected, 1);
    state.key(Key::Down);
    assert!(matches!(state.key(Key::Enter), Action::ReviewForm));
    state.execution = Some(state.form.as_ref().unwrap().args(&args()).unwrap());
    assert!(
        render::frame(&state, 80, 24)
            .iter()
            .any(|line| line.text.contains("保存IPv6为"))
    );
    let Action::ApplySetting(operation) = state.key(Key::Enter) else {
        panic!("native setting must remain inside TUI");
    };
    let clash_client_runtime::control::Command::ConfigSet { key, value } =
        settings::command(&operation).unwrap()
    else {
        panic!("must use existing setting patch");
    };
    assert_eq!(key, "ipv6");
    assert_eq!(value, !original);
    state.failure("protected by owner".into());
    assert!(state.form.is_some());
    assert_eq!(state.poll.as_ref().unwrap().meta.settings.ipv6, original);
    assert!(matches!(state.key(Key::Enter), Action::None));
    assert!(state.message.as_deref().unwrap().contains("草稿保留"));
}

#[test]
fn mode_profile_and_protection_use_dedicated_commands_without_generic_setting_bypass() {
    for key in ["run_mode", "proxy_mode", "claude_protection"] {
        let poll = demo_poll();
        let mut form = settings::open(&poll, key).unwrap();
        assert!(!form.fields[0].choices.is_empty());
        form.cycle(false);
        let operation = form.args(&args()).unwrap();
        assert_ne!(operation.path, "config set");
        if key == "run_mode" {
            assert!(matches!(
                settings::command(&operation).unwrap(),
                clash_client_runtime::control::Command::Mode { .. }
            ));
        } else {
            assert!(!settings::inline(&operation));
            assert!(settings::command(&operation).is_err());
        }
    }
    for key in [
        "proxy_mode",
        "claude_protection",
        "active_profile",
        "accent_color",
        "controller_port",
    ] {
        let operation = crate::args::parse(
            ["config", "set", key, "true"]
                .into_iter()
                .map(str::to_owned),
        )
        .unwrap();
        assert!(settings::command(&operation).is_err());
    }
}

#[test]
fn mode_choices_use_observed_state_and_proxy_preferences_are_labelled_separately() {
    use clash_client_runtime::engine::ProxyMode;
    let mut poll = demo_poll();
    poll.meta.settings.proxy_mode = ProxyMode::Tun;
    poll.meta.settings.run_mode = "rule".into();
    poll.snapshot.tun = false;
    poll.snapshot.system_proxy = false;
    poll.snapshot.mode = "global".into();
    assert_eq!(
        settings::display(&poll, "proxy_mode"),
        "关闭（启动偏好：TUN）"
    );
    assert_eq!(settings::display(&poll, "run_mode"), "全局");
    assert_eq!(
        settings::open(&poll, "proxy_mode").unwrap().fields[0].value,
        "off"
    );
    assert_eq!(
        settings::open(&poll, "run_mode").unwrap().fields[0].value,
        "global"
    );
    poll.snapshot.system_proxy = true;
    assert!(settings::display(&poll, "proxy_mode").starts_with("系统代理"));
    poll.snapshot.tun = true;
    assert_eq!(settings::display(&poll, "proxy_mode"), "TUN");
}

#[test]
fn exclusion_lists_accept_comma_separated_domains_and_keep_invalid_drafts() {
    let mut form = Form::setting("fake_ip_exclusions", &serde_json::json!([]));
    form.fields[1].value = "example.test, example.org, EXAMPLE.test".into();
    let operation = form.args(&args()).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&operation.positional[1]).unwrap(),
        serde_json::json!(["example.test", "example.org"])
    );
    form.fields[1].value = "https://invalid.test/path".into();
    assert!(form.args(&args()).is_err());
    assert_eq!(form.fields[1].value, "https://invalid.test/path");
}

#[test]
fn shortcut_values_follow_the_active_profile_and_stale_profile_drafts_are_cancelled() {
    use clash_client_core::{config::Profile, rule_manager::ProfileRules};
    use std::sync::Arc;

    let mut poll = demo_poll();
    poll.meta.profiles = vec![Profile {
        id: "test-profile".into(),
        name: "测试订阅".into(),
        source: "https://example.test/sub".into(),
        updated: 0,
        usage: None,
        builtin: false,
    }];
    poll.meta.settings.active_profile = Some("test-profile".into());
    poll.meta.settings.rule_overrides.enabled = [true; 5];
    poll.meta
        .settings
        .rule_profiles
        .insert("test-profile".into(), Arc::new(ProfileRules::default()));
    assert_eq!(settings::display(&poll, "shortcut_ads"), "关闭");
    let mut form = settings::open(&poll, "shortcut_ads").unwrap();
    assert_eq!(form.fields[1].value, "off");
    form.cycle(false);
    let clash_client_runtime::control::Command::RuleChange { id, change } =
        settings::command(&form.args(&args()).unwrap()).unwrap()
    else {
        panic!("shortcut must use per-profile rules");
    };
    assert_eq!(id, "test-profile");
    assert!(matches!(
        change,
        clash_client_runtime::control::RuleChange::Shortcut {
            index: 0,
            enabled: true
        }
    ));
    Arc::make_mut(
        poll.meta
            .settings
            .rule_profiles
            .get_mut("test-profile")
            .unwrap(),
    )
    .enabled[0] = true;
    assert_eq!(settings::display(&poll, "shortcut_ads"), "开启");
    poll.meta.settings.rule_profiles.clear();
    assert_eq!(settings::display(&poll, "shortcut_ads"), "开启");
    poll.meta.profiles[0].builtin = true;
    assert!(settings::open(&poll, "shortcut_ads").is_err());
    assert!(settings::display(&poll, "shortcut_ads").contains("非内置订阅"));
    poll.meta.profiles[0].builtin = false;
    let mut state = State::new(false);
    state.accept(Some(poll));
    state.page = Page::Rules;
    state.query = "shortcut_ads".into();
    state.shortcuts = Some(crate::tui::catalog::Shortcuts {
        profile: "test-profile".into(),
        revision: state.poll.as_ref().unwrap().meta.view_revision,
        included: [false; 5],
        error: None,
        sampled: std::time::Instant::now(),
    });
    state.key(Key::Enter);
    assert!(state.form.is_some());
    state.accept(Some(demo_poll()));
    assert!(state.form.is_none());
    assert!(state.message.as_deref().unwrap().contains("订阅已变化"));
}

#[test]
fn tiny_viewports_keep_selected_nodes_fields_and_confirmations_visible_after_resize() {
    let mut state = State::new(true);
    state.page = Page::Nodes;
    state.group = Some("PROXY".into());
    state.selected = 2;
    let name = state.items()[state.selected].to_owned();
    for (columns, rows) in [(40, 1), (40, 2), (32, 8), (40, 10), (120, 40), (40, 1)] {
        let frame = render::frame(&state, columns, rows);
        assert!(
            frame
                .iter()
                .any(|line| line.tone == render::Tone::Selected && line.text.contains(&name))
        );
        assert!(!frame.iter().any(|line| line.text.contains("终端过小")));
    }
    state.key(Key::Enter);
    let frame = render::frame(&state, 80, 1);
    assert!(frame[0].text.contains("切换 PROXY"));
    assert!(frame[0].text.contains("Enter"));
    state.key(Key::Escape);
    state.page = Page::Settings;
    state.query = "mixed_port".into();
    state.selected = 0;
    state.key(Key::Enter);
    state.key(Key::Enter);
    state.key(Key::Clear);
    state.key(Key::Paste("12345".into()));
    for rows in [1, 2, 8, 24, 1] {
        assert!(
            render::frame(&state, 40, rows)
                .iter()
                .any(|line| line.text.contains("12345"))
        );
    }
    state.form = None;
    state.page = Page::Network;
    state.network_results = vec!["首行结果".into(), "当前诊断结果".into()];
    state.selected = 1;
    for rows in [1, 2, 3, 4, 5, 8] {
        assert!(
            render::frame(&state, 40, rows)
                .iter()
                .any(|line| line.text.contains("当前诊断结果"))
        );
    }
}

#[test]
fn sniffed_domains_are_searchable_and_long_detail_values_remain_scrollable() {
    use unicode_width::UnicodeWidthStr;
    let mut state = State::new(true);
    state.page = Page::Connections;
    state
        .poll
        .as_mut()
        .unwrap()
        .snapshot
        .connections
        .connections[0]
        .metadata
        .host = "".into();
    state.query = "api.github.com".into();
    assert_eq!(state.items(), ["demo-1"]);
    state.key(Key::Enter);
    for width in [1, 10, 39, 80] {
        let content = crate::tui::panels::connection_rows(&state, width);
        assert!(content.iter().all(|(line, _)| line.width() <= width));
        let all = content
            .iter()
            .map(|(line, _)| line.as_str())
            .collect::<Vec<_>>()
            .join("");
        assert!(all.contains("api.github.com"));
        if width >= 2 {
            assert!(!all.contains("未提供"));
            assert!(!all.contains("域名：嗅探"));
            assert!(all.contains("嗅探域名：api.github.com"));
        }
        assert!(matches!(state.key(Key::Down), Action::ScrollDetail(true)));
        state.detail_offset = content.len().saturating_sub(1);
        assert!(
            crate::tui::panels::connection(&state, width, 1)
                .iter()
                .any(|(line, _)| line.contains('1'))
        );
    }
    state.detail_offset = 0;
    state
        .poll
        .as_mut()
        .unwrap()
        .snapshot
        .connections
        .connections[0]
        .metadata
        .process = "process\x1b[2J\n".repeat(80).into();
    let content = crate::tui::panels::connection_rows(&state, 20);
    assert!(content.len() > 30);
    assert!(content.iter().all(|(line, _)| line.width() <= 20));
    assert!(
        content
            .iter()
            .all(|(line, _)| !line.chars().any(char::is_control))
    );
    state
        .poll
        .as_mut()
        .unwrap()
        .snapshot
        .connections
        .connections
        .clear();
    assert_eq!(state.connection.as_deref(), Some("demo-1"));
    assert!(
        crate::tui::panels::connection(&state, 80, 2)
            .iter()
            .any(|(line, _)| line.contains("已关闭"))
    );
}

#[test]
fn settings_show_effective_fake_ip_and_empty_list_counts_instead_of_blank_values() {
    let mut poll = demo_poll();
    assert_eq!(settings::display(&poll, "fake_ip"), "开启");
    assert!(settings::display(&poll, "fake_ip_exclusions").starts_with("0 项自定义"));
    assert!(settings::display(&poll, "fake_ip_disabled_defaults").starts_with("0 项已停用"));
    poll.meta.settings.dns_override = false;
    assert_eq!(
        settings::display(&poll, "fake_ip"),
        "关闭（DNS 覆盖未开启）"
    );
    assert_eq!(
        settings::open(&poll, "fake_ip").unwrap().fields[1].value,
        "false"
    );
    assert!(settings::open(&poll, "fake_ip_exclusions").unwrap().fields[1].value == "[]");
    for key in settings::KEYS {
        assert!(!settings::display(&poll, key).trim().is_empty(), "{key}");
    }
}

#[test]
fn categories_and_dedicated_pages_keep_shortcuts_out_of_general_settings() {
    let mut state = State::new(true);
    state.page = Page::Settings;
    assert!(!state.items().iter().any(|key| key.starts_with("shortcut_")));
    assert!(!state.items().contains(&"active_profile"));
    let frame = render::frame(&state, 100, 40);
    assert!(frame.iter().any(|line| line.text.contains("常规与连接")));
    assert!(frame.iter().any(|line| line.text.contains("网络与 DNS")));
    for query in ["网络与 DNS", "网络与 dns", "DNS", "dNs"] {
        state.query = query.into();
        assert!(state.items().contains(&"fake_ip"), "{query}");
        assert!(!state.items().contains(&"update_prereleases"), "{query}");
    }
    state.query.clear();
    state.key(Key::Char('8'));
    assert!(state.page == Page::Rules);
    assert_eq!(state.items(), settings::RULE_KEYS);
    for query in ["IP", "ip", "Ip"] {
        state.query = query.into();
        assert_eq!(
            state.items(),
            ["shortcut_private_ip", "shortcut_cn_ip"],
            "{query}"
        );
    }
    state.query.clear();
    state.key(Key::Enter);
    assert!(state.form.is_none());
    assert!(state.message.as_deref().unwrap().contains("尚未读取"));
}

#[test]
fn subscription_list_selects_names_and_confirms_without_using_cli_text() {
    use clash_client_core::config::Profile;
    let mut state = State::new(false);
    let mut poll = demo_poll();
    poll.meta.profiles = ["家庭线路", "远程 SSH"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| Profile {
            id: format!("profile-{index}"),
            name: name.into(),
            source: "/tmp/example.yaml".into(),
            updated: 0,
            usage: None,
            builtin: false,
        })
        .collect();
    poll.meta.settings.active_profile = Some("profile-0".into());
    state.accept(Some(poll));
    state.page = Page::Profiles;
    assert!(
        render::frame(&state, 80, 24)
            .iter()
            .any(|line| { line.text.contains("[当前] 家庭线路") })
    );
    state.key(Key::Down);
    for (columns, rows) in [(40, 1), (40, 10), (120, 40)] {
        assert!(render::frame(&state, columns, rows).iter().any(|line| {
            line.tone == render::Tone::Selected && line.text.contains("远程 SSH")
        }));
    }
    state.key(Key::Enter);
    let form = state.form.as_ref().unwrap();
    assert_eq!(form.fields[0].value, "profile-1");
    assert_eq!(form.visible_value(0), "远程 SSH");
    assert_eq!(
        state
            .poll
            .as_ref()
            .unwrap()
            .meta
            .settings
            .active_profile
            .as_deref(),
        Some("profile-0")
    );
    state.key(Key::Down);
    assert!(matches!(state.key(Key::Enter), Action::ReviewForm));
    state.execution = Some(state.form.as_ref().unwrap().args(&args()).unwrap());
    let Action::ApplySetting(operation) = state.key(Key::Enter) else {
        panic!("profile switching must stay inside TUI");
    };
    assert!(matches!(
        settings::command(&operation).unwrap(),
        clash_client_runtime::control::Command::Activate { id } if id == "profile-1"
    ));
}

#[test]
fn included_shortcuts_are_read_only_and_scoped_metadata_is_released_on_page_exit() {
    use clash_client_core::config::Profile;
    let mut state = State::new(false);
    let mut poll = demo_poll();
    poll.meta.profiles.push(Profile {
        id: "profile".into(),
        name: "包含快捷规则".into(),
        source: "/tmp/example.yaml".into(),
        updated: 0,
        usage: None,
        builtin: false,
    });
    poll.meta.settings.active_profile = Some("profile".into());
    state.accept(Some(poll));
    state.page = Page::Rules;
    state.shortcuts = Some(crate::tui::catalog::Shortcuts {
        profile: "profile".into(),
        revision: 0,
        included: [true, false, false, false, false],
        error: None,
        sampled: std::time::Instant::now(),
    });
    assert!(
        render::frame(&state, 80, 24)
            .iter()
            .any(|line| line.text.contains("订阅已包含"))
    );
    assert!(
        render::frame(&state, 40, 1)
            .iter()
            .any(|line| line.text.contains("订阅已包含"))
    );
    state.key(Key::Enter);
    assert!(state.form.is_none());
    assert!(state.message.as_deref().unwrap().contains("无需重复添加"));
    state.key(Key::Down);
    state.key(Key::Enter);
    assert!(state.form.is_some());
    state.form = None;
    state.poll.as_mut().unwrap().meta.view_revision += 1;
    state.key(Key::Enter);
    assert!(state.form.is_none());
    assert!(state.message.as_deref().unwrap().contains("尚未读取"));
    state.release_page();
    assert!(state.shortcuts.is_none());
    assert!(state.poll.is_none());
}
