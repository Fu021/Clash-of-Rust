use crate::tui::{
    catalog::Shortcuts,
    input::Key,
    model::{Action, Page, State, demo_poll},
    render, rules,
};
use clash_client_core::rule_manager::{self, CustomRule, Document, ProfileRules};
use clash_client_runtime::control::Command;
use std::{sync::Arc, time::Instant};

fn browser() -> State {
    let document = Arc::new(
        Document::from_raw(
            "profile",
            false,
            "proxies: []\nrules:\n  - DOMAIN,original.test,DIRECT\n  - MATCH,DIRECT\n",
        )
        .unwrap(),
    );
    let saved = Arc::new(ProfileRules {
        custom: vec![CustomRule {
            id: "00000000-0000-4000-8000-000000000001".into(),
            rule: "DOMAIN,custom.test,DIRECT".into(),
            enabled: true,
        }],
        ..Default::default()
    });
    let mut poll = demo_poll();
    poll.meta.settings.active_profile = Some("profile".into());
    poll.meta
        .settings
        .rule_profiles
        .insert("profile".into(), saved);
    let mut state = State::new(false);
    state.accept(Some(poll));
    state.page = Page::Rules;
    state.shortcuts = Some(Shortcuts {
        profile: "profile".into(),
        revision: 0,
        included: document.included,
        document: Some(document),
        error: None,
        sampled: Instant::now(),
    });
    rules::sync(&mut state);
    state
}

