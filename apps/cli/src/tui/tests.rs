use super::{
    input::{Decoder, Key, Utf16Decoder},
    model::{Action, Page, State, Target, demo_poll},
    render,
};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

#[test]
fn prototype_preserves_machine_commands_and_rejects_machine_output_for_ui() {
    for input in [
        vec!["ui", "--demo"],
        vec!["--json", "status"],
        vec!["--json", "node", "select", "PROXY", "🇺🇸 美国 02"],
        vec!["--json", "network", "check", "example.com"],
    ] {
        assert!(crate::args::parse(input.into_iter().map(str::to_string)).is_ok());
    }
    assert!(crate::args::parse(["ui", "--json"].into_iter().map(str::to_string)).is_err());
}

#[test]
fn pasted_shortcuts_are_text_and_paste_buffers_remain_bounded() {
    let now = Instant::now();
    let mut decoder = Decoder::default();
    assert_eq!(
        decoder.feed(b"\x1b[200~q3r\r\x1b[201~", now),
        vec![Key::Paste("q3r\r".into())]
    );
    let mut state = State::new(true);
    state.page = Page::Nodes;
    state.key(Key::Char('/'));
    state.key(Key::Paste("日".repeat(1000)));
    assert!(state.query.len() <= 256);
    assert!(state.page == Page::Nodes);
    assert!(decoder.feed(b"\x1b[200~", now).is_empty());
    for _ in 0..100 {
        decoder.feed(&vec![b'x'; 4096], now);
    }
    let keys = decoder.feed(b"\x1b[201~", now);
    assert!(matches!(keys.as_slice(), [Key::Paste(value)] if value.len() == 1024));
}

#[test]
fn decoder_preserves_fragmented_unicode_and_arrow_sequences() {
    let mut decoder = Decoder::default();
    let now = Instant::now();
    assert_eq!(decoder.feed(&[0xe6, 0x97], now), Vec::new());
    assert_eq!(decoder.feed(&[0xa5], now), vec![Key::Char('日')]);
    assert!(decoder.feed(b"\x1b[", now).is_empty());
    assert_eq!(decoder.feed(b"B", now), vec![Key::Down]);
    assert!(decoder.feed(b"\x1b", now).is_empty());
    assert_eq!(
        decoder.feed(&[], now + Duration::from_millis(100)),
        vec![Key::Escape]
    );
    assert_eq!(decoder.feed(b"\x03", now), vec![Key::Interrupt]);
}

#[test]
fn windows_console_decodes_surrogate_pairs_without_corrupting_later_input() {
    let mut decoder = Utf16Decoder::default();
    assert_eq!(decoder.feed(0xd83c), None);
    assert_eq!(decoder.feed(0xddfa), Some('🇺'));
    assert_eq!(decoder.feed(0xd83c), None);
    assert_eq!(decoder.feed(0xddf8), Some('🇸'));
    assert_eq!(decoder.feed(0xdc00), None);
    assert_eq!(decoder.feed(0xd83c), None);
    assert_eq!(decoder.feed(u16::from(b'q')), Some('q'));
}

#[test]
fn searching_keeps_shortcuts_literal_and_confirmation_requires_a_second_enter() {
    let mut state = State::new(true);
    state.page = Page::Nodes;
    state.key(Key::Char('/'));
    state.key(Key::Char('q'));
    state.key(Key::Char('3'));
    assert_eq!(state.query, "q3");
    assert!(state.page == Page::Nodes);
    state.key(Key::Escape);
    state.key(Key::Back);
    state.selected = 1;
    state.key(Key::Enter);
    assert_eq!(state.group.as_deref(), Some("PROXY"));
    state.selected = 1;
    state.key(Key::Enter);
    assert!(state.confirmation.is_some());
    assert!(matches!(state.key(Key::Escape), Action::None));
    assert!(state.confirmation.is_none());
    state.key(Key::Enter);
    assert!(matches!(state.key(Key::Enter), Action::None));
    assert!(state.message.as_deref().unwrap().contains("演示模式"));
}

#[test]
fn stale_catalog_and_protection_cannot_submit_a_node_change() {
    let mut poll = demo_poll();
    let target = Target {
        profile: poll.meta.settings.active_profile.clone(),
        group: "PROXY".into(),
        node: "🇺🇸 美国 02".into(),
    };
    assert!(target.validate(&poll).is_ok());
    poll.meta.settings.active_profile = Some("changed".into());
    assert!(
        target
            .validate(&poll)
            .unwrap_err()
            .to_string()
            .contains("订阅已经变化")
    );
    poll.meta.settings.active_profile = target.profile.clone();
    poll.snapshot.running = false;
    assert!(target.validate(&poll).is_err());
    poll.snapshot.running = true;
    poll.snapshot
        .proxies
        .proxies
        .get_mut("PROXY")
        .unwrap()
        .all
        .clear();
    assert!(target.validate(&poll).is_err());
    let mut protected = demo_poll();
    use clash_client_core::claude_check::protection::{Previous, Protection};
    protected.meta.settings.claude_protection = Some(std::sync::Arc::new(Protection::new(
        uuid::Uuid::new_v4().to_string(),
        "protected".into(),
        Previous {
            mode: "rule".into(),
            proxy_mode: clash_client_runtime::engine::ProxyMode::Off,
            selectors: Default::default(),
        },
    )));
    assert!(target.validate(&protected).is_err());
}

#[test]
fn page_changes_release_old_lists_and_failures_cannot_confirm_stale_data() {
    let mut state = State::new(false);
    state.accept(Some(demo_poll()));
    state.page = Page::Nodes;
    state.group = Some("PROXY".into());
    state.selected = 1;
    state.failure("backend unavailable".into());
    state.key(Key::Enter);
    assert!(state.confirmation.is_none());
    state.release_page();
    assert!(state.poll.is_none());
    assert!(state.items().is_empty());
}

#[test]
fn network_check_in_demo_does_not_query_the_backend_and_clears_on_navigation() {
    let mut state = State::new(true);
    state.key(Key::Char('4'));
    state.release_page();
    assert_eq!(state.query, "github.com");
    state.key(Key::Char('/'));
    state.key(Key::Paste("example.com".into()));
    state.key(Key::Enter);
    assert!(matches!(state.key(Key::Enter), Action::None));
    assert!(
        state
            .network_results
            .iter()
            .any(|line| line.contains("example.com"))
    );
    assert!(
        state
            .network_results
            .iter()
            .any(|line| line.contains("实际线路"))
    );
    state.key(Key::Down);
    assert_eq!(state.selected, 1);
    state.key(Key::Enter);
    assert_eq!(state.network_results.len(), 8);
    state.key(Key::Char('1'));
    state.release_page();
    assert!(state.network_results.is_empty());
}

#[test]
fn terminal_frames_fit_small_large_and_unicode_content_without_control_injection() {
    for (columns, rows) in [(40, 10), (60, 18), (80, 24), (100, 30), (180, 70)] {
        for page in [Page::Home, Page::Nodes, Page::Connections, Page::Network] {
            let mut state = State::new(true);
            state.page = page;
            state.error = Some("恶意文本\x1b[2J\n不应控制终端".repeat(100));
            let frame = render::frame(&state, columns, rows);
            assert!(frame.len() <= rows);
            for line in frame {
                assert!(line.text.width() < columns);
                assert!(!line.text.chars().any(char::is_control));
            }
        }
    }
    assert_eq!(render::fit("🇺🇸 美国 / test", 8).width(), 8);
    assert_eq!(render::fit("宽字", 0), "");
}
