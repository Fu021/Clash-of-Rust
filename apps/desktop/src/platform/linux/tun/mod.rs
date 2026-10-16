//! A fixed-target polkit helper grants persistent network capabilities to the launcher.
//! It never reads user configuration or starts a privileged desktop process.
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    os::unix::{fs::OpenOptionsExt, io::AsRawFd},
    path::Path,
    process::Command,
};

mod trusted;
use std::os::unix::fs::MetadataExt;
use trusted::{CORE, LAUNCHER, NETWORK_CAPABILITIES, installed_resources, open_at};
const EXECUTABLE: &str = "/opt/clash-of-rust/clash-of-rust";
const HELPER_ARGUMENT: &str = "--authorize-tun";

fn status_number(status: &str, key: &str, radix: u32) -> Result<u64> {
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix(key))
        .context("无法读取进程网络权限")?;
    u64::from_str_radix(value.trim(), radix).context("进程权限信息无效")
}

pub fn core_has_tun_permissions(pid: u32) -> Result<bool> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .context("无法读取 mihomo 进程权限")?;
    Ok(status_has_tun_permissions(&status)?
        && trusted::installed_dns_helper()?.metadata()?.mode() & 0o7777 == 0o4755)
}

fn status_has_tun_permissions(status: &str) -> Result<bool> {
    ["CapEff:", "CapAmb:"]
        .into_iter()
        .try_fold(true, |ready, key| {
            Ok::<_, anyhow::Error>(
                ready
                    && status_number(status, key, 16)? & NETWORK_CAPABILITIES
                        == NETWORK_CAPABILITIES,
            )
        })
}

pub fn check_tun_environment() -> Result<()> {
    let metadata = std::fs::metadata("/dev/net/tun")
        .context("系统没有 /dev/net/tun，请检查 TUN 驱动；容器或 WSL 需要提供 TUN 设备")?;
    use std::os::unix::fs::FileTypeExt;
    ensure!(
        metadata.file_type().is_char_device(),
        "TUN 设备不是字符设备"
    );
    File::options()
        .read(true)
        .write(true)
        .open("/dev/net/tun")
        .context("无法访问 /dev/net/tun，请检查设备访问权限")?;
    let status = std::fs::read_to_string("/proc/self/status")?;
    ensure!(
        status_number(&status, "NoNewPrivs:", 10)? == 0,
        "启动环境禁止获取网络权限，请从系统应用菜单启动客户端"
    );
    ensure!(
        status_number(&status, "CapBnd:", 16)? & NETWORK_CAPABILITIES == NETWORK_CAPABILITIES,
        "启动环境限制了 TUN 网络权限，请检查容器或沙箱设置"
    );
    Ok(())
}

fn capability_attribute() -> [u8; 20] {
    // Linux vfs_cap_data revision 2: little-endian magic, then two
    // permitted/inheritable pairs. No ambient or inheritable capabilities.
    let mut attribute = [0; 20];
    attribute[..4].copy_from_slice(&0x0200_0001_u32.to_le_bytes());
    attribute[4..8].copy_from_slice(&(NETWORK_CAPABILITIES as u32).to_le_bytes());
    attribute
}

fn grant_installed_core() -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "TUN 授权助手需要管理员权限"
    );
    ensure!(
        std::env::current_exe()? == Path::new(EXECUTABLE),
        "请使用 DEB 安装的授权助手"
    );
    let resources = installed_resources()?;
    let mut core = open_at(&resources, c"mihomo", false)?;
    let manifest: serde_json::Value =
        serde_json::from_reader(open_at(&resources, c"core.json", false)?)?;
    let expected = manifest["exe_sha256"]
        .as_str()
        .context("内核校验记录缺失")?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = core.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    ensure!(
        format!("{:x}", hash.finalize()) == expected,
        "内核文件校验失败，请重新安装"
    );
    let launcher = File::options()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(LAUNCHER)?;
    trusted::trusted_metadata(&launcher.metadata()?, false)?;
    // File capabilities on mihomo would clear ambient capabilities at exec.
    // Migrate the old grant to the fixed launcher before starting a new core.
    if unsafe { libc::fremovexattr(core.as_raw_fd(), c"security.capability".as_ptr()) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ENODATA) {
            return Err(error).context("无法迁移旧内核网络权限");
        }
    }
    let attribute = capability_attribute();
    let result = unsafe {
        libc::fsetxattr(
            launcher.as_raw_fd(),
            c"security.capability".as_ptr(),
            attribute.as_ptr().cast(),
            attribute.len(),
            0,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error())
            .context("无法设置内核启动器网络权限，请检查文件系统是否支持 capabilities");
    }
    // Classic D-Bus does not authenticate an ordinary user's capabilities.
    // Enable the fixed-operation DNS helper in the same one-time grant.
    let dns = trusted::installed_dns_helper()?;
    if unsafe { libc::fchmod(dns.as_raw_fd(), 0o4755) } != 0 {
        return Err(std::io::Error::last_os_error()).context("无法启用专用 TUN DNS 助手");
    }
    Ok(())
}

