//! A copied native helper survives replacement of the GUI executable. No scripts.
use super::Downloaded;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct InstallationStatus {
    pub version: String,
    pub state: String,
    pub message: String,
    pub error: bool,
    pub updated_at: u64,
    directory: PathBuf,
}

fn load_installation_status(data_dir: &Path) -> Result<Option<InstallationStatus>> {
    let path = data_dir.join("update-install.json");
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("无法读取安装记录"),
    };
    let mut bytes = Vec::new();
    file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 16 * 1024, "安装记录过大");
    Ok(Some(
        serde_json::from_slice(&bytes).context("安装记录损坏")?,
    ))
}

pub fn installation_status(data_dir: &Path) -> Result<Option<InstallationStatus>> {
    let Some(mut status) = load_installation_status(data_dir)? else {
        return Ok(None);
    };
    if matches!(status.state.as_str(), "preparing" | "installing")
        && let Some((message, error)) = read_outcome(&status.directory)
    {
        status.state = if error { "failed" } else { "succeeded" }.into();
        status.message = message;
        status.error = error;
    }
    Ok(Some(status))
}

fn record_installation(
    plan: &Plan,
    directory: &Path,
    state: &str,
    message: &str,
    error: bool,
) -> Result<()> {
    let Some(data_dir) = &plan.data_dir else {
        return Ok(());
    };
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(data_dir.join("update-install.lock"))?;
    lock.lock().context("无法锁定安装记录")?;
    if let Some(previous) = load_installation_status(data_dir)? {
        if state != "preparing" && previous.directory != directory {
            return Ok(());
        }
        if previous.directory == directory
            && matches!(previous.state.as_str(), "succeeded" | "failed")
            && matches!(state, "preparing" | "installing")
        {
            return Ok(());
        }
    }
    let status = InstallationStatus {
        version: plan.version.clone(),
        state: state.into(),
        message: message.into(),
        error,
        updated_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
        directory: directory.into(),
    };
    crate::config::atomic_write(
        &data_dir.join("update-install.json"),
        &serde_json::to_vec(&status)?,
    )
    .context("无法保存安装记录")
}

#[derive(Clone, Serialize, Deserialize)]
struct Plan {
    package: PathBuf,
    executable: PathBuf,
    sha256: String,
    version: String,
    parent: u32,
    #[serde(default = "default_restart")]
    restart: bool,
    #[serde(default)]
    require_commit: bool,
    #[serde(default)]
    data_dir: Option<PathBuf>,
}
fn default_restart() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
struct Outcome {
    message: String,
    error: bool,
}

#[derive(Debug, Clone)]
pub struct InstallSession {
    pub directory: PathBuf,
    helper_pid: u32,
}
impl InstallSession {
    pub fn requires_shutdown(&self) -> bool {
        cfg!(target_os = "macos")
    }
    pub fn cancel(&self) {
        let _ = fs::write(self.directory.join("cancel"), b"");
    }
    #[cfg(target_os = "macos")]
    pub(crate) fn commit(&self) -> Result<()> {
        fs::write(self.directory.join("commit"), b"").context("无法确认更新交接")
    }
    pub fn outcome(&self) -> Option<(String, bool)> {
        read_outcome(&self.directory).or_else(|| {
            (!parent_alive(self.helper_pid))
                .then(|| ("更新失败：安装助手意外退出，请检查安全软件".into(), true))
        })
    }
}

fn verify(plan: &Plan, directory: &Path) -> Result<()> {
    if plan.package.parent() != Some(directory)
        || !plan.executable.is_absolute()
        || plan
            .data_dir
            .as_ref()
            .is_some_and(|data_dir| !data_dir.is_absolute())
    {
        bail!("更新路径无效");
    }
    let mut file = fs::File::open(&plan.package).context("安装包已被删除或拦截")?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).context("无法读取安装包")?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if clash_client_core::file_hash::hex(hash.finalize()) != plan.sha256 {
        bail!("安装包校验失败，已停止安装");
    }
    Ok(())
}

fn command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::CommandExt;
        // A login LaunchAgent's remaining process group is terminated when
        // its GUI exits. The updater and replacement GUI must outlive that
        // job instead of being killed between readiness and the app swap.
        command.process_group(0);
    }
    command
}

