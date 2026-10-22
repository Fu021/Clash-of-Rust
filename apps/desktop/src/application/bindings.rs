//! Native application owner. Slint runs only in a disposable child process.
use super::*;
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
    commands: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    frames: tokio::sync::watch::Sender<Option<Vec<u8>>>,
    metrics: wire::Metrics,
    closing: Option<Instant>,
    retiring: bool,
    revision: u64,
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
    fn send(&self, packet: HostPacket) {
        if matches!(
            &packet,
            HostPacket::Command(wire::WindowCommand::Show | wire::WindowCommand::Focus)
        ) {
            allow_renderer_focus(&self.child);
        }
        if let Ok(bytes) = wire::encode(&packet) {
            let _ = self.commands.send(bytes);
        }
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
    last_frame: Vec<u8>,
    clipboard: Option<arboard::Clipboard>,
    tray_listening: bool,
    next_update: Instant,
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
    let saved_view = state.position().map(|position| wire::ViewState {
        position: Some(position),
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
        last_frame: Vec::new(),
        clipboard: None,
        tray_listening: false,
        next_update: Instant::now() + update::CHECK_INTERVAL,
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
    result?;
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
        let task = self.app.update(message);
        self.spawn(task);
        self.finish_retirement();
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
            if self.app.claude.session.is_some() {
                self.message(Message::Claude(claude_view::Event::Tick));
            }
            let now = Instant::now();
            if now >= self.next_update {
                self.next_update = now + update::CHECK_INTERVAL;
                self.message(Message::CheckUpdate);
            }
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
        if self.app.window_state.capture_position(
            view.position,
            view.maximized,
            minimized,
            view.fullscreen,
        ) {
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
            return;
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
        }) && self.last_frame != bytes
        {
            self.last_frame = bytes.clone();
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
        window.send(HostPacket::Command(wire::WindowCommand::Destroy));
        window.closing = Some(Instant::now());
        let _ = window.frames.send(None);
        self.models = ViewModels::default();
        self.data = UiData::default();
        self.last_frame = Vec::new();
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
                    self.last_frame = Vec::new();
                    self.app.show_notice(format!("界面进程已退出（{status}），可从托盘或 Dock 重新打开；后台代理仍在运行。"),true);
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
                self.spawn(task);
                self.finish_retirement();
                self.sync();
            }
            UiEvent::Geometry { metrics, view } => {
                let dimensions = size(metrics.size);
                let dark = metrics.system_dark;
                self.window.as_mut().unwrap().metrics = metrics;
                self.remember_view(view);
                if self.app.system_dark != dark {
                    self.message(Message::SystemTheme(if dark {
                        runtime::theme::Mode::Dark
                    } else {
                        runtime::theme::Mode::Light
                    }));
                }
                self.message(Message::Window((id, window::Event::Resized(dimensions))));
            }
            UiEvent::Close(view) => {
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
            UiEvent::Error(error) => {
                self.app.show_notice(format!("界面错误：{error}"), true);
                self.sync();
            }
        }
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
        self.last_frame = Vec::new();
        self.app.present(&self.data, &mut self.models);
        let result = (|| -> anyhow::Result<Renderer> {
            let executable = std::env::current_exe()?.with_file_name(if cfg!(windows) {
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
            let (commands, mut input) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
            let (frames, mut changes) = tokio::sync::watch::channel::<Option<Vec<u8>>>(None);
            let open = HostPacket::Open {
                protocol: wire::PROTOCOL,
                version: clash_of_rust::VERSION.into(),
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
            commands.send(wire::encode(&open)?)?;
            self.worker.spawn(async move {
                loop {
                    let bytes=tokio::select!{
                        biased;
                        command=input.recv()=>match command{Some(bytes)=>bytes,None=>break},
                        changed=changes.changed()=>{if changed.is_err(){break;}let Some(bytes)=changes.borrow_and_update().clone()else{continue;};bytes}
                    };
                    let destroy=matches!(serde_json::from_slice::<HostPacket>(&bytes),Ok(HostPacket::Command(wire::WindowCommand::Destroy)));
                    if stdin.write_all(&(bytes.len() as u32).to_le_bytes()).await.is_err()||stdin.write_all(&bytes).await.is_err()||stdin.flush().await.is_err(){break;}
                    if destroy{break;}
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
                    match serde_json::from_slice::<UiEvent>(&bytes) {
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
            window::Command::Focus => wire::WindowCommand::Focus,
            window::Command::Minimize(v) => wire::WindowCommand::Minimize(v),
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
    fn write_clipboard(&mut self, text: String) {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        if let Some(c) = &mut self.clipboard {
            let _ = c.set_text(text);
        }
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
