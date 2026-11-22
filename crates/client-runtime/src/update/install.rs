//! A copied native helper survives replacement of the GUI executable. No scripts.
use super::Downloaded;
use anyhow::{Context, Result, bail};
use clash_client_core::config::Language;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
#[cfg(target_os = "linux")]
mod linux;
mod package;
mod windows;

pub fn localize_installation_message(message: &str, language: Language) -> Cow<'_, str> {
    if language == Language::SimplifiedChinese {
        return message.into();
    }
    let translated = match message {
        "安装包已校验，正在准备安装" => {
            Some("Package verified; preparing installation")
        }
        "安装包已校验，等待终端授权；后台仍在运行" => Some(
            "Package verified; waiting for terminal authorization; background process still running",
        ),
        "安装助手正在处理，尚未确认完成" => {
            Some("Installer running; completion not yet confirmed")
        }
        "正在安装，后台已暂停" => Some("Installing; background process paused"),
        "正在启动安装助手…" => Some("Starting installer…"),
        "正在挂载更新安装包…" => Some("Mounting update package…"),
        "正在校验更新应用、内核与资源…" => {
            Some("Verifying application, core and resources…")
        }
        "正在请求安装授权，请完成系统确认…" => {
            Some("Waiting for installation authorization; confirm the system prompt…")
        }
        "正在准备更新文件…" => Some("Preparing update files…"),
        "更新已准备好，正在退出旧客户端…" => {
            Some("Update ready; closing the previous client…")
        }
        "正在替换应用文件…" => Some("Replacing application files…"),
        "应用已更新，正在清理安装包并重启…" => {
            Some("Application updated; cleaning up and restarting…")
        }
        "已取消更新下载" => Some("Update download cancelled"),
        "安装授权已取消" => Some("Installation authorization cancelled"),
        "安装已取消" => Some("Installation cancelled"),
        _ => None,
    };
    if let Some(translated) = translated {
        return translated.into();
    }
    if let Some(rest) = message.strip_prefix("已更新至 ") {
        for (suffix, translated) in [
            (
                "；后台和内核已启动",
                "; background process and core started",
            ),
            (
                "；后台已恢复，内核保持关闭",
                "; background process restored; core remains stopped",
            ),
            (
                "；可用 cor start 启动后台",
                "; run cor start to start the background process",
            ),
        ] {
            if let Some(version) = rest.strip_suffix(suffix) {
                return format!("Updated to {version}{translated}").into();
            }
        }
        if !rest.contains('；') {
            return format!("Updated to {rest}").into();
        }
    }
    if let Some(rest) = message.strip_prefix("已安装 ")
        && let Some((version, detail)) = rest.split_once("，但恢复后台失败：")
        && let Some(reason) = detail.strip_suffix("；请运行 cor status 检查后重试")
    {
        return format!(
            "Installed {version}, but could not restore the background process: {reason}; check cor status and retry"
        )
        .into();
    }
    if let Some(reason) = message.strip_prefix("更新失败：") {
        let (reason, recovery) = if let Some((reason, recovery)) =
            reason.rsplit_once("；恢复后台也失败：")
            && let Some(detail) = recovery.strip_suffix("；请运行 cor start")
        {
            (
                reason,
                format!("; background recovery also failed: {detail}; run cor start"),
            )
        } else {
            [
                (
                    "；本次未停止原后台",
                    "; existing background process not stopped",
                ),
                (
                    "；已恢复更新前的后台和内核状态",
                    "; previous background and core state restored",
                ),
            ]
            .into_iter()
            .find_map(|(suffix, translated)| {
                reason
                    .strip_suffix(suffix)
                    .map(|reason| (reason, translated.to_owned()))
            })
            .unwrap_or_else(|| (reason, String::new()))
        };
        let reason = localize_installation_reason(reason);
        return format!("Update failed: {reason}{recovery}").into();
    }
    localize_installation_reason(message)
}