fn restart_client(plan: &Plan, directory: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        crate::platform::restart_after_update(&plan.executable, directory, plan.data_dir.as_deref())
    }
    #[cfg(not(windows))]
    {
        let mut restart = command(&plan.executable);
        restart.arg("--update-result").arg(directory);
        if let Some(data_dir) = &plan.data_dir {
            restart.env("CLASH_OF_RUST_DATA_DIR", data_dir);
        }
        restart.spawn().context("无法重新打开客户端")?;
        Ok(())
    }
}

pub fn handoff(download: Arc<Downloaded>) -> Result<InstallSession> {
    handoff_at(download, None)
}
pub fn handoff_at(download: Arc<Downloaded>, data_dir: Option<PathBuf>) -> Result<InstallSession> {
    handoff_owner_at(download, std::process::id(), true, data_dir)
}
pub fn handoff_owner(
    download: Arc<Downloaded>,
    parent: u32,
    restart: bool,
) -> Result<InstallSession> {
    handoff_owner_at(download, parent, restart, None)
}
pub fn handoff_owner_at(
    download: Arc<Downloaded>,
    parent: u32,
    restart: bool,
    data_dir: Option<PathBuf>,
) -> Result<InstallSession> {
    handoff_with_commit(download, parent, restart, false, data_dir)
}

#[cfg(target_os = "macos")]
pub(crate) fn handoff_owner_deferred_at(
    download: Arc<Downloaded>,
    parent: u32,
    restart: bool,
    data_dir: Option<PathBuf>,
) -> Result<InstallSession> {
    handoff_with_commit(download, parent, restart, true, data_dir)
}

