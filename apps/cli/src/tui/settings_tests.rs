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
    state.page = Page::Settings;
    state.query = "shortcut_ads".into();
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
            assert!(all.contains("域名：未提供"));
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
