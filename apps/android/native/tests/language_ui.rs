#[path = "../src/language_ui.rs"]
mod language_ui;
#[path = "../src/live_language.rs"]
mod live_language;

use anyhow::Result;
use clash_mobile_runtime::core::{
    claude_check::{self, BlockRow, Evidence, Group, Signal},
    config::{Language, LanguagePreference},
};
use clash_mobile_runtime::{NativeHost, Session};
use serde_json::{Value, json};
use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc, sync::Arc};

slint::include_modules!();

struct Headless(Rc<MinimalSoftwareWindow>);

struct LanguageHost(Vec<&'static str>);

impl NativeHost for LanguageHost {
    fn request(&self, value: Value) -> Result<Value> {
        assert_eq!(value["op"], "system-languages");
        Ok(json!({"languages": self.0}))
    }
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn click(window: &slint::Window, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    for event in [
        WindowEvent::PointerMoved { position },
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    ] {
        window.dispatch_event(event);
    }
}

fn capture_report_previews(window: &AndroidWindow, adapter: &MinimalSoftwareWindow, prefix: &str) {
    let samples = [
        (
            0,
            Group::Desktop,
            "实时保护与异常拦截",
            "已观察 8 次请求 · 已关闭 1 条异常连接 · 系统提示请求 0 次",
            3,
        ),
        (
            1,
            Group::Connectivity,
            "claude.ai 连通性",
            "响应 3/3 · 中位 42 ms · HTTP 200",
            1,
        ),
        (
            2,
            Group::Quality,
            "出口 IP 质量 · 203.0.113.12",
            "机房或托管网络：是 · 2 项待确认 · 1 个来源查询未完成",
            2,
        ),
        (3, Group::Dns, "实际递归 DNS", "203.0.113.53", 1),
        (4, Group::Browser, "语言偏好", "zh-CN、en-US", 1),
    ];
    let view = window.global::<MobileView>();
    let data = window.global::<MobileData>();
    view.set_detail(0);
    view.set_claude_section("".into());
    window.set_current_page(8);
    data.set_claude_normal("3".into());
    data.set_claude_risk("1".into());
    data.set_claude_clues("1".into());
    adapter.set_size(slint::PhysicalSize::new(393, 1800));
    for (locale, language, suffix) in [
        ("", Language::SimplifiedChinese, "chinese"),
        ("en", Language::English, "english"),
    ] {
        slint::select_bundled_translation(locale).unwrap();
        let rows = samples
            .iter()
            .map(|(section, group, title, value, tone)| {
                let signal = Signal::new(*group, title, *value, "fixture", Evidence::Measured, "");
                BlockRow {
                    section: claude_check::REPORT_SECTIONS[*section],
                    title: signal.display_title_in(language),
                    value: signal.display_value_in(language),
                    priority: 1,
                    tone: *tone,
                    summary_hint: false,
                    item: (),
                }
            })
            .collect();
        let headers = claude_check::report_blocks_in(rows, language)
            .into_iter()
            .map(|block| ReportItem {
                id: block.section.into(),
                title: block.section.into(),
                value: block.overview.into(),
                detail: block.attention.into(),
                delay_tone: block.tone,
                block_items: block.rows.len() as i32,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        data.set_claude_results(Rc::new(slint::VecModel::from(headers)).into());
        let mut pixels = vec![slint::Rgb8Pixel::default(); 393 * 1800];
        for _ in 0..2 {
            slint::platform::update_timers_and_animations();
            window.window().request_redraw();
            adapter.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, 393);
            });
        }
        let mut bytes = b"P6\n393 1800\n255\n".to_vec();
        for pixel in pixels {
            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
        }
        std::fs::write(format!("{prefix}-{suffix}.ppm"), bytes).unwrap();
    }
}

#[test]
fn android_window_loads_both_catalogs_and_preserves_report_section_keys() {
    let adapter = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(adapter.clone()))).unwrap();
    adapter.set_size(slint::PhysicalSize::new(393, 852));
    let window = AndroidWindow::new().unwrap();
    language_ui::install(&window);
    let directory = tempfile::tempdir().unwrap();
    let mut session = Session::open(
        directory.path().to_owned(),
        Arc::new(LanguageHost(vec!["fr-FR", "zh-CN", "en"])),
    )
    .unwrap();
    session.running = true;
    session.vpn = true;
    let section = "Claude 连接与选路";
    let view = window.global::<MobileView>();
    view.set_claude_section(section.into());
    window.global::<Motion>().set_enabled(false);
    let requests = Rc::new(RefCell::new(Vec::new()));
    let recorded = requests.clone();
    window
        .global::<MobileActions>()
        .on_request(move |intent, value, _| {
            recorded.borrow_mut().push((intent, value.to_string()));
        });
    let targets = Rc::new(RefCell::new(BTreeMap::<String, Vec<f32>>::new()));
    let observed = targets.clone();
    let geometry = window.global::<UiGeometry>();
    geometry.on_measured(move |key, bounds| {
        observed
            .borrow_mut()
            .insert(key.to_string(), bounds.iter().collect());
    });
    geometry.set_enabled(true);
    window.show().unwrap();
    let mut pixels = vec![slint::Rgb8Pixel::default(); 393 * 852];
    let mut render = || {
        slint::platform::update_timers_and_animations();
        window.window().request_redraw();
        adapter.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 393);
        });
    };

    for (locale, page_title, section_title, network_title, status_label, option, preference) in [
        (
            "en",
            "Settings",
            "Claude connections and routing",
            "Website check",
            "Available",
            1,
            "zh-CN",
        ),
        (
            "",
            "设置",
            "Claude 连接与选路",
            "网站检查",
            "确认可用",
            2,
            "en",
        ),
    ] {
        let current = if locale == "en" {
            Language::English
        } else {
            Language::SimplifiedChinese
        };
        language_ui::apply(&window, current).unwrap();
        window.set_current_page(9);
        assert_eq!(window.get_page_title(), page_title);
        assert_eq!(
            window
                .global::<NetworkTools>()
                .get_titles()
                .row_data(0)
                .unwrap(),
            network_title
        );
        assert_eq!(
            window
                .global::<ReportText>()
                .invoke_section_title(view.get_claude_section()),
            section_title
        );
        assert_eq!(view.get_claude_section(), section);
        assert_eq!(
            window
                .global::<IpStatusText>()
                .invoke_label("确认可用".into()),
            status_label
        );
        assert_eq!(
            window.global::<IpStatusText>().invoke_label("全球".into()),
            if locale == "en" { "Global" } else { "全球" }
        );
        assert_eq!(
            window.global::<IpStatusText>().invoke_label("德国".into()),
            if locale == "en" { "Germany" } else { "德国" }
        );
        let text = window.global::<MobilePageText>();
        assert_eq!(
            text.invoke_label("下载速度 ↓".into()),
            if locale == "en" {
                "Download speed ↓"
            } else {
                "下载速度 ↓"
            }
        );
        for (source, english) in [
            ("上传量 ↓", "Uploaded ↓"),
            ("开始时间 ↑", "Start time ↑"),
            ("规则 A–Z", "Rule A–Z"),
            ("代理链 Z–A", "Proxy chain Z–A"),
            ("默认顺序", "Default order"),
        ] {
            assert_eq!(
                text.invoke_label(source.into()),
                if locale == "en" { english } else { source }
            );
        }
        assert_eq!(
            text.invoke_label("自定义".into()),
            if locale == "en" {
                "Custom"
            } else {
                "自定义"
            }
        );
        assert_eq!(
            text.invoke_label("Example user value".into()),
            "Example user value"
        );

        for (page, trigger_key, option, expected, intent) in [
            (
                3,
                "connections-sort",
                2,
                "上传速度 ↓",
                MobileIntent::ConnectionSort,
            ),
            (4, "rule-source", 1, "自定义", MobileIntent::RuleFilter),
            (5, "log-level", 3, "INFO", MobileIntent::LogLevel),
        ] {
            window.set_current_page(page);
            view.set_detail(0);
            render();
            targets.borrow_mut().clear();
            geometry.set_revision(geometry.get_revision() + 1);
            render();
            let trigger = targets.borrow()[trigger_key].clone();
            click(
                window.window(),
                trigger[0] + trigger[2] / 2.,
                trigger[1] + trigger[3] / 2.,
            );
            render();
            targets.borrow_mut().clear();
            geometry.set_revision(geometry.get_revision() + 1);
            render();
            let popup = targets.borrow()[&format!("popup:{trigger_key}")].clone();
            let choice = targets.borrow()[&format!("option:{trigger_key}/{option}")].clone();
            requests.borrow_mut().clear();
            click(
                window.window(),
                popup[0].clamp(0., (393. - popup[2]).max(0.)) + choice[0] + choice[2] / 2.,
                popup[1].clamp(0., (852. - popup[3]).max(0.)) + choice[1] + choice[3] / 2.,
            );
            render();
            assert_eq!(
                requests.borrow().as_slice(),
                &[(intent, expected.to_owned())]
            );
        }

        window.global::<MobileData>().set_protected(true);
        window.set_current_page(9);
        render();
        targets.borrow_mut().clear();
        geometry.set_revision(geometry.get_revision() + 1);
        render();
        let trigger = targets.borrow()["language"].clone();
        click(
            window.window(),
            trigger[0] + trigger[2] / 2.,
            trigger[1] + trigger[3] / 2.,
        );
        render();
        targets.borrow_mut().clear();
        geometry.set_revision(geometry.get_revision() + 1);
        render();
        let popup = targets.borrow()["popup:language"].clone();
        let choice = targets.borrow()[&format!("option:language/{option}")].clone();
        requests.borrow_mut().clear();
        click(
            window.window(),
            popup[0].clamp(0., (393. - popup[2]).max(0.)) + choice[0] + choice[2] / 2.,
            popup[1].clamp(0., (852. - popup[3]).max(0.)) + choice[1] + choice[3] / 2.,
        );
        render();
        assert_eq!(
            requests.borrow().as_slice(),
            &[(MobileIntent::Language, preference.to_owned())]
        );
        let preference = LanguagePreference::from_tag(preference).unwrap();
        let next = live_language::save(&mut session, preference).unwrap();
        language_ui::apply(&window, next).unwrap();
        window
            .global::<MobileData>()
            .set_language(preference.tag().into());
        render();
        assert_eq!(
            window.get_page_title(),
            if next == Language::English {
                "Settings"
            } else {
                "设置"
            }
        );
        assert_eq!(
            window.global::<IpStatusText>().invoke_label("德国".into()),
            if next == Language::English {
                "Germany"
            } else {
                "德国"
            }
        );
        assert_eq!(session.store.load_settings().unwrap().language, preference);
        assert!(session.running && session.vpn);
        window.global::<MobileData>().set_protected(false);
        render();
        if let Some(prefix) = std::env::var_os("COR_LIVE_LANGUAGE_PREVIEW") {
            let data = window.global::<MobileData>();
            data.set_core_running(true);
            data.set_vpn_connected(true);
            let android = window.global::<AndroidData>();
            android.set_authorization_status(next.text("已连接", "Connected").into());
            android.set_application_scope(next.text("全部应用", "All apps").into());
            let mut screenshot = vec![slint::Rgb8Pixel::default(); 393 * 852];
            window.window().request_redraw();
            adapter.draw_if_needed(|renderer| {
                renderer.render(&mut screenshot, 393);
            });
            let mut bytes = b"P6\n393 852\n255\n".to_vec();
            for pixel in screenshot {
                bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
            }
            std::fs::write(
                format!("{}-android-{}.ppm", prefix.to_string_lossy(), next.tag()),
                bytes,
            )
            .unwrap();
        }

        window.set_current_page(7);
        view.set_site_filters(true);
        view.set_site_status("全部结果".into());
        view.set_detail(6);
        render();
        targets.borrow_mut().clear();
        geometry.set_revision(geometry.get_revision() + 1);
        render();
        let trigger = targets.borrow()["site-filter-status"].clone();
        click(
            window.window(),
            trigger[0] + trigger[2] / 2.,
            trigger[1] + trigger[3] / 2.,
        );
        render();
        targets.borrow_mut().clear();
        geometry.set_revision(geometry.get_revision() + 1);
        render();
        let popup = targets.borrow()["popup:site-filter-status"].clone();
        let choice = targets.borrow()["option:site-filter-status/1"].clone();
        requests.borrow_mut().clear();
        click(
            window.window(),
            popup[0].clamp(0., (393. - popup[2]).max(0.)) + choice[0] + choice[2] / 2.,
            popup[1].clamp(0., (852. - popup[3]).max(0.)) + choice[1] + choice[3] / 2.,
        );
        render();
        assert_eq!(view.get_site_status(), "确认可用");
        assert_eq!(
            requests.borrow().as_slice(),
            &[(MobileIntent::SiteFilter, "确认可用".to_owned())]
        );
        view.set_detail(0);
        view.set_site_filters(false);

        for (detail, pending, visible, hidden) in [
            (
                18,
                0,
                "preview-applications-scroll",
                "application-policy-scroll",
            ),
            (
                19,
                0,
                "application-policy-scroll",
                "preview-applications-scroll",
            ),
            (
                12,
                19,
                "application-policy-scroll",
                "preview-applications-scroll",
            ),
        ] {
            view.set_pending_detail(pending);
            view.set_detail(detail);
            render();
            targets.borrow_mut().clear();
            geometry.set_revision(geometry.get_revision() + 1);
            render();
            assert!(
                targets.borrow().contains_key(visible),
                "{locale}, detail {detail}: missing {visible}"
            );
            assert!(
                !targets.borrow().contains_key(hidden),
                "{locale}, detail {detail}: unexpected {hidden}"
            );
        }
        view.set_detail(0);
    }
    assert_eq!(
        live_language::save(&mut session, LanguagePreference::System).unwrap(),
        Language::SimplifiedChinese
    );
    for (languages, expected) in [
        (vec!["en-US", "zh-CN"], Language::English),
        (vec!["zh-CN", "en-US"], Language::SimplifiedChinese),
        (vec!["ja-JP", "fr-FR"], Language::English),
    ] {
        assert_eq!(
            live_language::resolve(LanguagePreference::System, &LanguageHost(languages)),
            expected
        );
    }
    let path = session.store.root.join("settings.json");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(live_language::save(&mut session, LanguagePreference::English).is_err());
    assert_eq!(session.settings.language, LanguagePreference::System);
    assert!(session.running && session.vpn);
    drop(render);
    if let Ok(prefix) = std::env::var("COR_ANDROID_LANGUAGE_PREVIEW") {
        capture_report_previews(&window, &adapter, &prefix);
    }
    slint::select_bundled_translation("en").unwrap();
    adapter.set_size(slint::PhysicalSize::new(800, 360));
    window.global::<Theme>().set_text_scale(1.3);
    window.set_current_page(9);
    view.set_detail(13);
    let mut landscape_pixels = vec![slint::Rgb8Pixel::default(); 800 * 360];
    let mut render_landscape = || {
        slint::platform::update_timers_and_animations();
        window.window().request_redraw();
        adapter.draw_if_needed(|renderer| {
            renderer.render(&mut landscape_pixels, 800);
        });
    };
    render_landscape();
    targets.borrow_mut().clear();
    geometry.set_revision(geometry.get_revision() + 1);
    render_landscape();
    let viewport = targets.borrow()["mobile-modal-scroll"].clone();
    assert!(viewport[1] >= 0. && viewport[1] + viewport[3] <= 360.);
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(
                viewport[0] + viewport[2] / 2.,
                viewport[1] + viewport[3] / 2.,
            ),
            delta_x: 0.,
            delta_y: -2000.,
        });
    render_landscape();
    targets.borrow_mut().clear();
    geometry.set_revision(geometry.get_revision() + 1);
    render_landscape();
    let cancel = targets.borrow()["mobile-modal-cancel"].clone();
    assert!(cancel[1] >= viewport[1] && cancel[1] + cancel[3] <= viewport[1] + viewport[3] + 1.);
    requests.borrow_mut().clear();
    click(
        window.window(),
        cancel[0] + cancel[2] / 2.,
        cancel[1] + cancel[3] / 2.,
    );
    render_landscape();
    assert_eq!(view.get_detail(), 0);
    assert_eq!(
        requests.borrow().as_slice(),
        &[(MobileIntent::InstallUpdate, String::new())]
    );
    window.hide().unwrap();
}