fn localize_installation_reason(reason: &str) -> Cow<'_, str> {
    let translated = match reason {
        "安装授权已取消" => Some("Installation authorization cancelled"),
        "安装已取消" => Some("Installation cancelled"),
        "未获得安装授权，未执行停止后台；请在交互终端重试，或由管理员手动安装" => {
            Some(
                "Installation authorization denied; retry in an interactive terminal or ask an administrator to install the package",
            )
        }
        "未获得安装授权，未执行停止后台；当前账户无权通过 sudo 安装此软件包" => {
            Some(
                "Installation authorization denied; this account cannot install the package with sudo",
            )
        }
        "安装助手意外退出" => Some("Installer exited unexpectedly"),
        "安装助手意外退出，请检查安全软件" => {
            Some("Installer exited unexpectedly; check security software")
        }
        "安装准备或授权超时" => {
            Some("Installation preparation or authorization timed out")
        }
        "安装准备或授权超时，客户端仍在运行，请重试或手动安装" => Some(
            "Installation preparation or authorization timed out; the client is still running; retry or install manually",
        ),
        "缺少 pkexec，无法请求安装授权" => {
            Some("pkexec is unavailable; cannot request installation authorization")
        }
        "无法获得安装授权，请检查桌面授权服务" => {
            Some("Installation authorization failed; check the desktop authorization service")
        }
        "DEB 安装失败，请检查包管理器占用、依赖与磁盘空间" => {
            Some("DEB installation failed; check the package manager, dependencies and disk space")
        }
        "Arch 原生包 安装失败，请检查包管理器占用、依赖与磁盘空间" => {
            Some(
                "Arch package installation failed; check the package manager, dependencies and disk space",
            )
        }
        "客户端未能正常退出或文件被占用，请退出后重试" => {
            Some("The client could not exit or files are in use; exit and retry")
        }
        "安装包架构与当前 Windows 不兼容，请下载匹配的安装包" => Some(
            "The package architecture is incompatible with Windows; download a matching package",
        ),
        "安装文件写入失败，具体原因未确认，请导出诊断后重试" => {
            Some("Installation files could not be written; export diagnostics and retry")
        }
        "无法替换主程序：文件被占用或安装目录不可写，已保留旧版本" => {
            Some(
                "Could not replace the application: files are in use or the directory is not writable; the previous version was retained",
            )
        }
        "无法替换界面程序：文件被占用或安装目录不可写，已恢复旧版本" => {
            Some(
                "Could not replace the interface: files are in use or the directory is not writable; the previous version was restored",
            )
        }
        "无法写入卸载程序：文件被占用或安装目录不可写，已恢复旧版本" => {
            Some(
                "Could not write the uninstaller: files are in use or the directory is not writable; the previous version was restored",
            )
        }
        "Windows 网络助手安装失败，已恢复旧版主程序与界面" => Some(
            "Windows network helper installation failed; the previous application and interface were restored",
        ),
        "无法写入安装登记信息，已恢复旧版主程序与界面" => Some(
            "Could not write installation registration; the previous application and interface were restored",
        ),
        "安装交接未确认或已超时，已保留旧版本" => Some(
            "Installation handoff was not confirmed or timed out; the previous version was retained",
        ),
        "安装目录中的文件不可写或仍被占用，未替换旧文件，请稍后重试" => {
            Some(
                "Installation files are not writable or are in use; old files retained; retry later",
            )
        }
        _ => None,
    };
    if let Some(translated) = translated {
        return translated.into();
    }
    if let Some((cause, file)) = reason
        .strip_prefix("安装未完成：")
        .and_then(|rest| rest.strip_suffix("），请处理后重试"))
        .and_then(|rest| rest.split_once('（'))
    {
        let cause = match cause {
            "没有写入权限、文件只读或被安全软件阻止" => {
                Some("No write permission, file is read-only, or blocked by security software")
            }
            "磁盘为只读" => Some("Disk is read-only"),
            "文件仍被占用" => Some("File is still in use"),
            "磁盘空间不足" => Some("Not enough disk space"),
            "无法写入文件" => Some("Could not write file"),
            _ => None,
        };
        if let Some(cause) = cause {
            return format!(
                "Installation incomplete: {cause} ({file}); resolve the issue and retry"
            )
            .into();
        }
    }
    if let Some(code) = reason
        .strip_prefix("安装程序失败（代码 ")
        .and_then(|rest| rest.strip_suffix("），请重试"))
        .filter(|code| code.parse::<u32>().is_ok())
    {
        return format!("Installer failed (code {code}); retry").into();
    }
    if let Some((status, detail)) = reason
        .strip_prefix("DEB 安装失败（")
        .and_then(|rest| rest.split_once("）："))
    {
        return format!("DEB installation failed ({status}): {detail}").into();
    }
    if let Some((status, detail)) = reason
        .strip_prefix("Arch 原生包 安装失败（")
        .and_then(|rest| rest.split_once("）："))
    {
        return format!("Arch package installation failed ({status}): {detail}").into();
    }
    if let Some(detail) = reason.strip_prefix("DEB 安装失败：") {
        return format!("DEB installation failed: {detail}").into();
    }
    reason.into()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InstallationStatus {
    pub version: String,
    pub state: String,
    pub message: String,
    pub error: bool,
    pub updated_at: u64,
    directory: PathBuf,
    #[serde(default)]
    acknowledged: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    installer_failure: Option<windows::Failure>,
}

