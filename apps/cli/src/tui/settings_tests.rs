use crate::tui::{
    form::Form,
    input::Key,
    model::{Action, Page, State, demo_poll},
    render, settings,
};
use clash_client_core::config::Language::{English, SimplifiedChinese};

fn args() -> crate::args::Args {
    crate::args::parse(["ui", "--lang=zh-CN"].into_iter().map(str::to_owned)).unwrap()
}

#[tokio::test]
async fn changed_subscription_shows_the_selected_language_in_shortcuts() {
    use clash_client_core::config::Profile;
    use clash_client_runtime::control::{Endpoint, PROTOCOL, Remote};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        for index in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let length = stream.read_u32().await.unwrap() as usize;
            let mut request = vec![0; length];
            stream.read_exact(&mut request).await.unwrap();
            let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
            assert_eq!(request["operation"]["command"], "rule-document");
            assert_eq!(request["operation"]["id"], "selected-profile");
            let reply = if index == 0 {
                serde_json::json!({
                    "result": {
                        "profile_id": "different-profile",
                        "builtin": false,
                        "original": [],
                        "policies": [],
                        "included": [false, false, false, false, false]
                    },
                    "error": null
                })
            } else {
                serde_json::json!({
                    "result": null,
                    "error": "设置已被另一客户端修改，请刷新后重试"
                })
            };
            let reply = serde_json::to_vec(&reply).unwrap();
            stream.write_u32(reply.len() as u32).await.unwrap();
            stream.write_all(&reply).await.unwrap();
        }
    });
    let endpoint: Endpoint = serde_json::from_value(serde_json::json!({
        "protocol": PROTOCOL,
        "port": port,
        "pid": std::process::id(),
        "owner": "daemon",
        "token": "a".repeat(64)
    }))
    .unwrap();
    let remote = Remote::new(endpoint, std::time::Duration::from_secs(2));
    let mut poll = demo_poll();
    poll.meta.settings.active_profile = Some("selected-profile".into());
    poll.meta.profiles.push(Profile {
        id: "selected-profile".into(),
        name: "Example".into(),
        source: "https://example.test/sub".into(),
        updated: 0,
        usage: None,
        builtin: false,
    });
    let mut state = State::new(false);
    state.accept(Some(poll));
    for (chinese, english) in [
        (
            "订阅已变化，请刷新后重试",
            "The subscription changed; refresh and try again",
        ),
        (
            "设置已被另一客户端修改，请刷新后重试",
            "Settings changed in another client; refresh and try again",
        ),
    ] {
        state.set_language(SimplifiedChinese);
        let status =
            crate::tui::catalog::read_shortcuts(&remote, state.poll.as_ref().unwrap(), None, true)
                .await
                .unwrap();
        assert!(status.document.is_none());
        state.shortcuts = Some(status);
        for (language, expected) in [(SimplifiedChinese, chinese), (English, english)] {
            state.set_language(language);
            let lines = crate::tui::catalog::rules(&state, 120, 24)
                .into_iter()
                .map(|(line, _)| line)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(lines.contains(expected), "{lines}");
            assert!(crate::tui::catalog::open_shortcut(&state, "shortcut_ads").is_err());
        }
    }
    server.await.unwrap();
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
    assert!(matches!(state.key(Key::Enter), Action::ReviewForm));
    assert_eq!(
        state.form.as_ref().unwrap().fields[1].value,
        (!original).to_string()
    );
    state.execution = Some(state.form.as_ref().unwrap().args(&args()).unwrap());
    assert!(render::frame(&state, 80, 24).iter().any(|line| {
        line.text.contains(&format!(
            "保存{}为",
            settings::title("ipv6", SimplifiedChinese)
        ))
    }));
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
fn language_choices_use_localized_interface_and_save_the_stable_setting_value() {
    for (language, query, system, confirmation) in [
        (
            SimplifiedChinese,
            "语言",
            "跟随系统",
            "保存语言/language为「English」",
        ),
        (
            English,
            "language",
            "Follow system",
            "Save Language/语言 as English",
        ),
    ] {
        let mut state = State::new(false);
        state.accept(Some(demo_poll()));
        state.language = language;
        state.page = Page::Settings;
        state.query = query.into();
        assert_eq!(state.items(), ["language"]);
        assert!(
            render::frame(&state, 80, 24)
                .iter()
                .any(|line| line.text.contains(system))
        );
        state.key(Key::Enter);
        assert_eq!(state.form.as_ref().unwrap().visible_value(1), system);
        state.key(Key::Left);
        assert_eq!(state.form.as_ref().unwrap().visible_value(1), "English");
        assert!(matches!(state.key(Key::Enter), Action::ReviewForm));
        state.execution = Some(state.form.as_ref().unwrap().args(&args()).unwrap());
        assert!(
            render::frame(&state, 80, 24)
                .iter()
                .any(|line| line.text.contains(confirmation))
        );
        let Action::ApplySetting(operation) = state.key(Key::Enter) else {
            panic!("language must use the existing setting patch");
        };
        let clash_client_runtime::control::Command::ConfigSet { key, value } =
            settings::command(&operation).unwrap()
        else {
            panic!("language must use the existing setting patch");
        };
        assert_eq!(key, "language");
        assert_eq!(value, "en");
        assert_eq!(operation.get("lang"), Some("zh-CN"));
    }
}