#[test]
fn opening_preview_update_dismisses_the_active_connection_detail() {
    let adapter = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(adapter.clone()))).unwrap();
    adapter.set_size(slint::PhysicalSize::new(393, 852));
    let window = AndroidWindow::new().unwrap();
    window.global::<Motion>().set_enabled(false);
    window.global::<AndroidData>().set_update_available(true);
    window.global::<MobileData>().set_update_preview(true);
    let targets = Rc::new(RefCell::new(BTreeMap::<String, Vec<f32>>::new()));
    let captured_targets = targets.clone();
    let geometry = window.global::<UiGeometry>();
    geometry.on_measured(move |key, bounds| {
        captured_targets
            .borrow_mut()
            .insert(key.to_string(), bounds.iter().collect());
    });
    geometry.set_enabled(true);
    let dismissed = Rc::new(RefCell::new(Vec::<(String, i32)>::new()));
    let captured_dismissed = dismissed.clone();
    let updates = Rc::new(RefCell::new(Vec::<String>::new()));
    let captured_updates = updates.clone();
    window
        .global::<MobileActions>()
        .on_request(move |intent, _, action| {
            if intent == MobileIntent::InstallUpdate {
                captured_updates.borrow_mut().push(action.to_string());
            }
        });
    window
        .global::<MobileActions>()
        .on_dismiss_connection_detail(move |id, request| {
            captured_dismissed
                .borrow_mut()
                .push((id.to_string(), request));
        });
    let view = window.global::<MobileView>();
    view.invoke_navigate(3);
    window.show().unwrap();
    let mut pixels = vec![slint::Rgb8Pixel::default(); 393 * 852];
    let mut render = || {
        slint::platform::update_timers_and_animations();
        window.window().request_redraw();
        adapter.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 393);
        });
        geometry.set_revision(geometry.get_revision().wrapping_add(1));
        slint::platform::update_timers_and_animations();
        window.window().request_redraw();
        adapter.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 393);
        });
    };
    for use_system_back in [false, true] {
        view.invoke_open_connection("fixture-connection".into());
        let request = view.get_connection_detail_request();
        assert_eq!(window.get_current_detail(), 5);
        render();
        let install = targets.borrow()["mobile-update-install"].clone();
        click(
            window.window(),
            install[0] + install[2] / 2.,
            install[1] + install[3] / 2.,
        );
        render();
        assert_eq!(window.get_current_detail(), 13);
        assert_eq!(
            dismissed.borrow().last(),
            Some(&("fixture-connection".into(), request))
        );
        assert!(view.get_selected_id().is_empty());
        if use_system_back {
            window.invoke_native_back();
        } else {
            let cancel = targets.borrow()["mobile-modal-cancel"].clone();
            click(
                window.window(),
                cancel[0] + cancel[2] / 2.,
                cancel[1] + cancel[3] / 2.,
            );
        }
        render();
        assert_eq!(window.get_current_detail(), 0);
    }
    assert_eq!(dismissed.borrow().len(), 2);
    assert_eq!(
        updates.borrow().as_slice(),
        ["prepare", "cancel", "prepare", "cancel"]
    );
    window.hide().unwrap();
}
