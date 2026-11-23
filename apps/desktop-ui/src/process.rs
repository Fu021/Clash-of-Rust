//! The renderer owns no Engine, settings writer, notification sender or tray.
use crate::{activation, apply, apply_if_current, intent, repaint, view_state};
use clash_client_core::config::Language;
use clash_client_ui::{AppWindow, Motion, Theme, UiActions, UiData, models::ViewModels};
use clash_desktop_protocol::{self as wire, HostPacket, UiEvent, WindowCommand};
use slint::{
    ComponentHandle, Model, Timer,
    winit_030::{EventResult, WinitWindowAccessor, winit},
};
use std::{
    cell::{Cell, RefCell},
    io,
    rc::Rc,
    sync::mpsc,
    time::Duration,
};
thread_local! {static FRONTEND:RefCell<Option<Frontend>>=const{RefCell::new(None)};}
struct Frontend {
    window: AppWindow,
    models: ViewModels,
    pending_view: Option<wire::ViewState>,
    output: mpsc::Sender<UiEvent>,
    revision: Rc<Cell<u64>>,
    pending_copy: Rc<RefCell<Option<PendingCopy>>>,
    reclaim: Timer,
}
struct PendingCopy {
    host_revision: i32,
    request_revision: i32,
    value: wire::Text,
}

fn project_copy(frame: &mut wire::Frame, pending: Option<PendingCopy>, displayed: (i32, bool)) {
    frame.copy_value = Default::default();
    if let Some(copy) = pending {
        frame.copy_failed |= frame.copy_revision == 0 || copy.host_revision != frame.copy_revision;
        frame.copy_revision = copy.request_revision;
        if !frame.copy_failed {
            frame.copy_value = copy.value;
        }
    } else {
        (frame.copy_revision, frame.copy_failed) = displayed;
    }
}

