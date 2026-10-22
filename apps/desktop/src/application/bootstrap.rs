//! Process helpers and the single-instance guard precede GUI initialization.
use super::*;
pub(crate) fn run() -> anyhow::Result<()> {
    #[cfg(windows)]
    if let Some(result) = clash_of_rust::control::background_helper_main() {
        return result;
    }
    if let Some(result) = clash_of_rust::claude_check::system::helper_main() {
        if let Err(error) = result {
            eprintln!("终端地区检测失败：{error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    #[cfg(windows)]
    if platform::autostart_helper_main() {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if platform::tun_helper_main() {
        return Ok(());
    }
    if update::helper_main() {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    // This is the first initialization, before threads or GUI/DBus libraries.
    unsafe {
        platform::initialize_desktop();
    }
    #[cfg(windows)]
    let update_handoff = update_restart();
    #[cfg(windows)]
    let _installation_guard = {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            match platform::application_guard() {
                Ok(Some(guard)) => break guard,
                Ok(None) if update_handoff && std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(250))
                }
                Ok(None) => {
                    if !update_handoff && !background_start() {
                        platform::show_existing();
                    }
                    return Ok(());
                }
                Err(error) => {
                    eprintln!("桌面初始化失败：{error:#}");
                    if !update_handoff && !background_start() {
                        platform::show_existing();
                    }
                    return Ok(());
                }
            }
        }
    };
    #[cfg(not(windows))]
    let _installation_guard = {
        let store = match startup_store() {
            Ok(store) => store,
            Err(error) => {
                eprintln!("桌面数据目录初始化失败：{error:#}");
                return Ok(());
            }
        };
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(store.root.join("desktop.lock"))
        {
            Ok(file) => file,
            Err(error) => {
                eprintln!("无法打开桌面实例锁：{error}");
                return Ok(());
            }
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while file.try_lock().is_err() {
            if !update_restart() || std::time::Instant::now() >= deadline {
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                if !background_start() {
                    let _ =
                        clash_of_rust::config::atomic_write(&store.root.join("show-window"), b"");
                }
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        file
    };
    let _gui_session = clash_of_rust::control::GuiSession::register(startup_store()?)?;
    #[cfg(windows)]
    if let Err(error) = platform::notification::prepare_process() {
        eprintln!("通知窗口身份初始化失败：{error:#}");
    }
    #[cfg(windows)]
    // There is no winit event loop in the host anymore. Keep its native tray
    // menus aware of per-monitor DPI before the tray thread creates a window.
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    // Publish the native icon before cold font loading and Geo integrity checks.
    let tray_startup = std::sync::Mutex::new(Some(tray::Startup::new(Duration::from_secs(
        if background_start() { 120 } else { 5 },
    ))));
    #[cfg(windows)]
    std::thread::spawn(platform::migrate_autostart);
    let state = startup_store()
        .map(|store| window_state::State::load(&store))
        .unwrap_or_default();
    bindings::run(tray_startup.into_inner().unwrap(), state)
}