impl InstallationStatus {
    pub fn needs_notification(&self) -> bool {
        self.error && self.state == "failed" && !self.acknowledged
    }

    pub fn matches_directory(&self, directory: &Path) -> bool {
        self.directory == directory
    }

    pub fn acknowledge(&self, data_dir: &Path) -> Result<bool> {
        if !self.needs_notification() {
            return Ok(false);
        }
        let lock = open_installation_lock(data_dir)?;
        lock.try_lock()
            .context("无法锁定安装记录进行确认，请稍后重试")?;
        let Some(mut current) = installation_status(data_dir)? else {
            return Ok(false);
        };
        if !current.needs_notification()
            || current.directory != self.directory
            || current.version != self.version
            || current.state != self.state
            || current.updated_at != self.updated_at
            || current.message != self.message
        {
            return Ok(false);
        }
        current.acknowledged = true;
        crate::config::atomic_write(
            &data_dir.join("update-install.json"),
            &serde_json::to_vec(&current)?,
        )
        .context("无法保存安装失败确认")?;
        Ok(true)
    }
}

fn open_installation_lock(data_dir: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(data_dir.join("update-install.lock"))
        .context("无法打开安装记录锁")
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
        status.installer_failure = windows::read_failure(&status.directory);
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
    let lock = open_installation_lock(data_dir)?;
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
        acknowledged: false,
        installer_failure: windows::read_failure(directory),
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
    #[cfg(target_os = "linux")]
    #[serde(default)]
    terminal: Option<terminal::Recovery>,
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
    #[cfg(any(windows, target_os = "macos"))]
    pub(crate) fn commit(&self) -> Result<()> {
        anyhow::ensure!(
            self.directory.is_dir() && !self.directory.join("cancel").exists(),
            "安装交接已取消，已保留旧版本"
        );
        if let Some((message, _)) = self.outcome() {
            bail!("安装交接已结束：{message}");
        }
        fs::write(self.directory.join("commit"), b"").context("无法确认更新交接")
    }
    pub fn outcome(&self) -> Option<(String, bool)> {
        read_outcome(&self.directory).or_else(|| {
            (!parent_alive(self.helper_pid))
                .then(|| ("更新失败：安装助手意外退出，请检查安全软件".into(), true))
        })
    }
}

