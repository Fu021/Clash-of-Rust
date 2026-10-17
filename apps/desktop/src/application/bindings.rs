//! The only desktop adapter that knows about Slint or winit.
use super::*;
use clash_client_ui::{AppWindow, Intent, UiActions, UiData, models::ViewModels};
use futures_util::StreamExt;
use runtime::{Size, native, window};
use slint::{
    ComponentHandle, Timer, TimerMode,
    winit_030::{EventResult, WinitWindowAccessor, winit},
};
use std::time::Instant;

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
        let _ = worker.block_on(async { engine.lock().await.stop().await });
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
    fn message(&mut self, message: Message) {
        let task = self.app.update(message);
        self.sync();
        self.spawn(task);
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
        }
    }
}
impl native::Host for DesktopHost {
    fn current_window(&self) -> Option<window::Id> {
        self.window.as_ref().map(|(id, _)| *id)
    }
    fn open_window(&mut self, id: window::Id, settings: window::Settings) {
        if let Some((_, window)) = &self.window {
            let _ = window.show();
            return;
        }
        let window = match AppWindow::new() {
            Ok(window) => window,
            Err(error) => {
                self.fatal = Some(error.to_string());
                let _ = slint::quit_event_loop();
                return;
            }
        };
        self.models.attach(&window.global::<UiData>());
        window.global::<UiActions>().on_dispatch(|intent, a, b| {
            // Queue even UI-originated commands to avoid reentrant state/model updates.
            let _ = slint::invoke_from_event_loop(move || {
                HOST.with(|slot| {
                    if let Some(host) = slot.borrow_mut().as_mut() {
                        let task = host.app.intent(intent, a.as_str(), b.as_str());
                        host.sync();
                        host.spawn(task);
                    }
                })
            });
        });
        window.window().on_close_requested(move || {
            enqueue(Message::Window((id, window::Event::CloseRequested)));
            slint::CloseRequestResponse::KeepWindowShown
        });
        let mut modifiers = winit::keyboard::ModifiersState::empty();
        window.window().on_winit_window_event(move |window, event| {
            use winit::event::{ElementState, WindowEvent};
            match event {
                WindowEvent::Resized(size) => {
                    let logical = size.to_logical::<f32>(f64::from(window.scale_factor()));
                    enqueue(Message::Window((
                        id,
                        window::Event::Resized(Size::new(logical.width, logical.height)),
                    )));
                }
                WindowEvent::ScaleFactorChanged { .. } => {
                    enqueue(Message::Window((id, window::Event::Rescaled)))
                }
                WindowEvent::Focused(true) => {
                    enqueue(Message::Window((id, window::Event::Focused)))
                }
                WindowEvent::ThemeChanged(theme) => enqueue(Message::SystemTheme(
                    if *theme == winit::window::Theme::Dark {
                        runtime::theme::Mode::Dark
                    } else {
                        runtime::theme::Mode::Light
                    },
                )),
                WindowEvent::ModifiersChanged(value) => modifiers = value.state(),
                WindowEvent::DroppedFile(path) => {
                    enqueue(Message::ImportDialog(true));
                    enqueue(Message::ProfileSource(path.to_string_lossy().into_owned()));
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
                        enqueue(Message::Key(runtime::keyboard::Event::KeyPressed {
                            key: runtime::keyboard::Key::Character(text.to_string()),
                            modifiers: runtime::keyboard::Modifiers { command: true },
                        }));
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
        self.window = Some((id, window));
        self.sync();
        if settings.visible
            && let Some((_, window)) = &self.window
        {
            let _ = window.show();
        }
        enqueue(Message::Window((id, window::Event::Opened {})));
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
        let Some((current, window)) = &self.window else {
            return;
        };
        if *current != id {
            return;
        }
        match command {
            window::Command::Mode(window::Mode::Hidden) => {
                let _ = window.hide();
                self.models.clear();
            }
            window::Command::Mode(window::Mode::Windowed) => {
                let _ = window.show();
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
            let _ = clipboard.set_text(text);
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
            Intent::WslInput => Message::Wsl(wsl::Event::Input(a.into(), b.into())),
            Intent::WslAction => Message::Wsl(wsl::Event::Action(a.into(), b.into())),
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
