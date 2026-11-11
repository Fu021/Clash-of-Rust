use clash_client_ui::{AppWindow, Motion};
use slint::ComponentHandle;
use std::sync::atomic::{AtomicBool, Ordering};

static QUERY_RUNNING: AtomicBool = AtomicBool::new(false);

pub(crate) fn refresh(window: &AppWindow) {
    if QUERY_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let preference = reduced_motion();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = weak.upgrade()
                && let Some(reduced) = preference
            {
                window.global::<Motion>().set_reduced_motion(reduced);
            }
        });
        QUERY_RUNNING.store(false, Ordering::Release);
    });
}

#[cfg(windows)]
fn reduced_motion() -> Option<bool> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut enabled = 1i32;
    let result = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut enabled).cast(), 0)
    };
    (result != 0).then_some(enabled == 0)
}

#[cfg(not(windows))]
fn preference(program: &str, arguments: &[&str]) -> Option<String> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut process = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match process.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut output = String::new();
                process
                    .stdout
                    .take()?
                    .take(1024)
                    .read_to_string(&mut output)
                    .ok()?;
                return Some(output.trim().to_owned());
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = process.kill();
                let _ = process.wait();
                return None;
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn reduced_motion() -> Option<bool> {
    if std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|desktop| desktop.to_ascii_lowercase().contains("kde"))
    {
        let value = preference(
            "kreadconfig6",
            &[
                "--file",
                "kdeglobals",
                "--group",
                "KDE",
                "--key",
                "AnimationDurationFactor",
            ],
        )
        .or_else(|| {
            preference(
                "kreadconfig5",
                &[
                    "--file",
                    "kdeglobals",
                    "--group",
                    "KDE",
                    "--key",
                    "AnimationDurationFactor",
                ],
            )
        })?;
        return value.parse::<f64>().ok().map(|value| value <= 0.);
    }
    match preference(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "enable-animations"],
    )?
    .as_str()
    {
        "false" => Some(true),
        "true" => Some(false),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn reduced_motion() -> Option<bool> {
    match preference(
        "/usr/bin/defaults",
        &["read", "com.apple.universalaccess", "reduceMotion"],
    )?
    .as_str()
    {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}
