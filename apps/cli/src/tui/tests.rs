use super::{
    input::{Decoder, Key, Utf16Decoder},
    model::{Action, Page, State, Target, demo_poll},
    render,
};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;
#[path = "fake_ip_tests.rs"]
mod fake_ip_features;
#[path = "feature_tests.rs"]
mod features;
#[path = "network_tests.rs"]
mod network_features;
#[path = "settings_tests.rs"]
mod settings_features;

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
    assert_eq!(decoder.feed(b"\x15", now), vec![Key::Clear]);
    assert_eq!(
        decoder.feed(b"\x1b[C\x1b[D", now),
        vec![Key::Right, Key::Left]
    );
}

#[test]
fn escape_keeps_following_shortcuts_across_input_chunk_boundaries() {
    for split in [0, 1, 2] {
        let mut decoder = Decoder::default();
        let now = Instant::now();
        let sequence = b"\x1b3";
        let mut keys = decoder.feed(&sequence[..split], now);
        keys.extend(decoder.feed(&sequence[split..], now + Duration::from_millis(20)));
        assert_eq!(keys, [Key::Escape, Key::Char('3')]);
        let mut state = State::new(true);
        state.page = Page::Network;
        for key in keys {
            state.key(key);
        }
        assert!(state.page == Page::Connections);
    }
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
    assert!(matches!(state.key(Key::Down), Action::ScrollNetwork(true)));
    super::network::scroll(&mut state, 40, true);
    assert_eq!(state.selected, 1);
    state.key(Key::Enter);
    assert_eq!(state.network_results.len(), 8);
    state.key(Key::Char('1'));
    state.release_page();
    assert!(state.network_results.is_empty());
}

#[test]
fn network_tool_selection_wraps_cancels_and_keeps_input_literal() {
    let mut state = State::new(false);
    state.key(Key::Char('4'));
    state.release_page();
    state.network_results = vec!["旧结果".into()];
    assert!(matches!(state.key(Key::Char('[')), Action::CancelNetwork));
    assert_eq!(
        state.network_tool,
        clash_client_core::network_tools::Tool::ALL.len() - 1
    );
    assert!(state.network_results.is_empty());
    assert!(matches!(state.key(Key::Char(']')), Action::CancelNetwork));
    assert_eq!(state.network_tool, 0);
    state.key(Key::Char('/'));
    state.key(Key::Paste("example.test/a[1]".into()));
    state.key(Key::Enter);
    assert!(matches!(state.key(Key::Enter), Action::CheckNetwork(
        clash_client_core::network_tools::Tool::Website, target, options
    ) if target == "example.test/a[1]" && options.seconds == 10));
    assert!(matches!(state.key(Key::Escape), Action::CancelNetwork));
}

#[test]
fn network_tools_require_a_target_and_leave_no_page_history() {
    use clash_client_core::network_tools::Tool;
    for tool in Tool::ALL {
        let mut state = State::new(false);
        state.key(Key::Char('4'));
        state.release_page();
        state.query.clear();
        state.network_tool = tool.index() as usize;
        assert!(tool.needs_target());
        assert!(!matches!(
            state.key(Key::Enter),
            Action::CheckNetwork(_, _, _)
        ));
        state.query = "example.test".into();
        assert!(
            matches!(state.key(Key::Enter), Action::CheckNetwork(selected, _, _) if selected == tool)
        );
        state.network_results = vec!["本次结果".into()];
        state.key(Key::Char('1'));
        state.release_page();
        assert!(state.network_results.is_empty());
    }
}

