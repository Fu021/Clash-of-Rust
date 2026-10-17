//! Fixed-operation SYSTEM audit service. The persistent Hyper-V policy is the
//! enforcement boundary; this process never opens it or executes user WSL.
#[cfg(windows)]
mod service {
    use std::{
        os::windows::{ffi::OsStrExt, process::CommandExt},
        path::PathBuf,
        process::{Command, Stdio},
        sync::atomic::{AtomicBool, AtomicPtr, Ordering},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::System::Services::*;

    const NAME: &str = "ClashOfRust.WslEgress";
    static STOP: AtomicBool = AtomicBool::new(false);
    static STATUS: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

    fn wide(text: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        text.as_ref().encode_wide().chain([0]).collect()
    }

    fn status(state: u32, error: u32) {
        let handle = STATUS.load(Ordering::SeqCst);
        if handle.is_null() {
            return;
        }
        let value = SERVICE_STATUS {
            dwServiceType: SERVICE_WIN32_OWN_PROCESS,
            dwCurrentState: state,
            dwControlsAccepted: if state == SERVICE_RUNNING {
                SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
            } else {
                0
            },
            dwWin32ExitCode: error,
            dwServiceSpecificExitCode: 0,
            dwCheckPoint: 0,
            dwWaitHint: 0,
        };
        unsafe { SetServiceStatus(handle, &value) };
    }

    unsafe extern "system" fn control(
        code: u32,
        _event: u32,
        _data: *mut std::ffi::c_void,
        _context: *mut std::ffi::c_void,
    ) -> u32 {
        if code == SERVICE_CONTROL_STOP || code == SERVICE_CONTROL_SHUTDOWN {
            STOP.store(true, Ordering::SeqCst);
            status(SERVICE_STOP_PENDING, 0);
        }
        0
    }

    fn executable_directory() -> std::io::Result<PathBuf> {
        let executable = std::env::current_exe()?;
        let directory = executable
            .parent()
            .ok_or_else(|| std::io::Error::other("Missing application directory"))?;
        // Installer places this executable and fixed resources under Program
        // Files. Reject redirects in every ancestor, not only the leaf script.
        use std::os::windows::fs::MetadataExt;
        for parent in directory.ancestors() {
            if std::fs::symlink_metadata(parent)?.file_attributes() & 0x400 != 0 {
                return Err(std::io::Error::other("Redirected application directory"));
            }
        }
        Ok(directory.to_path_buf())
    }

    fn audit() -> std::io::Result<()> {
        use sha2::{Digest, Sha256};
        use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
        let directory = executable_directory()?;
        let package = directory.join("resources/wsl-secure-egress");
        use std::os::windows::fs::MetadataExt;
        for parent in [directory.join("resources"), package.clone()] {
            if std::fs::symlink_metadata(parent)?.file_attributes() & 0x400 != 0 {
                return Err(std::io::Error::other("Redirected resource directory"));
            }
        }
        for (name, expected) in [
            (
                "Guardian-Audit.ps1",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../scripts/wsl-secure-egress/Guardian-Audit.ps1"
                ))
                .as_slice(),
            ),
            (
                "WslSecureEgress.psm1",
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../scripts/wsl-secure-egress/WslSecureEgress.psm1"
                ))
                .as_slice(),
            ),
        ] {
            let path = package.join(name);
            use std::os::windows::fs::MetadataExt;
            if std::fs::symlink_metadata(&path)?.file_attributes() & 0x400 != 0
                || Sha256::digest(std::fs::read(path)?) != Sha256::digest(expected)
            {
                return Err(std::io::Error::other("Audit resource identity mismatch"));
            }
        }
        let mut system = vec![0u16; 32768];
        let length =
            unsafe { GetSystemDirectoryW(system.as_mut_ptr(), system.len() as u32) } as usize;
        if length == 0 || length >= system.len() {
            return Err(std::io::Error::other("System directory unavailable"));
        }
        let powershell = PathBuf::from(String::from_utf16_lossy(&system[..length]))
            .join("WindowsPowerShell/v1.0/powershell.exe");
        let mut child = Command::new(powershell)
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
            .arg(package.join("Guardian-Audit.ps1"))
            .current_dir(&directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = child.try_wait()? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(std::io::Error::other("WSL guardian audit failed"))
                };
            }
            if STOP.load(Ordering::SeqCst) || Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(std::io::Error::other(
                    "WSL guardian audit timed out or stopped",
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    unsafe extern "system" fn main(_count: u32, _arguments: *mut *mut u16) {
        let name = wide(NAME);
        let handle = unsafe {
            RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(control), std::ptr::null_mut())
        };
        if handle.is_null() {
            return;
        }
        STATUS.store(handle, Ordering::SeqCst);
        status(SERVICE_RUNNING, 0);
        let mut consecutive_failures = 0u32;
        while !STOP.load(Ordering::SeqCst) {
            consecutive_failures = if audit().is_ok() {
                0
            } else {
                consecutive_failures.saturating_add(1)
            };
            // Keep the service alive so it can recover after a transient WSL
            // startup, but publish repeated audit failure as a service error.
            if consecutive_failures >= 3 {
                status(SERVICE_RUNNING, 1060);
            }
            for _ in 0..150 {
                if STOP.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        // No firewall cleanup on Stop/Shutdown/crash. This is an audit service,
        // not a heartbeat-based kill switch.
        status(SERVICE_STOPPED, 0);
    }

    pub fn run() -> std::io::Result<()> {
        if std::env::args_os().skip(1).collect::<Vec<_>>()
            != [std::ffi::OsString::from("--service")]
        {
            return Err(std::io::Error::other(
                "Only the fixed --service operation is supported",
            ));
        }
        let mut name = wide(NAME);
        let table = [
            SERVICE_TABLE_ENTRYW {
                lpServiceName: name.as_mut_ptr(),
                lpServiceProc: Some(main),
            },
            SERVICE_TABLE_ENTRYW {
                lpServiceName: std::ptr::null_mut(),
                lpServiceProc: None,
            },
        ];
        if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

fn main() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        service::run()
    }
    #[cfg(not(windows))]
    {
        Err(std::io::Error::other(
            "WSL guardian is available only on Windows",
        ))
    }
}