#[test]
fn saved_language_immediately_redraws_the_existing_terminal_state() {
    use clash_client_core::config::LanguagePreference;

    let mut state = State::new(false);
    let mut poll = demo_poll();
    let mut current_args = args();
    state.page = Page::Settings;
    state.query = "language".into();
    state.language = SimplifiedChinese;
    poll.meta.settings.language = LanguagePreference::English;
    settings::apply_saved_language(&mut state, &poll, Some("ipv6"), &mut current_args);
    assert_eq!(state.language, SimplifiedChinese);
    settings::apply_saved_language(&mut state, &poll, Some("language"), &mut current_args);
    state.accept(Some(poll));
    assert_eq!(state.language, English);
    assert_eq!(current_args.get("lang"), Some("en"));
    assert_eq!(state.query, "language");
    assert!(
        render::frame(&state, 80, 24)
            .iter()
            .any(|line| line.text.contains("Language/语言"))
    );
    let mut poll = demo_poll();
    poll.meta.settings.language = LanguagePreference::SimplifiedChinese;
    settings::apply_saved_language(&mut state, &poll, Some("language"), &mut current_args);
    assert_eq!(state.language, SimplifiedChinese);
    poll.meta.settings.language = LanguagePreference::System;
    settings::apply_saved_language(&mut state, &poll, Some("language"), &mut current_args);
    assert_eq!(state.language, LanguagePreference::System.resolve_desktop());
}

#[test]
fn external_language_changes_follow_preferences_without_losing_drafts_or_overrides() {
    use clash_client_core::config::LanguagePreference;

    let mut state = State::new(false);
    let mut current_args = args();
    state.language = SimplifiedChinese;
    state.language_preference = LanguagePreference::English;
    state.query = "example".into();
    assert!(!settings::sync_language(
        &mut state,
        &mut current_args,
        false,
        77
    ));
    assert_eq!(state.language, SimplifiedChinese);

    state.form = Some(settings::language_form(
        LanguagePreference::SimplifiedChinese,
        SimplifiedChinese,
    ));
    state.form.as_mut().unwrap().draft = "unfinished".into();
    assert!(!settings::sync_language(
        &mut state,
        &mut current_args,
        true,
        77
    ));
    assert_eq!(state.form.as_ref().unwrap().draft, "unfinished");
    assert_eq!(state.language, SimplifiedChinese);

    state.form = None;
    assert!(settings::sync_language(
        &mut state,
        &mut current_args,
        true,
        77
    ));
    assert_eq!(state.language, English);
    assert_eq!(current_args.get("lang"), Some("en"));
    assert_eq!(state.query, "example");
    assert!(!settings::sync_language(
        &mut state,
        &mut current_args,
        true,
        77
    ));
}