fn verify(plan: &Plan, directory: &Path) -> Result<package::Verified> {
    if plan.package.parent() != Some(directory)
        || !plan.executable.is_absolute()
        || plan
            .data_dir
            .as_ref()
            .is_some_and(|data_dir| !data_dir.is_absolute())
    {
        bail!("更新路径无效");
    }
    package::open(&plan.package, &plan.sha256)
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

#[cfg(any(windows, target_os = "macos", test))]
pub(crate) fn supports_deferred_install(version: &str) -> bool {
    semver::Version::parse(version).is_ok_and(|version| {
        version >= semver::Version::parse("0.6.7-fix.9.session-lifecycle").expect("valid version")
    })
}

#[cfg(any(windows, target_os = "macos"))]
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
    let (plan, directory) =
        prepare_plan(download.as_ref(), parent, restart, require_commit, data_dir)?;
    let helper = directory.join(if cfg!(windows) {
        "update-helper.exe"
    } else {
        "update-helper"
    });
    fs::copy(&plan.executable, &helper).context("无法准备安装助手，请检查磁盘空间或安全软件")?;
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

fn prepare_plan(
    download: &Downloaded,
    parent: u32,
    restart: bool,
    require_commit: bool,
    data_dir: Option<PathBuf>,
) -> Result<(Plan, PathBuf)> {
    let directory = download.directory().to_owned();
    for name in [
        "ready",
        "cancel",
        "commit",
        "outcome.json",
        "stage.json",
        "installer-failure.json",
    ] {
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
        #[cfg(target_os = "linux")]
        terminal: None,
    };
    fs::write(directory.join("plan.json"), serde_json::to_vec(&plan)?)
        .context("无法保存更新信息")?;
    Ok((plan, directory))
}

pub fn ensure_installable() -> Result<PathBuf> {
    let executable = crate::platform::autostart_executable().context("无法定位当前客户端")?;
    #[cfg(windows)]
    if !crate::platform::installed_executable(&executable)? {
        bail!("请先使用 EXE 安装程序安装客户端，再使用自动更新");
    }
    #[cfg(target_os = "linux")]
    {
        anyhow::ensure!(
            executable == Path::new("/opt/clash-of-rust/clash-of-rust"),
            "请先安装官方 Linux 软件包，再使用自动更新"
        );
        linux::Installer::for_executable(&executable)?;
    }
    #[cfg(target_os = "macos")]
    macos::ensure_installable(&executable)?;
    Ok(executable)
}

#[cfg(target_os = "macos")]
#[path = "install/macos.rs"]
mod macos;
#[cfg(target_os = "linux")]
#[path = "install/terminal.rs"]
mod terminal;
#[cfg(target_os = "linux")]
pub use terminal::run_terminal_install;
#[cfg(target_os = "linux")]
pub(crate) use terminal::{commit_terminal_install, prepare_terminal_install};

/// Called before application locks, GUI libraries, or any Tokio runtime.
pub fn helper_main() -> bool {
    let mut args = std::env::args_os();
    let _ = args.next();
    let mode = args.next();
    #[cfg(target_os = "linux")]
    if mode.as_deref() == Some(std::ffi::OsStr::new("--install-update-terminal")) {
        if let Some(directory) = args.next() {
            terminal::helper(Path::new(&directory));
        }
        return true;
    }
    if mode.as_deref() != Some(std::ffi::OsStr::new("--install-update")) {
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
        let verified = verify(&plan, &directory)?;
        record_installation(
            &plan,
            &directory,
            "installing",
            "安装助手正在处理，尚未确认完成",
            false,
        )?;
        install(&plan, &directory, &verified)?;
        Ok(plan)
    })();
    let cancelled = directory.join("cancel").exists();
    let (message, error) = match &result {
        Ok(plan) => (format!("已更新至 {}", plan.version), false),
        Err(error) => (format!("更新失败：{error:#}"), true),
    };
    if let Ok(data) = fs::read(directory.join("plan.json"))
        && let Ok(plan) = serde_json::from_slice::<Plan>(&data)
    {
        let _ = record_installation(
            &plan,
            &directory,
            if error { "failed" } else { "succeeded" },
            &message,
            error,
        );
    }
    let _ = crate::config::atomic_write(
        &directory.join("outcome.json"),
        &serde_json::to_vec(&Outcome { message, error }).unwrap_or_default(),
    );
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
            let _ = record_installation(&plan, &directory, "failed", &message, true);
            let _ = crate::config::atomic_write(
                &directory.join("outcome.json"),
                &serde_json::to_vec(&Outcome {
                    message: message.clone(),
                    error: true,
                })
                .unwrap_or_default(),
            );
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
fn install(plan: &Plan, directory: &Path, verified: &package::Verified) -> Result<()> {
    anyhow::ensure!(
        !plan.require_commit || supports_deferred_install(&plan.version),
        "旧安装包不支持延迟安装交接"
    );
    let mut installer = crate::platform::launch_update_installer(
        &verified.path,
        plan.require_commit.then_some((directory, plan.parent)),
    )?;
    fs::write(directory.join("ready"), b"")?;
    windows::result(installer.wait()?, directory)
}

#[cfg(target_os = "linux")]
fn install(plan: &Plan, directory: &Path, verified: &package::Verified) -> Result<()> {
    let installer =
        linux::Installer::for_executable(Path::new("/opt/clash-of-rust/clash-of-rust"))?;
    installer.validate(plan, verified)?;
    let elevated = crate::platform::is_elevated();
    if !elevated && !Path::new("/usr/bin/pkexec").is_file() {
        bail!("缺少 pkexec，无法请求安装授权");
    }
    let mut install = if elevated {
        command(Path::new(installer.tool()))
    } else {
        let mut command = command(Path::new("/usr/bin/pkexec"));
        command.args(["--disable-internal-agent", installer.tool()]);
        command
    };
    let mut child = install
        .args(installer.arguments())
        .arg(&verified.path)
        .spawn()
        .with_context(|| format!("无法启动 {} 安装程序", installer.format()))?;
    fs::write(directory.join("ready"), b"")?;
    match child.wait()?.code() {
        Some(0) => Ok(()),
        Some(126) => bail!("安装授权已取消"),
        Some(127) => bail!("无法获得安装授权，请检查桌面授权服务"),
        _ => bail!(
            "{} 安装失败，请检查包管理器占用、依赖与磁盘空间",
            installer.format()
        ),
    }
}

#[cfg(target_os = "macos")]
fn install(plan: &Plan, directory: &Path, verified: &package::Verified) -> Result<()> {
    macos::install(plan, directory, &verified.path)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn install(_: &Plan, _: &Path, _: &package::Verified) -> Result<()> {
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

    #[test]
    fn windows_installation_failures_are_localized_from_actual_installer_results() {
        let directory = tempfile::tempdir().unwrap();
        for (code, expected) in [
            (2, "Installation cancelled"),
            (
                3,
                "The client could not exit or files are in use; exit and retry",
            ),
            (
                4,
                "The package architecture is incompatible with Windows; download a matching package",
            ),
            (
                6,
                "Installation files could not be written; export diagnostics and retry",
            ),
            (
                61,
                "Could not replace the application: files are in use or the directory is not writable; the previous version was retained",
            ),
            (
                62,
                "Could not replace the interface: files are in use or the directory is not writable; the previous version was restored",
            ),
            (
                64,
                "Could not write the uninstaller: files are in use or the directory is not writable; the previous version was restored",
            ),
            (
                65,
                "Windows network helper installation failed; the previous application and interface were restored",
            ),
            (
                66,
                "Could not write installation registration; the previous application and interface were restored",
            ),
            (
                67,
                "Installation handoff was not confirmed or timed out; the previous version was retained",
            ),
            (
                68,
                "Installation files are not writable or are in use; old files retained; retry later",
            ),
            (100, "Installer failed (code 100); retry"),
        ] {
            let reason = windows::result(code, directory.path())
                .unwrap_err()
                .to_string();
            let message = format!("更新失败：{reason}");
            assert_eq!(
                localize_installation_message(&message, Language::English),
                format!("Update failed: {expected}"),
                "installer exit {code}"
            );
            assert_eq!(
                localize_installation_message(&message, Language::SimplifiedChinese),
                message
            );
        }

        for (error, expected) in [
            (
                5,
                "No write permission, file is read-only, or blocked by security software",
            ),
            (19, "Disk is read-only"),
            (32, "File is still in use"),
            (112, "Not enough disk space"),
            (999, "Could not write file"),
        ] {
            let failure = serde_json::json!({
                "schema": 1, "stage": "preflight", "file": "resources/GeoIP.dat",
                "win32_error": error, "exit_code": 68
            });
            fs::write(
                directory.path().join("installer-failure.json"),
                serde_json::to_vec(&failure).unwrap(),
            )
            .unwrap();
            let reason = windows::result(68, directory.path())
                .unwrap_err()
                .to_string();
            let message = format!("更新失败：{reason}");
            assert_eq!(
                localize_installation_message(&message, Language::English),
                format!(
                    "Update failed: Installation incomplete: {expected} (resources/GeoIP.dat); resolve the issue and retry"
                ),
                "Windows error {error}"
            );
        }
    }

    #[test]
    fn installation_translation_preserves_unknown_causes_and_package_details() {
        for (message, expected) in [
            (
                "更新失败：无法获得安装授权，请检查桌面授权服务",
                "Update failed: Installation authorization failed; check the desktop authorization service",
            ),
            (
                "更新失败：DEB 安装失败，请检查包管理器占用、依赖与磁盘空间",
                "Update failed: DEB installation failed; check the package manager, dependencies and disk space",
            ),
            (
                "更新失败：Arch 原生包 安装失败（exit status: 1）：外部详情",
                "Update failed: Arch package installation failed (exit status: 1): 外部详情",
            ),
            (
                "更新失败：安装未完成：未知原因（resources/GeoIP.dat），请处理后重试",
                "Update failed: 安装未完成：未知原因（resources/GeoIP.dat），请处理后重试",
            ),
        ] {
            assert_eq!(
                localize_installation_message(message, Language::English),
                expected
            );
            assert_eq!(
                localize_installation_message(message, Language::SimplifiedChinese),
                message
            );
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn cancelled_or_finished_handoff_cannot_commit() {
        let directory = tempfile::tempdir().unwrap();
        let session = InstallSession {
            directory: directory.path().into(),
            helper_pid: std::process::id(),
        };
        fs::write(directory.path().join("cancel"), b"").unwrap();
        assert!(session.commit().is_err());
        assert!(!directory.path().join("commit").exists());
        fs::remove_file(directory.path().join("cancel")).unwrap();
        fs::write(
            directory.path().join("outcome.json"),
            serde_json::to_vec(&Outcome {
                message: "交接已超时".into(),
                error: true,
            })
            .unwrap(),
        )
        .unwrap();
        assert!(session.commit().is_err());
        assert!(!directory.path().join("commit").exists());
        fs::remove_file(directory.path().join("outcome.json")).unwrap();
        session.commit().unwrap();
        assert!(directory.path().join("commit").is_file());
    }

    #[test]
    fn deferred_install_requires_a_version_with_the_installer_gate() {
        for version in ["0.6.6", "0.6.7-fix.8.rules-shortcuts", "invalid"] {
            assert!(!supports_deferred_install(version));
        }
        for version in [
            "0.6.7-fix.9.session-lifecycle",
            "0.6.7-fix.10.next",
            "0.6.7",
            "0.6.8",
        ] {
            assert!(supports_deferred_install(version));
        }
    }

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
            #[cfg(target_os = "linux")]
            terminal: None,
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
    fn installation_failure_confirmation_survives_restart_and_retains_the_record() {
        let data = tempfile::tempdir().unwrap();
        let download = tempfile::tempdir().unwrap();
        let plan = status_plan(data.path(), download.path());
        record_installation(
            &plan,
            download.path(),
            "failed",
            "安装失败（代码 68）",
            true,
        )
        .unwrap();
        let failure = installation_status(data.path()).unwrap().unwrap();
        assert!(failure.needs_notification());
        assert!(failure.acknowledge(data.path()).unwrap());
        download.close().unwrap();
        let confirmed = installation_status(data.path()).unwrap().unwrap();
        assert!(!confirmed.needs_notification());
        assert!(confirmed.error);
        assert_eq!(confirmed.state, "failed");
        assert_eq!(confirmed.message, "安装失败（代码 68）");
        assert_eq!(confirmed.version, plan.version);
        assert!(!failure.acknowledge(data.path()).unwrap());
    }

    #[test]
    fn an_old_failure_confirmation_cannot_acknowledge_a_new_installation() {
        let data = tempfile::tempdir().unwrap();
        let previous = tempfile::tempdir().unwrap();
        let next = tempfile::tempdir().unwrap();
        let plan = status_plan(data.path(), previous.path());
        record_installation(&plan, previous.path(), "failed", "安装失败", true).unwrap();
        let old = installation_status(data.path()).unwrap().unwrap();
        record_installation(&plan, next.path(), "preparing", "正在准备", false).unwrap();
        assert!(!old.acknowledge(data.path()).unwrap());
        let current = installation_status(data.path()).unwrap().unwrap();
        assert_eq!(current.state, "preparing");
        assert!(!current.needs_notification());
        record_installation(&plan, next.path(), "failed", "安装失败", true).unwrap();
        assert!(!old.acknowledge(data.path()).unwrap());
        assert!(
            installation_status(data.path())
                .unwrap()
                .unwrap()
                .needs_notification()
        );
    }

    #[test]
    fn legacy_and_recovered_failure_records_remain_actionable_until_confirmed() {
        let data = tempfile::tempdir().unwrap();
        let download = tempfile::tempdir().unwrap();
        let plan = status_plan(data.path(), download.path());
        record_installation(&plan, download.path(), "installing", "尚未完成", false).unwrap();
        let path = data.path().join("update-install.json");
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        legacy.as_object_mut().unwrap().remove("acknowledged");
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        fs::write(
            download.path().join("outcome.json"),
            serde_json::to_vec(&Outcome {
                message: "安装权限被拒绝".into(),
                error: true,
            })
            .unwrap(),
        )
        .unwrap();
        let failure = installation_status(data.path()).unwrap().unwrap();
        assert!(failure.needs_notification());
        assert!(failure.acknowledge(data.path()).unwrap());
        download.close().unwrap();
        let confirmed = installation_status(data.path()).unwrap().unwrap();
        assert_eq!(confirmed.state, "failed");
        assert_eq!(confirmed.message, "安装权限被拒绝");
        assert!(!confirmed.needs_notification());
    }

    #[test]
    fn busy_installation_records_reject_confirmation_without_losing_the_failure() {
        let data = tempfile::tempdir().unwrap();
        let download = tempfile::tempdir().unwrap();
        let plan = status_plan(data.path(), download.path());
        record_installation(&plan, download.path(), "failed", "安装失败", true).unwrap();
        let failure = installation_status(data.path()).unwrap().unwrap();
        let lock = open_installation_lock(data.path()).unwrap();
        lock.lock().unwrap();
        assert!(failure.acknowledge(data.path()).is_err());
        assert!(
            installation_status(data.path())
                .unwrap()
                .unwrap()
                .needs_notification()
        );
        drop(lock);
        assert!(failure.acknowledge(data.path()).unwrap());
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

    #[test]
    fn installer_file_failure_remains_in_diagnostics_after_the_download_is_cleaned() {
        let data = tempfile::tempdir().unwrap();
        let download = tempfile::tempdir().unwrap();
        let plan = status_plan(data.path(), download.path());
        record_installation(&plan, download.path(), "installing", "正在安装", false).unwrap();
        let details = serde_json::json!({
            "schema": 1, "stage": "preflight", "file": "./cor.exe",
            "win32_error": 32, "exit_code": 68
        });
        fs::write(
            download.path().join("installer-failure.json"),
            serde_json::to_vec(&details).unwrap(),
        )
        .unwrap();
        record_installation(&plan, download.path(), "failed", "文件被占用", true).unwrap();
        download.close().unwrap();
        let status = installation_status(data.path()).unwrap().unwrap();
        assert!(status.error);
        assert_eq!(
            serde_json::to_value(status).unwrap()["installer_failure"],
            details
        );
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
            #[cfg(target_os = "linux")]
            terminal: None,
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
