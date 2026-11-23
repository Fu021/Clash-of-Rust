//! Native tray uses AppKit's main thread on macOS, its own thread elsewhere.
use crate::{config::Language, messages};
use std::{
    sync::{OnceLock, mpsc},
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, mpsc as async_channel, watch};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, Submenu},
};
mod startup_diagnostics;

const NATIVE_START_TIMEOUT: Duration =
    Duration::from_secs(if cfg!(target_os = "linux") { 15 } else { 5 });
pub const FOREGROUND_START_TIMEOUT: Duration = if cfg!(target_os = "linux") {
    Duration::from_secs(90)
} else {
    NATIVE_START_TIMEOUT
};

#[derive(Debug, Clone)]
pub enum Command {
    Show,
    Rule,
    Global,
    Direct,
    SystemProxy,
    Tun,
    ProxyOff,
    RefreshNodes,
    Select(String, String),
    Exit,
}
struct Events {
    sender: async_channel::UnboundedSender<Command>,
    receiver: Mutex<async_channel::UnboundedReceiver<Command>>,
    commands: std::sync::Mutex<Vec<(tray_icon::menu::MenuId, Command)>>,
}
static EVENTS: OnceLock<Events> = OnceLock::new();

pub async fn next() -> Option<Command> {
    EVENTS.get()?.receiver.lock().await.recv().await
}

/// Starts before fonts and Geo data are loaded. A login session may not have
/// its StatusNotifier host yet; retry without blocking the GUI thread.
pub struct Startup {
    ready: mpsc::Receiver<Result<Guard, String>>,
    cancel: mpsc::Sender<()>,
    diagnostics: std::sync::Arc<std::sync::Mutex<startup_diagnostics::Diagnostics>>,
    language: Language,
}

impl Startup {
    #[cfg(not(target_os = "macos"))]
    pub fn new(timeout: Duration, language: Language) -> Self {
        let (ready_tx, ready) = mpsc::channel();
        let (cancel, cancelled) = mpsc::channel();
        let started = Instant::now();
        let diagnostics = std::sync::Arc::new(std::sync::Mutex::new(
            startup_diagnostics::Diagnostics::new(timeout),
        ));
        let report = diagnostics.clone();
        std::thread::spawn(move || {
            let deadline = started + timeout;
            loop {
                if !matches!(cancelled.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                    return;
                }
                report.lock().unwrap().attempt();
                match start(language) {
                    Ok(guard) => {
                        report.lock().unwrap().success();
                        // A dropped receiver also drops the guard and native icon.
                        let _ = ready_tx.send(Ok(guard));
                        return;
                    }
                    Err(error) => {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        report
                            .lock()
                            .unwrap()
                            .failure(&error.to_string(), remaining.is_zero());
                        if remaining.is_zero() {
                            let _ = ready_tx.send(Err(error.to_string()));
                            return;
                        }
                        if !matches!(
                            cancelled.recv_timeout(remaining.min(Duration::from_millis(250))),
                            Err(mpsc::RecvTimeoutError::Timeout)
                        ) {
                            return;
                        }
                    }
                }
            }
        });
        Self {
            ready,
            cancel,
            diagnostics,
            language,
        }
    }

    #[cfg(target_os = "macos")]
    pub fn new(timeout: Duration, language: Language) -> Self {
        let (sender, ready) = mpsc::channel();
        let (cancel, cancelled) = mpsc::channel();
        let diagnostics = std::sync::Arc::new(std::sync::Mutex::new(
            startup_diagnostics::Diagnostics::new(timeout),
        ));
        MAC_PENDING.lock().unwrap().push(MacPending {
            ready: sender,
            cancelled,
            timeout,
            deadline: None,
            diagnostics: diagnostics.clone(),
            language,
        });
        Self {
            ready,
            cancel,
            diagnostics,
            language,
        }
    }

    pub fn diagnostics(&self) -> serde_json::Value {
        self.diagnostics.lock().unwrap().snapshot()
    }

    pub fn poll(&self) -> Option<Result<Guard, String>> {
        match self.ready.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(messages::tray_startup_ended(self.language).into()))
            }
        }
    }
}

impl Drop for Startup {
    fn drop(&mut self) {
        let _ = self.cancel.send(());
    }
}

pub struct Guard {
    shutdown: Option<Box<dyn FnOnce() + Send>>,
    updates: watch::Sender<State>,
    thread_id: u32,
}