fn text(state: &State, columns: usize, rows: usize) -> String {
    render::frame(state, columns, rows)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn rule_sources_have_discoverable_actions_and_subscription_rows_only_move() {
    let mut state = browser();
    for columns in [40, 60, 80, 120] {
        let content = text(&state, columns, 24).replace([' ', '\n'], "");
        for expected in [
            "规则",
            "快捷规则→",
            "新增自定义规则→",
            "自定义",
            "订阅",
            "custom.test",
        ] {
            assert!(content.contains(expected), "{columns}: {content}");
        }
    }
    state.selected = 2;
    state.key(Key::Enter);
    let content = text(&state, 80, 24);
    for action in ["下移", "停用", "编辑", "删除"] {
        assert!(content.contains(action), "{content}");
    }
    state.key(Key::Escape);
    state.selected = 3;
    state.key(Key::Enter);
    let content = text(&state, 80, 24);
    assert!(content.contains("上移"));
    assert!(content.contains("下移"));
    assert!(!content.contains("› 删除") && !content.contains("  删除"));
    assert!(!content.contains("› 编辑") && !content.contains("  编辑"));
    assert!(!content.contains("› 停用") && !content.contains("  停用"));
}

#[test]
fn rule_additions_use_the_shared_validator_and_atomic_revision_without_modifying_source() {
    let mut state = browser();
    let original = state
        .rules
        .as_ref()
        .unwrap()
        .document
        .original
        .iter()
        .map(|row| row.raw.clone())
        .collect::<Vec<_>>();
    let expected = rule_manager::revision(&state.rules.as_ref().unwrap().saved).unwrap();
    state.selected = 1;
    state.key(Key::Enter);
    state.key(Key::Paste("DOMAIN,new.test,DIRECT".into()));
    assert!(state.entering_text());
    state.key(Key::Enter);
    assert!(state.rules.as_ref().unwrap().pending.is_some());
    let Action::SaveRules(command) = state.key(Key::Enter) else {
        panic!("rules must save inside TUI")
    };
    let Command::SaveRules {
        id,
        rules,
        expected: actual,
    } = *command
    else {
        panic!("must use a revision check")
    };
    assert_eq!(id, "profile");
    assert_eq!(actual.as_deref(), Some(expected.as_str()));
    assert_eq!(rules.custom.len(), 2);
    assert_eq!(rules.custom[1].rule, "DOMAIN,new.test,DIRECT");
    assert_eq!(
        state
            .rules
            .as_ref()
            .unwrap()
            .document
            .original
            .iter()
            .map(|row| row.raw.clone())
            .collect::<Vec<_>>(),
        original
    );
    assert_eq!(
        state.poll.as_ref().unwrap().meta.settings.rule_profiles["profile"]
            .custom
            .len(),
        1
    );
}

#[test]
fn invalid_and_duplicate_rules_preserve_input_and_do_not_create_partial_changes() {
    for raw in [
        "INVALID",
        "DOMAIN,custom.test,DIRECT",
        "DOMAIN,original.test,DIRECT",
    ] {
        let mut state = browser();
        state.selected = 1;
        state.key(Key::Enter);
        state.key(Key::Paste(raw.into()));
        state.key(Key::Enter);
        let browser = state.rules.as_ref().unwrap();
        assert!(browser.pending.is_none(), "{raw}");
        assert_eq!(browser.input.as_ref().unwrap().raw, raw);
        assert_eq!(browser.saved.custom.len(), 1);
        assert!(state.message.is_some());
    }
}

#[test]
fn concurrent_rule_changes_reject_old_confirmation_and_keep_the_draft() {
    let mut state = browser();
    state.selected = 1;
    state.key(Key::Enter);
    state.key(Key::Paste("DOMAIN,new.test,DIRECT".into()));
    state.key(Key::Enter);
    let mut poll = demo_poll();
    poll.meta.settings = state.poll.as_ref().unwrap().meta.settings.clone();
    Arc::make_mut(poll.meta.settings.rule_profiles.get_mut("profile").unwrap()).enabled[0] = true;
    state.accept(Some(poll));
    assert!(matches!(state.key(Key::Enter), Action::None));
    assert!(state.rules.as_ref().unwrap().pending.is_none());
    assert_eq!(
        state.rules.as_ref().unwrap().input.as_ref().unwrap().raw,
        "DOMAIN,new.test,DIRECT"
    );
    assert!(state.message.as_deref().unwrap().contains("规则已变化"));
}

#[test]
fn moving_a_subscription_rule_preserves_stable_identity_and_original_text() {
    let mut state = browser();
    state.selected = 4;
    let id = state.items()[4].to_owned();
    state.key(Key::Enter);
    state.key(Key::Enter);
    let Action::SaveRules(command) = state.key(Key::Enter) else {
        panic!("must confirm move")
    };
    let Command::SaveRules { rules, .. } = *command else {
        panic!("must save order")
    };
    assert_eq!(rules.order[1], id);
    assert_eq!(
        state
            .rules
            .as_ref()
            .unwrap()
            .document
            .original
            .last()
            .unwrap()
            .raw
            .as_ref(),
        "MATCH,DIRECT"
    );
}

#[test]
fn switching_profile_or_page_releases_rule_documents_and_pending_edits() {
    let mut state = browser();
    state.selected = 1;
    state.key(Key::Enter);
    state.key(Key::Paste("DOMAIN,new.test,DIRECT".into()));
    state.key(Key::Enter);
    let mut poll = demo_poll();
    poll.meta.settings = state.poll.as_ref().unwrap().meta.settings.clone();
    poll.meta.settings.active_profile = Some("other".into());
    state.accept(Some(poll));
    assert!(state.rules.is_none());
    assert!(state.message.as_deref().unwrap().contains("已取消旧表单"));
    state.page = Page::Home;
    state.release_page();
    assert!(state.rules.is_none() && state.shortcuts.is_none() && !state.shortcuts_open);
}

#[test]
fn shortcut_subpage_releases_the_full_rule_list_and_document() {
    let mut state = browser();
    state.key(Key::Enter);
    assert!(state.shortcuts_open);
    assert!(state.rules.is_none());
    assert!(state.shortcuts.as_ref().unwrap().document.is_none());
    state.key(Key::Escape);
    assert!(!state.shortcuts_open);
}

#[test]
fn demo_rules_return_from_shortcuts_without_waiting_for_a_backend() {
    let mut state = State::new(true);
    state.page = Page::Rules;
    state.release_page();
    assert!(state.rules.is_some());
    state.key(Key::Enter);
    assert!(state.shortcuts_open && state.rules.is_none());
    state.key(Key::Escape);
    assert!(!state.shortcuts_open && state.rules.is_some());
    assert!(state.error.is_none());
}

#[test]
fn protection_preserves_rule_inspection_and_rejects_changes_without_losing_input() {
    use clash_client_core::claude_check::protection::{Previous, Protection};
    let mut state = browser();
    state.poll.as_mut().unwrap().meta.settings.claude_protection = Some(Arc::new(Protection::new(
        uuid::Uuid::new_v4().to_string(),
        "protected".into(),
        Previous {
            mode: "rule".into(),
            proxy_mode: clash_client_runtime::engine::ProxyMode::Off,
            selectors: Default::default(),
        },
    )));
    assert!(text(&state, 80, 24).contains("custom.test"));
    state.selected = 1;
    state.key(Key::Enter);
    state.key(Key::Paste("DOMAIN,new.test,DIRECT".into()));
    state.key(Key::Enter);
    assert!(state.rules.as_ref().unwrap().pending.is_none());
    assert_eq!(
        state.rules.as_ref().unwrap().input.as_ref().unwrap().raw,
        "DOMAIN,new.test,DIRECT"
    );
    assert!(state.message.as_deref().unwrap().contains("Claude 保护"));
}

#[test]
fn subscription_refresh_rejects_the_old_confirmation_and_preserves_the_input() {
    let mut state = browser();
    state.selected = 1;
    state.key(Key::Enter);
    state.key(Key::Paste("DOMAIN,new.test,DIRECT".into()));
    state.key(Key::Enter);
    state.shortcuts.as_mut().unwrap().document = Some(Arc::new(
        Document::from_raw(
            "profile",
            false,
            "proxies: []\nrules:\n  - DOMAIN,changed.test,DIRECT\n  - MATCH,DIRECT\n",
        )
        .unwrap(),
    ));
    assert!(matches!(state.key(Key::Enter), Action::None));
    assert!(state.rules.as_ref().unwrap().pending.is_none());
    assert_eq!(
        state.rules.as_ref().unwrap().input.as_ref().unwrap().raw,
        "DOMAIN,new.test,DIRECT"
    );
    assert!(state.message.as_deref().unwrap().contains("订阅规则已更新"));
}