#[tokio::test]
async fn offline_language_save_preserves_settings_and_respects_the_owner_lock() {
    use crate::tui::{Reply, save_setting};
    use clash_client_core::config::{LanguagePreference, Store};

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("data");
    let context = crate::args::parse(
        ["--data-dir", root.to_str().unwrap(), "ui"]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap();
    let operation = crate::args::parse(
        ["config", "set", "language", "en"]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap();
    assert!(matches!(
        save_setting(&context, &operation, SimplifiedChinese, false)
            .await
            .unwrap(),
        Reply::LanguageSaved(LanguagePreference::English)
    ));
    let store = Store::at(root.clone()).unwrap();
    let mut original = store.load_settings().unwrap();
    original.ipv6 = !original.ipv6;
    store.save_settings(&original).unwrap();
    let owner = store.instance_lock().unwrap();
    let conflicting = crate::args::parse(
        ["config", "set", "language", "zh-CN"]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap();
    assert!(
        save_setting(&context, &conflicting, English, false)
            .await
            .is_err()
    );
    assert_eq!(store.load_settings().unwrap(), original);
    drop(owner);
    assert!(!root.join("control.json").exists());
    assert!(matches!(
        save_setting(&context, &conflicting, English, false)
            .await
            .unwrap(),
        Reply::LanguageSaved(LanguagePreference::SimplifiedChinese)
    ));
    let saved = store.load_settings().unwrap();
    assert_eq!(saved.language, LanguagePreference::SimplifiedChinese);
    assert_eq!(saved.ipv6, original.ipv6);
    assert!(!root.join("control.json").exists());
}

#[test]
fn language_sync_retains_the_selected_subscription_when_display_matches_change() {
    use clash_client_core::config::{LanguagePreference, Profile};

    let mut state = State::new(false);
    state.page = Page::Profiles;
    state.language = SimplifiedChinese;
    state.query = "default direct".into();
    let mut poll = demo_poll();
    poll.meta.profiles = [
        ("builtin", "默认直连", true),
        ("added", "Default direct plan", false),
    ]
    .into_iter()
    .map(|(id, name, builtin)| Profile {
        id: id.into(),
        name: name.into(),
        source: "default.yaml".into(),
        updated: 0,
        usage: None,
        builtin,
    })
    .collect();
    poll.meta.settings.language = LanguagePreference::English;
    state.accept(Some(poll));
    assert_eq!(state.items(), ["added"]);

    assert!(settings::sync_language(&mut state, &mut args(), true, 77));
    assert_eq!(state.items(), ["builtin", "added"]);
    assert_eq!(state.items()[state.selected], "added");
    state.key(Key::Enter);
    assert_eq!(state.form.as_ref().unwrap().fields[0].value, "added");
}

#[test]
fn mode_profile_and_protection_use_dedicated_commands_without_generic_setting_bypass() {
    for key in ["run_mode", "proxy_mode", "claude_protection"] {
        let poll = demo_poll();
        let mut form = settings::open(&poll, key, SimplifiedChinese).unwrap();
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
        settings::display(&poll, "proxy_mode", SimplifiedChinese),
        "关闭（启动偏好：TUN）"
    );
    assert_eq!(
        settings::display(&poll, "run_mode", SimplifiedChinese),
        "全局"
    );
    assert_eq!(
        settings::open(&poll, "proxy_mode", SimplifiedChinese)
            .unwrap()
            .fields[0]
            .value,
        "off"
    );
    assert_eq!(
        settings::open(&poll, "run_mode", SimplifiedChinese)
            .unwrap()
            .fields[0]
            .value,
        "global"
    );
    poll.snapshot.system_proxy = true;
    assert!(settings::display(&poll, "proxy_mode", SimplifiedChinese).starts_with("系统代理"));
    poll.snapshot.tun = true;
    assert_eq!(
        settings::display(&poll, "proxy_mode", SimplifiedChinese),
        "TUN"
    );
}

#[test]
fn exclusion_lists_accept_comma_separated_domains_and_keep_invalid_drafts() {
    let mut form = Form::setting(
        "fake_ip_exclusions",
        &serde_json::json!([]),
        SimplifiedChinese,
    );
    form.fields[1].value = "example.test, example.org, EXAMPLE.test".into();
    let operation = form.args(&args()).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&operation.positional[1]).unwrap(),
        serde_json::json!(["example.test", "example.org"])
    );
    form.fields[1].value = "https://invalid.test/path".into();
    let error = form.args(&args()).err().unwrap();
    assert_eq!(
        crate::output::localized_error(&error, English),
        "Fake-IP exclusions may contain domains, wildcards, and . - _ + *"
    );
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
    assert_eq!(
        settings::display(&poll, "shortcut_ads", SimplifiedChinese),
        "关闭"
    );
    let mut form = settings::open(&poll, "shortcut_ads", SimplifiedChinese).unwrap();
    assert_eq!(form.fields[1].value, "off");
    form.cycle(false);
    let clash_client_runtime::control::Command::ActiveShortcut { id, index, enabled } =
        settings::command(&form.args(&args()).unwrap()).unwrap()
    else {
        panic!("shortcut must use per-profile rules");
    };
    assert_eq!(id, "test-profile");
    assert_eq!(index, 0);
    assert!(enabled);
    Arc::make_mut(
        poll.meta
            .settings
            .rule_profiles
            .get_mut("test-profile")
            .unwrap(),
    )
    .enabled[0] = true;
    assert_eq!(
        settings::display(&poll, "shortcut_ads", SimplifiedChinese),
        "开启"
    );
    poll.meta.settings.rule_profiles.clear();
    assert_eq!(
        settings::display(&poll, "shortcut_ads", SimplifiedChinese),
        "开启"
    );
    poll.meta.profiles[0].builtin = true;
    assert!(settings::open(&poll, "shortcut_ads", SimplifiedChinese).is_err());
    assert!(settings::display(&poll, "shortcut_ads", SimplifiedChinese).contains("非内置订阅"));
    poll.meta.profiles[0].builtin = false;
    let mut state = State::new(false);
    state.accept(Some(poll));
    state.page = Page::Rules;
    state.shortcuts_open = true;
    state.query = "shortcut_ads".into();
    state.shortcuts = Some(crate::tui::catalog::Shortcuts {
        profile: "test-profile".into(),
        revision: state.poll.as_ref().unwrap().meta.view_revision,
        included: [false; 5],
        document: None,
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
    assert_eq!(
        settings::display(&poll, "fake_ip", SimplifiedChinese),
        "开启"
    );
    assert!(
        settings::display(&poll, "fake_ip_exclusions", SimplifiedChinese).starts_with("0 项自定义")
    );
    assert!(
        settings::display(&poll, "fake_ip_disabled_defaults", SimplifiedChinese)
            .starts_with("0 项已停用")
    );
    poll.meta.settings.dns_override = false;
    assert_eq!(
        settings::display(&poll, "fake_ip", SimplifiedChinese),
        "关闭（DNS 覆盖未开启）"
    );
    assert_eq!(
        settings::open(&poll, "fake_ip", SimplifiedChinese)
            .unwrap()
            .fields[1]
            .value,
        "false"
    );
    assert!(settings::open(&poll, "fake_ip_exclusions", SimplifiedChinese).is_err());
    for key in settings::KEYS {
        assert!(
            !settings::display(&poll, key, SimplifiedChinese)
                .trim()
                .is_empty(),
            "{key}"
        );
    }
}

#[test]
fn categories_and_dedicated_pages_keep_shortcuts_out_of_general_settings() {
    let mut state = State::new(true);
    state.page = Page::Settings;
    assert!(!state.items().iter().any(|key| key.starts_with("shortcut_")));
    assert!(!state.items().contains(&"active_profile"));
    let frame = render::frame(&state, 100, 40);
    assert!(frame.iter().any(|line| line.text.contains("── 常规 ──")));
    assert!(frame.iter().any(|line| line.text.contains("── 网络 ──")));
    assert_eq!(
        settings::category("update_check_interval_minutes", SimplifiedChinese),
        "计划任务"
    );
    for query in ["网络", "dns", "DNS", "dNs"] {
        state.query = query.into();
        assert!(state.items().contains(&"fake_ip"), "{query}");
        assert!(!state.items().contains(&"update_prereleases"), "{query}");
    }
    state.query.clear();
    state.key(Key::Char('5'));
    assert!(state.page == Page::Rules);
    state.shortcuts_open = true;
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
fn long_settings_and_choice_help_wrap_without_losing_the_selected_item() {
    use unicode_width::UnicodeWidthStr;
    let mut state = State::new(true);
    state.page = Page::Settings;
    let key = "subscription_update_interval_minutes";
    state.selected = state.items().iter().position(|item| *item == key).unwrap();
    for (columns, rows) in [(36, 24), (40, 24), (60, 24), (80, 24), (120, 40)] {
        let frame = render::frame(&state, columns, rows);
        assert!(frame.len() <= rows);
        assert!(frame.iter().all(|line| line.text.width() < columns));
        let selected = frame
            .iter()
            .filter(|line| line.tone == render::Tone::Selected)
            .map(|line| line.text.trim())
            .collect::<Vec<_>>()
            .join("");
        assert!(selected.contains(settings::title(key, SimplifiedChinese)));
        assert!(!selected.contains('…'));
    }
    state.form =
        Some(settings::open(state.poll.as_ref().unwrap(), "ipv6", SimplifiedChinese).unwrap());
    for columns in [36, 40, 60, 80, 120] {
        let frame = render::frame(&state, columns, 24);
        let text = frame
            .iter()
            .map(|line| line.text.trim())
            .collect::<Vec<_>>()
            .join("\n");
        for action in [
            "↑↓ 选择",
            "←→ 调整",
            "Enter 确认当前值",
            "Esc 返回",
            "q 退出",
        ] {
            assert!(text.contains(action), "{columns}: {action}");
        }
        assert!(!text.contains('…'));
    }
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
    state.key(Key::Right);
    state.key(Key::Left);
    assert!(matches!(state.key(Key::Enter), Action::ReviewForm));
    assert_eq!(state.form.as_ref().unwrap().fields[0].value, "profile-1");
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
    state.shortcuts_open = true;
    state.shortcuts = Some(crate::tui::catalog::Shortcuts {
        profile: "profile".into(),
        revision: 0,
        included: [true, false, false, false, false],
        document: None,
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
