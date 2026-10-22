//! The only desktop adapter that knows about Slint or winit.
use super::*;
use clash_client_ui::{AppWindow, Intent, Motion, Theme, UiActions, UiData, models::ViewModels};
use futures_util::StreamExt;
use runtime::{Size, native, window};
use slint::{
    ComponentHandle, Timer, TimerMode,
    winit_030::{EventResult, WinitWindowAccessor, winit},
};
use std::time::Instant;
#[path = "repaint.rs"]
mod repaint;

thread_local! { static HOST: RefCell<Option<DesktopHost>> = const { RefCell::new(None) }; }
fn enqueue(message: Message) {
    let _ = slint::invoke_from_event_loop(move || {
        HOST.with(|slot| {
            if let Some(host) = slot.borrow_mut().as_mut() {
                host.message(message);
            }
        })
    });
}
fn enqueue_window(id: window::Id, message: Message) {
    let _ = slint::invoke_from_event_loop(move || {
        HOST.with(|slot| {
            if let Some(host) = slot.borrow_mut().as_mut()
                && host.current_window_id() == Some(id)
            {
                host.message(message);
            }
        })
    });
}
struct DesktopHost {
    app: App,
    worker: Arc<tokio::runtime::Runtime>,
    window: Option<(window::Id, AppWindow)>,
    models: ViewModels,
    clipboard: Option<arboard::Clipboard>,
    tray_listening: bool,
    next_update: Instant,
    last_delay: Instant,
    fatal: Option<String>,
    event_loop_started: bool,
    retiring: Option<window::Id>,
    saved_view: Option<view_state::State>,
}
pub(super) fn run(
    startup: Option<tray::Startup>,
    state: window_state::State,
) -> anyhow::Result<()> {
    let worker = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?,
    );
    let _runtime_context = worker.enter();
    let (app, initial) = App::new(startup, state);
    let mut host = DesktopHost {
        app,
        worker: worker.clone(),
        window: None,
        models: ViewModels::default(),
        clipboard: None,
        tray_listening: false,
        next_update: Instant::now() + update::CHECK_INTERVAL,
        last_delay: Instant::now(),
        fatal: None,
        event_loop_started: false,
        retiring: None,
        saved_view: None,
    };
    if host.app.visible {
        let settings = window::Settings {
            size: host.app.window_state.size(),
            visible: true,
        };
        native::Host::open_window(&mut host, window::Id::unique(), settings);
    }
    if let Some(error) = host.fatal.take() {
        anyhow::bail!(error);
    }
    host.event_loop_started = true;
    HOST.with(|slot| *slot.borrow_mut() = Some(host));
    native::install(|operation| {
        let _ = slint::invoke_from_event_loop(move || {
            HOST.with(|slot| {
                if let Some(host) = slot.borrow_mut().as_mut() {
                    operation(host);
                }
            })
        });
    });
    HOST.with(|slot| {
        let mut slot = slot.borrow_mut();
        let host = slot.as_mut().unwrap();
        host.spawn(initial);
        host.spawn(interaction::theme_updates());
    });
    let tick = Timer::default();
    tick.start(TimerMode::Repeated, Duration::from_secs(1), || {
        HOST.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(host) = slot.as_mut() else {
                return;
            };
            host.message(Message::Tick);
            if host.app.claude.session.is_some() {
                host.message(Message::Claude(claude_view::Event::Tick));
            }
            let now = Instant::now();
            if now >= host.next_update {
                host.next_update = now + update::CHECK_INTERVAL;
                host.message(Message::CheckUpdate);
            }
            let interval = host.app.settings.delay_interval_minutes;
            if interval > 0
                && now.duration_since(host.last_delay)
                    >= Duration::from_secs(u64::from(interval) * 60)
            {
                host.last_delay = now;
                host.message(Message::PeriodicDelay);
            }
        })
    });
    let result = slint::run_event_loop_until_quit();
    tick.stop();
    let host = HOST.with(|slot| slot.borrow_mut().take()).unwrap();
    // A backend failure must also restore the proxy/core, just like an ordinary exit.
    if !host.app.exiting
        && let Some(engine) = host.app.engine.clone()
    {
        let _ = worker.block_on(async {
            let mut engine = engine.lock().await;
            if !engine.is_remote() {
                engine.stop().await?;
            }
            Ok::<_, anyhow::Error>(())
        });
    }
    if let Some(error) = host.fatal {
        anyhow::bail!(error);
    }
    result?;
    Ok(())
}
impl DesktopHost {
    fn spawn(&self, task: Task<Message>) {
        self.worker.spawn(async move {
            let mut events = task.0;
            while let Some(message) = events.next().await {
                enqueue(message);
            }
        });
    }
    fn message(&mut self, mut message: Message) {
        // Native callbacks and queued show requests may belong to a retired
        // window. Business task results are independent of window lifetime.
        if let Message::Window((id, _)) = &message
            && self.current_window_id() != Some(*id)
        {
            return;
        }
        #[cfg(target_os = "linux")]
        if let Message::WindowCreated(id) = &message
            && self.current_window_id() != Some(*id)
        {
            return;
        }
        if matches!(message, Message::ShowWindow(_)) {
            message = Message::ShowWindow(self.current_window_id());
            self.retiring = None;
        }
        if matches!(message, Message::Window((_, window::Event::CloseRequested)))
            && let Some((_, window)) = &self.window
        {
            // Capture before the hidden-state projection empties the models
            // and changes ScrollView's layout bounds.
            self.saved_view = Some(view_state::State::capture(window));
        }
        let task = self.app.update(message);
        self.sync();
        self.spawn(task);
        self.finish_retirement();
        if self.app.tray.is_some() && !self.tray_listening {
            self.tray_listening = true;
            self.worker.spawn(async {
                while let Some(command) = tray::next().await {
                    enqueue(Message::Tray(command));
                }
            });
        }
    }
    fn sync(&mut self) {
        if let Some((_, window)) = &self.window {
            self.app
                .present(&window.global::<UiData>(), &mut self.models);
            let view_ready = !self.app.busy
                || !matches!(
                    self.app.page,
                    Page::Proxies | Page::Connections | Page::Rules | Page::Logs
                );
            if self.app.visible
                && view_ready
                && !self.app.window_opening
                && !self.app.size_pending
                && let Some(state) = self.saved_view.take()
            {
                state.restore(window);
            }
        }
    }
    fn current_window_id(&self) -> Option<window::Id> {
        self.window.as_ref().map(|(id, _)| *id)
    }
    fn retire_window(&mut self, id: window::Id) {
        if self.app.visible {
            return;
        }
        let Some((current, window)) = &self.window else {
            return;
        };
        if *current != id {
            return;
        }
        let size = window
            .window()
            .size()
            .to_logical(window.window().scale_factor());
        if self.app.window_state.capture(
            Size::new(size.width, size.height),
            window.window().is_maximized(),
            Some(window.window().is_minimized()),
        ) {
            let state = self.app.window_state.clone();
            self.spawn(Task::perform(
                async move { state.save().await.map_err(|error| format!("{error:#}")) },
                Message::WindowStateSaved,
            ));
        }
        self.retiring = Some(id);
        let _ = window.hide();
        self.models.clear();
        self.finish_retirement();
    }
    fn finish_retirement(&mut self) {
        if self.retiring.is_none() || self.app.visible || self.app.picking_file {
            return;
        }
        if self.retiring != self.current_window_id() {
            self.retiring = None;
            return;
        }
        // Dropping the last application handle releases the component tree and
        // its native adapter. The event loop, tray and Engine remain alive.
        self.retiring = None;
        self.window = None;
        self.models = ViewModels::default();
        clash_of_rust::flags::release_cache();
    }
}
impl native::Host for DesktopHost {
    fn current_window(&self) -> Option<window::Id> {
        self.current_window_id()
    }
    fn open_window(&mut self, id: window::Id, settings: window::Settings) {
        if let Some((_, window)) = &self.window {
            self.retiring = None;
            let _ = window.show();
            repaint::invalidate(window.window());
            return;
        }
        let window = match AppWindow::new() {
            Ok(window) => window,
            Err(error) => {
                if self.event_loop_started {
                    self.app.window_opening = false;
                    self.app.visible = false;
                    let message = format!("窗口打开失败，可从托盘重试：{error}");
                    self.app.show_notice(message.clone(), true);
                    self.worker.spawn(async move {
                        let _ = platform::notification::show("Clash of Rust", &message).await;
                    });
                } else {
                    self.fatal = Some(error.to_string());
                    let _ = slint::quit_event_loop();
                }
                return;
            }
        };
        self.models.attach(&window.global::<UiData>());
        window
            .global::<UiActions>()
            .on_dispatch(move |intent, a, b| {
                // Queue even UI-originated commands to avoid reentrant state/model updates.
                let _ = slint::invoke_from_event_loop(move || {
                    HOST.with(|slot| {
                        if let Some(host) = slot.borrow_mut().as_mut() {
                            if host.current_window_id() != Some(id) {
                                return;
                            }
                            let task = host.app.intent(intent, a.as_str(), b.as_str());
                            host.sync();
                            if intent == Intent::ConnectionSort
                                && let Some((_, window)) = &host.window
                            {
                                // Sorting can reorder every row. Invalidate the software
                                // surface too, so the header is painted in the same frame.
                                repaint::invalidate(window.window());
                            }
                            host.spawn(task);
                        }
                    })
                });
            });
        window.window().on_close_requested(move || {
            enqueue_window(id, Message::Window((id, window::Event::CloseRequested)));
            slint::CloseRequestResponse::KeepWindowShown
        });
        let mut modifiers = winit::keyboard::ModifiersState::empty();
        let mut first_frame = true;
        let first_frame_ui = window.as_weak();
        window.window().on_winit_window_event(move |window, event| {
            use winit::event::{ElementState, WindowEvent};
            match event {
                WindowEvent::RedrawRequested => {
                    if first_frame {
                        first_frame = false;
                        let restore_motion = first_frame_ui.upgrade().map(|ui| {
                            let motion = ui.global::<Motion>();
                            let enabled = motion.get_enabled();
                            motion.set_enabled(false);
                            (ui.as_weak(), enabled)
                        });
                        // The native window only exists once the event loop is
                        // active. Resolve System before its very first paint.
                        if let Some(Some(theme)) = window.with_winit_window(|native| native.theme())
                        {
                            HOST.with(|slot| {
                                if let Some(host) = slot.borrow_mut().as_mut() {
                                    host.app.system_dark = theme == winit::window::Theme::Dark;
                                    host.app.sync_appearance();
                                    host.sync();
                                    if let Some((_, ui)) = &host.window {
                                        ui.global::<Theme>().set_system_dark(host.app.system_dark);
                                    }
                                }
                            });
                        }
                        if let Some((weak, enabled)) = restore_motion {
                            // The first frame uses the resolved appearance in
                            // full. Only later user changes should transition.
                            Timer::single_shot(Duration::from_millis(16), move || {
                                if let Some(ui) = weak.upgrade() {
                                    ui.global::<Motion>().set_enabled(enabled);
                                }
                            });
                        }
                    }
                    repaint::before_draw(window);
                }
                WindowEvent::Occluded(false) => {
                    repaint::invalidate(window);
                }
                WindowEvent::Resized(size) => {
                    let logical = size.to_logical::<f32>(f64::from(window.scale_factor()));
                    repaint::invalidate_size(window, logical.width, logical.height);
                    enqueue_window(
                        id,
                        Message::Window((
                            id,
                            window::Event::Resized(Size::new(logical.width, logical.height)),
                        )),
                    );
                }
                WindowEvent::ScaleFactorChanged { .. } => {
                    enqueue_window(id, Message::Window((id, window::Event::Rescaled)))
                }
                WindowEvent::Focused(true) => {
                    repaint::invalidate(window);
                    enqueue_window(id, Message::Window((id, window::Event::Focused)))
                }
                WindowEvent::ThemeChanged(theme) => enqueue_window(
                    id,
                    Message::SystemTheme(if *theme == winit::window::Theme::Dark {
                        runtime::theme::Mode::Dark
                    } else {
                        runtime::theme::Mode::Light
                    }),
                ),
                WindowEvent::ModifiersChanged(value) => modifiers = value.state(),
                WindowEvent::DroppedFile(path) => {
                    enqueue_window(id, Message::ImportDialog(true));
                    enqueue_window(
                        id,
                        Message::ProfileSource(path.to_string_lossy().into_owned()),
                    );
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state == ElementState::Pressed =>
                {
                    use winit::keyboard::Key;
                    let command = if cfg!(target_os = "macos") {
                        modifiers.super_key()
                    } else {
                        modifiers.control_key()
                    };
                    if let Key::Character(text) = &event.logical_key
                        && command
                        && (text.eq_ignore_ascii_case("f")
                            || (cfg!(target_os = "macos") && text.eq_ignore_ascii_case("q")))
                    {
                        enqueue_window(
                            id,
                            Message::Key(runtime::keyboard::Event::KeyPressed {
                                key: runtime::keyboard::Key::Character(text.to_string()),
                                modifiers: runtime::keyboard::Modifiers { command: true },
                            }),
                        );
                        return EventResult::PreventDefault;
                    }
                }
                _ => {}
            }
            EventResult::Propagate
        });
        window.window().set_size(slint::LogicalSize::new(
            settings.size.width,
            settings.size.height,
        ));
        let preserve_native_state = self
            .saved_view
            .as_ref()
            .is_some_and(|state| state.prepare(&window));
        self.window = Some((id, window));
        if let Some((_, window)) = &self.window {
            // Slint changed callbacks run later; initialize the first frame now.
            let theme = window.global::<Theme>();
            theme.set_appearance(match self.app.appearance_mode() {
                ThemeMode::Dark => 0,
                ThemeMode::Light => 1,
                ThemeMode::System => 2,
            });
            theme.set_system_dark(self.app.system_dark);
            theme.set_accent_index(self.app.settings.accent_color as i32);
            theme.set_text_scale(f32::from(self.app.settings.text_scale) / 100.);
        }
        self.sync();
        if settings.visible
            && let Some((_, window)) = &self.window
        {
            let _ = window.show();
            view_state::State::ensure_reachable(window);
        }
        enqueue_window(
            id,
            Message::Window((
                id,
                window::Event::Opened {
                    preserve_native_state,
                },
            )),
        );
    }
    fn window_size(&self, id: window::Id) -> Size {
        self.window
            .as_ref()
            .filter(|(current, _)| *current == id)
            .map_or(Size::ZERO, |(_, window)| {
                let size = window
                    .window()
                    .size()
                    .to_logical(window.window().scale_factor());
                Size::new(size.width, size.height)
            })
    }
    fn maximized(&self, id: window::Id) -> bool {
        self.window
            .as_ref()
            .is_some_and(|(current, window)| *current == id && window.window().is_maximized())
    }
    fn fit_window_size(&self, _id: window::Id, desired: Size) -> Size {
        #[cfg(target_os = "macos")]
        if let Some((current, window)) = &self.window
            && *current == _id
            && let Some(Some(size)) = window.window().with_winit_window(|native| {
                use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                let handle = native.window_handle().ok()?;
                let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                    return None;
                };
                mac_desktop::available_content_size(handle.ns_view)
            })
        {
            return window_state::fit_size(desired, size);
        }
        desired
    }
    fn minimized(&self, id: window::Id) -> Option<bool> {
        self.window
            .as_ref()
            .filter(|(current, _)| *current == id)
            .map(|(_, window)| window.window().is_minimized())
    }
    fn window_command(&mut self, id: window::Id, command: window::Command) {
        if matches!(command, window::Command::Destroy) {
            self.retire_window(id);
            return;
        }
        let Some((current, window)) = &self.window else {
            return;
        };
        if *current != id {
            return;
        }
        match command {
            window::Command::Destroy => unreachable!(),
            window::Command::Show => {
                self.retiring = None;
                let _ = window.show();
                repaint::invalidate(window.window());
            }
            window::Command::Minimize(value) => window.window().set_minimized(value),
            window::Command::Maximize(value) => window.window().set_maximized(value),
            window::Command::Resize(size) => window
                .window()
                .set_size(slint::LogicalSize::new(size.width, size.height)),
            window::Command::MinSize(size) => {
                let size = size.unwrap_or(Size::ZERO);
                window.set_window_min_width(size.width);
                window.set_window_min_height(size.height);
            }
            window::Command::Focus => {
                window
                    .window()
                    .with_winit_window(|native| native.focus_window());
            }
        }
    }
    fn read_clipboard(&mut self) -> Option<String> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()?.get_text().ok()
    }
    fn write_clipboard(&mut self, text: String) {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        if let Some(clipboard) = &mut self.clipboard {
            match clipboard.set_text(text.as_str()) {
                Ok(()) => {
                    if let Some((_, window)) = &self.window {
                        let data = window.global::<UiData>();
                        data.set_copy_value(text.into());
                        data.set_copy_revision(data.get_copy_revision().wrapping_add(1));
                    }
                }
                Err(error) => self.app.show_notice(format!("复制失败：{error}"), true),
            }
        }
    }
    fn focus(&mut self, target: &str) {
        if let Some((_, window)) = &self.window {
            let data = window.global::<UiData>();
            data.set_focus_target(target.into());
            data.set_focus_revision(data.get_focus_revision().wrapping_add(1));
        }
    }
    fn system_theme(&self) -> runtime::theme::Mode {
        self.window
            .as_ref()
            .and_then(|(_, window)| {
                window
                    .window()
                    .with_winit_window(|native| native.theme())
                    .flatten()
            })
            .map_or(
                if self.app.system_dark {
                    runtime::theme::Mode::Dark
                } else {
                    runtime::theme::Mode::Light
                },
                |theme| {
                    if theme == winit::window::Theme::Dark {
                        runtime::theme::Mode::Dark
                    } else {
                        runtime::theme::Mode::Light
                    }
                },
            )
    }
    fn quit(&mut self) {
        let _ = slint::quit_event_loop();
    }
}

