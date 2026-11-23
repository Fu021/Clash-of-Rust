use crate::tui::{
    home::{self, CheckFailure},
    input::Key,
    model::{Action, Page, State, demo_poll},
    render,
};
use clash_client_core::config::Language;
use clash_client_runtime::{
    control::Owner,
    update::{Asset, Available, Package},
};
use std::time::{Duration, Instant};

fn state() -> State {
    let mut state = State::new(false);
    state.accept(Some(demo_poll()));
    state
}

fn select(state: &mut State, key: &str) {
    state.selected = home::items(state)
        .iter()
        .position(|item| *item == key)
        .unwrap();
}

fn available(package: bool) -> Available {
    Available {
        version: "99.0.0".into(),
        url: "https://example.test/release".into(),
        preview: false,
        package_manager: None,
        package: package.then(|| Package {
            asset: Asset {
                name: "clash-of-rust.deb".into(),
                size: 1024,
                digest: None,
            },
            url: "https://example.test/package".into(),
            checksum_url: "https://example.test/checksum".into(),
        }),
    }
}

fn content(state: &State, columns: usize, rows: usize) -> String {
    render::frame(state, columns, rows)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn home_starts_only_after_confirmation_and_reuses_current_mode_forms() {
    let mut disconnected = State::new(false);
    assert_eq!(home::items(&disconnected), ["core", "language"]);
    assert!(matches!(disconnected.key(Key::Enter), Action::None));
    assert_eq!(disconnected.form.as_ref().unwrap().spec.path, "start");
    assert!(matches!(disconnected.key(Key::Enter), Action::ReviewForm));
    assert!(disconnected.poll.is_none());

    let mut state = state();
    state.key(Key::Enter);
    assert_eq!(state.form.as_ref().unwrap().spec.path, "stop");
    state.key(Key::Escape);
    select(&mut state, "run_mode");
    state.key(Key::Enter);
    state.key(Key::Right);
    assert_eq!(state.form.as_ref().unwrap().fields[0].value, "global");
    assert!(matches!(state.key(Key::Enter), Action::ReviewForm));
    assert_eq!(state.form.as_ref().unwrap().fields[0].value, "global");
    assert_eq!(state.poll.as_ref().unwrap().snapshot.mode, "rule");
    let context = crate::args::parse(["ui"].into_iter().map(str::to_owned)).unwrap();
    state.execution = Some(state.form.as_ref().unwrap().args(&context).unwrap());
    let Action::ApplySetting(operation) = state.key(Key::Enter) else {
        panic!("mode must use the existing inline save");
    };
    assert_eq!(operation.path, "mode");
    assert!(!operation.flag("yes"));
    state.failure("保存被拒绝".into());
    assert_eq!(state.form.as_ref().unwrap().fields[0].value, "global");
    assert_eq!(state.poll.as_ref().unwrap().snapshot.mode, "rule");
}

#[test]
fn home_update_states_never_invent_an_installable_package() {
    let mut state = state();
    assert!(!home::items(&state).contains(&"update_install"));
    home::accept(&mut state, Ok(None));
    assert!(content(&state, 100, 30).contains("当前通道已是最新版本"));
    home::accept(&mut state, Err(CheckFailure::Check("连接失败".into())));
    assert!(content(&state, 100, 30).contains("检查失败：连接失败"));
    home::accept(&mut state, Ok(Some(available(false))));
    assert!(!home::items(&state).contains(&"update_install"));
    assert!(content(&state, 120, 40).contains("当前平台无自动安装包"));
    let mut managed = available(false);
    managed.package_manager = Some("pacman");
    home::accept(&mut state, Ok(Some(managed)));
    assert!(!home::items(&state).contains(&"update_install"));
    assert!(content(&state, 120, 40).contains("AUR 请用助手"));
    assert!(content(&state, 120, 40).contains("makepkg -si"));
    select(&mut state, "update_check");
    let narrow = content(&state, 80, 24);
    assert!(narrow.contains("AUR 请用助手"));
    assert!(narrow.contains("makepkg"));
    home::accept(&mut state, Ok(Some(available(true))));
    assert!(home::items(&state).contains(&"update_install"));
    assert!(content(&state, 100, 30).contains("安装更新 · 99.0.0"));
}

#[test]
fn english_home_preserves_routes_and_explains_native_updates() {
    let mut state = state();
    state.language = clash_client_core::config::Language::English;
    let mut managed = available(false);
    managed.package_manager = Some("pacman");
    home::accept(&mut state, Ok(Some(managed)));

    let screen = content(&state, 120, 40);
    for expected in [
        "Core running",
        "CLI backend",
        "Reference route: PROXY → AUTO → 🇺🇸 美国 02",
        "Found release 99.0.0",
        "For AUR, use an AUR helper",
        "makepkg -si",
    ] {
        assert!(screen.contains(expected), "{screen}");
    }

    home::accept(&mut state, Ok(None));
    assert!(content(&state, 120, 40).contains("Up to date on this channel"));
    home::accept(
        &mut state,
        Err(CheckFailure::Check("upstream detail".into())),
    );
    assert!(content(&state, 120, 40).contains("Check failed: upstream detail"));
    let reason = "检查更新失败（已尝试 4 次）：校验文件格式无效: 東京";
    home::accept(&mut state, Err(CheckFailure::Check(reason.into())));
    let screen = content(&state, 160, 40);
    assert!(
        screen.contains("Update check failed after 4 attempts: The checksum file is invalid: 東京"),
        "{screen}"
    );
    assert!(!screen.contains("Check failed: Update check failed"));
    assert!(matches!(
        &state.updates.result,
        Some(Err(CheckFailure::Check(error))) if error == reason
    ));
    state.set_language(Language::SimplifiedChinese);
    let screen = content(&state, 160, 40);
    assert!(screen.contains(reason), "{screen}");
    assert!(!screen.contains("检查失败：检查更新失败"));
}

#[test]
fn update_task_failure_is_rendered_in_the_current_language() {
    let mut state = state();
    state.language = Language::SimplifiedChinese;
    home::begin_check(&mut state, Instant::now()).unwrap();
    state.language = Language::English;
    home::accept(&mut state, Err(CheckFailure::Task("task cancelled".into())));
    let english = content(&state, 120, 40);
    assert!(english.contains("Update check failed: task cancelled"));
    assert!(!english.contains("Check failed: Update check failed"));
    state.language = Language::SimplifiedChinese;
    let chinese = content(&state, 120, 40);
    assert!(chinese.contains("更新检查任务失败：task cancelled"));
    assert!(!chinese.contains("检查失败：更新检查任务失败"));
}

#[test]
fn home_localizes_the_builtin_subscription_name_and_preserves_imported_names() {
    use clash_client_core::config::{Language, Profile};

    for (builtin, source, name, english_name) in [
        (
            true,
            "/resources/default.yaml",
            "默认直连",
            "Default direct",
        ),
        (
            false,
            "https://example.test/subscription",
            "默认直连",
            "默认直连",
        ),
        (false, "/imports/travel.yaml", "旅行节点", "旅行节点"),
        (true, "/resources/default.yaml", "本机直连", "本机直连"),
    ] {
        let mut state = State::new(false);
        let mut poll = demo_poll();
        let id = "active-profile";
        poll.meta.profiles.push(Profile {
            id: id.into(),
            name: name.into(),
            source: source.into(),
            updated: 0,
            usage: None,
            builtin,
        });
        poll.meta.settings.active_profile = Some(id.into());
        state.accept(Some(poll));

        state.language = Language::English;
        let screen = content(&state, 120, 40);
        assert!(
            screen.contains(&format!("Active subscription · {english_name}")),
            "{screen}"
        );

        state.language = Language::SimplifiedChinese;
        let screen = content(&state, 120, 40);
        assert!(screen.contains(&format!("当前订阅 · {name}")), "{screen}");
        assert_eq!(state.poll.as_ref().unwrap().meta.profiles[0].name, name);
    }
}

#[test]
fn update_installation_is_cancellable_and_keeps_the_displayed_selection() {
    let mut state = state();
    let selected = available(true);
    home::accept(&mut state, Ok(Some(selected.clone())));
    select(&mut state, "update_install");
    assert!(matches!(state.key(Key::Enter), Action::None));
    assert_eq!(state.updates.pending.as_ref(), Some(&selected));
    state.key(Key::Escape);
    assert!(state.updates.pending.is_none());
    assert_eq!(state.updates.available(), Some(&selected));
    state.key(Key::Enter);
    let Action::InstallUpdate(installation) = state.key(Key::Enter) else {
        panic!("confirmed update must use the selected package");
    };
    assert_eq!(*installation, selected);
    assert!(state.updates.pending.is_none());
}

#[test]
fn repeated_checks_keep_one_request_and_only_enable_installation_after_completion() {
    let mut state = state();
    home::accept(&mut state, Ok(Some(available(true))));
    home::begin_check(&mut state, Instant::now()).unwrap();
    select(&mut state, "update_check");
    assert!(matches!(state.key(Key::Enter), Action::None));
    assert!(state.message.is_none());
    select(&mut state, "update_install");
    assert!(matches!(state.key(Key::Enter), Action::None));
    assert!(state.updates.pending.is_none());
    assert!(state.message.is_none());
    assert!(content(&state, 120, 40).contains("检查中，暂不可安装"));
    home::accept(&mut state, Ok(Some(available(true))));
    assert!(!content(&state, 120, 40).contains("检查中"));
    state.key(Key::Enter);
    assert!(state.updates.pending.is_some());
}

#[test]
fn update_checking_obeys_schedule_and_does_not_duplicate_the_gui_owner() {
    let mut state = state();
    let now = Instant::now();
    state
        .poll
        .as_mut()
        .unwrap()
        .meta
        .settings
        .update_check_interval_minutes = 0;
    assert!(!home::due(&mut state, now));
    assert!(!home::begin_check(&mut state, now).unwrap().1);
    assert!(state.updates.checking);
    assert!(home::begin_check(&mut state, now).is_err());
    home::accept(&mut state, Ok(None));
    state
        .poll
        .as_mut()
        .unwrap()
        .meta
        .settings
        .update_check_interval_minutes = 1;
    assert!(!home::due(&mut state, now));
    assert!(home::due(&mut state, now + Duration::from_secs(60)));
    state.poll.as_mut().unwrap().meta.owner = Owner::Gui;
    assert!(!home::due(&mut state, now + Duration::from_secs(120)));
    assert!(home::begin_check(&mut state, now).is_ok());
    state.demo = true;
    state.updates.checking = false;
    assert!(home::begin_check(&mut state, now).is_err());
    assert!(!home::due(&mut state, now + Duration::from_secs(180)));
}

#[test]
fn automatic_update_checks_resume_after_repeated_connection_failures() {
    let mut state = state();
    state
        .poll
        .as_mut()
        .unwrap()
        .meta
        .settings
        .update_check_interval_minutes = 5;
    let start = Instant::now();
    assert!(home::due(&mut state, start));
    home::begin_check(&mut state, start).unwrap();
    assert!(!home::due(&mut state, start + Duration::from_secs(300)));
    home::accept(&mut state, Err(CheckFailure::Check("网络连接失败".into())));
    assert!(content(&state, 120, 40).contains("检查失败：网络连接失败"));
    assert!(!home::due(&mut state, start + Duration::from_secs(299)));

    let retry = start + Duration::from_secs(300);
    assert!(home::due(&mut state, retry));
    home::begin_check(&mut state, retry).unwrap();
    home::accept(
        &mut state,
        Err(CheckFailure::Check("GitHub 暂不可达".into())),
    );
    assert!(content(&state, 120, 40).contains("检查失败：GitHub 暂不可达"));
    assert!(!home::due(&mut state, start + Duration::from_secs(599)));

    let recovered = start + Duration::from_secs(600);
    assert!(home::due(&mut state, recovered));
    home::begin_check(&mut state, recovered).unwrap();
    home::accept(&mut state, Ok(None));
    let rendered = content(&state, 120, 40);
    assert!(rendered.contains("当前通道已是最新版本"));
    assert!(!rendered.contains("GitHub 暂不可达"));
    assert!(!home::due(&mut state, start + Duration::from_secs(899)));
    assert!(home::due(&mut state, start + Duration::from_secs(900)));
}

#[test]
fn channel_changes_and_owner_loss_cancel_old_update_requests_and_confirmations() {
    let mut state = state();
    let now = Instant::now();
    home::begin_check(&mut state, now).unwrap();
    state
        .poll
        .as_mut()
        .unwrap()
        .meta
        .settings
        .update_prereleases = true;
    assert!(home::sync(&mut state));
    assert!(!state.updates.checking);
    assert!(state.updates.result.is_none());
    home::accept(&mut state, Ok(Some(available(true))));
    select(&mut state, "update_install");
    state.key(Key::Enter);
    assert!(state.updates.pending.is_some());
    state.failure("读取后台失败".into());
    assert!(home::sync(&mut state));
    assert!(state.updates.pending.is_none());
    assert!(!matches!(state.key(Key::Enter), Action::InstallUpdate(_)));
    state.error = None;
    home::begin_check(&mut state, now).unwrap();
    state.accept(None);
    assert!(home::sync(&mut state));
    assert!(!state.updates.checking);
    assert_eq!(home::items(&state), ["core", "language"]);
}

#[test]
fn home_language_is_available_offline_and_during_protection() {
    use clash_client_core::{
        claude_check::protection::{Previous, Protection},
        config::{Language, LanguagePreference},
    };

    let mut offline = State::new(false);
    select(&mut offline, "language");
    assert!(content(&offline, 100, 30).contains("语言/language"));
    offline.key(Key::Enter);
    assert_eq!(
        offline.form.as_ref().unwrap().setting.as_deref(),
        Some("language")
    );
    assert_eq!(offline.form.as_ref().unwrap().fields[1].value, "system");

    let mut protected = state();
    protected.language = Language::English;
    protected.language_preference = LanguagePreference::English;
    protected
        .poll
        .as_mut()
        .unwrap()
        .meta
        .settings
        .claude_protection = Some(std::sync::Arc::new(Protection::new(
        uuid::Uuid::new_v4().to_string(),
        "protected".into(),
        Previous {
            mode: "rule".into(),
            proxy_mode: clash_client_runtime::engine::ProxyMode::Off,
            selectors: Default::default(),
        },
    )));
    select(&mut protected, "language");
    assert!(content(&protected, 100, 30).contains("Language/语言"));
    protected.key(Key::Enter);
    assert_eq!(
        protected.form.as_ref().unwrap().setting.as_deref(),
        Some("language")
    );
    assert_eq!(protected.form.as_ref().unwrap().fields[1].value, "en");
}

#[test]
fn protection_blocks_home_mutations_and_updates_but_keeps_the_unlock_entry() {
    use clash_client_core::claude_check::protection::{Previous, Protection};
    let mut state = state();
    home::accept(&mut state, Ok(Some(available(true))));
    home::begin_check(&mut state, Instant::now()).unwrap();
    state.poll.as_mut().unwrap().meta.settings.claude_protection =
        Some(std::sync::Arc::new(Protection::new(
            uuid::Uuid::new_v4().to_string(),
            "protected".into(),
            Previous {
                mode: "rule".into(),
                proxy_mode: clash_client_runtime::engine::ProxyMode::Off,
                selectors: Default::default(),
            },
        )));
    assert!(home::sync(&mut state));
    assert!(!state.updates.checking);
    assert!(!home::items(&state).contains(&"update_install"));
    assert!(home::begin_check(&mut state, Instant::now()).is_err());
    state.language = clash_client_core::config::Language::English;
    for key in ["core", "run_mode", "proxy_mode", "active_profile"] {
        select(&mut state, key);
        state.key(Key::Enter);
        assert!(state.form.is_none(), "{key}");
        assert!(
            state
                .message
                .as_deref()
                .unwrap()
                .contains("Disable Claude protection first"),
            "{key}: {:?}",
            state.message
        );
    }
    select(&mut state, "claude_protection");
    state.key(Key::Enter);
    assert_eq!(state.form.as_ref().unwrap().spec.path, "claude protect");
    state.poll.as_mut().unwrap().meta.settings.claude_protection = None;
    assert!(home::items(&state).contains(&"update_install"));
}

#[test]
fn protection_allows_stopped_core_recovery_but_rejects_stop_and_stale_state() {
    use clash_client_core::claude_check::protection::{Previous, Protection};
    let mut state = state();
    let protection = std::sync::Arc::new(Protection::new(
        uuid::Uuid::new_v4().to_string(),
        "protected".into(),
        Previous {
            mode: "rule".into(),
            proxy_mode: clash_client_runtime::engine::ProxyMode::Off,
            selectors: Default::default(),
        },
    ));
    state.poll.as_mut().unwrap().meta.settings.claude_protection = Some(protection.clone());

    select(&mut state, "core");
    state.key(Key::Enter);
    assert!(state.form.is_none());
    assert!(state.message.as_deref().unwrap().contains("Claude 保护"));

    state.poll.as_mut().unwrap().snapshot.running = false;
    state.key(Key::Enter);
    assert_eq!(state.form.as_ref().unwrap().spec.path, "start");
    assert!(matches!(state.key(Key::Enter), Action::ReviewForm));
    let context = crate::args::parse(["ui"].into_iter().map(str::to_owned)).unwrap();
    state.execution = Some(state.form.as_ref().unwrap().args(&context).unwrap());
    let Action::Execute(operation) = state.key(Key::Enter) else {
        panic!("protected recovery must use the normal start command");
    };
    assert_eq!(operation.path, "start");
    assert_eq!(
        state.poll.as_ref().unwrap().meta.settings.claude_protection,
        Some(protection)
    );

    state.form = None;
    state.failure("后台读取失败".into());
    state.key(Key::Enter);
    assert!(state.form.is_none());
    assert!(state.message.as_deref().unwrap().contains("后台状态过期"));
}

#[test]
fn phone_home_keeps_its_title_and_selected_action_after_resizing_or_refresh() {
    let mut state = state();
    home::accept(&mut state, Ok(Some(available(true))));
    select(&mut state, "update_install");
    for (columns, rows) in [(40, 24), (60, 24), (80, 24), (120, 40)] {
        let text = content(&state, columns, rows);
        assert!(text.contains("首页"), "{columns}×{rows}: {text}");
        assert!(
            text.contains("› 安装更新 · 99.0.0"),
            "{columns}×{rows}: {text}"
        );
        assert!(text.contains("Enter 操作"), "{columns}×{rows}: {text}");
    }
    state.page = Page::Nodes;
    state.release_page();
    assert!(state.updates.available().is_some());
    state.page = Page::Home;
    home::accept(&mut state, Ok(None));
    assert!(state.selected < home::items(&state).len());
}

#[tokio::test]
async fn dropping_the_tui_update_request_cancels_its_waiting_future() {
    let (completion, result) = tokio::sync::oneshot::channel::<()>();
    let request = crate::tui::Request(tokio::spawn(async move {
        let _completion = completion;
        std::future::pending::<anyhow::Result<Option<Available>>>().await
    }));
    tokio::task::yield_now().await;
    drop(request);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), result)
            .await
            .unwrap()
            .is_err()
    );
}