fn send(output: &mpsc::Sender<UiEvent>, event: UiEvent) {
    let _ = output.send(event);
}
fn metrics(window: &AppWindow) -> wire::Metrics {
    // Event filters run before Slint updates its cached geometry. Read the
    // native state to avoid echoing a stale size/DPI/minimized state to the host.
    let (maximized, fullscreen) = view_state::native_state(window);
    if let Some(metrics) = window.window().with_winit_window(|native| {
        let size = native.inner_size().to_logical::<f32>(native.scale_factor());
        wire::Metrics {
            size: wire::Size {
                width: size.width,
                height: size.height,
            },
            available: available(window),
            maximized: maximized || fullscreen,
            minimized: native.is_minimized().unwrap_or(false),
            system_dark: native
                .theme()
                .map_or(window.global::<UiData>().get_dark(), |theme| {
                    theme == winit::window::Theme::Dark
                }),
        }
    }) {
        return metrics;
    }
    let size = window
        .window()
        .size()
        .to_logical(window.window().scale_factor());
    let system_dark = window
        .window()
        .with_winit_window(|w| w.theme())
        .flatten()
        .map_or(window.global::<UiData>().get_dark(), |theme| {
            theme == winit::window::Theme::Dark
        });
    wire::Metrics {
        size: wire::Size {
            width: size.width,
            height: size.height,
        },
        available: available(window),
        maximized: window.window().is_maximized() || window.window().is_fullscreen(),
        minimized: window.window().is_minimized(),
        system_dark,
    }
}
#[cfg(not(target_os = "macos"))]
fn available(_: &AppWindow) -> Option<wire::Size> {
    None
}
#[cfg(target_os = "macos")]
fn available(window: &AppWindow) -> Option<wire::Size> {
    window
        .window()
        .with_winit_window(|native| {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let handle = native.window_handle().ok()?;
            let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                return None;
            };
            let view = unsafe { handle.ns_view.cast::<objc2_app_kit::NSView>().as_ref() };
            let window = view.window()?;
            let visible = window.screen()?.visibleFrame().size;
            let frame = window.frame();
            let content = window.contentRectForFrameRect(frame);
            Some(wire::Size {
                width: (visible.width - (frame.size.width - content.size.width)) as f32,
                height: (visible.height - (frame.size.height - content.size.height)) as f32,
            })
        })
        .flatten()
}
fn geometry(output: &mpsc::Sender<UiEvent>, window: &AppWindow) {
    send(
        output,
        UiEvent::Geometry {
            metrics: metrics(window),
            view: view_state::capture(window),
        },
    );
}
fn appearance(frame: &wire::Frame, window: &AppWindow) {
    let theme = window.global::<Theme>();
    theme.set_appearance(frame.appearance);
    theme.set_system_dark(frame.dark);
    theme.set_accent_index(frame.accent);
    theme.set_text_scale(frame.text_scale as f32 / 100.);
}
pub fn run() -> anyhow::Result<()> {
    anyhow::ensure!(
        std::env::args_os().any(|a| a == "--desktop-renderer"),
        "请通过 Clash of Rust 主程序启动界面"
    );
    let first = wire::read_frame(&mut io::stdin())?
        .ok_or_else(|| anyhow::anyhow!("桌面宿主未发送初始化数据"))?;
    let HostPacket::Open {
        protocol,
        version,
        language,
        id: _,
        size,
        visible,
        view,
        frame,
        view_ready,
    } = wire::decode(&first)?
    else {
        anyhow::bail!("界面初始化消息无效")
    };
    anyhow::ensure!(
        protocol == wire::PROTOCOL && version == clash_client_core::VERSION,
        "界面与后台版本不一致"
    );
    #[cfg(windows)]
    clash_desktop_shell::platform::notification::prepare_process()?;
    let selector = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into());
    #[cfg(target_os = "macos")]
    let selector = {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        let mut builder: slint::winit_030::EventLoopBuilder =
            winit::event_loop::EventLoop::with_user_event();
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_default_menu(false);
        selector.with_winit_event_loop_builder(builder)
    };
    selector.select().map_err(|error| {
        anyhow::anyhow!(
            "无法初始化桌面图形服务：{error}；纯命令行或 SSH 会话请使用 cor start，不要启动桌面主程序"
        )
    })?;
    clash_client_ui::fonts::register_emoji()?;
    #[cfg(target_os = "linux")]
    slint::set_xdg_app_id("clash-of-rust")?;
    let (output, events) = mpsc::channel::<UiEvent>();
    std::thread::spawn(move || {
        let mut stdout = io::stdout();
        while let Ok(event) = events.recv() {
            let bytes = match wire::encode(&event) {
                Ok(bytes) => bytes,
                Err(error) => {
                    wire::encode(&UiEvent::Error(format!("界面操作数据过大：{error}"))).unwrap()
                }
            };
            if wire::write_frame(&mut stdout, &bytes).is_err() {
                break;
            }
        }
    });
    let window = AppWindow::new()?;
    let language = Language::from_tag(&language).unwrap_or(Language::English);
    slint::select_bundled_translation(language.slint_tag())?;
    crate::install_localized_names(&window);
    let weak = window.as_weak();
    window.on_minimize_requested(move || {
        if let Some(window) = weak.upgrade() {
            window.window().set_minimized(true);
        }
    });
    crate::motion::refresh(&window);
    window.set_app_title(clash_client_core::WINDOW_TITLE.into());
    let mut models = ViewModels::default();
    models.attach(&window.global::<UiData>());
    apply(&frame, &window.global::<UiData>(), &mut models);
    appearance(&frame, &window);
    window
        .window()
        .set_size(slint::LogicalSize::new(size.width, size.height));
    let preserve_native_state = view
        .as_ref()
        .is_some_and(|state| view_state::prepare(state, &window));
    let revision = Rc::new(Cell::new(0u64));
    let action_revision = revision.clone();
    let pending_copy = Rc::new(RefCell::new(None));
    let action_copy = pending_copy.clone();
    let weak = window.as_weak();
    let actions = output.clone();
    window
        .global::<UiActions>()
        .on_dispatch(move |action, a, b| {
            if let Some(window) = weak.upgrade() {
                let revision = action_revision.get().saturating_add(1);
                action_revision.set(revision);
                let intent = intent(action);
                if matches!(intent, wire::Intent::Copy | wire::Intent::ClaudeCopyScript) {
                    *action_copy.borrow_mut() = Some(PendingCopy {
                        host_revision: revision as i32,
                        request_revision: window.global::<UiActions>().get_copy_request_revision(),
                        value: a.as_str().into(),
                    });
                }
                send(
                    &actions,
                    UiEvent::Intent {
                        revision,
                        intent,
                        a: a.to_string(),
                        b: b.to_string(),
                        view: view_state::capture(&window),
                    },
                );
            }
        });
    let weak = window.as_weak();
    let close = output.clone();
    window.window().on_close_requested(move || {
        if let Some(window) = weak.upgrade() {
            send(&close, UiEvent::Close(view_state::capture(&window)));
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    let weak = window.as_weak();
    let native = output.clone();
    let native_revision = revision.clone();
    let mut modifiers = winit::keyboard::ModifiersState::empty();
    let mut first_draw = true;
    let geometry_timer = Timer::default();
    let mut repaint_state = repaint::RepaintState::default();
    window
        .window()
        .on_winit_window_event(move |native_window, event| {
            use winit::event::{ElementState, WindowEvent};
            let Some(window) = weak.upgrade() else {
                return EventResult::Propagate;
            };
            match event {
                WindowEvent::RedrawRequested => {
                    if first_draw {
                        first_draw = false;
                        if let Some(theme) =
                            native_window.with_winit_window(|w| w.theme()).flatten()
                        {
                            let dark = theme == winit::window::Theme::Dark;
                            let motion = window.global::<Motion>();
                            let enabled = motion.get_enabled();
                            motion.set_enabled(false);
                            if window.global::<UiData>().get_appearance() == 2 {
                                window.global::<UiData>().set_dark(dark);
                                window.global::<Theme>().set_system_dark(dark);
                            }
                            let weak = window.as_weak();
                            Timer::single_shot(Duration::from_millis(16), move || {
                                if let Some(window) = weak.upgrade() {
                                    window.global::<Motion>().set_enabled(enabled);
                                }
                            });
                        }
                        geometry(&native, &window);
                        // Initialization and the first paint allocate temporary
                        // font/layout buffers. Reclaim only freed heap pages,
                        // once; keep the live renderer and its caches intact.
                        Timer::single_shot(Duration::from_millis(200), || {
                            clash_desktop_shell::platform::memory::release_unused_heap();
                        });
                    }
                    repaint_state.before_draw(native_window);
                }
                WindowEvent::Resized(size) => {
                    let s = size.to_logical::<f32>(f64::from(native_window.scale_factor()));
                    repaint::resized(native_window, s.width, s.height);
                    let weak = window.as_weak();
                    let output = native.clone();
                    geometry_timer.start(
                        slint::TimerMode::SingleShot,
                        Duration::from_millis(80),
                        move || {
                            if let Some(window) = weak.upgrade() {
                                geometry(&output, &window);
                            }
                        },
                    );
                }
                WindowEvent::Moved(_) | WindowEvent::ThemeChanged(_) => geometry(&native, &window),
                WindowEvent::ScaleFactorChanged { .. } => {
                    geometry(&native, &window);
                    send(&native, UiEvent::Rescaled);
                }
                WindowEvent::Focused(true) => {
                    crate::motion::refresh(&window);
                    repaint::invalidate(native_window);
                    send(&native, UiEvent::Focused);
                }
                WindowEvent::Occluded(false) => repaint::invalidate(native_window),
                WindowEvent::ModifiersChanged(value) => modifiers = value.state(),
                WindowEvent::DroppedFile(path) => {
                    for (intent, a) in [
                        (wire::Intent::AddProfile, String::new()),
                        (
                            wire::Intent::ProfileSource,
                            path.to_string_lossy().into_owned(),
                        ),
                    ] {
                        send(
                            &native,
                            UiEvent::Intent {
                                revision: {
                                    let n = native_revision.get().saturating_add(1);
                                    native_revision.set(n);
                                    n
                                },
                                intent,
                                a,
                                b: String::new(),
                                view: view_state::capture(&window),
                            },
                        );
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state == ElementState::Pressed =>
                {
                    let command = if cfg!(target_os = "macos") {
                        modifiers.super_key()
                    } else {
                        modifiers.control_key()
                    };
                    if let winit::keyboard::Key::Character(text) = &event.logical_key
                        && command
                    {
                        let action = if text.eq_ignore_ascii_case("f") {
                            Some(wire::Intent::FocusSearch)
                        } else if cfg!(target_os = "macos") && text.eq_ignore_ascii_case("q") {
                            Some(wire::Intent::Quit)
                        } else {
                            None
                        };
                        if let Some(intent) = action {
                            send(
                                &native,
                                UiEvent::Intent {
                                    revision: {
                                        let n = native_revision.get().saturating_add(1);
                                        native_revision.set(n);
                                        n
                                    },
                                    intent,
                                    a: String::new(),
                                    b: String::new(),
                                    view: view_state::capture(&window),
                                },
                            );
                            return EventResult::PreventDefault;
                        }
                    }
                }
                _ => {}
            }
            EventResult::Propagate
        });
    let initial_view = if view_ready { view.clone() } else { None };
    let weak = window.as_weak();
    FRONTEND.with(|slot| {
        *slot.borrow_mut() = Some(Frontend {
            window,
            models,
            pending_view: if view_ready { None } else { view },
            output: output.clone(),
            revision,
            pending_copy,
            reclaim: Timer::default(),
        })
    });
    let opened = output.clone();
    slint::spawn_local(async move {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if visible && let Err(error) = activation::show(&window).await {
            send(&opened, UiEvent::Error(format!("无法显示窗口：{error}")));
            let _ = slint::quit_event_loop();
            return;
        }
        if let Some(state) = &initial_view {
            view_state::restore(state, &window);
        }
        // The host must not resize/focus a window using provisional metrics
        // from before its native handle and DPI have been initialized.
        send(
            &opened,
            UiEvent::Opened {
                protocol: wire::PROTOCOL,
                version: clash_client_core::VERSION.into(),
                metrics: metrics(&window),
                preserve_native_state,
            },
        );
    })?;
    std::thread::spawn(move || {
        let mut stdin = io::stdin();
        loop {
            match wire::read_frame(&mut stdin) {
                Ok(Some(bytes)) => {
                    // Do not accumulate full frames while the GUI thread is
                    // busy. Backpressure reaches the host's latest-value slot.
                    let (done, applied) = mpsc::sync_channel(1);
                    if slint::invoke_from_event_loop(move || {
                        packet(&bytes);
                        let _ = done.send(());
                    })
                    .is_err()
                        || applied.recv().is_err()
                    {
                        break;
                    }
                }
                _ => {
                    let _ = slint::invoke_from_event_loop(|| {
                        let _ = slint::quit_event_loop();
                    });
                    break;
                }
            }
        }
    });
    let result = slint::run_event_loop_until_quit();
    FRONTEND.with(|slot| {
        slot.borrow_mut().take();
    });
    result?;
    Ok(())
}
fn packet(bytes: &[u8]) {
    FRONTEND.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(front) = slot.as_mut() else {
            return;
        };
        let packet = match wire::decode::<HostPacket>(bytes) {
            Ok(packet) => packet,
            Err(error) => {
                send(&front.output, UiEvent::Error(error.to_string()));
                return;
            }
        };
        match packet {
            HostPacket::Frame {
                mut data,
                view_ready,
                accepted,
            } => {
                let view = front.window.global::<UiData>();
                let current = front.revision.get();
                if accepted < current {
                    return;
                }
                project_copy(
                    &mut data,
                    front.pending_copy.borrow_mut().take(),
                    (view.get_copy_revision(), view.get_copy_failed()),
                );
                let old_page = view.get_page();
                let old_nodes: usize = front
                    .models
                    .groups
                    .iter()
                    .map(|group| group.nodes.row_count())
                    .sum();
                let applied = apply_if_current(&data, &view, &mut front.models, accepted, current);
                match applied {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(error) => {
                        send(&front.output, UiEvent::Error(error.to_string()));
                        return;
                    }
                }
                let nodes: usize = front
                    .models
                    .groups
                    .iter()
                    .map(|group| group.nodes.row_count())
                    .sum();
                if old_page != data.page || nodes < old_nodes {
                    front.reclaim.start(
                        slint::TimerMode::SingleShot,
                        Duration::from_millis(350),
                        clash_desktop_shell::platform::memory::release_unused_heap,
                    );
                }
                appearance(&data, &front.window);
                repaint::invalidate(front.window.window());
                if view_ready && let Some(view) = front.pending_view.take() {
                    view_state::restore(&view, &front.window);
                }
            }
            HostPacket::Command(command) => match command {
                WindowCommand::Destroy => {
                    let _ = front.window.hide();
                    let _ = slint::quit_event_loop();
                }
                WindowCommand::Show | WindowCommand::Focus => {
                    let weak = front.window.as_weak();
                    let output = front.output.clone();
                    if let Err(error) = slint::spawn_local(async move {
                        let Some(window) = weak.upgrade() else {
                            return;
                        };
                        if let Err(error) = activation::show(&window).await {
                            send(&output, UiEvent::Error(format!("无法显示窗口：{error}")));
                        }
                    }) {
                        send(
                            &front.output,
                            UiEvent::Error(format!("无法唤醒窗口：{error}")),
                        );
                    }
                }
                WindowCommand::Minimize(value) => front.window.window().set_minimized(value),
                WindowCommand::Maximize(value) => front.window.window().set_maximized(value),
                WindowCommand::Resize(size) => front
                    .window
                    .window()
                    .set_size(slint::LogicalSize::new(size.width, size.height)),
                WindowCommand::MinSize(size) => {
                    let size = size.unwrap_or(wire::Size {
                        width: 0.,
                        height: 0.,
                    });
                    front.window.set_window_min_width(size.width);
                    front.window.set_window_min_height(size.height);
                }
                WindowCommand::FocusInput(target) => {
                    let data = front.window.global::<UiData>();
                    data.set_focus_target(target.into());
                    data.set_focus_revision(data.get_focus_revision().wrapping_add(1));
                }
                WindowCommand::PickFile(id) => pick_file(id, &front.output),
            },
            HostPacket::Open { .. } => {
                send(&front.output, UiEvent::Error("重复的界面初始化消息".into()))
            }
        }
    });
}
#[cfg(target_os = "macos")]
fn pick_file(id: u64, output: &mpsc::Sender<UiEvent>) {
    match clash_desktop_shell::file_dialog::begin() {
        Ok(receiver) => {
            let output = output.clone();
            std::thread::spawn(move || {
                let result = receiver
                    .blocking_recv()
                    .unwrap_or_else(|_| Err("文件选择器已关闭".into()));
                send(&output, UiEvent::FilePicked { id, result });
            });
        }
        Err(error) => send(
            output,
            UiEvent::FilePicked {
                id,
                result: Err(error),
            },
        ),
    }
}
#[cfg(not(target_os = "macos"))]
fn pick_file(id: u64, output: &mpsc::Sender<UiEvent>) {
    send(
        output,
        UiEvent::FilePicked {
            id,
            result: Err("此平台由后台打开文件选择器".into()),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    slint::slint! {
        #[edition = "1.19"]
        import { CopyButton } from "../../../crates/client-ui/ui/components/desktop-copy.slint";
        import { Intent, UiActions, UiData } from "../../../crates/client-ui/ui/state.slint";

        export component DesktopCopyProbe inherits Window {
            in-out property <string> copy-value <=> UiData.copy-value;
            in-out property <bool> copy-failed <=> UiData.copy-failed;
            in-out property <int> copy-revision <=> UiData.copy-revision;
            out property <bool> script-copied: script.success;
            out property <bool> log-copied: log.success;
            out property <int> request-revision: UiActions.copy-request-revision;
            public function copy-script() { script.clicked(); }
            public function copy-log() { log.clicked(); }
            script := CopyButton { intent: Intent.claude-copy-script; }
            log := CopyButton { value: "fictional log"; }
        }
    }

    #[test]
    fn clipboard_failure_and_other_success_cannot_confirm_the_script_button() {
        use slint::platform::{
            Platform, PlatformError, WindowAdapter,
            software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        };

        struct Headless;
        impl Platform for Headless {
            fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
                Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
            }
        }
        slint::platform::set_platform(Box::new(Headless)).unwrap();
        let window = DesktopCopyProbe::new().unwrap();
        slint::platform::update_timers_and_animations();
        let reply = |revision, failed, value: &str| {
            window.set_copy_value(value.into());
            window.set_copy_failed(failed);
            window.set_copy_revision(revision);
            slint::platform::update_timers_and_animations();
        };

        window.invoke_copy_script();
        assert_eq!(window.get_request_revision(), 1);
        reply(1, true, "");
        assert!(!window.get_script_copied());

        window.invoke_copy_log();
        reply(2, false, "fictional log");
        assert!(window.get_log_copied());
        assert!(!window.get_script_copied());

        window.invoke_copy_script();
        window.invoke_copy_log();
        reply(4, false, "fictional log");
        assert!(window.get_log_copied());
        assert!(!window.get_script_copied());

        window.invoke_copy_script();
        reply(5, false, "");
        assert!(window.get_script_copied());
    }

    #[test]
    fn generated_bridge_sets_clipboard_value_and_outcome_before_revision() {
        let bridge = include_str!(concat!(env!("OUT_DIR"), "/bridge.rs"));
        let revision = bridge.find("data.set_copy_revision(").unwrap();
        assert!(bridge.find("data.set_copy_value(").unwrap() < revision);
        assert!(bridge.find("data.set_copy_failed(").unwrap() < revision);
    }

    #[test]
    fn clipboard_feedback_resolves_the_local_request_with_the_matching_native_result() {
        let mut frame = wire::Frame {
            copy_revision: 8,
            ..Default::default()
        };
        let latest = || PendingCopy {
            host_revision: 9,
            request_revision: 3,
            value: "fictional log".into(),
        };
        project_copy(&mut frame, Some(latest()), (2, false));
        assert_eq!(frame.copy_revision, 3);
        assert!(frame.copy_failed);
        assert!(frame.copy_value.as_str().is_empty());

        frame.copy_revision = 9;
        frame.copy_failed = true;
        project_copy(&mut frame, Some(latest()), (2, false));
        assert_eq!(frame.copy_revision, 3);
        assert!(frame.copy_failed);
        assert!(frame.copy_value.as_str().is_empty());

        frame.copy_revision = 9;
        frame.copy_failed = false;
        project_copy(&mut frame, Some(latest()), (2, false));
        assert_eq!(frame.copy_revision, 3);
        assert!(!frame.copy_failed);
        assert_eq!(frame.copy_value.as_str(), "fictional log");
    }

    #[test]
    fn clipboard_feedback_does_not_replay_without_a_pending_request() {
        let mut frame = wire::Frame {
            copy_revision: 9,
            copy_failed: true,
            copy_value: "old copy".into(),
            ..Default::default()
        };
        project_copy(&mut frame, None, (9, false));
        assert_eq!(frame.copy_revision, 9);
        assert!(!frame.copy_failed);
        assert!(frame.copy_value.as_str().is_empty());

        project_copy(&mut frame, None, (9, true));
        assert_eq!(frame.copy_revision, 9);
        assert!(frame.copy_failed);

        project_copy(&mut frame, None, (0, false));
        assert_eq!(frame.copy_revision, 0);
        assert!(!frame.copy_failed);
    }
}
