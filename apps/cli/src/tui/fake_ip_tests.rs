use crate::tui::{
    input::Key,
    model::{Action, Page, State, demo_poll},
    render,
};
use clash_client_core::config::DEFAULT_FAKE_IP_EXCLUSIONS;
use clash_client_runtime::control::Command;

fn editor() -> State {
    let mut state = State::new(false);
    state.accept(Some(demo_poll()));
    state.page = Page::Settings;
    state.query = "fake_ip_exclusions".into();
    state.key(Key::Enter);
    state
}

#[test]
fn additions_preserve_both_settings_fields_and_use_the_full_expected_snapshot() {
    let mut state = editor();
    let before = state.poll.as_ref().unwrap().meta.settings.clone();
    state.key(Key::Enter);
    state.key(Key::Paste("custom.example.test".into()));
    state.key(Key::Enter);
    assert!(state.fake_ip.as_ref().unwrap().pending.is_some());
    assert_eq!(state.poll.as_ref().unwrap().meta.settings, before);
    let Action::SaveFakeIp(command) = state.key(Key::Enter) else {
        panic!("expected an atomic settings save");
    };
    let Command::Settings { settings, expected } = *command else {
        panic!("expected the existing settings command");
    };
    assert_eq!(expected, Some(serde_json::to_value(&before).unwrap()));
    assert_eq!(settings.fake_ip_exclusions, ["custom.example.test"]);
    assert_eq!(
        settings.fake_ip_disabled_defaults,
        before.fake_ip_disabled_defaults
    );
    state.failure("保存被拒绝".into());
    assert_eq!(state.fake_ip.as_ref().unwrap().draft, "custom.example.test");
    assert_eq!(state.poll.as_ref().unwrap().meta.settings, before);
}

#[test]
fn pasted_domains_share_gui_parsing_and_invalid_batches_do_not_partially_save() {
    let mut state = editor();
    state.key(Key::Enter);
    state.key(Key::Paste(
        "first.example.test,\nSECOND.EXAMPLE.TEST\nfirst.example.test".into(),
    ));
    state.key(Key::Enter);
    let Action::SaveFakeIp(command) = state.key(Key::Enter) else {
        panic!("expected a batch exclusion save");
    };
    let Command::Settings { settings, .. } = *command else {
        panic!("expected settings");
    };
    assert_eq!(
        settings.fake_ip_exclusions,
        ["first.example.test", "SECOND.EXAMPLE.TEST"]
    );
    state.key(Key::Enter);
    state.key(Key::Clear);
    state.key(Key::Paste("valid.example.test,invalid/domain".into()));
    state.key(Key::Enter);
    assert!(state.fake_ip.as_ref().unwrap().pending.is_none());
    assert!(
        state
            .poll
            .as_ref()
            .unwrap()
            .meta
            .settings
            .fake_ip_exclusions
            .is_empty()
    );
}

#[test]
fn disabled_defaults_remain_visible_and_restore_keeps_custom_domains() {
    let mut state = editor();
    let domain = DEFAULT_FAKE_IP_EXCLUSIONS[0];
    state.fake_ip.as_mut().unwrap().selected = 2;
    state.key(Key::Enter);
    let Action::SaveFakeIp(command) = state.key(Key::Enter) else {
        panic!("expected a default exclusion change");
    };
    let Command::Settings { mut settings, .. } = *command else {
        panic!("expected settings");
    };
    assert_eq!(settings.fake_ip_disabled_defaults, [domain]);
    settings.fake_ip_exclusions = vec!["kept.example.test".into()];
    let mut poll = demo_poll();
    poll.meta.settings = settings;
    state.accept(Some(poll));
    let editor = state.fake_ip.as_ref().unwrap();
    assert_eq!(editor.entries[editor.selected - 2].domain, domain);
    assert!(!editor.entries[editor.selected - 2].enabled);
    for (columns, rows) in [(40, 24), (60, 30), (120, 40)] {
        let frame = render::frame(&state, columns, rows);
        assert!(frame.iter().any(|line| line.text.contains("Fake-IP")));
        let selected = frame
            .iter()
            .filter(|line| line.tone == render::Tone::Selected)
            .map(|line| line.text.trim())
            .collect::<Vec<_>>()
            .join("");
        assert!(selected.contains(domain));
        assert!(selected.contains("已停用"));
        assert!(!selected.contains('…'));
    }
    state.fake_ip.as_mut().unwrap().selected = 1;
    state.key(Key::Enter);
    let Action::SaveFakeIp(command) = state.key(Key::Enter) else {
        panic!("expected a restore");
    };
    let Command::Settings { settings, .. } = *command else {
        panic!("expected settings");
    };
    assert!(settings.fake_ip_disabled_defaults.is_empty());
    assert_eq!(settings.fake_ip_exclusions, ["kept.example.test"]);
}

