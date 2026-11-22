use crate::tui::{
    catalog::Shortcuts,
    input::Key,
    model::{Action, Page, State, demo_poll},
    render, rules,
};
use clash_client_core::{
    config::Language,
    rule_manager::{self, CustomRule, Document, ProfileRules},
};
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
fn english_rule_actions_and_search_keep_stable_rule_ids() {
    let mut state = browser();
    state.language = Language::English;
    let content = text(&state, 80, 24);
    for expected in [
        "Rules",
        "Rule shortcuts",
        "Add custom rule",
        "Custom",
        "Subscription",
    ] {
        assert!(content.contains(expected), "{content}");
    }
    assert!(content.contains("r refresh"));
    state.query = "custom".into();
    let ids = state.items();
    assert_eq!(ids.len(), 3);
    assert!(ids[2].starts_with("c:"));
    state.query.clear();
    state.selected = 2;
    state.key(Key::Enter);
    let content = text(&state, 80, 24);
    for expected in ["Move down", "Disable", "Edit", "Delete"] {
        assert!(content.contains(expected), "{content}");
    }
    assert!(content.contains("Enter open or confirm"));
    state.key(Key::Enter);
    assert!(
        state
            .rules
            .as_ref()
            .unwrap()
            .pending
            .as_ref()
            .unwrap()
            .1
            .contains("rule:")
    );
}

#[test]
fn rule_warning_follows_language_without_modifying_the_original() {
    let mut state = browser();
    let raw = "MATCH 之后有 1 条规则不会被匹配，请检查顺序。";
    state.rules.as_mut().unwrap().warnings = vec![raw.into()];
    state.language = Language::English;
    let english = text(&state, 120, 24);
    assert!(
        english.contains("Rules after MATCH will never be evaluated (1 affected)."),
        "{english}"
    );
    assert_eq!(state.rules.as_ref().unwrap().warnings, [raw]);
    state.language = Language::SimplifiedChinese;
    assert!(text(&state, 120, 24).contains(raw));
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
fn invalid_and_duplicate_rules_display_localized_errors_and_preserve_input() {
    for (raw, chinese, english) in [
        (
            "DOMAIN,,节点 東京；候选",
            "规则内容不能为空",
            "Rule content cannot be empty",
        ),
        (
            "DOMAIN,custom.test,DIRECT",
            "已有相同的启用规则，请修改原规则或调整顺序",
            "An identical rule is already enabled; edit it or change its order",
        ),
        (
            "DOMAIN,original.test,DIRECT",
            "已有相同的启用规则，请修改原规则或调整顺序",
            "An identical rule is already enabled; edit it or change its order",
        ),
    ] {
        for language in [Language::English, Language::SimplifiedChinese] {
            let mut state = browser();
            state.language = language;
            let saved = Arc::clone(&state.rules.as_ref().unwrap().saved);
            state.selected = 1;
            state.key(Key::Enter);
            state.key(Key::Paste(raw.into()));
            assert!(matches!(state.key(Key::Enter), Action::None));
            let browser = state.rules.as_ref().unwrap();
            assert!(browser.pending.is_none(), "{raw}");
            assert_eq!(browser.input.as_ref().unwrap().raw, raw);
            assert_eq!(browser.saved, saved);
            assert_eq!(
                state.poll.as_ref().unwrap().meta.settings.rule_profiles["profile"],
                saved
            );
            let frame = text(&state, 160, 30);
            assert!(frame.contains(raw), "{frame}");
            assert!(frame.contains(language.text(chinese, english)), "{frame}");
            if language == Language::English {
                assert!(!frame.contains(chinese), "{frame}");
            }
        }
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