impl App {
    fn category(&self, text: &str) -> CategoryFilter {
        ip_report::Report::from_results(&self.site_results)
            .categories
            .into_iter()
            .find(|category| category.name == text)
            .map_or(CategoryFilter::All, |category| {
                CategoryFilter::Group(category.name)
            })
    }
    fn region(&self, text: &str) -> RegionFilter {
        if text == "未识别地区" {
            return RegionFilter::Unidentified;
        }
        self.site_results
            .iter()
            .filter_map(|result| ip_report::region(result.as_ref()))
            .find(|code| RegionFilter::Country(code).to_string() == text)
            .map_or(RegionFilter::All, RegionFilter::Country)
    }
    pub(super) fn intent(&mut self, intent: Intent, a: &str, b: &str) -> Task<Message> {
        use claude_view::Event as C;
        use rules_view::Event as R;
        let boolean = |value: &str| value == "true";
        let message = match intent {
            Intent::PickFile => {
                if !boolean(a) {
                    self.import_open = true;
                }
                Message::PickFile(boolean(a))
            }
            Intent::RuleToggle => match a.parse::<usize>() {
                Ok(index) if index < 5 => Message::Rules(R::Toggle(index, boolean(b))),
                _ => Message::Noop,
            },
            Intent::RuleProfile => self.profiles.iter().find(|profile| profile.id == a).map_or(
                Message::Noop,
                |profile| {
                    Message::Rules(R::Profile(rules_view::ProfileChoice {
                        id: profile.id.clone(),
                        name: profile.name.clone(),
                    }))
                },
            ),
            Intent::ConnectionSort => Message::ConnectionSort(match a {
                "rule" => ConnectionField::Rule,
                "chain" => ConnectionField::Chain,
                "upload-rate" => ConnectionField::UploadRate,
                "download-rate" => ConnectionField::DownloadRate,
                "upload-total" => ConnectionField::UploadTotal,
                "download-total" => ConnectionField::DownloadTotal,
                "start" => ConnectionField::Start,
                _ => ConnectionField::Name,
            }),
            Intent::Confirm => return self.accept_dialog(),
            Intent::CancelConfirm | Intent::CloseDetail => return self.cancel_dialog(),
            Intent::Navigate => Message::Navigate(
                Page::ALL
                    .get(a.parse::<usize>().unwrap_or(0))
                    .copied()
                    .unwrap_or(Page::Home),
            ),
            Intent::Query => Message::Query(a.into()),
            Intent::DismissNotice => Message::DismissNotice,
            Intent::SystemNotification => Message::SystemNotification,
            Intent::RestartCore => Message::RestartCore,
            Intent::Mode => Message::Action(Action::Mode(
                ["rule", "global", "direct"]
                    .get(a.parse::<usize>().unwrap_or(0))
                    .unwrap_or(&"rule")
                    .to_string(),
            )),
            Intent::ProxyMode => Message::Action(Action::ProxyMode(match a {
                "0" => ProxyMode::System,
                "1" => ProxyMode::Tun,
                _ => ProxyMode::Off,
            })),
            Intent::NodeSort => Message::NodeSort(
                NodeSort::ALL
                    .get(a.parse::<usize>().unwrap_or(0))
                    .copied()
                    .unwrap_or_default(),
            ),
            Intent::ToggleGroup => Message::ToggleGroup(a.into()),
            Intent::GroupDelay => Message::GroupDelay(a.into()),
            Intent::GroupPage => Message::GroupPage(a.into(), boolean(b)),
            Intent::SelectNode => Message::Action(Action::Select(a.into(), b.into())),
            Intent::NodeDelay => Message::Action(Action::Delay(a.into())),
            Intent::ListPage => Message::ListPage(a.parse().unwrap_or(0)),
            Intent::ConnectionDetail => Message::ConnectionDetail(a.into()),
            Intent::CloseConnection => Message::Action(Action::Close(a.into())),
            Intent::CloseAll => Message::Action(Action::CloseAll),
            Intent::ProfileName => Message::ProfileName(a.into()),
            Intent::ProfileSource => Message::ProfileSource(a.into()),
            Intent::ImportProfile => Message::ImportProfile,
            Intent::OpenProfiles => Message::Action(Action::OpenProfiles),
            Intent::ActivateProfile => Message::Action(Action::Activate(a.into())),
            Intent::UpdateProfile => Message::Action(Action::UpdateProfile(a.into())),
            Intent::DeleteProfile => return self.confirm_delete(a.into()),
            Intent::EditProfile => Message::EditProfile(a.into()),
            Intent::AddProfile => Message::ImportDialog(true),
            Intent::EditName => Message::EditName(a.into()),
            Intent::EditSource => Message::EditSource(a.into()),
            Intent::SaveProfile => Message::SaveProfileEdit,
            Intent::CloseProfile => Message::CloseProfileEdit,
            Intent::RuleConfirmProfile => Message::Rules(R::ConfirmProfile),
            Intent::RuleCancelProfile => Message::Rules(R::CancelProfile),
            Intent::RuleCollapse => Message::Rules(R::Collapse),
            Intent::RuleFilter => Message::Rules(R::Filter(match a {
                "1" => rules_view::SourceFilter::Custom,
                "2" => rules_view::SourceFilter::Shortcut,
                "3" => rules_view::SourceFilter::Subscription,
                _ => rules_view::SourceFilter::All,
            })),
            Intent::RuleSelect => Message::Rules(R::Select(a.into())),
            Intent::RuleAdd => Message::Rules(R::Add),
            Intent::RuleEdit => Message::Rules(R::Edit),
            Intent::RuleDelete => Message::Rules(R::Delete),
            Intent::RuleEnable => Message::Rules(R::Enable),
            Intent::RuleMove => Message::Rules(R::Move(a.parse().unwrap_or(0))),
            Intent::RuleMoveOpen => Message::Rules(R::MoveOpen),
            Intent::RuleMoveInput => Message::Rules(R::MoveInput(a.into())),
            Intent::RuleMoveTo => Message::Rules(R::MoveTo),
            Intent::RuleMoveEdge => Message::Rules(R::MoveEdge(boolean(a))),
            Intent::RuleRestore => Message::Rules(R::Restore),
            Intent::RuleDiscard => Message::Rules(R::Discard),
            Intent::RuleApply => Message::Rules(R::Apply),
            Intent::RuleCloseEditor => Message::Rules(R::CloseEditor),
            Intent::RuleTextMode => Message::Rules(R::TextMode(boolean(a))),
            Intent::RuleRaw => Message::Rules(R::Raw(a.into())),
            Intent::RuleKind => Message::Rules(R::Kind(a.into())),
            Intent::RulePayload => Message::Rules(R::Payload(a.into())),
            Intent::RulePolicy => Message::Rules(R::Policy(a.into())),
            Intent::RuleNoResolve => Message::Rules(R::NoResolve(boolean(a))),
            Intent::RuleEnabled => Message::Rules(R::Enabled(boolean(a))),
            Intent::RulePosition => Message::Rules(R::Position(a.into())),
            Intent::RuleSave => Message::Rules(R::SaveEditor),
            Intent::LogLevel => Message::LogLevel(logs::Level::parse(a)),
            Intent::LogDetail => Message::LogDetail(a.into()),
            Intent::ClearLogs => Message::Action(Action::ClearLogs),
            Intent::ExportLogs => Message::Action(Action::ExportLogs),
            Intent::TestUrl => Message::TestUrl(a.into()),
            Intent::DnsHost => Message::DnsHost(a.into()),
            Intent::Probe => Message::Probe(boolean(a)),
            Intent::Dns => Message::Dns,
            Intent::SiteAll => Message::SiteAll,
            Intent::SiteCancel => Message::SiteCancel,
            Intent::SiteCategory => Message::SiteCategory(self.category(a)),
            Intent::SiteOpenCategory => {
                Message::SiteSummaryFilter(self.category(a), StatusFilter::All)
            }
            Intent::SiteStatus => {
                let status = Status::ALL
                    .into_iter()
                    .find(|status| status.to_string() == a)
                    .map_or(StatusFilter::All, StatusFilter::Status);
                if b == "all" {
                    Message::SiteSummaryFilter(CategoryFilter::All, status)
                } else {
                    Message::SiteStatus(status)
                }
            }
            Intent::SiteRegion => Message::SiteRegion(self.region(a)),
            Intent::SiteDetail => ip_check::services()
                .iter()
                .position(|service| service.id == a)
                .map_or(Message::Noop, Message::SiteDetail),
            Intent::SiteReset => Message::SiteResetFilters,
            Intent::SiteReport => Message::SiteReport,
            Intent::SiteRetest => Message::SiteRetest,
            Intent::SiteProbe => Message::SiteProbe(a.into()),
            Intent::ClaudeStart => Message::Claude(C::Start),
            Intent::ClaudeCloseSession => Message::Claude(C::CloseSession),
            Intent::ClaudeCancel => Message::Claude(C::Cancel),
            Intent::ClaudeProtection => Message::Claude(C::Protection(boolean(a))),
            Intent::ClaudeConfirmProtection => Message::Claude(C::ConfirmProtection),
            Intent::ClaudeFilter => Message::Claude(C::ResultsFilter(match a {
                "0" => Some(claude_view::ResultKind::Normal),
                "1" => Some(claude_view::ResultKind::Risk),
                "2" => Some(claude_view::ResultKind::Unknown),
                _ => None,
            })),
            Intent::ClaudePage => Message::Claude(C::ResultsPage(a.parse().unwrap_or(0))),
            Intent::ClaudeGuide => Message::Claude(C::Guide(match a {
                "diagnostics" => Some(claude_view::Guide::Diagnostics),
                "supplement" => Some(claude_view::Guide::Supplement),
                _ => None,
            })),
            Intent::ClaudeBrowser => Message::Claude(C::Browser),
            Intent::ClaudeCopyScript => Message::Claude(C::CopySiteScript),
            Intent::ClaudeImport => Message::Claude(C::Import),
            Intent::ClaudeExport => Message::Claude(C::Export),
            Intent::ClaudeKey => Message::Claude(C::ReputationKey(a.into())),
            Intent::ClaudeSaveKey => Message::Claude(C::SaveReputationKey),
            Intent::ClaudeAdditional => Message::Claude(C::Additional(match a {
                "registration" => clash_of_rust::claude_check::additional::Kind::Registration,
                "global-ping" => clash_of_rust::claude_check::additional::Kind::GlobalPing,
                _ => clash_of_rust::claude_check::additional::Kind::Speed,
            })),
            Intent::Appearance => Message::Appearance(match a {
                "0" => ThemeMode::Dark,
                "1" => ThemeMode::Light,
                _ => ThemeMode::System,
            }),
            Intent::TextScale => Message::TextScale(a.trim_end_matches('%').parse().unwrap_or(100)),
            Intent::AccentPicker => Message::AccentDialog(true),
            Intent::AccentColor => {
                let Some(color) = a
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| AccentColor::ALL.get(i).copied())
                else {
                    return Task::none();
                };
                self.accent_open = false;
                Message::Action(Action::Accent(color))
            }
            Intent::TraySpeed => Message::TraySpeed(boolean(a)),
            Intent::DnsOverride => Message::Action(Action::DnsOverride(boolean(a))),
            Intent::Ipv6 => Message::Action(Action::Ipv6(boolean(a))),
            Intent::Udp => Message::Action(Action::Udp(boolean(a))),
            Intent::Autostart => Message::Action(Action::Autostart(boolean(a))),
            Intent::ControllerPort => Message::ControllerPort(a.into()),
            Intent::MixedPort => Message::MixedPort(a.into()),
            Intent::SavePorts => Message::SavePorts,
            Intent::WindowWidth => Message::WindowWidth(a.into()),
            Intent::WindowHeight => Message::WindowHeight(a.into()),
            Intent::ResetWindow => Message::ResetWindowSize,
            Intent::IntervalInput => Message::IntervalInput(a.into()),
            Intent::SaveInterval => Message::SaveInterval,
            Intent::UpdateGeo => Message::Action(Action::UpdateGeo),
            Intent::CheckUpdate => Message::CheckUpdate,
            Intent::InstallUpdate => Message::InstallUpdate,
            Intent::CancelUpdate => Message::CancelUpdateDownload,
            Intent::OpenRelease => Message::OpenRelease,
            Intent::Copy => Message::Copy(a.into()),
            Intent::Quit => return self.request_exit(),
            Intent::FocusSearch => return runtime::widget::operation::focus("page-search"),
            Intent::Escape => Message::Key(runtime::keyboard::Event::KeyPressed {
                key: runtime::keyboard::Key::Named(runtime::keyboard::key::Named::Escape),
                modifiers: runtime::keyboard::Modifiers::default(),
            }),
        };
        self.update(message)
    }
    fn accept_dialog(&mut self) -> Task<Message> {
        if self.confirmation.is_some() {
            return self.update(Message::Confirm);
        }
        if self.profile_editor.is_some() {
            return self.update(Message::SaveProfileEdit);
        }
        if self.import_open {
            return self.update(Message::ImportProfile);
        }
        if self.rule_state.pending_profile.is_some() {
            return self.update(Message::Rules(rules_view::Event::ConfirmProfile));
        }
        if self.rule_state.editor.is_some() {
            return self.update(Message::Rules(rules_view::Event::SaveEditor));
        }
        if self.rule_state.move_open {
            return self.update(Message::Rules(rules_view::Event::MoveTo));
        }
        if self.claude.guide == Some(claude_view::Guide::Protection) {
            return self.update(Message::Claude(claude_view::Event::ConfirmProtection));
        }
        self.accent_open = false;
        Task::none()
    }
    fn cancel_dialog(&mut self) -> Task<Message> {
        if self.confirmation.is_some() {
            return self.update(Message::CancelConfirm);
        }
        if self.profile_editor.is_some() {
            return self.update(Message::CloseProfileEdit);
        }
        if self.import_open {
            return self.update(Message::ImportDialog(false));
        }
        if self.rule_state.pending_profile.is_some() {
            return self.update(Message::Rules(rules_view::Event::CancelProfile));
        }
        if self.rule_state.modal_open() {
            return self.update(Message::Rules(rules_view::Event::CloseEditor));
        }
        if self.claude.guide.is_some() {
            return self.update(Message::Claude(claude_view::Event::Guide(None)));
        }
        if self.connection_detail.is_some() {
            return self.update(Message::CloseConnectionDetail);
        }
        if self.log_detail.is_some() {
            return self.update(Message::CloseLogDetail);
        }
        self.site_detail = None;
        self.accent_open = false;
        Task::none()
    }
}

