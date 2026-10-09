//! Linux TUN launcher and fixed-operation DNS bridge. The GUI stays unprivileged.
#[cfg(target_os = "linux")]
#[path = "platform/linux_tun/trusted.rs"]
mod trusted;

#[cfg(target_os = "linux")]
#[path = "platform/linux_tun/dns.rs"]
mod dns;

#[cfg(target_os = "linux")]
fn launch() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use std::{os::unix::process::CommandExt, path::Path, process::Command};
    use trusted::{CORE, LAUNCHER, NETWORK_CAPABILITIES, installed_resources, open_at};

    ensure!(
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
        let header = Header {
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
        if unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error()).context("无法设置可继承的网络权限");
        }
        if unsafe {
            libc::prctl(
                libc::PR_CAP_AMBIENT,
                libc::PR_CAP_AMBIENT_CLEAR_ALL,
                0,
                0,
                0,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error()).context("无法清理 ambient 权限");
        }
        for capability in [12, 13] {
            if unsafe {
                libc::prctl(
                    libc::PR_CAP_AMBIENT,
                    libc::PR_CAP_AMBIENT_RAISE,
                    capability,
                    0,
                    0,
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

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("The TUN core launcher is only available on Linux");
    std::process::exit(1);
}