fn handoff_with_commit(
    download: Arc<Downloaded>,
    parent: u32,
    restart: bool,
    require_commit: bool,
    data_dir: Option<PathBuf>,
) -> Result<InstallSession> {
    let directory = download.directory().to_owned();
    for name in ["ready", "cancel", "commit", "outcome.json", "stage.json"] {
        let path = directory.join(name);
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    let executable = ensure_installable()?;
    let plan = Plan {
        package: download.package.clone(),
        executable,
        sha256: download.sha256.clone(),
        version: download.version.clone(),
        parent,
        restart,
        require_commit,
        data_dir,
    };
    let helper = directory.join(if cfg!(windows) {
        "update-helper.exe"
    } else {
        "update-helper"
    });
    fs::copy(&plan.executable, &helper).context("无法准备安装助手，请检查磁盘空间或安全软件")?;
    fs::write(directory.join("plan.json"), serde_json::to_vec(&plan)?)
        .context("无法保存更新信息")?;
    record_installation(
        &plan,
        &directory,
        "preparing",
        "安装包已校验，正在准备安装",
        false,
    )?;
    let started = command(&helper)
        .arg("--install-update")
        .arg(&directory)
        .spawn()
        .context("无法启动安装助手，请检查安全软件或权限");
    let mut child = match started {
        Ok(child) => child,
        Err(error) => {
            let _ = record_installation(&plan, &directory, "failed", &format!("{error:#}"), true);
            return Err(error);
        }
    };
    let deadline = Instant::now() + Duration::from_secs(240);
    loop {
        if let Some((reason, true)) = read_outcome(&directory) {
            let _ = fs::write(directory.join("cancel"), b"");
            let _ = child.kill();
            let _ = child.wait();
            let _ = record_installation(&plan, &directory, "failed", &reason, true);
            bail!("{}", reason.strip_prefix("更新失败：").unwrap_or(&reason));
        }
        if directory.join("ready").exists() {
            download.relinquish();
            return Ok(InstallSession {
                directory,
                helper_pid: child.id(),
            });
        }
        if child.try_wait()?.is_some() {
            let _ = record_installation(&plan, &directory, "failed", "安装助手意外退出", true);
            bail!("安装助手意外退出，请检查安全软件");
        }
        if Instant::now() >= deadline {
            let _ = fs::write(directory.join("cancel"), b"");
            let _ = child.kill();
            let _ = child.wait();
            let _ = record_installation(&plan, &directory, "failed", "安装准备或授权超时", true);
            bail!("安装准备或授权超时，客户端仍在运行，请重试或手动安装");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn ensure_installable() -> Result<PathBuf> {
    let executable = crate::platform::autostart_executable().context("无法定位当前客户端")?;
    #[cfg(windows)]
    if !crate::platform::installed_executable(&executable)? {
        bail!("请先使用 EXE 安装程序安装客户端，再使用自动更新");
    }
    #[cfg(target_os = "linux")]
    if executable != Path::new("/opt/clash-of-rust/clash-of-rust") {
        bail!("请先使用 DEB 安装客户端，再使用自动更新");
    }
    #[cfg(target_os = "linux")]
    if !crate::platform::is_elevated() && !Path::new("/usr/bin/pkexec").is_file() {
        bail!("缺少 pkexec，无法请求安装授权");
    }
    #[cfg(target_os = "macos")]
    macos::ensure_installable(&executable)?;
    Ok(executable)
}

#[cfg(target_os = "macos")]
#[path = "install/macos.rs"]
mod macos;

/// Called before application locks, GUI libraries, or any Tokio runtime.
pub fn helper_main() -> bool {
    let mut args = std::env::args_os();
    let _ = args.next();
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--install-update")) {
        return false;
    }
    let Some(directory) = args.next().map(PathBuf::from) else {
        return true;
    };
    let result = (|| -> Result<Plan> {
        let data = fs::read(directory.join("plan.json"))?;
        if data.len() > 64 * 1024 {
            bail!("更新信息过大");
        }
        let plan: Plan = serde_json::from_slice(&data)?;
        verify(&plan, &directory)?;
        record_installation(
            &plan,
            &directory,
            "installing",
            "安装助手正在处理，尚未确认完成",
            false,
        )?;
        install(&plan, &directory)?;
        Ok(plan)
    })();
    let cancelled = directory.join("cancel").exists();
    let (message, error) = match &result {
        Ok(plan) => (format!("已更新至 {}", plan.version), false),
        Err(error) => (format!("更新失败：{error:#}"), true),
    };
    let _ = crate::config::atomic_write(
        &directory.join("outcome.json"),
        &serde_json::to_vec(&Outcome { message, error }).unwrap_or_default(),
    );
    if let Ok(data) = fs::read(directory.join("plan.json"))
        && let Ok(plan) = serde_json::from_slice::<Plan>(&data)
        && let Some((message, error)) = read_outcome(&directory)
    {
        let _ = record_installation(
            &plan,
            &directory,
            if error { "failed" } else { "succeeded" },
            &message,
            error,
        );
    }
    // Before readiness the original GUI is still open and handles the error.
    if !cancelled
        && directory.join("ready").exists()
        && let Ok(data) = fs::read(directory.join("plan.json"))
        && let Ok(plan) = serde_json::from_slice::<Plan>(&data)
        && (!plan.require_commit || directory.join("commit").exists())
    {
        // The existing GUI consumes failures if it is still alive.
        // Restart only once its normal shutdown releases the lock.
        let deadline = Instant::now() + Duration::from_secs(30);
        while parent_alive(plan.parent) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        if !parent_alive(plan.parent)
            && plan.restart
            && let Err(error) = restart_client(&plan, &directory)
        {
            let message = format!("更新安装已结束，重新打开客户端失败：{error:#}");
            let _ = crate::config::atomic_write(
                &directory.join("outcome.json"),
                &serde_json::to_vec(&Outcome {
                    message: message.clone(),
                    error: true,
                })
                .unwrap_or_default(),
            );
            let _ = record_installation(&plan, &directory, "failed", &message, true);
        }
    }
    #[cfg(target_os = "macos")]
    if let Ok(data) = fs::read(directory.join("plan.json"))
        && let Ok(plan) = serde_json::from_slice::<Plan>(&data)
        && plan.require_commit
        && (cancelled || (!parent_alive(plan.parent) && !directory.join("commit").exists()))
        && let Some(resolved) = owned_update_directory(&directory)
    {
        // Deferred handoffs have no GUI session left to clean an abandoned helper.
        let _ = fs::remove_dir_all(resolved);
    }
    true
}

#[cfg(target_os = "macos")]
fn stage(directory: &Path, message: &str) -> Result<()> {
    crate::config::atomic_write(&directory.join("stage.json"), &serde_json::to_vec(message)?)
}

pub fn read_outcome(directory: &Path) -> Option<(String, bool)> {
    let data = fs::read(directory.join("outcome.json")).ok()?;
    if data.len() > 4096 {
        return None;
    }
    let result: Outcome = serde_json::from_slice(&data).ok()?;
    Some((result.message, result.error))
}

pub fn startup_outcome() -> Option<(String, bool)> {
    let mut args = std::env::args_os();
    while let Some(arg) = args.next() {
        if arg == "--update-result" {
            let directory = PathBuf::from(args.next()?);
            let outcome = read_outcome(&directory)?;
            // Remove only our own isolated temporary update directory.
            cleanup(&directory);
            return Some(outcome);
        }
    }
    None
}

fn owned_update_directory(directory: &Path) -> Option<PathBuf> {
    let resolved = directory.canonicalize().ok()?;
    let temporary = std::env::temp_dir().canonicalize().ok()?;
    (resolved.parent() == Some(temporary.as_path())
        && resolved
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("clash-of-rust-update-")))
    .then_some(resolved)
}

pub fn cleanup(directory: &Path) {
    if let Some(resolved) = owned_update_directory(directory) {
        std::thread::spawn(move || {
            // On Windows the finishing helper briefly holds its own EXE open.
            for _ in 0..60 {
                if fs::remove_dir_all(&resolved).is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        });
    }
}

#[cfg(windows)]
fn install(plan: &Plan, directory: &Path) -> Result<()> {
    let mut installer = crate::platform::launch_update_installer(&plan.package)?;
    fs::write(directory.join("ready"), b"")?;
    match installer.wait()? {
        0 => Ok(()),
        2 => bail!("安装已取消"),
        3 | 5 => bail!("客户端未能正常退出或文件被占用，请退出后重试"),
        6 => bail!("安装文件写入失败，请检查磁盘空间与权限"),
        61 => bail!("无法替换主程序：文件被占用或安装目录不可写，已保留旧版本"),
        62 => bail!("无法替换界面程序：文件被占用或安装目录不可写，已恢复旧版本"),
        64 => bail!("无法写入卸载程序：文件被占用或安装目录不可写，已恢复旧版本"),
        65 => bail!("Windows 网络助手安装失败，已恢复旧版主程序与界面"),
        66 => bail!("无法写入安装登记信息，已恢复旧版主程序与界面"),
        code => bail!("安装程序失败（代码 {code}），请重试"),
    }
}

#[cfg(target_os = "linux")]
fn install(plan: &Plan, directory: &Path) -> Result<()> {
    if !Path::new("/usr/bin/dpkg").is_file() {
        bail!("当前系统没有 dpkg，无法安装 DEB");
    }
    let elevated = crate::platform::is_elevated();
    if !elevated && !Path::new("/usr/bin/pkexec").is_file() {
        bail!("缺少 pkexec，无法请求安装授权");
    }
    let mut install = if elevated {
        command(Path::new("/usr/bin/dpkg"))
    } else {
        let mut command = command(Path::new("/usr/bin/pkexec"));
        command.args(["--disable-internal-agent", "/usr/bin/dpkg"]);
        command
    };
    let mut child = install
        .arg("-i")
        .arg(&plan.package)
        .spawn()
        .context("无法启动 DEB 安装程序")?;
    fs::write(directory.join("ready"), b"")?;
    match child.wait()?.code() {
        Some(0) => Ok(()),
        Some(126) => bail!("安装授权已取消"),
        Some(127) => bail!("无法获得安装授权，请检查桌面授权服务"),
        _ => bail!("DEB 安装失败，请检查包管理器占用、依赖与磁盘空间"),
    }
}

#[cfg(target_os = "macos")]
fn install(plan: &Plan, directory: &Path) -> Result<()> {
    macos::install(plan, directory)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn install(_: &Plan, _: &Path) -> Result<()> {
    bail!("当前平台暂不支持自动安装");
}

#[cfg(windows)]
fn parent_alive(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return false;
    }
    let alive = unsafe { WaitForSingleObject(handle, 0) } != 0;
    unsafe {
        CloseHandle(handle);
    }
    alive
}

#[cfg(unix)]
fn parent_alive(pid: u32) -> bool {
    unsafe {
        libc::kill(pid as i32, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status_plan(data_dir: &Path, directory: &Path) -> Plan {
        Plan {
            package: directory.join("package"),
            executable: std::env::current_exe().unwrap(),
            sha256: "0".repeat(64),
            version: "0.6.7-fix.8.example".into(),
            parent: std::process::id(),
            restart: false,
            require_commit: false,
            data_dir: Some(data_dir.into()),
        }
    }

    #[test]
    fn installation_result_survives_temporary_directory_cleanup_and_does_not_regress() {
        let data = tempfile::tempdir().unwrap();
        let download = tempfile::tempdir().unwrap();
        let plan = status_plan(data.path(), download.path());
        assert!(installation_status(data.path()).unwrap().is_none());
        record_installation(&plan, download.path(), "preparing", "准备安装", false).unwrap();
        record_installation(&plan, download.path(), "succeeded", "安装成功", false).unwrap();
        record_installation(&plan, download.path(), "installing", "正在安装", false).unwrap();
        download.close().unwrap();
        let status = installation_status(data.path()).unwrap().unwrap();
        assert_eq!(status.state, "succeeded");
        assert_eq!(status.message, "安装成功");
        assert!(!status.error);
        let next = tempfile::tempdir().unwrap();
        record_installation(&plan, next.path(), "preparing", "下一次安装", false).unwrap();
        record_installation(&plan, &status.directory, "failed", "上次安装迟到", true).unwrap();
        assert_eq!(
            installation_status(data.path()).unwrap().unwrap().message,
            "下一次安装"
        );
    }

    #[test]
    fn installation_status_uses_outcome_without_claiming_handoff_is_success() {
        let data = tempfile::tempdir().unwrap();
        let download = tempfile::tempdir().unwrap();
        let plan = status_plan(data.path(), download.path());
        record_installation(&plan, download.path(), "installing", "尚未完成", false).unwrap();
        let pending = installation_status(data.path()).unwrap().unwrap();
        assert_eq!(pending.state, "installing");
        fs::write(
            download.path().join("outcome.json"),
            serde_json::to_vec(&Outcome {
                message: "安装授权取消".into(),
                error: true,
            })
            .unwrap(),
        )
        .unwrap();
        let failed = installation_status(data.path()).unwrap().unwrap();
        assert_eq!(failed.state, "failed");
        assert_eq!(failed.message, "安装授权取消");
        assert!(failed.error);
        fs::write(
            data.path().join("update-install.json"),
            vec![b' '; 16 * 1024 + 1],
        )
        .unwrap();
        assert!(installation_status(data.path()).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn update_processes_are_isolated_from_the_gui_process_group() {
        let mut child = command(Path::new("/bin/sleep")).arg("30").spawn().unwrap();
        let pid = child.id() as libc::pid_t;
        let group = unsafe { libc::getpgid(pid) };
        let original = unsafe { libc::getpgrp() };
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(group, pid, "update process must lead its own process group");
        assert_ne!(group, original, "update process inherited the GUI group");
    }

    #[test]
    fn helper_rechecks_the_package_before_installing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("package");
        fs::write(&path, b"verified bytes").unwrap();
        let plan = Plan {
            package: path.clone(),
            executable: std::env::current_exe().unwrap(),
            sha256: clash_client_core::file_hash::hex(Sha256::digest(b"verified bytes")),
            version: "0.4.6".into(),
            parent: std::process::id(),
            restart: true,
            require_commit: false,
            data_dir: Some(std::env::current_dir().unwrap()),
        };
        let mut legacy = serde_json::to_value(&plan).unwrap();
        legacy.as_object_mut().unwrap().remove("restart");
        legacy.as_object_mut().unwrap().remove("require_commit");
        legacy.as_object_mut().unwrap().remove("data_dir");
        let legacy: Plan = serde_json::from_value(legacy).unwrap();
        assert!(legacy.restart && !legacy.require_commit);
        assert!(legacy.data_dir.is_none());
        let encoded = serde_json::to_value(&plan).unwrap();
        let decoded: Plan = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.data_dir, plan.data_dir);
        verify(&plan, directory.path()).unwrap();
        let relative = Plan {
            data_dir: Some(PathBuf::from("relative-data")),
            ..plan.clone()
        };
        assert!(verify(&relative, directory.path()).is_err());
        fs::write(path, b"tampered").unwrap();
        assert!(verify(&plan, directory.path()).is_err());
    }

    #[test]
    fn update_cleanup_accepts_only_owned_temporary_directories() {
        let owned = tempfile::Builder::new()
            .prefix("clash-of-rust-update-")
            .tempdir()
            .unwrap();
        let other = tempfile::tempdir().unwrap();
        assert_eq!(
            owned_update_directory(owned.path()),
            Some(owned.path().canonicalize().unwrap())
        );
        assert!(owned_update_directory(other.path()).is_none());
        let nested = owned.path().join("clash-of-rust-update-nested");
        fs::create_dir(&nested).unwrap();
        assert!(owned_update_directory(&nested).is_none());
    }
}
