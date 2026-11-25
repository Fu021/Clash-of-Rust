//! Process helpers and the single-instance guard precede GUI initialization.
use super::*;

fn startup_language(root: &std::path::Path) -> Language {
    LanguagePreference::read(root)
        .unwrap_or_default()
        .resolve_desktop()
}

pub(crate) fn run() -> anyhow::Result<()> {
    #[cfg(windows)]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--launch-after-install"))
    {
        return platform::launch_after_install();
    }
    #[cfg(windows)]
    if let Some(result) = clash_of_rust::control::background_helper_main() {
        return result;
    }
    if let Some(result) = clash_of_rust::claude_check::system::helper_main() {
        if let Err(error) = result {
            eprintln!(
                "{}",
                messages::startup_terminal_region_failed(
                    LanguagePreference::System.resolve_desktop(),
                    format_args!("{error:#}"),
                )
            );
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
                    eprintln!(
                        "{}",
                        messages::startup_desktop_failed(
                            LanguagePreference::System.resolve_desktop(),
                            format_args!("{error:#}"),
                        )
                    );
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
                eprintln!(
                    "{}",
                    messages::startup_data_directory_failed(
                        LanguagePreference::System.resolve_desktop(),
                        format_args!("{error:#}"),
                    )
                );
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
                eprintln!(
                    "{}",
                    messages::startup_instance_lock_failed(startup_language(&store.root), error)
                );
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
    let store = startup_store()?;
    let language = startup_language(&store.root);
    let _gui_session = clash_of_rust::control::GuiSession::register(store)?;
    #[cfg(windows)]
    if let Err(error) = platform::notification::prepare_process() {
        eprintln!(
            "{}",
            messages::startup_notification_identity_failed(language, format_args!("{error:#}"))
        );
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
    let tray_timeout = if background_start() {
        tray::BACKGROUND_START_TIMEOUT
    } else {
        tray::FOREGROUND_START_TIMEOUT
    };
    let tray_startup = std::sync::Mutex::new(Some(tray::Startup::new(tray_timeout, language)));
    #[cfg(any(windows, target_os = "linux"))]
    std::thread::spawn(platform::migrate_autostart);
    #[cfg(target_os = "macos")]
    std::thread::spawn(mac_cli_path::install);
    let state = startup_store()
        .map(|store| window_state::State::load(&store))
        .unwrap_or_default();
    bindings::run(tray_startup.into_inner().unwrap(), state)
}
