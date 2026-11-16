#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
//! Native network helpers. The GUI stays unprivileged on every platform.
#[cfg(target_os = "linux")]
#[path = "../../../crates/client-runtime/src/platform/linux/tun/trusted.rs"]
mod trusted;

#[cfg(target_os = "linux")]
#[path = "../../../crates/client-runtime/src/platform/linux/tun/dns.rs"]
mod dns;

#[cfg(target_os = "linux")]
fn launch() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use std::{os::unix::process::CommandExt, path::Path, process::Command};
    use trusted::{CORE, LAUNCHER, NETWORK_CAPABILITIES, installed_resources, open_at};

    ensure!(
        // SAFETY: geteuid takes no pointers and only reads this process's credentials.
        unsafe { libc::geteuid() } != 0,
        "内核启动器不能以 root 运行"
    );
    ensure!(
        std::env::current_exe()? == Path::new(LAUNCHER),
        "请使用 DEB 安装的内核启动器"
    );
    let resources = installed_resources()?;
    let core = open_at(&resources, c"mihomo", false)?;
    let status = std::fs::read_to_string("/proc/self/status")?;
    let permitted = status
        .lines()
        .find_map(|line| line.strip_prefix("CapPrm:"))
        .context("无法读取启动器网络权限")?;
    let permitted = u64::from_str_radix(permitted.trim(), 16)?;
    if permitted & NETWORK_CAPABILITIES == NETWORK_CAPABILITIES {
        use std::os::unix::io::AsRawFd;
        // A privileged executable clears the ambient set when exec'd.
        // SAFETY: core owns the open FD for the entire call, and the name is a
        // static C string. A null buffer with size zero requests only the size.
        let size = unsafe {
            libc::fgetxattr(
                core.as_raw_fd(),
                c"security.capability".as_ptr(),
                std::ptr::null_mut(),
                0,
            )
        };
        ensure!(
            size == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENODATA),
            "旧内核权限尚未迁移，请重新授权 TUN"
        );
        #[repr(C)]
        struct Header {
            version: u32,
            pid: i32,
        }
        #[repr(C)]
        #[derive(Clone, Copy, Default)]
        struct Data {
            effective: u32,
            permitted: u32,
            inheritable: u32,
        }
        let mut header = Header {
            version: 0x2008_0522,
            pid: 0,
        };
        let bits = NETWORK_CAPABILITIES as u32;
        let data = [
            Data {
                effective: bits,
                permitted: bits,
                inheritable: bits,
            },
            Data::default(),
        ];
        // SAFETY: repr(C) matches the V3 layout and its two initialized Data
        // entries. The header is writable for version negotiation on error;
        // both stack allocations outlive the call. Pid zero targets this thread.
        if unsafe { libc::syscall(libc::SYS_capset, &raw mut header, data.as_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error()).context("无法设置可继承的网络权限");
        }
        // SAFETY: all four varargs are c_ulong, as required by the C ABI.
        // This scalar operation reads no pointers; unused arguments are zero.
        if unsafe {
            libc::prctl(
                libc::PR_CAP_AMBIENT,
                libc::PR_CAP_AMBIENT_CLEAR_ALL as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error()).context("无法清理 ambient 权限");
        }
        for capability in [12, 13] {
            // SAFETY: only CAP_NET_ADMIN/RAW are raised. All four scalar
            // varargs are c_ulong, and unused arguments are zero.
            if unsafe {
                libc::prctl(
                    libc::PR_CAP_AMBIENT,
                    libc::PR_CAP_AMBIENT_RAISE as libc::c_ulong,
                    capability as libc::c_ulong,
                    0 as libc::c_ulong,
                    0 as libc::c_ulong,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error()).context("无法继承 TUN 网络权限");
            }
        }
    }
    // Keep user-supplied executables and loader settings out of the privileged
    // core's subprocess chain. Only execute the fixed, root-owned mihomo.
    let error = Command::new(CORE)
        .args(std::env::args_os().skip(1))
        .env_clear()
        .env(
            "PATH",
            "/opt/clash-of-rust/libexec:/usr/sbin:/usr/bin:/sbin:/bin",
        )
        .exec();
    Err(error).context("无法启动已安装的 mihomo 内核")
}

#[cfg(target_os = "linux")]
fn main() {
    let result = if std::env::current_exe().ok().as_deref()
        == Some(std::path::Path::new(trusted::DNS_HELPER))
    {
        dns::run()
    } else {
        launch()
    };
    if let Err(error) = result {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

// Do not link the desktop library, AppKit, fonts or the Slint renderer as root.
// The helper shares the core's protection-domain policy with the runtime.
#[cfg(all(target_os = "macos", not(test)))]
use clash_client_core::claude_check;
#[cfg(all(target_os = "macos", not(test)))]
#[path = "../../../crates/client-core/src/storage/hash.rs"]
mod assets;
#[cfg(all(target_os = "macos", not(test)))]
#[path = "../../../crates/client-runtime/src/platform/macos/authorization.rs"]
mod authorization;
#[cfg(all(target_os = "macos", not(test)))]
use authorization::{administrator, run as run_command, shell_quote};
#[cfg(all(target_os = "macos", not(test)))]
#[allow(dead_code)]
#[path = "../../../crates/client-runtime/src/platform/macos/cf.rs"]
mod cf;
#[cfg(all(target_os = "macos", not(test)))]
#[allow(dead_code)]
#[path = "../../../crates/client-runtime/src/platform/macos/tun.rs"]
mod macos_tun;
#[cfg(all(target_os = "macos", not(test)))]
#[path = "../../../crates/client-runtime/src/platform/proxy_state.rs"]
mod proxy_state;
#[cfg(all(target_os = "macos", not(test)))]
#[path = "../../../crates/client-runtime/src/platform/macos/runtime.rs"]
mod runtime;
#[cfg(all(target_os = "macos", not(test)))]
mod platform {
    pub use crate::proxy_state::{ProxyState, ProxyValue};
}
#[cfg(all(target_os = "macos", not(test)))]
#[allow(dead_code)]
#[path = "../../../crates/client-runtime/src/platform/macos/dns.rs"]
mod dns;
#[cfg(all(target_os = "macos", not(test)))]
#[allow(dead_code)]
#[path = "../../../crates/client-runtime/src/platform/macos/native_proxy.rs"]
mod native_proxy;
#[cfg(all(target_os = "macos", not(test)))]
#[allow(dead_code)]
#[path = "../../../crates/client-runtime/src/platform/macos/proxy_preferences.rs"]
mod proxy_preferences;
#[cfg(all(target_os = "macos", not(test)))]
#[allow(dead_code)]
#[path = "../../../crates/client-runtime/src/platform/macos/service.rs"]
mod service;
#[cfg(all(target_os = "macos", not(test)))]
use macos_tun as tun;
#[cfg(all(target_os = "macos", not(test)))]
fn main() {
    match macos_tun::launcher_main() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    }
}

#[cfg(windows)]
#[path = "../../../crates/client-core/src/storage/hash.rs"]
mod file_hash;
#[cfg(windows)]
#[path = "../../../crates/client-runtime/src/platform/windows/service/mod.rs"]
#[allow(dead_code)]
mod windows_service;
#[cfg(windows)]
fn main() {
    match windows_service::helper_main() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn main() {
    eprintln!("The TUN core launcher is only available on Linux");
    std::process::exit(1);
}