pub fn core_launcher(core: &Path) -> &Path {
    if core == Path::new(CORE) {
        Path::new(LAUNCHER)
    } else {
        core
    }
}

pub fn authorize_tun(core: &Path) -> Result<()> {
    ensure!(
        core == Path::new(CORE),
        "自动 TUN 授权仅支持 DEB 安装的内核，请先安装客户端"
    );
    ensure!(
        std::env::current_exe()? == Path::new(EXECUTABLE),
        "请从已安装的客户端请求 TUN 授权"
    );
    // Also validate before showing the authorization prompt.
    let _resources = installed_resources()?;
    ensure!(
        Path::new("/usr/bin/pkexec").is_file(),
        "缺少 pkexec，请安装 pkexec 后重新开启 TUN"
    );
    let status = Command::new("/usr/bin/pkexec")
        .args(["--disable-internal-agent", EXECUTABLE, HELPER_ARGUMENT])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .context("无法启动 TUN 授权助手")?;
    match status.status.code() {
        Some(0) => Ok(()),
        Some(126) => bail!("TUN 授权已取消，当前代理模式保持不变"),
        Some(127) => bail!("无法获得 TUN 授权，请检查桌面授权服务"),
        _ => {
            // This fixed helper emits only local installation diagnostics.
            let detail = String::from_utf8_lossy(&status.stderr);
            bail!(
                "TUN 授权失败：{}",
                detail.chars().take(500).collect::<String>().trim()
            );
        }
    }
}

pub fn tun_helper_main() -> bool {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if !arguments.iter().any(|arg| arg == HELPER_ARGUMENT) {
        return false;
    }
    let result = if arguments.len() == 1 && arguments[0] == HELPER_ARGUMENT {
        grant_installed_core()
    } else {
        Err(anyhow::anyhow!("TUN 授权助手不接受其他参数"))
    };
    match result {
        Ok(()) => true,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_require_both_bits_and_use_kernel_file_format() {
        for (bits, ready) in [
            (0, false),
            (1 << 12, false),
            (1 << 13, false),
            (0x3000, true),
            (u64::MAX, true),
        ] {
            let value = status_number(
                &format!("Name:\tmihomo\nCapEff:\t{bits:016x}\n"),
                "CapEff:",
                16,
            )
            .unwrap();
            assert_eq!(value & NETWORK_CAPABILITIES == NETWORK_CAPABILITIES, ready);
        }
        assert!(status_number("Name: mihomo", "CapEff:", 16).is_err());
        assert_eq!(
            capability_attribute(),
            [1, 0, 0, 2, 0, 48, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn tun_requires_effective_and_inherited_dns_permissions() {
        for (effective, ambient, ready) in [
            (0, 0, false),
            (0x3000, 0, false), // Previous file-only grant loses DNS permissions.
            (0, 0x3000, false),
            (0x3000, 0x1000, false),
            (0x3000, 0x3000, true),
        ] {
            let status = format!("CapEff:\t{effective:x}\nCapAmb:\t{ambient:x}\n");
            assert_eq!(status_has_tun_permissions(&status).unwrap(), ready);
        }
        assert!(status_has_tun_permissions("CapEff: 3000\n").is_err());
    }

    #[test]
    fn helper_refuses_writable_and_symlink_targets_without_changing_them() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("core");
        std::fs::write(&path, b"untrusted").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        let parent = File::open(tmp.path()).unwrap();
        assert!(open_at(&parent, c"core", false).is_err());
        std::os::unix::fs::symlink(&path, tmp.path().join("link")).unwrap();
        assert!(open_at(&parent, c"link", false).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"untrusted");
    }
}