pub use clash_client_runtime::tray_model::Nodes;

#[derive(Clone, PartialEq, Eq)]
struct State {
    language: Language,
    mode: String,
    system_proxy: bool,
    tun: bool,
    running: bool,
    nodes: Option<std::sync::Arc<Nodes>>,
    speed: Option<(u64, u64)>,
}

impl State {
    fn new(language: Language) -> Self {
        Self {
            language,
            mode: String::new(),
            system_proxy: false,
            tun: false,
            running: false,
            nodes: None,
            speed: None,
        }
    }
}

fn tray_icon(kind: crate::icons::Kind) -> Icon {
    Icon::from_rgba(
        crate::icons::pixels(kind).to_vec(),
        crate::icons::SIZE,
        crate::icons::SIZE,
    )
    .expect("committed tray icons must contain valid RGBA pixels")
}

fn update_icon(icon: &tray_icon::TrayIcon, kind: &mut crate::icons::Kind, state: &State) {
    let next = crate::icons::tray_kind(state.system_proxy, state.tun);
    if next != *kind {
        #[cfg(target_os = "macos")]
        let result = icon.set_icon_templated(Some(tray_icon(next)));
        #[cfg(not(target_os = "macos"))]
        let result = icon.set_icon(Some(tray_icon(next)));
        if result.is_ok() {
            *kind = next;
        }
    }
    let title = state
        .speed
        .map(|(up, down)| format!("↑ {}  ↓ {}", speed(up), speed(down)));
    // Windows has no native tray title; its tooltip and menu show the rates.
    #[cfg(target_os = "macos")]
    icon.set_title(title.as_deref());
    #[cfg(target_os = "linux")]
    icon.set_title(Some(title.as_deref().unwrap_or("Clash of Rust")));
    let tooltip = title.as_ref().map_or_else(
        || "Clash of Rust".into(),
        |s| format!("Clash of Rust · {s}"),
    );
    let _ = icon.set_tooltip(Some(tooltip));
}

fn speed(value: u64) -> String {
    if value >= 1024 * 1024 {
        format!("{:.1}M/s", value as f64 / 1048576.0)
    } else {
        format!("{}K/s", value / 1024)
    }
}

impl Guard {
    pub fn update(&self, snapshot: &crate::engine::Snapshot, language: Language) {
        self.publish(snapshot, language, false);
    }

    /// Native check items toggle before their command is delivered. Reapply
    /// confirmed state even when clicking an already-selected mode.
    pub fn refresh(&self, snapshot: &crate::engine::Snapshot, language: Language) {
        self.publish(snapshot, language, true);
    }

    pub fn nodes(&self, nodes: Option<Nodes>) {
        self.updates
            .send_modify(|state| state.nodes = nodes.map(std::sync::Arc::new));
        self.wake();
    }
    pub fn speed(&self, snapshot: &crate::engine::Snapshot, enabled: bool) {
        let speed = enabled.then_some((snapshot.upload_rate, snapshot.download_rate));
        if self.updates.send_if_modified(|state| {
            if state.speed == speed {
                false
            } else {
                state.speed = speed;
                true
            }
        }) {
            self.wake();
        }
    }

    fn publish(&self, snapshot: &crate::engine::Snapshot, language: Language, force: bool) {
        let changed = self.updates.send_if_modified(|current| {
            if !force
                && current.language == language
                && current.mode == snapshot.mode
                && current.system_proxy == snapshot.system_proxy
                && current.tun == snapshot.tun
                && current.running == snapshot.running
            {
                false
            } else {
                current.language = language;
                current.mode.clone_from(&snapshot.mode);
                current.system_proxy = snapshot.system_proxy;
                current.tun = snapshot.tun;
                current.running = snapshot.running;
                if !snapshot.running {
                    current.nodes = None;
                }
                true
            }
        });
        if !changed {
            return;
        }
        self.wake();
    }

    fn wake(&self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                self.thread_id,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1,
                0,
                0,
            );
        }
        #[cfg(not(windows))]
        let _ = self.thread_id;
    }
}

