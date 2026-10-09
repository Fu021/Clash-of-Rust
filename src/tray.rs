//! Native tray lives on its own event thread; only commands cross into the GUI.
use std::{
    sync::{OnceLock, mpsc},
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, mpsc as async_channel, watch};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, Submenu},
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
    started: Instant,
}

impl Startup {
    pub fn new(timeout: Duration) -> Self {
        let (ready_tx, ready) = mpsc::channel();
        let (cancel, cancelled) = mpsc::channel();
        let started = Instant::now();
        std::thread::spawn(move || {
            let deadline = started + timeout;
            loop {
                if !matches!(cancelled.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                    return;
                }
                match start() {
                    Ok(guard) => {
                        // A dropped receiver also drops the guard and native icon.
                        let _ = ready_tx.send(Ok(guard));
                        return;
                    }
                    Err(error) => {
                        let remaining = deadline.saturating_duration_since(Instant::now());
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
            started,
        }
    }

    pub fn poll(&self) -> Option<Result<Guard, String>> {
        match self.ready.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("托盘初始化线程已结束".into())),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
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

#[derive(Clone, Default, PartialEq, Eq)]
struct State {
    mode: String,
    system_proxy: bool,
    tun: bool,
    running: bool,
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
    if next != *kind && icon.set_icon(Some(tray_icon(next))).is_ok() {
        *kind = next;
    }
}

impl Guard {
    pub fn update(&self, snapshot: &crate::engine::Snapshot) {
        self.publish(snapshot, false);
    }

    /// Native check items toggle before their command is delivered. Reapply
    /// confirmed state even when clicking an already-selected mode.
    pub fn refresh(&self, snapshot: &crate::engine::Snapshot) {
        self.publish(snapshot, true);
    }

    fn publish(&self, snapshot: &crate::engine::Snapshot, force: bool) {
        let state = State {
            mode: snapshot.mode.clone(),
            system_proxy: snapshot.system_proxy,
            tun: snapshot.tun,
            running: snapshot.running,
        };
        let changed = self.updates.send_if_modified(|current| {
            if !force && *current == state {
                false
            } else {
                *current = state;
                true
            }
        });
        if !changed {
            return;
        }
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
    modes: [(CheckMenuItem, &'static str); 3],
    system_proxy: CheckMenuItem,
    tun: CheckMenuItem,
    off: CheckMenuItem,
}

impl Checks {
    fn update(&self, state: State) {
        for (item, mode) in &self.modes {
            item.set_checked(state.running && state.mode == *mode);
            item.set_enabled(state.running);
        }
        self.system_proxy.set_checked(state.system_proxy);
        self.system_proxy.set_enabled(state.running);
        self.tun.set_checked(state.tun);
        self.tun.set_enabled(state.running);
        self.off.set_checked(!state.system_proxy && !state.tun);
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(stop) = self.shutdown.take() {
            stop();
        }
    }
}

pub fn start() -> anyhow::Result<Guard> {
    #[cfg(any(windows, target_os = "linux"))]
    crate::platform::ensure_tray_available()?;
    let events = EVENTS.get_or_init(|| {
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
    });
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    // Only the newest state matters; a stalled tray must not accumulate polls.
    let (update_tx, mut update_rx) = watch::channel(State::default());
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
        let created = (|| -> anyhow::Result<_> {
            let menu = Menu::new();
            let mut commands = Vec::new();
            let show = MenuItem::new("显示窗口", true, None);
            menu.append(&show)?;
            commands.push((show.id().clone(), Command::Show));
            let modes = Submenu::new("运行模式", true);
            let rule = CheckMenuItem::new("规则模式", false, false, None);
            let global = CheckMenuItem::new("全局模式", false, false, None);
            let direct = CheckMenuItem::new("直连模式", false, false, None);
            for (item, command) in [
                (&rule, Command::Rule),
                (&global, Command::Global),
                (&direct, Command::Direct),
            ] {
                modes.append(item)?;
                commands.push((item.id().clone(), command));
            }
            menu.append(&modes)?;
            let proxy_modes = Submenu::new("代理模式", true);
            let system_proxy = CheckMenuItem::new("系统代理", false, false, None);
            let tun = CheckMenuItem::new("TUN模式", false, false, None);
            let off = CheckMenuItem::new("关闭", true, true, None);
            proxy_modes.append_items(&[&system_proxy, &tun, &off])?;
            menu.append(&proxy_modes)?;
            let exit = MenuItem::new("退出", true, None);
            menu.append(&exit)?;
            commands.push((system_proxy.id().clone(), Command::SystemProxy));
            commands.push((tun.id().clone(), Command::Tun));
            commands.push((off.id().clone(), Command::ProxyOff));
            commands.push((exit.id().clone(), Command::Exit));
            let checks = Checks {
                modes: [(rule, "rule"), (global, "global"), (direct, "direct")],
                system_proxy,
                tun,
                off,
            };
            let icon = TrayIconBuilder::new()
                .with_title("Clash of Rust")
                .with_tooltip("Clash of Rust")
                .with_icon(tray_icon(crate::icons::Kind::App))
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()?;
            *events.commands.lock().unwrap() = commands;
            Ok((icon, checks))
        })();
        match created {
            Ok((icon, checks)) => {
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
        .recv_timeout(Duration::from_secs(5))
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
