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
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
struct Plan {
    package: PathBuf,
    executable: PathBuf,
    sha256: String,
    version: String,
    parent: u32,
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
    pub fn outcome(&self) -> Option<(String, bool)> {
        read_outcome(&self.directory).or_else(|| {
            (!parent_alive(self.helper_pid))
                .then(|| ("更新失败：安装助手意外退出，请检查安全软件".into(), true))
        })
    }
}

fn verify(plan: &Plan, directory: &Path) -> Result<()> {
    if plan.package.parent() != Some(directory) || !plan.executable.is_absolute() {
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
    if format!("{:x}", hash.finalize()) != plan.sha256 {
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
    command
}

pub fn handoff(download: Downloaded) -> Result<InstallSession> {
    let directory = download.directory.path().to_owned();
    let executable = ensure_installable()?;
    let plan = Plan {
        package: download.package,
        executable,
        sha256: download.sha256,
        version: download.version,
        parent: std::process::id(),
    };
    let helper = directory.join(if cfg!(windows) {
        "update-helper.exe"
    } else {
        "update-helper"
    });
    fs::copy(&plan.executable, &helper).context("无法准备安装助手，请检查磁盘空间或安全软件")?;
    fs::write(directory.join("plan.json"), serde_json::to_vec(&plan)?)
        .context("无法保存更新信息")?;
    let mut child = command(&helper)
        .arg("--install-update")
        .arg(&directory)
        .spawn()
        .context("无法启动安装助手，请检查安全软件或权限")?;
    loop {
        if directory.join("ready").exists() {
            let _ = download.directory.keep();
            return Ok(InstallSession {
                directory,
                helper_pid: child.id(),
            });
        }
        if let Some((reason, _)) = read_outcome(&directory) {
            bail!("{}", reason.strip_prefix("更新失败：").unwrap_or(&reason));
        }
        if child.try_wait()?.is_some() {
            bail!("安装助手意外退出，请检查安全软件");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn ensure_installable() -> Result<PathBuf> {
    let executable = std::env::current_exe().context("无法定位当前客户端")?;
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
    if executable
        .file_name()
        .is_none_or(|name| name != "clash-of-rust")
        || executable
            .parent()
            .and_then(Path::parent)
            .is_none_or(|contents| contents.file_name().is_none_or(|name| name != "Contents"))
    {
        bail!("请先使用 DMG 安装客户端，再使用自动更新");
    }
    Ok(executable)
}

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
        install(&plan, &directory)?;
        Ok(plan)
    })();
    let (message, error) = match &result {
        Ok(plan) => (format!("已更新至 {}", plan.version), false),
        Err(error) => (format!("更新失败：{error}"), true),
    };
    let _ = crate::config::atomic_write(
        &directory.join("outcome.json"),
        &serde_json::to_vec(&Outcome { message, error }).unwrap_or_default(),
    );
    // Before readiness the original GUI is still open and handles the error.
    if directory.join("ready").exists()
        && let Ok(data) = fs::read(directory.join("plan.json"))
        && let Ok(plan) = serde_json::from_slice::<Plan>(&data)
    {
        // The existing GUI consumes failures if it is still alive.
        // Restart only once its normal shutdown releases the lock.
        let deadline = Instant::now() + Duration::from_secs(30);
        while parent_alive(plan.parent) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        if !parent_alive(plan.parent) {
            let _ = command(&plan.executable)
                .arg("--update-result")
                .arg(&directory)
                .spawn();
        }
    }
    true
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

pub fn cleanup(directory: &Path) {
    let Ok(resolved) = directory.canonicalize() else {
        return;
    };
    let Ok(temporary) = std::env::temp_dir().canonicalize() else {
        return;
    };
    if resolved.parent() == Some(temporary.as_path())
        && resolved
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("clash-of-rust-update-"))
    {
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
    use crate::platform::macos::{administrator, shell_quote};
    let app = plan
        .executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("应用包路径无效")?;
    let parent = app.parent().context("应用包目录无效")?;
    let mount = directory.join("volume");
    fs::create_dir(&mount)?;
    let attached = Command::new("/usr/bin/hdiutil")
        .args([
            "attach",
            "-readonly",
            "-nobrowse",
            "-noautoopen",
            "-mountpoint",
        ])
        .arg(&mount)
        .arg(&plan.package)
        .output()?;
    if !attached.status.success() {
        bail!(
            "无法挂载更新 DMG：{}",
            String::from_utf8_lossy(&attached.stderr)
        );
    }
    let result = (|| {
        let replacement = mount.join("Clash of Rust.app");
        let info = replacement.join("Contents/Info.plist");
        for (key, expected) in [
            ("CFBundleIdentifier", "org.clashofrust.desktop"),
            ("ClashOfRustVersion", plan.version.as_str()),
        ] {
            let output = Command::new("/usr/bin/plutil")
                .args(["-extract", key, "raw", "-o", "-"])
                .arg(&info)
                .output()?;
            if !output.status.success()
                || String::from_utf8_lossy(&output.stdout).trim() != expected
            {
                bail!("更新应用标识或版本不匹配");
            }
        }
        if !Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&replacement)
            .status()?
            .success()
        {
            bail!("更新应用签名校验失败");
        }
        let resources = replacement.join("Contents/Resources/resources");
        crate::assets::verify_geo(&resources)?;
        let record: serde_json::Value =
            serde_json::from_slice(&fs::read(resources.join("core.json"))?)?;
        if crate::assets::hash_file(&resources.join("mihomo"))?.0
            != record["exe_sha256"].as_str().context("内核校验记录缺失")?
        {
            bail!("更新内核校验失败");
        }
        let staged = parent.join(format!(".clash-update-{}.app", uuid::Uuid::new_v4()));
        let backup = parent.join(format!(".clash-backup-{}.app", uuid::Uuid::new_v4()));
        // Complete validation and request permission before closing the GUI.
        // Stage on the destination filesystem so both replacements are renames.
        let local = Command::new("/usr/bin/ditto")
            .arg(&replacement)
            .arg(&staged)
            .status()?
            .success();
        if !local {
            let _ = fs::remove_dir_all(&staged);
            administrator(&format!(
                "/usr/bin/ditto {} {}",
                shell_quote(&replacement.to_string_lossy()),
                shell_quote(&staged.to_string_lossy())
            ))?;
        }
        fs::write(directory.join("ready"), b"")?;
        let deadline = Instant::now() + Duration::from_secs(45);
        while parent_alive(plan.parent) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        if parent_alive(plan.parent) {
            let _ = fs::remove_dir_all(&staged);
            bail!("客户端未能正常退出，已保留旧版本");
        }
        let swapped = fs::rename(app, &backup).and_then(|_| fs::rename(&staged, app));
        if swapped.is_err() {
            if backup.exists() && !app.exists() {
                let _ = fs::rename(&backup, app);
            }
            // User-owned installs need no dialog. System-owned installs request
            // authorization for this fixed, already-validated app replacement.
            administrator(&format!(
                "/bin/mv {} {} && ( /bin/mv {} {} || {{ /bin/mv {} {}; exit 1; }} )",
                shell_quote(&app.to_string_lossy()),
                shell_quote(&backup.to_string_lossy()),
                shell_quote(&staged.to_string_lossy()),
                shell_quote(&app.to_string_lossy()),
                shell_quote(&backup.to_string_lossy()),
                shell_quote(&app.to_string_lossy())
            ))?;
        }
        let _ = fs::remove_dir_all(&backup);
        Ok(())
    })();
    let _ = Command::new("/usr/bin/hdiutil")
        .arg("detach")
        .arg(&mount)
        .output();
    result
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
    #[test]
    fn helper_rechecks_the_package_before_installing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("package");
        fs::write(&path, b"verified bytes").unwrap();
        let plan = Plan {
            package: path.clone(),
            executable: std::env::current_exe().unwrap(),
            sha256: format!("{:x}", Sha256::digest(b"verified bytes")),
            version: "0.4.6".into(),
            parent: std::process::id(),
        };
        verify(&plan, directory.path()).unwrap();
        fs::write(path, b"tampered").unwrap();
        assert!(verify(&plan, directory.path()).is_err());
    }
}
