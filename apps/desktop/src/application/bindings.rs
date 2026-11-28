//! Native application owner. Slint runs only in a disposable child process.
use super::*;
use anyhow::Context;
use clash_client_core::config::{documents, overrides};
use clash_desktop_protocol::{self as wire, HostPacket, UiData, UiEvent, models::ViewModels};
use futures_util::StreamExt;
use runtime::native::Host as _;
use runtime::{Size, native, window};
use std::{process::Stdio, time::Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(target_os = "macos")]
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    window::WindowId,
};

enum HostEvent {
    Message(Message),
    Native(native::Call),
    Ui(window::Id, UiEvent),
    Yaml(
        window::Id,
        i32,
        documents::Operation,
        bool,
        anyhow::Result<documents::Reply>,
        Option<tempfile::TempDir>,
    ),
}
#[cfg(target_os = "macos")]
type HostProxy = EventLoopProxy<HostEvent>;
#[cfg(not(target_os = "macos"))]
#[derive(Clone)]
struct HostProxy(std::sync::mpsc::Sender<HostEvent>);
#[cfg(not(target_os = "macos"))]
impl HostProxy {
    fn send_event(&self, event: HostEvent) -> std::io::Result<()> {
        self.0
            .send(event)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }
}
struct Renderer {
    id: window::Id,
    child: tokio::process::Child,
    commands: tokio::sync::mpsc::UnboundedSender<Outbound>,
    frames: tokio::sync::watch::Sender<Option<Arc<[u8]>>>,
    metrics: wire::Metrics,
    closing: Option<Instant>,
    retiring: bool,
    revision: u64,
    reclaim_pending: bool,
    yaml_request: Option<(i32, tokio::sync::watch::Sender<bool>)>,
    yaml_external: Option<tempfile::TempDir>,
    yaml_poll_failed: bool,
}
#[derive(Debug)]
struct Outbound {
    bytes: Arc<[u8]>,
    destroy: bool,
}
fn allow_renderer_focus(child: &tokio::process::Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        // The tray receives the user's click in the host process. Grant only
        // our renderer permission to raise its window in response.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
        }
    }
    #[cfg(not(windows))]
    let _ = child;
}
impl Renderer {
    fn cancel_yaml(&mut self) {
        if let Some((_, cancel)) = self.yaml_request.take() {
            cancel.send_replace(true);
        }
    }

    fn send(&self, packet: HostPacket) {
        if matches!(
            &packet,
            HostPacket::Command(wire::WindowCommand::Show | wire::WindowCommand::Focus)
        ) {
            allow_renderer_focus(&self.child);
        }
        let destroy = matches!(&packet, HostPacket::Command(wire::WindowCommand::Destroy));
        if let Ok(bytes) = wire::encode(&packet) {
            let _ = self.commands.send(Outbound {
                bytes: bytes.into(),
                destroy,
            });
        }
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        self.cancel_yaml();
    }
}
struct DesktopHost {
    app: App,
    worker: Arc<tokio::runtime::Runtime>,
    proxy: HostProxy,
    window: Option<Renderer>,
    retired: Vec<Renderer>,
    data: UiData,
    models: ViewModels,
    last_frame: Option<Arc<[u8]>>,
    clipboard: Option<arboard::Clipboard>,
    tray_listening: bool,
    last_delay: Instant,
    saved_view: Option<wire::ViewState>,
    position_save_at: Option<Instant>,
    quit: bool,
    started: bool,
    #[cfg(target_os = "macos")]
    pickers:
        BTreeMap<u64, tokio::sync::oneshot::Sender<Result<Option<std::path::PathBuf>, String>>>,
    #[cfg(target_os = "macos")]
    next_picker: u64,
}