#[test]
fn terminal_frames_fit_small_large_and_unicode_content_without_control_injection() {
    for (columns, rows) in [
        (0, 0),
        (0, 10),
        (1, 1),
        (2, 2),
        (40, 1),
        (40, 2),
        (32, 8),
        (40, 10),
        (60, 18),
        (80, 24),
        (100, 30),
        (180, 70),
    ] {
        for page in [
            Page::Home,
            Page::Nodes,
            Page::Connections,
            Page::Network,
            Page::Settings,
            Page::Tools,
            Page::Profiles,
            Page::Rules,
        ] {
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

#[test]
fn home_footer_lists_refresh_and_exit_without_unavailable_actions() {
    let state = State::new(true);
    for (columns, rows) in [(120, 40), (100, 30), (40, 20), (32, 8)] {
        let text = render::frame(&state, columns, rows)
            .iter()
            .map(|line| line.text.trim_end())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("r 刷新"), "{columns}×{rows}:\n{text}");
        assert!(text.contains("q 退出"), "{columns}×{rows}:\n{text}");
        for action in ["Enter 打开", "/ 搜索", "Esc 返回", "↑↓ 选择"] {
            assert!(!text.contains(action), "{columns}×{rows}: {action}\n{text}");
        }
    }
}

#[test]
fn phone_node_footer_wraps_all_actions_without_losing_the_selected_node() {
    let mut state = State::new(true);
    state.page = Page::Nodes;
    state.group = Some("PROXY".into());
    state.selected = 2;
    let selected = state.items()[state.selected].to_owned();
    for (columns, rows) in [(36, 16), (40, 12), (40, 20), (48, 24), (60, 18), (80, 20)] {
        let frame = render::frame(&state, columns, rows);
        let text = frame
            .iter()
            .map(|line| line.text.trim_end())
            .collect::<Vec<_>>()
            .join("\n");
        for action in [
            "↑↓ 选择",
            "Enter 打开",
            "s 排序",
            "/ 搜索",
            "Esc 返回",
            "q 退出",
        ] {
            assert!(text.contains(action), "{columns}×{rows}: {action}\n{text}");
        }
        assert_eq!(text.matches("q 退出").count(), 1);
        assert_eq!(text.matches("Esc 返回").count(), 1);
        assert!(!text.contains('…'), "{columns}×{rows}:\n{text}");
        assert!(
            frame.iter().any(|line| {
                line.tone == render::Tone::Selected && line.text.contains(&selected)
            })
        );
        assert_eq!(frame.len(), rows);
        assert!(frame.iter().all(|line| line.text.width() < columns));
    }
}

#[test]
fn phone_footer_wraps_failure_and_keeps_confirmation_and_input_keys_accurate() {
    let mut state = State::new(true);
    state.page = Page::Nodes;
    state.message = Some("保存失败：订阅已包含此规则；旧配置保持不变，请刷新后重试。".into());
    let frame = render::frame(&state, 40, 12);
    let text = frame
        .iter()
        .map(|line| line.text.trim_end())
        .collect::<String>();
    assert!(text.contains(state.message.as_deref().unwrap()));
    assert!(text.contains("q 退出"));
    assert!(!text.contains('…'));
    state.message = None;
    state.group = Some("PROXY".into());
    state.selected = 2;
    state.key(Key::Enter);
    let frame = render::frame(&state, 40, 12);
    let text = frame
        .iter()
        .map(|line| line.text.as_str())
        .collect::<String>();
    assert!(text.contains("Enter 确认"));
    assert!(text.contains("Esc 取消"));
    state.key(Key::Escape);
    state.key(Key::Char('/'));
    let frame = render::frame(&state, 40, 12);
    let text = frame
        .iter()
        .map(|line| line.text.as_str())
        .collect::<String>();
    assert!(text.contains("Enter 完成输入"));
    assert!(text.contains("Ctrl+U 清空"));
    assert!(text.contains("Ctrl+C 退出"));
    assert!(!text.contains("q 退出"));
}

#[test]
fn compact_pages_keep_a_separate_title_and_details_identify_the_current_view() {
    for (page, title) in [
        (Page::Home, "首页"),
        (Page::Nodes, "节点"),
        (Page::Connections, "连接"),
        (Page::Network, "网络"),
        (Page::Settings, "设置"),
        (Page::Tools, "操作"),
        (Page::Profiles, "订阅"),
        (Page::Rules, "快捷规则"),
    ] {
        let mut state = State::new(true);
        state.page = page;
        for (columns, rows) in [
            (36, 16),
            (40, 12),
            (48, 24),
            (53, 30),
            (60, 18),
            (79, 23),
            (32, 8),
        ] {
            let frame = render::frame(&state, columns, rows);
            let title = format!("[{title}]");
            assert_eq!(frame[0].text.trim(), title);
            assert_eq!(
                frame[0].text.find('['),
                Some((columns - 1 - title.width()) / 2)
            );
            assert!(frame[0].tone == render::Tone::Accent);
            if rows >= 10 {
                assert!(frame[1].text.contains("1–8 切页"));
                assert!(frame[1].text.contains("Tab 下一页"));
            }
        }
    }
    let mut state = State::new(true);
    state.page = Page::Connections;
    state.key(Key::Enter);
    assert_eq!(render::frame(&state, 40, 12)[0].text.trim(), "[连接详情]");
    state.key(Key::Escape);
    state.key(Key::Char('5'));
    state.query = "mixed_port".into();
    state.key(Key::Enter);
    assert!(state.form.is_some());
    assert_eq!(render::frame(&state, 40, 12)[0].text.trim(), "[设置]");
}

#[test]
fn navigation_keeps_all_pages_below_eighty_columns_when_the_labels_fit() {
    let pages = [
        (Page::Home, "1 首页"),
        (Page::Nodes, "2 节点"),
        (Page::Connections, "3 连接"),
        (Page::Network, "4 网络"),
        (Page::Settings, "5 设置"),
        (Page::Tools, "6 操作"),
        (Page::Profiles, "7 订阅"),
        (Page::Rules, "8 快捷规则"),
    ];
    for (page, selected) in pages {
        let mut state = State::new(true);
        state.page = page;
        for columns in [54, 60, 64, 72, 79, 80, 120] {
            let frame = render::frame(&state, columns, 30);
            let header = frame[..5]
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(header.contains("CLASH OF RUST"), "{columns}:\n{header}");
            for (_, label) in pages {
                assert!(header.contains(label), "{columns}: {label}\n{header}");
            }
            assert!(header.contains(&format!("[{selected}]")));
            assert!(!header.contains('…'), "{columns}:\n{header}");
            assert!(frame.iter().all(|line| line.text.width() < columns));
        }
    }
    let mut state = State::new(true);
    state.page = Page::Nodes;
    state.key(Key::Enter);
    for columns in [54, 60, 79] {
        let text = render::frame(&state, columns, 30)
            .into_iter()
            .map(|line| line.text)
            .collect::<Vec<_>>()
            .join("\n");
        for action in ["s 排序", "/ 搜索", "Esc 返回", "q 退出"] {
            assert!(text.contains(action), "{columns}: {action}\n{text}");
        }
    }
}