#[test]
fn concurrent_settings_changes_invalidate_confirmation_and_keep_input() {
    let mut state = editor();
    state.key(Key::Enter);
    state.key(Key::Paste("draft.example.test".into()));
    state.key(Key::Enter);
    let mut changed = demo_poll();
    changed.meta.settings.ipv6 = !changed.meta.settings.ipv6;
    state.accept(Some(changed));
    assert!(matches!(state.key(Key::Enter), Action::None));
    assert!(state.message.as_deref().unwrap().contains("设置已变化"));
    assert!(state.fake_ip.as_ref().unwrap().pending.is_none());
    assert_eq!(state.fake_ip.as_ref().unwrap().draft, "draft.example.test");
    assert!(
        state
            .poll
            .as_ref()
            .unwrap()
            .meta
            .settings
            .fake_ip_exclusions
            .is_empty()
    );
}

#[test]
fn disabled_dns_allows_inspection_but_rejects_writes_without_losing_input() {
    let mut state = editor();
    let mut poll = demo_poll();
    poll.meta.settings.dns_override = false;
    poll.meta.settings.fake_ip = false;
    state.accept(Some(poll));
    assert!(!state.fake_ip.as_ref().unwrap().entries.is_empty());
    assert!(
        render::frame(&state, 120, 40)
            .iter()
            .any(|line| line.text.contains("DNS 覆盖已关闭"))
    );
    state.key(Key::Enter);
    state.key(Key::Paste("q.example.test".into()));
    assert!(state.entering_text());
    state.key(Key::Enter);
    assert!(
        state
            .message
            .as_deref()
            .unwrap()
            .contains("请先开启 DNS 覆盖")
    );
    assert!(state.fake_ip.as_ref().unwrap().pending.is_none());
    assert_eq!(state.fake_ip.as_ref().unwrap().draft, "q.example.test");
    state.release_page();
    assert!(state.fake_ip.is_none());
}

#[test]
fn protection_keeps_the_list_readable_and_rejects_exclusion_changes() {
    use clash_client_core::claude_check::protection::{Previous, Protection};
    let mut state = editor();
    let mut poll = demo_poll();
    poll.meta.settings.claude_protection = Some(std::sync::Arc::new(Protection::new(
        uuid::Uuid::new_v4().to_string(),
        "protected".into(),
        Previous {
            mode: "rule".into(),
            proxy_mode: clash_client_runtime::engine::ProxyMode::Off,
            selectors: Default::default(),
        },
    )));
    state.accept(Some(poll));
    state.fake_ip.as_mut().unwrap().selected = 2;
    state.key(Key::Enter);
    assert!(state.fake_ip.as_ref().unwrap().pending.is_none());
    assert!(state.message.as_deref().unwrap().contains("Claude 保护"));
    assert!(
        render::frame(&state, 120, 40)
            .iter()
            .any(|line| line.text.contains(DEFAULT_FAKE_IP_EXCLUSIONS[0]))
    );
    state.key(Key::Down);
    assert_eq!(state.fake_ip.as_ref().unwrap().selected, 3);
}
