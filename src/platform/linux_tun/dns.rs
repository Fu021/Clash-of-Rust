//! Fixed-operation DNS bridge for classic D-Bus, which cannot authenticate caps.
//! Installed inactive (0755); the one-time TUN grant enables setuid (4755).
use super::trusted;
use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    net::IpAddr,
    os::unix::{fs::MetadataExt, io::FromRawFd, process::CommandExt},
    path::Path,
    process::{Command, Stdio},
};

fn validate_arguments(args: &[String]) -> Result<()> {
    ensure!(args.len() >= 2, "DNS 请求参数不完整");
    let interface = &args[1];
    ensure!(
        !interface.is_empty()
            && interface.len() < libc::IFNAMSIZ
            && !interface.starts_with('-')
            && interface != "."
            && interface != ".."
            && interface
                .chars()
                .all(|c| !c.is_whitespace() && !"/:\0".contains(c)),
        "TUN 网卡名称无效"
    );
    let valid = match args[0].as_str() {
        "domain" => args.len() == 3 && args[2] == "~.",
        "default-route" => args.len() == 3 && args[2] == "true",
        "revert" => args.len() == 2,
        "dns" => {
            (3..=18).contains(&args.len()) && args[2..].iter().all(|s| s.parse::<IpAddr>().is_ok())
        }
        _ => false,
    };
    ensure!(valid, "仅允许 TUN DNS 设置与撤销");
    Ok(())
}

fn status_value<'a>(status: &'a str, key: &str) -> Result<&'a str> {
    status
        .lines()
        .find_map(|line| line.strip_prefix(key))
        .context("无法核验内核进程")
}

fn authorized_status(status: &str, uid: libc::uid_t) -> Result<()> {
    let uids = status_value(status, "Uid:")?
        .split_whitespace()
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(
        uid != 0 && uids == [uid; 4],
        "DNS 请求必须来自同一普通用户的内核"
    );
    for key in ["CapEff:", "CapAmb:"] {
        let bits = u64::from_str_radix(status_value(status, key)?.trim(), 16)?;
        ensure!(
            bits & trusted::NETWORK_CAPABILITIES == trusted::NETWORK_CAPABILITIES,
            "内核尚未完成 TUN 授权"
        );
    }
    Ok(())
}