#[cfg(test)]
pub(super) fn invalidate_window_for_test(window: &slint::Window) {
    repaint::invalidate(window);
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use slint::Model;

    fn host() -> DesktopHost {
        ui_testing::initialize(950, 700);
        let mut host = DesktopHost {
            app: ui_tests::fixture(Page::Rules),
            worker: Arc::new(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap(),
            ),
            window: None,
            models: ViewModels::default(),
            clipboard: None,
            tray_listening: false,
            next_update: Instant::now(),
            last_delay: Instant::now(),
            fatal: None,
            event_loop_started: true,
            retiring: None,
            saved_view: None,
        };
        attach(&mut host);
        host
    }
    fn attach(host: &mut DesktopHost) -> window::Id {
        let id = window::Id::unique();
        let window = AppWindow::new().unwrap();
        host.models.attach(&window.global::<UiData>());
        host.window = Some((id, window));
        host.app.visible = true;
        host.sync();
        id
    }

    #[test]
    fn retiring_releases_the_component_and_models_but_keeps_the_rule_draft() {
        let mut host = host();
        let _ = host.app.intent(Intent::RuleAdd, "", "");
        let _ = host.app.intent(Intent::RulePayload, "example.test", "");
        host.sync();
        for _ in 0..20 {
            let (id, window) = host.window.as_ref().unwrap();
            let id = *id;
            let weak = window.as_weak();
            let old_rules = host.models.rules.clone();
            host.saved_view = Some(view_state::State::capture(window));
            host.app.visible = false;
            host.retire_window(id);
            assert!(
                weak.upgrade().is_none(),
                "a strong handle retained the retired component"
            );
            assert!(host.window.is_none());
            assert_eq!(old_rules.row_count(), 0);
            assert_eq!(
                host.app.rule_state.editor.as_ref().unwrap().payload,
                "example.test"
            );
            attach(&mut host);
            let data = host.window.as_ref().unwrap().1.global::<UiData>();
            assert_eq!(data.get_rule_payload(), "example.test");
            assert_eq!(data.get_modal(), 4);
        }
    }

    #[test]
    fn a_file_picker_defers_retirement_and_reopening_cancels_it() {
        let mut host = host();
        let id = host.current_window_id().unwrap();
        host.app.visible = false;
        host.app.picking_file = true;
        host.retire_window(id);
        assert!(host.window.is_some());
        host.message(Message::ShowWindow(Some(id)));
        host.app.picking_file = false;
        host.finish_retirement();
        assert!(host.window.is_some());
        host.app.visible = false;
        host.retire_window(id);
        assert!(host.window.is_none());
    }

    #[test]
    fn stale_window_events_cannot_change_the_new_window_or_prevent_a_reopen() {
        let mut host = host();
        let old = host.current_window_id().unwrap();
        host.app.visible = false;
        host.retire_window(old);
        host.message(Message::Window((old, window::Event::Focused)));
        assert!(!host.app.visible);
        host.message(Message::ShowWindow(Some(old)));
        assert!(host.app.window_opening);
        let current = attach(&mut host);
        assert_ne!(old, current);
        host.app.visible = false;
        host.message(Message::Window((old, window::Event::CloseRequested)));
        assert!(host.window.is_some());
        assert_eq!(host.current_window_id(), Some(current));
        host.app.visible = true;
        host.retire_window(current);
        assert!(
            host.window.is_some(),
            "a late close command hid a newly reopened window"
        );
    }
}