struct Checks {
    language: Language,
    show: MenuItem,
    routing_mode: Submenu,
    modes: [(CheckMenuItem, &'static str); 3],
    proxy_mode: Submenu,
    system_proxy: CheckMenuItem,
    tun: CheckMenuItem,
    off: CheckMenuItem,
    nodes: Submenu,
    node_items: Vec<CheckMenuItem>,
    last_nodes: Option<std::sync::Arc<Nodes>>,
    nodes_ready: bool,
    traffic: MenuItem,
    exit: MenuItem,
}

impl Checks {
    fn update(&mut self, state: State) {
        let language = state.language;
        let language_changed = self.language != language;
        if language_changed {
            self.language = language;
            self.show.set_text(messages::tray_show(language));
            self.routing_mode.set_text(messages::tray_mode(language));
            for ((item, _), text) in self.modes.iter().zip([
                messages::tray_rule(language),
                messages::tray_global(language),
                messages::tray_direct(language),
            ]) {
                item.set_text(text);
            }
            self.proxy_mode
                .set_text(messages::tray_proxy_mode(language));
            self.system_proxy
                .set_text(messages::tray_system_proxy(language));
            self.tun.set_text(messages::tray_tun(language));
            self.off.set_text(messages::tray_off(language));
            self.exit.set_text(messages::tray_close(language));
        }
        for (item, mode) in &self.modes {
            item.set_checked(state.running && state.mode == *mode);
            item.set_enabled(state.running);
        }
        self.system_proxy.set_checked(state.system_proxy);
        self.system_proxy.set_enabled(state.running);
        self.tun.set_checked(state.tun);
        self.tun.set_enabled(state.running);
        self.off.set_checked(!state.system_proxy && !state.tun);
        self.traffic.set_text(state.speed.map_or_else(
            || messages::tray_speed_off(language).into(),
            |(up, down)| format!("↑ {}  ↓ {}", speed(up), speed(down)),
        ));
        if language_changed || !self.nodes_ready || self.last_nodes != state.nodes {
            self.nodes_ready = true;
            if let Some(events) = EVENTS.get() {
                let mut commands = events.commands.lock().unwrap();
                commands.retain(|(_, command)| !matches!(command, Command::Select(..)));
                while self.nodes.remove_at(0).is_some() {}
                self.node_items.clear();
                if let Some(nodes) = &state.nodes {
                    self.nodes.set_text(if nodes.locked {
                        messages::tray_locked_group(language, &nodes.group)
                    } else {
                        messages::tray_node_group(language, &nodes.group)
                    });
                    for name in &nodes.names {
                        let item = CheckMenuItem::new(
                            name,
                            state.running && !nodes.locked,
                            name == &nodes.selected,
                            None,
                        );
                        if self.nodes.append(&item).is_ok() {
                            commands.push((
                                item.id().clone(),
                                Command::Select(nodes.group.clone(), name.clone()),
                            ));
                            self.node_items.push(item);
                        }
                    }
                    if nodes.total > nodes.names.len() {
                        let _ = self.nodes.append(&MenuItem::new(
                            messages::tray_more_nodes(language),
                            false,
                            None,
                        ));
                    }
                } else {
                    self.nodes.set_text(messages::tray_nodes(language));
                    let _ = self.nodes.append(&MenuItem::new(
                        messages::tray_no_group(language),
                        false,
                        None,
                    ));
                }
                let refresh =
                    MenuItem::new(messages::tray_refresh_nodes(language), state.running, None);
                let _ = self.nodes.append(&refresh);
                commands.retain(|(_, command)| !matches!(command, Command::RefreshNodes));
                commands.push((refresh.id().clone(), Command::RefreshNodes));
            }
            self.last_nodes = state.nodes;
        }
        // Native menu items toggle before the core accepts the selection.
        // Restore confirmed marks even when the group contents did not change.
        if let Some(nodes) = &self.last_nodes {
            for (item, name) in self.node_items.iter().zip(&nodes.names) {
                let selected = name == &nodes.selected;
                if item.is_checked() != selected {
                    item.set_checked(selected);
                }
                if item.is_enabled() != (state.running && !nodes.locked) {
                    item.set_enabled(state.running && !nodes.locked);
                }
            }
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(stop) = self.shutdown.take() {
            stop();
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn start(language: Language) -> anyhow::Result<Guard> {
    #[cfg(any(windows, target_os = "linux"))]
    crate::platform::ensure_tray_available()?;
    let events = events();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    // Only the newest state matters; a stalled tray must not accumulate polls.
    let (update_tx, mut update_rx) = watch::channel(State::new(language));
    std::thread::spawn(move || {
        #[cfg(windows)]
        let thread_id = unsafe {
            // Ensure the native message queue exists before publishing the thread id.
            let mut message = std::mem::zeroed();
            windows_sys::Win32::UI::WindowsAndMessaging::PeekMessageW(
                &mut message,
                std::ptr::null_mut(),
                0,
                0,
                0,
            );
            windows_sys::Win32::System::Threading::GetCurrentThreadId()
        };
        let created = create_native(events, language);
        match created {
            Ok((icon, mut checks)) => {
                let mut icon_kind = crate::icons::Kind::App;
                #[cfg(windows)]
                let ready = ready_tx.send(Ok(thread_id));
                #[cfg(not(windows))]
                let ready = ready_tx.send(Ok(0u32));
                if ready.is_err() {
                    return;
                }
                #[cfg(windows)]
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::*;
                    let mut message = std::mem::zeroed();
                    while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                        if update_rx.has_changed().unwrap_or(false) {
                            let state = update_rx.borrow_and_update().clone();
                            update_icon(&icon, &mut icon_kind, &state);
                            checks.update(state);
                        }
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                #[cfg(not(windows))]
                loop {
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }
                    match update_rx.has_changed() {
                        Ok(true) => {
                            let state = update_rx.borrow_and_update().clone();
                            update_icon(&icon, &mut icon_kind, &state);
                            checks.update(state);
                        }
                        Ok(false) => {}
                        Err(_) => break,
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                drop(icon);
            }
            Err(error) => {
                let _ = ready_tx.send(Err(error.to_string()));
            }
        }
        drop(stop_rx);
    });
    let id = ready_rx
        .recv_timeout(NATIVE_START_TIMEOUT)
        .map_err(|e| anyhow::anyhow!(e))?
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(Guard {
        updates: update_tx,
        thread_id: id,
        shutdown: Some(Box::new(move || {
            #[cfg(windows)]
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                    id,
                    windows_sys::Win32::UI::WindowsAndMessaging::WM_QUIT,
                    0,
                    0,
                );
            }
            #[cfg(not(windows))]
            let _ = id;
            let _ = stop_tx.send(());
        })),
    })
}

fn events() -> &'static Events {
    EVENTS.get_or_init(|| {
        let (sender, receiver) = async_channel::unbounded_channel();
        // Native handlers can only be set once. Keep the current menu IDs
        // separately so a retried creation never retains a failed menu.
        MenuEvent::set_event_handler(Some(|event: MenuEvent| {
            if let Some(events) = EVENTS.get()
                && let Ok(commands) = events.commands.lock()
                && let Some((_, command)) = commands.iter().find(|(id, _)| *id == event.id)
            {
                let _ = events.sender.send(command.clone());
            }
        }));
        TrayIconEvent::set_event_handler(Some(|event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) && let Some(events) = EVENTS.get()
            {
                let _ = events.sender.send(Command::Show);
            }
        }));
        Events {
            sender,
            receiver: Mutex::new(receiver),
            commands: std::sync::Mutex::new(Vec::new()),
        }
    })
}

fn create_native(
    events: &Events,
    language: Language,
) -> anyhow::Result<(tray_icon::TrayIcon, Checks)> {
    let menu = Menu::new();
    let mut commands = Vec::new();
    let show = MenuItem::new(messages::tray_show(language), true, None);
    menu.append(&show)?;
    commands.push((show.id().clone(), Command::Show));
    let modes = Submenu::new(messages::tray_mode(language), true);
    let rule = CheckMenuItem::new(messages::tray_rule(language), false, false, None);
    let global = CheckMenuItem::new(messages::tray_global(language), false, false, None);
    let direct = CheckMenuItem::new(messages::tray_direct(language), false, false, None);
    for (item, command) in [
        (&rule, Command::Rule),
        (&global, Command::Global),
        (&direct, Command::Direct),
    ] {
        modes.append(item)?;
        commands.push((item.id().clone(), command));
    }
    menu.append(&modes)?;
    let proxy_modes = Submenu::new(messages::tray_proxy_mode(language), true);
    let system_proxy =
        CheckMenuItem::new(messages::tray_system_proxy(language), false, false, None);
    let tun = CheckMenuItem::new(messages::tray_tun(language), false, false, None);
    let off = CheckMenuItem::new(messages::tray_off(language), true, true, None);
    proxy_modes.append_items(&[&system_proxy, &tun, &off])?;
    menu.append(&proxy_modes)?;
    let nodes = Submenu::new(messages::tray_nodes(language), true);
    let refresh = MenuItem::new(messages::tray_load_nodes(language), true, None);
    nodes.append(&refresh)?;
    menu.append(&nodes)?;
    commands.push((refresh.id().clone(), Command::RefreshNodes));
    let traffic = MenuItem::new(messages::tray_speed_off(language), false, None);
    menu.append(&traffic)?;
    let exit = MenuItem::new(messages::tray_close(language), true, None);
    menu.append(&exit)?;
    commands.push((system_proxy.id().clone(), Command::SystemProxy));
    commands.push((tun.id().clone(), Command::Tun));
    commands.push((off.id().clone(), Command::ProxyOff));
    commands.push((exit.id().clone(), Command::Exit));
    let checks = Checks {
        language,
        show,
        routing_mode: modes,
        modes: [(rule, "rule"), (global, "global"), (direct, "direct")],
        proxy_mode: proxy_modes,
        system_proxy,
        tun,
        off,
        nodes,
        node_items: Vec::new(),
        last_nodes: None,
        nodes_ready: false,
        traffic,
        exit,
    };
    let builder = TrayIconBuilder::new()
        .with_tooltip("Clash of Rust")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false);
    // A text title beside the icon wastes scarce menu-bar space on notched Macs.
    // AppKit template artwork follows the menu bar's light/dark appearance.
    #[cfg(target_os = "macos")]
    let builder = builder.with_icon_templated(tray_icon(crate::icons::Kind::App));
    #[cfg(not(target_os = "macos"))]
    let builder = builder
        .with_title("Clash of Rust")
        .with_icon(tray_icon(crate::icons::Kind::App));
    let icon = builder.build()?;
    #[cfg(target_os = "macos")]
    configure_mac_item(&icon);
    *events.commands.lock().unwrap() = commands;
    Ok((icon, checks))
}

#[cfg(target_os = "macos")]
struct MacPending {
    ready: mpsc::Sender<Result<Guard, String>>,
    cancelled: mpsc::Receiver<()>,
    timeout: Duration,
    deadline: Option<Instant>,
    diagnostics: std::sync::Arc<std::sync::Mutex<startup_diagnostics::Diagnostics>>,
    language: Language,
}
#[cfg(target_os = "macos")]
static MAC_PENDING: std::sync::Mutex<Vec<MacPending>> = std::sync::Mutex::new(Vec::new());
#[cfg(target_os = "macos")]
struct MacNative {
    icon: tray_icon::TrayIcon,
    checks: Checks,
    kind: crate::icons::Kind,
    updates: watch::Receiver<State>,
    stop: mpsc::Receiver<()>,
    retry_after: Instant,
}
#[cfg(target_os = "macos")]
thread_local! {
    static MAC_NATIVE: std::cell::RefCell<Option<MacNative>> = const { std::cell::RefCell::new(None) };
}
/// Called by the native desktop adapter after AppKit's event loop has started.
/// Native creation, mutation and final destruction all stay on this thread.
#[cfg(target_os = "macos")]
pub fn pump() {
    assert!(
        unsafe { libc::pthread_main_np() } != 0,
        "AppKit tray must stay on the main thread"
    );
    MAC_NATIVE.with_borrow_mut(|native| {
        if let Some(value) = native {
            if !matches!(value.stop.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                *native = None;
            } else {
                if value.updates.has_changed().unwrap_or(false) {
                    let state = value.updates.borrow_and_update().clone();
                    update_icon(&value.icon, &mut value.kind, &state);
                    value.checks.update(state);
                }
                let now = Instant::now();
                if now >= value.retry_after && !mac_item_healthy(&value.icon) {
                    // A successful builder alone does not prove that AppKit
                    // installed its status window. Recover a lost native item
                    // while preserving its owner, menu IDs and confirmed state.
                    value.retry_after = now + Duration::from_secs(5);
                    let repair = value
                        .icon
                        .set_visible(false)
                        .and_then(|()| value.icon.set_visible(true));
                    if let Err(error) = repair {
                        eprintln!("macOS 菜单栏图标恢复失败：{error}");
                    } else {
                        configure_mac_item(&value.icon);
                        eprintln!("macOS 菜单栏图标已重新注册");
                    }
                }
            }
        }
    });
    MAC_PENDING.lock().unwrap().retain_mut(|pending| {
        if !matches!(pending.cancelled.try_recv(), Err(mpsc::TryRecvError::Empty)) {
            return false;
        }
        // Mac startup waits for the GUI loop, unlike the background-thread
        // implementation. Cold fonts/Geo loading must not consume this budget.
        let deadline = *pending
            .deadline
            .get_or_insert_with(|| Instant::now() + pending.timeout);
        pending.diagnostics.lock().unwrap().attempt();
        match start(pending.language) {
            Ok(guard) => {
                pending.diagnostics.lock().unwrap().success();
                let _ = pending.ready.send(Ok(guard));
                false
            }
            Err(error) if Instant::now() >= deadline => {
                pending
                    .diagnostics
                    .lock()
                    .unwrap()
                    .failure(&error.to_string(), true);
                let _ = pending.ready.send(Err(error.to_string()));
                false
            }
            Err(error) => {
                pending
                    .diagnostics
                    .lock()
                    .unwrap()
                    .failure(&error.to_string(), false);
                true
            }
        }
    });
}
#[cfg(target_os = "macos")]
fn start(language: Language) -> anyhow::Result<Guard> {
    anyhow::ensure!(
        unsafe { libc::pthread_main_np() } != 0,
        messages::tray_main_thread(language)
    );
    let mtm = objc2_foundation::MainThreadMarker::new().unwrap();
    anyhow::ensure!(
        objc2_app_kit::NSApplication::sharedApplication(mtm).isRunning(),
        messages::tray_waiting_for_event_loop(language)
    );
    anyhow::ensure!(
        MAC_NATIVE.with_borrow(Option::is_none),
        messages::tray_already_started(language)
    );
    let (icon, checks) = create_native(events(), language)?;
    let (updates, receiver) = watch::channel(State::new(language));
    let (stop, stopped) = mpsc::channel();
    MAC_NATIVE.with_borrow_mut(|native| {
        *native = Some(MacNative {
            icon,
            checks,
            kind: crate::icons::Kind::App,
            updates: receiver,
            stop: stopped,
            retry_after: Instant::now(),
        })
    });
    Ok(Guard {
        updates,
        thread_id: 0,
        shutdown: Some(Box::new(move || {
            let _ = stop.send(());
        })),
    })
}

#[cfg(target_os = "macos")]
fn configure_mac_item(icon: &tray_icon::TrayIcon) {
    use objc2_app_kit::NSAutoresizingMaskOptions;
    let mtm = objc2_foundation::MainThreadMarker::new().unwrap();
    if let Some(item) = icon.ns_status_item() {
        item.setVisible(true);
        if let Some(button) = item.button(mtm) {
            for view in button.subviews() {
                // tray-icon snapshots the button's initial frame, which
                // can have zero height before AppKit finishes its layout. Keep
                // its mouse-event view attached to the final button bounds.
                if view.class().name() == c"TaoTrayTarget" {
                    view.setAutoresizingMask(
                        NSAutoresizingMaskOptions::ViewWidthSizable
                            | NSAutoresizingMaskOptions::ViewHeightSizable,
                    );
                    view.setFrame(button.bounds());
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn mac_item_healthy(icon: &tray_icon::TrayIcon) -> bool {
    let mtm = objc2_foundation::MainThreadMarker::new().unwrap();
    icon.ns_status_item().is_some_and(|item| {
        item.isVisible()
            && item.button(mtm).is_some_and(|button| {
                button.image().is_some()
                    && button.window().is_some_and(|window| {
                        // Fullscreen and automatic menu-bar hiding are normal;
                        // do not continually recreate an intentionally hidden bar.
                        window.isVisible() || !objc2_app_kit::NSMenu::menuBarVisible(mtm)
                    })
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_language_changes_without_a_network_change() {
        let (updates, mut receiver) = watch::channel(State::new(Language::English));
        let guard = Guard {
            shutdown: None,
            updates,
            thread_id: 0,
        };
        let snapshot = crate::engine::Snapshot::default();

        guard.update(&snapshot, Language::English);
        assert!(!receiver.has_changed().unwrap());
        for language in [Language::SimplifiedChinese, Language::English] {
            guard.update(&snapshot, language);
            assert!(receiver.has_changed().unwrap());
            assert_eq!(receiver.borrow_and_update().language, language);
        }
    }
}