fn pin_process(pid: libc::pid_t) -> Result<File> {
    ensure!(pid > 1, "内核父进程无效");
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as i32;
    ensure!(fd >= 0, "内核进程已退出");
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn same_executable(actual: &File, expected: &File) -> Result<()> {
    let actual = actual.metadata()?;
    let expected = expected.metadata()?;
    ensure!(
        actual.dev() == expected.dev() && actual.ino() == expected.ino(),
        "DNS 请求不是已安装的 mihomo 发出"
    );
    Ok(())
}

fn system_command(name: &str) -> Result<Command> {
    for directory in ["/", "/usr", "/usr/bin"] {
        trusted::trusted_metadata(&std::fs::metadata(directory)?, true)?;
    }
    let path = Path::new("/usr/bin").join(name);
    trusted::trusted_metadata(&File::open(&path)?.metadata()?, false)?;
    let mut command = Command::new(path);
    command
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .current_dir("/")
        .stdin(Stdio::null());
    Ok(command)
}

pub(super) fn run() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    validate_arguments(&args)?;
    ensure!(
        std::env::current_exe()? == Path::new(trusted::DNS_HELPER),
        "DNS 助手安装路径无效"
    );
    let uid = unsafe { libc::getuid() };
    ensure!(
        uid != 0 && unsafe { libc::geteuid() } == 0,
        "DNS 助手需要一次 TUN 授权"
    );
    let resources = trusted::installed_resources()?;
    let expected = trusted::open_at(&resources, c"mihomo", false)?;
    let parent = unsafe { libc::getppid() };
    // Pin the process and its proc directory, then recheck the parent after all
    // reads. PID recycling or reparenting cannot substitute an unrelated caller.
    let _parent = pin_process(parent)?;
    let proc = File::open(format!("/proc/{parent}"))?;
    let proc_path = format!("/proc/self/fd/{}", std::os::fd::AsRawFd::as_raw_fd(&proc));
    same_executable(&File::open(format!("{proc_path}/exe"))?, &expected)?;
    authorized_status(
        &std::fs::read_to_string(format!("{proc_path}/status"))?,
        uid,
    )?;
    ensure!(unsafe { libc::getppid() } == parent, "内核父进程已改变");
    // Query this process's network namespace; host sysfs may show another one.
    let link = system_command("ip")?
        .args(["-json", "-details", "link", "show", "dev", &args[1]])
        .output()?;
    ensure!(link.status.success(), "TUN 网卡不存在");
    let link: serde_json::Value = serde_json::from_slice(&link.stdout)?;
    ensure!(
        link[0]["linkinfo"]["info_kind"] == "tun",
        "DNS 助手只能操作 TUN 网卡"
    );
    ensure!(unsafe { libc::getppid() } == parent, "内核父进程已退出");
    // Classic system D-Bus authenticates root credentials, not inherited caps.
    // Only the validated resolvectl operation crosses this privilege boundary.
    if unsafe { libc::setresuid(0, 0, 0) } != 0 {
        return Err(std::io::Error::last_os_error()).context("无法准备 DNS 授权");
    }
    let error = system_command("resolvectl")?.args(args).exec();
    Err(error).context("无法执行 TUN DNS 操作")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn accepts_only_core_dns_operations() {
        for args in [
            vec!["domain", "ClashRustTest", "~."],
            vec!["default-route", "ClashRustTest", "true"],
            vec!["dns", "ClashRustTest", "198.18.0.2", "fdfe:dcba:9876::2"],
            vec!["revert", "ClashRustTest"],
            vec!["dns", "代理Tun", "198.18.0.2"],
        ] {
            validate_arguments(&arguments(&args)).unwrap();
        }
    }

    #[test]
    fn rejects_options_commands_paths_and_non_ip_dns_values() {
        for args in [
            vec![],
            vec!["dns"],
            vec!["flush-caches", "tun0"],
            vec!["dns", "--help", "198.18.0.2"],
            vec!["dns", "../../eth0", "198.18.0.2"],
            vec!["dns", "tun 0", "198.18.0.2"],
            vec!["dns", ".", "198.18.0.2"],
            vec!["dns", "tun0", "example.com"],
            vec!["dns", "tun0", "--help"],
            vec!["domain", "tun0", "example.com"],
            vec!["default-route", "tun0", "false"],
            vec!["revert", "tun0", "--help"],
        ] {
            assert!(validate_arguments(&arguments(&args)).is_err(), "{args:?}");
        }
    }

    #[test]
    fn rejects_foreign_root_or_ungranted_callers() {
        let granted =
            "Uid:\t1000 1000 1000 1000\nCapEff:\t0000000000003000\nCapAmb:\t0000000000003000\n";
        authorized_status(granted, 1000).unwrap();
        assert!(authorized_status(granted, 1001).is_err());
        assert!(authorized_status(&granted.replace("1000", "0"), 0).is_err());
        for key in ["CapEff:", "CapAmb:"] {
            let missing = granted.replace(
                &format!("{key}\t0000000000003000"),
                &format!("{key}\t0000000000001000"),
            );
            assert!(authorized_status(&missing, 1000).is_err());
        }
    }

    #[test]
    fn process_identity_uses_kernel_executable_not_names() {
        let actual = File::open("/proc/self/exe").unwrap();
        let expected = File::open(std::env::current_exe().unwrap()).unwrap();
        same_executable(&actual, &expected).unwrap();
        assert!(same_executable(&actual, &File::open("/usr/bin/true").unwrap()).is_err());
        pin_process(unsafe { libc::getpid() }).unwrap();
        assert!(pin_process(1).is_err());
    }
}