pub(super) fn run(
    startup: Option<tray::Startup>,
    state: window_state::State,
) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    let events = EventLoop::<HostEvent>::with_user_event().build()?;
    #[cfg(target_os = "macos")]
    let proxy = events.create_proxy();
    #[cfg(not(target_os = "macos"))]
    let (proxy, events) = {
        let (sender, receiver) = std::sync::mpsc::channel();
        (HostProxy(sender), receiver)
    };
    let worker = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?,
    );
    let _context = worker.enter();
    let (nodes_list, profiles_list) = state.list_views();
    let saved_view = Some(wire::ViewState {
        position: state.position(),
        nodes_list,
        profiles_list,
        ..Default::default()
    });
    let (app, initial) = App::new(startup, state);
    let mut host = DesktopHost {
        app,
        worker: worker.clone(),
        proxy: proxy.clone(),
        window: None,
        retired: Vec::new(),
        data: UiData::default(),
        models: ViewModels::default(),
        last_frame: None,
        clipboard: None,
        tray_listening: false,
        last_delay: Instant::now(),
        saved_view,
        position_save_at: None,
        quit: false,
        started: false,
        #[cfg(target_os = "macos")]
        pickers: BTreeMap::new(),
        #[cfg(target_os = "macos")]
        next_picker: 0,
    };
    let native_proxy = proxy.clone();
    native::install(move |call| {
        let _ = native_proxy.send_event(HostEvent::Native(call));
    });
    host.spawn(initial);
    host.spawn(interaction::theme_updates());
    let ticker = worker.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            if proxy.send_event(HostEvent::Message(Message::Tick)).is_err() {
                break;
            }
        }
    });
    #[cfg(target_os = "macos")]
    let result = events.run_app(&mut host).map_err(anyhow::Error::from);
    #[cfg(not(target_os = "macos"))]
    let result: anyhow::Result<()> = {
        // Linux/Windows tray and notification integrations own their native
        // event threads. The resident owner needs no display connection.
        host.start();
        for event in events {
            host.event(event);
            if host.quit {
                break;
            }
        }
        Ok(())
    };
    ticker.abort();
    let mut renderers = host.retired;
    if let Some(window) = host.window.take() {
        renderers.push(window);
    }
    worker.block_on(async move {
        for mut renderer in renderers {
            let _ = renderer.child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(5), renderer.child.wait()).await;
        }
    });
    if !host.app.exiting
        && let Some(engine) = host.app.engine.clone()
    {
        let _ = worker.block_on(async move {
            let mut engine = engine.lock().await;
            if !engine.is_remote() {
                engine.stop().await?;
            }
            Ok::<_, anyhow::Error>(())
        });
    }
    let saved_state = worker.block_on(host.app.window_state.save());
    result?;
    saved_state?;
    Ok(())
}
fn size(value: wire::Size) -> Size {
    Size::new(value.width, value.height)
}
fn wire_size(value: Size) -> wire::Size {
    wire::Size {
        width: value.width,
        height: value.height,
    }
}
impl DesktopHost {
    fn spawn(&self, task: Task<Message>) {
        let proxy = self.proxy.clone();
        self.worker.spawn(async move {
            let mut stream = task.0;
            while let Some(message) = stream.next().await {
                if proxy.send_event(HostEvent::Message(message)).is_err() {
                    break;
                }
            }
        });
    }
    fn current_window_id(&self) -> Option<window::Id> {
        self.window
            .as_ref()
            .filter(|w| w.closing.is_none())
            .map(|w| w.id)
    }
    fn message(&mut self, mut message: Message) {
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
            if let Some(window) = self.window.as_mut() {
                window.retiring = false;
            }
        }
        let tick = matches!(message, Message::Tick);
        let finished_page = matches!(
            &message,
            Message::Finished(reply)
                if matches!(reply.scope, Scope::Home | Scope::Proxies | Scope::Connections | Scope::Rules | Scope::Logs)
        );
        let page = self.app.page;
        let overview_busy = self.app.overview.busy();
        let task = self.app.update(message);
        if (page != self.app.page || (overview_busy && !self.app.overview.busy()))
            && let Some(window) = self.window.as_mut()
        {
            window.reclaim_pending = true;
        }
        self.spawn(task);
        self.finish_retirement();
        if finished_page && !self.app.visible && self.window.is_none() {
            platform::memory::release_unused_heap();
        }
        self.sync();
        if self.app.tray.is_some() && !self.tray_listening {
            self.tray_listening = true;
            let proxy = self.proxy.clone();
            self.worker.spawn(async move {
                while let Some(command) = tray::next().await {
                    if proxy
                        .send_event(HostEvent::Message(Message::Tray(command)))
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        if tick {
            if self.position_save_at.is_some_and(|at| Instant::now() >= at) {
                self.save_geometry();
            }
            self.reap();
            if self.app.visible
                && self.app.engine.is_some()
                && !self.app.window_opening
                && !self.app.busy
                && !self.app.working
                && let Some(window) = self
                    .window
                    .as_mut()
                    .filter(|window| window.reclaim_pending && window.closing.is_none())
            {
                window.reclaim_pending = false;
                platform::memory::release_unused_heap();
            }
            if self.app.claude.session.is_some() {
                self.message(Message::Claude(claude_view::Event::Tick));
            }
            let now = Instant::now();
            let task = self.app.auto_check_update();
            self.spawn(task);
            let interval = self.app.settings.delay_interval_minutes;
            if interval > 0
                && now.duration_since(self.last_delay)
                    >= Duration::from_secs(u64::from(interval) * 60)
            {
                self.last_delay = now;
                self.message(Message::PeriodicDelay);
            }
        }
    }
    fn remember_view(&mut self, mut view: wire::ViewState) {
        let minimized = self.window.as_ref().is_some_and(|w| w.metrics.minimized);
        let views_changed = self
            .app
            .window_state
            .capture_list_views(view.nodes_list, view.profiles_list);
        if self.app.window_state.capture_position(
            view.position,
            view.maximized,
            minimized,
            view.fullscreen,
        ) || views_changed
        {
            self.position_save_at = Some(Instant::now() + Duration::from_millis(750));
        }
        if view.maximized || view.fullscreen || minimized {
            view.position = self.app.window_state.position();
        }
        self.saved_view = Some(view);
    }
    fn save_geometry(&mut self) {
        self.position_save_at = None;
        if !self.app.window_state.dirty() {
            return;
        }
        let state = self.app.window_state.clone();
        let proxy = self.proxy.clone();
        self.worker.spawn(async move {
            let result = state.save().await.map_err(|error| format!("{error:#}"));
            let _ = proxy.send_event(HostEvent::Message(Message::WindowStateSaved(result)));
        });
    }
    fn sync(&mut self) {
        if !self.app.visible || self.current_window_id().is_none() {
            self.app.copy_result = None;
            return;
        }
        if self.app.exiting {
            self.app.copy_result = None;
        }
        self.app.present(&self.data, &mut self.models);
        if let Ok(bytes) = wire::encode(&HostPacket::Frame {
            data: self.data.snapshot(),
            accepted: self.window.as_ref().map_or(0, |w| w.revision),
            view_ready: !self.app.window_opening
                && !self.app.size_pending
                && (!self.app.busy
                    || !matches!(
                        self.app.page,
                        Page::Proxies | Page::Connections | Page::Rules | Page::Logs
                    )),
        }) && self.last_frame.as_deref() != Some(bytes.as_slice())
        {
            let bytes: Arc<[u8]> = bytes.into();
            self.last_frame = Some(bytes.clone());
            if let Some(window) = &self.window {
                let _ = window.frames.send(Some(bytes));
            }
        }
    }
    fn finish_retirement(&mut self) {
        if self.app.visible || self.app.picking_file {
            return;
        }
        let Some(window) = self
            .window
            .as_mut()
            .filter(|w| w.retiring && w.closing.is_none())
        else {
            return;
        };
        if self.app.window_state.capture(
            size(window.metrics.size),
            window.metrics.maximized,
            Some(window.metrics.minimized),
        ) || self.app.window_state.dirty()
        {
            self.position_save_at = None;
            let state = self.app.window_state.clone();
            let proxy = self.proxy.clone();
            self.worker.spawn(async move {
                let result = state.save().await.map_err(|error| format!("{error:#}"));
                let _ = proxy.send_event(HostEvent::Message(Message::WindowStateSaved(result)));
            });
        }
        window.cancel_yaml();
        window.yaml_external = None;
        window.send(HostPacket::Command(wire::WindowCommand::Destroy));
        window.closing = Some(Instant::now());
        let _ = window.frames.send(None);
        self.models = ViewModels::default();
        self.data = UiData::default();
        self.last_frame = None;
        platform::memory::release_unused_heap();
    }
    fn reap(&mut self) {
        let mut reaped = false;
        if let Some(window) = &mut self.window {
            if window
                .closing
                .is_some_and(|at| at.elapsed() > Duration::from_secs(3))
            {
                let _ = window.child.start_kill();
            }
            if let Ok(Some(status)) = window.child.try_wait() {
                reaped = true;
                let expected = window.closing.is_some();
                self.window = None;
                if !expected && !self.app.exiting {
                    #[cfg(target_os = "macos")]
                    for (_, sender) in std::mem::take(&mut self.pickers) {
                        let _ = sender.send(Err("界面进程已退出".into()));
                    }
                    self.app.visible = false;
                    self.app.window_opening = false;
                    self.app.size_pending = false;
                    self.models = ViewModels::default();
                    self.data = UiData::default();
                    self.last_frame = None;
                    self.app.show_notice(
                        format!("界面进程已退出（{status}），可从托盘或 Dock 重新打开；后台代理仍在运行。"),
                        true,
                    );
                }
            }
        }
        self.retired.retain_mut(|window| {
            if window
                .closing
                .is_some_and(|at| at.elapsed() > Duration::from_secs(3))
            {
                let _ = window.child.start_kill();
            }
            let alive = !matches!(window.child.try_wait(), Ok(Some(_)));
            reaped |= !alive;
            alive
        });
        if reaped && !self.app.visible {
            // The pipe writer has also dropped its final encoded frame now.
            platform::memory::release_unused_heap();
        }
    }
    fn ui_event(&mut self, id: window::Id, event: UiEvent) {
        if self.current_window_id() != Some(id) {
            return;
        }
        match event {
            UiEvent::Opened {
                protocol,
                version,
                metrics,
                preserve_native_state,
            } => {
                if protocol != wire::PROTOCOL || version != clash_of_rust::VERSION {
                    self.app.visible = false;
                    self.app
                        .show_notice("界面与后台版本不一致，请重新安装完整客户端。".into(), true);
                    if let Some(w) = self.window.as_mut() {
                        w.retiring = true;
                    }
                    self.finish_retirement();
                    return;
                }
                self.window.as_mut().unwrap().metrics = metrics;
                self.message(Message::Window((
                    id,
                    window::Event::Opened {
                        preserve_native_state,
                    },
                )));
            }
            UiEvent::Intent {
                revision,
                intent,
                a,
                b,
                view,
            } => {
                self.remember_view(view);
                self.window.as_mut().unwrap().revision = revision;
                let task = self.app.intent(intent, &a, &b);
                if let Some(copy) = self.app.copy_request.take()
                    && self.app.visible
                    && !self.app.exiting
                {
                    let script = matches!(copy, desktop_view::CopyRequest::ClaudeScript);
                    let written = self.write_clipboard(copy.into_text()).is_ok();
                    self.app.finish_copy(revision as i32, script, written);
                }
                self.spawn(task);
                self.finish_retirement();
                self.sync();
            }
            UiEvent::Geometry { metrics, view } => {
                let dimensions = size(metrics.size);
                let dark = metrics.system_dark;
                let previous = &self.window.as_ref().unwrap().metrics;
                let resized = previous.size != metrics.size
                    || previous.maximized != metrics.maximized
                    || previous.minimized != metrics.minimized;
                self.window.as_mut().unwrap().metrics = metrics;
                self.remember_view(view);
                if self.app.system_dark != dark {
                    self.message(Message::SystemTheme(if dark {
                        runtime::theme::Mode::Dark
                    } else {
                        runtime::theme::Mode::Light
                    }));
                }
                // Moves and theme changes also report geometry. Do not turn
                // them into another resize/minimum-size/presentation round trip.
                if resized {
                    self.message(Message::Window((id, window::Event::Resized(dimensions))));
                }
            }
            UiEvent::Close(view) => {
                let window = self.window.as_mut().unwrap();
                window.cancel_yaml();
                window.yaml_external = None;
                self.remember_view(view);
                self.message(Message::Window((id, window::Event::CloseRequested)));
            }
            UiEvent::Focused => self.message(Message::Window((id, window::Event::Focused))),
            UiEvent::Rescaled => self.message(Message::Window((id, window::Event::Rescaled))),
            UiEvent::FilePicked { id: picker, result } => {
                #[cfg(target_os = "macos")]
                if let Some(sender) = self.pickers.remove(&picker) {
                    let _ = sender.send(result);
                }
                #[cfg(not(target_os = "macos"))]
                let _ = (picker, result);
            }
            UiEvent::Yaml {
                id: request,
                action,
            } => self.run_yaml(id, request, action),
            UiEvent::Error(error) => {
                self.app.show_notice(format!("界面错误：{error}"), true);
                self.sync();
            }
        }
    }

    fn run_yaml(&mut self, window_id: window::Id, id: i32, action: Option<String>) {
        let window = self.window.as_mut().expect("current renderer");
        window.cancel_yaml();
        let Some(action) = action else {
            window.yaml_external = None;
            window.yaml_poll_failed = false;
            return;
        };
        let action = match serde_json::from_str::<documents::Action>(&action) {
            Ok(action) => action,
            Err(_) => {
                window.send(HostPacket::Command(wire::WindowCommand::Yaml {
                    id,
                    result: Err("YAML 请求无效".into()),
                }));
                return;
            }
        };
        let external_path = window
            .yaml_external
            .as_ref()
            .map(|directory| directory.path().join("merge.yaml"));
        let polling = matches!(&action, documents::Action::PollExternal(_));
        let mut operation = action.operation();
        let (cancel, cancellation) = tokio::sync::watch::channel(false);
        window.yaml_request = Some((id, cancel));
        let engine = self.app.engine.clone();
        let transition = self.app.claude.protection_pending.is_some();
        let proxy = self.proxy.clone();
        self.worker.spawn(async move {
            let mut opened = None;
            let result = async {
                use documents::{Action, Reply};
                let engine = engine.context("客户端后台未就绪")?;
                let result = match action {
                    Action::Source(request) => Reply::Source(
                        Engine::subscription_source(&engine, request, Some(cancellation.clone()))
                            .await?,
                    ),
                    Action::OpenExternal(text) => {
                        anyhow::ensure!(!transition, "Claude 保护正在切换");
                        engine.lock().await.settings.require_mutable()?;
                        anyhow::ensure!(
                            text.len() <= overrides::EDIT_LIMIT,
                            "全局覆写不能超过 128 KiB"
                        );
                        let cancel = cancellation.clone();
                        opened = Some(
                            tokio::task::spawn_blocking(move || {
                                let directory =
                                    tempfile::Builder::new().prefix("cor-override-").tempdir()?;
                                let path = directory.path().join("merge.yaml");
                                std::fs::write(&path, text)?;
                                anyhow::ensure!(!*cancel.borrow(), "YAML 请求已取消");
                                platform::open_text_editor(&path)?;
                                Ok::<_, anyhow::Error>(directory)
                            })
                            .await??,
                        );
                        Reply::ExternalOpened
                    }
                    Action::PollExternal(previous) => {
                        anyhow::ensure!(!transition, "Claude 保护正在切换");
                        engine.lock().await.settings.require_mutable()?;
                        let path = external_path.context("尚未打开外部编辑器")?;
                        let draft =
                            tokio::task::spawn_blocking(move || overrides::read_draft(&path))
                                .await??;
                        if overrides::revision(&draft) == previous {
                            Reply::ExternalUnchanged
                        } else {
                            Reply::ExternalDraft(draft)
                        }
                    }
                    Action::WriteExternal { mut change, apply } => {
                        anyhow::ensure!(!transition, "Claude 保护正在切换");
                        engine.lock().await.settings.require_mutable()?;
                        let path = external_path.context("尚未打开外部编辑器")?;
                        operation = documents::Operation::ReadExternal;
                        change.text =
                            tokio::task::spawn_blocking(move || overrides::read_draft(&path))
                                .await??;
                        operation = if apply {
                            documents::Operation::Apply
                        } else {
                            documents::Operation::Check
                        };
                        let mut engine = engine.lock().await;
                        anyhow::ensure!(!*cancellation.borrow(), "YAML 请求已取消");
                        if apply {
                            let revision = overrides::revision(&change.text);
                            engine.save_global_override(change).await?;
                            Reply::ExternalSaved {
                                revision,
                                closed: false,
                            }
                        } else {
                            engine.check_global_override(change).await?;
                            Reply::Valid
                        }
                    }
                    _ => {
                        let mut engine = engine.lock().await;
                        anyhow::ensure!(!*cancellation.borrow(), "YAML 请求已取消");
                        match action {
                            Action::OpenOverride => {
                                Reply::Override(engine.global_override_document().await?)
                            }
                            Action::Check(change) => {
                                anyhow::ensure!(!transition, "Claude 保护正在切换");
                                engine.check_global_override(change).await?;
                                Reply::Valid
                            }
                            Action::Apply(change) => {
                                anyhow::ensure!(!transition, "Claude 保护正在切换");
                                let revision = overrides::revision(&change.text);
                                engine.save_global_override(change).await?;
                                Reply::Saved(revision)
                            }
                            Action::Source(_)
                            | Action::OpenExternal(_)
                            | Action::PollExternal(_)
                            | Action::WriteExternal { .. } => {
                                unreachable!()
                            }
                        }
                    }
                };
                Ok::<_, anyhow::Error>(result)
            }
            .await;
            let critical = result
                .as_ref()
                .err()
                .and_then(|error| error.downcast_ref::<documents::Recovery>())
                .copied()
                .is_some_and(documents::Recovery::critical);
            if critical || !*cancellation.borrow() {
                let _ = proxy.send_event(HostEvent::Yaml(
                    window_id, id, operation, polling, result, opened,
                ));
            }
        });
    }
}
impl DesktopHost {
    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        if self.app.visible {
            self.open_window(
                window::Id::unique(),
                window::Settings {
                    size: self.app.window_state.size(),
                    visible: true,
                },
            );
        }
    }
    fn event(&mut self, event: HostEvent) {
        match event {
            HostEvent::Message(message) => self.message(message),
            HostEvent::Native(call) => {
                call(self);
                self.sync();
            }
            HostEvent::Ui(id, event) => self.ui_event(id, event),
            HostEvent::Yaml(window_id, id, operation, polling, mut result, opened) => {
                let window = self.window.as_mut().filter(|window| {
                    window.id == window_id
                        && window.closing.is_none()
                        && window
                            .yaml_request
                            .as_ref()
                            .is_some_and(|(current, _)| *current == id)
                });
                let active = window.is_some();
                let mut failure = result
                    .as_ref()
                    .err()
                    .map(|error| documents::Failure::new(operation, error));
                if let Some(window) = window {
                    window.yaml_request.take();
                    if polling {
                        if failure.is_some() && window.yaml_poll_failed {
                            failure = None;
                        }
                        window.yaml_poll_failed = result.is_err();
                    }
                    if opened.is_some() {
                        window.yaml_external = opened;
                        window.yaml_poll_failed = false;
                    }
                    if let Ok(documents::Reply::ExternalSaved { revision, closed }) = &mut result
                        && let Some(directory) = &window.yaml_external
                        && overrides::read_draft(&directory.path().join("merge.yaml"))
                            .is_ok_and(|draft| overrides::revision(&draft) == *revision)
                    {
                        match std::fs::remove_dir_all(directory.path()) {
                            Ok(()) => {
                                *closed = true;
                                window.yaml_external = None;
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                *closed = true;
                                window.yaml_external = None;
                            }
                            Err(error) => {
                                failure = Some(documents::Failure::new(
                                    documents::Operation::CleanupExternal,
                                    &error.into(),
                                ));
                            }
                        }
                    }
                    window.send(HostPacket::Command(wire::WindowCommand::Yaml {
                        id,
                        result: result
                            .and_then(|reply| Ok(serde_json::to_string(&reply)?))
                            .map_err(|error| format!("{error:#}")),
                    }));
                }
                if let Some(failure) = failure.filter(|failure| active || failure.critical()) {
                    self.app.show_yaml_failure(failure);
                    self.sync();
                }
            }
        }
    }
}
#[cfg(target_os = "macos")]
impl ApplicationHandler<HostEvent> for DesktopHost {
    fn resumed(&mut self, _: &ActiveEventLoop) {
        self.start();
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    fn user_event(&mut self, events: &ActiveEventLoop, event: HostEvent) {
        self.event(event);
        if self.quit {
            events.exit();
        }
    }
}
impl native::Host for DesktopHost {
    fn current_window(&self) -> Option<window::Id> {
        self.current_window_id()
    }
    fn open_window(&mut self, id: window::Id, settings: window::Settings) {
        if let Some(window) = self.window.as_ref().filter(|w| w.closing.is_none()) {
            window.send(HostPacket::Command(wire::WindowCommand::Show));
            return;
        }
        if let Some(window) = self.window.take() {
            self.retired.push(window);
        }
        self.models = ViewModels::default();
        self.data = UiData::default();
        self.models.attach(&self.data);
        self.last_frame = None;
        self.app.copy_result = None;
        self.app.presentation_language = self.app.settings.language.resolve_desktop();
        self.app.refresh_network_result();
        self.app.present(&self.data, &mut self.models);
        let result = (|| -> anyhow::Result<Renderer> {
            let owner = std::env::current_exe()?;
            #[cfg(target_os = "macos")]
            let executable = if owner
                .ancestors()
                .any(|path| path.extension().is_some_and(|s| s == "app"))
            {
                owner
                    .parent()
                    .unwrap()
                    .join("../Helpers/Clash of Rust UI.app/Contents/MacOS/clash-desktop-ui")
            } else {
                owner.with_file_name("clash-desktop-ui")
            };
            #[cfg(not(target_os = "macos"))]
            let executable = owner.with_file_name(if cfg!(windows) {
                "clash-desktop-ui.exe"
            } else {
                "clash-desktop-ui"
            });
            let mut command = tokio::process::Command::new(executable);
            command
                .arg("--desktop-renderer")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .kill_on_drop(true);
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            let mut child = command.spawn()?;
            allow_renderer_focus(&child);
            let mut stdin = child.stdin.take().unwrap();
            let mut stdout = child.stdout.take().unwrap();
            let (commands, mut input) = tokio::sync::mpsc::unbounded_channel::<Outbound>();
            let (frames, mut changes) = tokio::sync::watch::channel::<Option<Arc<[u8]>>>(None);
            let open = HostPacket::Open {
                protocol: wire::PROTOCOL,
                version: clash_of_rust::VERSION.into(),
                language: self.app.presentation_language.tag().into(),
                id: id.0,
                size: wire_size(settings.size),
                visible: settings.visible,
                view: self.saved_view.clone(),
                frame: self.data.snapshot(),
                view_ready: !self.app.window_opening
                    && !self.app.size_pending
                    && (!self.app.busy
                        || !matches!(
                            self.app.page,
                            Page::Proxies | Page::Connections | Page::Rules | Page::Logs
                        )),
            };
            commands.send(Outbound {
                bytes: wire::encode(&open)?.into(),
                destroy: false,
            })?;
            self.worker.spawn(async move {
                loop {
                    let message=tokio::select!{
                        biased;
                        command=input.recv()=>match command{Some(bytes)=>bytes,None=>break},
                        changed=changes.changed()=>{if changed.is_err(){break;}let Some(bytes)=changes.borrow_and_update().clone()else{continue;};Outbound{bytes,destroy:false}}
                    };
                    let bytes=&message.bytes;
                    if stdin.write_all(&(bytes.len() as u32).to_le_bytes()).await.is_err()||stdin.write_all(bytes).await.is_err()||stdin.flush().await.is_err(){break;}
                    if message.destroy{break;}
                }
            });
            let proxy = self.proxy.clone();
            self.worker.spawn(async move {
                loop {
                    let mut length = [0; 4];
                    if stdout.read_exact(&mut length).await.is_err() {
                        break;
                    }
                    let length = u32::from_le_bytes(length) as usize;
                    if length > wire::LIMIT {
                        break;
                    }
                    let mut bytes = vec![0; length];
                    if stdout.read_exact(&mut bytes).await.is_err() {
                        break;
                    }
                    match wire::decode::<UiEvent>(&bytes) {
                        Ok(event) => {
                            if proxy.send_event(HostEvent::Ui(id, event)).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
            Ok(Renderer {
                id,
                child,
                commands,
                frames,
                metrics: wire::Metrics {
                    size: wire_size(settings.size),
                    ..Default::default()
                },
                closing: None,
                retiring: false,
                revision: 0,
                reclaim_pending: true,
                yaml_request: None,
                yaml_external: None,
                yaml_poll_failed: false,
            })
        })();
        match result {
            Ok(window) => self.window = Some(window),
            Err(error) => {
                self.app.visible = false;
                self.app.window_opening = false;
                self.app.size_pending = false;
                self.app
                    .show_notice(format!("界面启动失败，可从托盘重试：{error:#}"), true);
            }
        }
    }
    fn window_size(&self, id: window::Id) -> Size {
        self.window
            .as_ref()
            .filter(|w| w.id == id)
            .map_or(Size::ZERO, |w| size(w.metrics.size))
    }
    fn maximized(&self, id: window::Id) -> bool {
        self.window
            .as_ref()
            .is_some_and(|w| w.id == id && w.metrics.maximized)
    }
    fn minimized(&self, id: window::Id) -> Option<bool> {
        self.window
            .as_ref()
            .filter(|w| w.id == id)
            .map(|w| w.metrics.minimized)
    }
    fn fit_window_size(&self, id: window::Id, desired: Size) -> Size {
        #[cfg(target_os = "macos")]
        if let Some(available) = self
            .window
            .as_ref()
            .filter(|w| w.id == id)
            .and_then(|w| w.metrics.available)
        {
            return window_state::fit_size(desired, size(available));
        }
        let _ = id;
        desired
    }
    fn window_command(&mut self, id: window::Id, command: window::Command) {
        let Some(window) = self
            .window
            .as_mut()
            .filter(|w| w.id == id && w.closing.is_none())
        else {
            return;
        };
        let command = match command {
            window::Command::Destroy => {
                if !self.app.visible {
                    window.retiring = true;
                    self.finish_retirement();
                }
                return;
            }
            window::Command::Show => wire::WindowCommand::Show,
            window::Command::Maximize(v) => wire::WindowCommand::Maximize(v),
            window::Command::Resize(v) => wire::WindowCommand::Resize(wire_size(v)),
            window::Command::MinSize(v) => wire::WindowCommand::MinSize(v.map(wire_size)),
        };
        window.send(HostPacket::Command(command));
    }
    fn read_clipboard(&mut self) -> Option<String> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut().and_then(|c| c.get_text().ok())
    }
    fn write_clipboard(&mut self, text: String) -> Result<(), arboard::Error> {
        if self.clipboard.is_none() {
            self.clipboard = Some(arboard::Clipboard::new()?);
        }
        self.clipboard.as_mut().unwrap().set_text(text)
    }
    fn focus(&mut self, target: &str) {
        // The target can have just been mounted by a navigation intent. Put
        // that frame in the ordered queue before focusing, rather than letting
        // the command overtake the latest-value presentation slot.
        self.app.present(&self.data, &mut self.models);
        if let Some(window) = &self.window {
            window.send(HostPacket::Frame {
                data: self.data.snapshot(),
                accepted: window.revision,
                view_ready: false,
            });
            window.send(HostPacket::Command(wire::WindowCommand::FocusInput(
                target.into(),
            )));
        }
    }
    fn system_theme(&self) -> runtime::theme::Mode {
        if self.app.system_dark {
            runtime::theme::Mode::Dark
        } else {
            runtime::theme::Mode::Light
        }
    }
    #[cfg(target_os = "macos")]
    fn pick_file(&mut self) -> Result<clash_of_rust::file_dialog::Session, String> {
        let Some(window) = self.window.as_ref().filter(|w| w.closing.is_none()) else {
            return Err("没有可用的界面窗口".into());
        };
        self.next_picker = self.next_picker.wrapping_add(1);
        let id = self.next_picker;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.pickers.insert(id, sender);
        window.send(HostPacket::Command(wire::WindowCommand::PickFile(id)));
        Ok(receiver)
    }
    fn quit(&mut self) {
        self.quit = true;
    }
}
#[cfg(test)]
pub(super) fn invalidate_window_for_test(window: &slint::Window) {
    clash_desktop_ui::invalidate_window(window);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_frame_localizes_window_failures_without_changing_original_notices() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("fixture")), None);
        let data = UiData::default();
        let mut models = ViewModels::default();
        models.attach(&data);

        for (original, english) in [
            (
                "界面进程已退出（exit status: 2），可从托盘或 Dock 重新打开；后台代理仍在运行。",
                "The window process exited (exit status: 2). Reopen it from the tray or Dock; the background proxy is still running.",
            ),
            (
                "界面与后台版本不一致，请重新安装完整客户端。",
                "The window and background service versions do not match. Reinstall the full client.",
            ),
            (
                "界面启动失败，可从托盘重试：权限不足",
                "Could not launch the window. Retry from the tray: 权限不足",
            ),
            (
                "界面错误：无法显示窗口：native error: 2",
                "Window error: Could not show the window: native error: 2",
            ),
            ("界面进程已退出", "The window process exited"),
            ("没有可用的界面窗口", "No window is available"),
            (
                "初始化失败：无法读取设置文件: native error",
                "Initialization failed: Could not read the settings file: native error",
            ),
            (
                "无法保存窗口状态：保存窗口状态任务失败: native detail 原文",
                "Could not save window state: Window state save task failed: native detail 原文",
            ),
            (
                "托盘不可用：当前桌面没有可用托盘宿主；关闭窗口将退出。",
                "System tray unavailable: No system tray host is available. Closing the window will exit the app.",
            ),
        ] {
            app.dismiss_notice();
            app.show_notice(original.into(), true);
            for (language, expected) in [
                (Language::English, english),
                (Language::SimplifiedChinese, original),
            ] {
                app.presentation_language = language;
                app.present(&data, &mut models);
                assert_eq!(data.snapshot().notice.as_str(), expected);
                assert_eq!(app.notice, original);
            }
        }
    }

    #[test]
    fn desktop_frame_localizes_notification_timeout_and_preserves_copy_feedback() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("fixture")), None);
        app.dismiss_notice();
        app.presentation_language = Language::English;
        let _ = app.update(Message::NotificationFinished(Err(
            "系统通知服务响应超时: deadline has elapsed".into(),
        )));
        app.show_copy_failure();
        let data = UiData::default();
        let mut models = ViewModels::default();
        models.attach(&data);
        app.present(&data, &mut models);
        assert_eq!(
            data.snapshot().notice.as_str(),
            "System notification failed: The system notification service timed out: deadline has elapsed\n\nCould not write to the system clipboard. Please try again."
        );
        assert_eq!(
            app.notice,
            "系统通知失败：系统通知服务响应超时: deadline has elapsed"
        );
    }

    #[test]
    fn diagnostic_followups_keep_the_primary_warning_localized_in_the_frame() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("fixture")), None);
        app.dismiss_notice();
        app.presentation_language = Language::English;
        let original = "界面与后台版本不一致，请重新安装完整客户端。";
        app.show_critical(original.into());
        let data = UiData::default();
        let mut models = ViewModels::default();
        models.attach(&data);
        app.present(&data, &mut models);
        let primary = data.snapshot().notice;
        assert_eq!(
            primary.as_str(),
            "The window and background service versions do not match. Reinstall the full client."
        );

        let path = "/tmp/本机 诊断.json";
        let native = "磁盘不可写: EIO\n\n无法打开问题反馈页面：这段属于系统";
        let _ = app.update(Message::DiagnosticsExported(Ok((path.into(), None))));
        let _ = app.update(Message::DiagnosticsExported(Err(native.into())));
        app.present(&data, &mut models);
        assert_eq!(
            data.snapshot().notice.as_str(),
            format!(
                "{}\n\nDiagnostics exported: {path}\n\nFailed to collect or export diagnostics: {native}",
                primary.as_str()
            )
        );
        assert!(app.notice_critical && app.error);
        assert_eq!(app.notice, original);

        app.presentation_language = Language::SimplifiedChinese;
        app.present(&data, &mut models);
        assert_eq!(data.snapshot().notice.as_str(), app.raw_notice().as_ref());
        app.dismiss_notice();
        app.show_notice("next notice".into(), false);
        app.present(&data, &mut models);
        assert_eq!(data.snapshot().notice.as_str(), "next notice");
    }
}
