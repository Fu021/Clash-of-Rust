//! Validate and stage an app before handing shutdown to its normal GUI.
use super::{Plan, parent_alive, stage};
use crate::platform::macos::{administrator, run_command, shell_quote};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Read,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::Path,
    process::{Command, Output},
    time::{Duration, Instant},
};

fn app_path(executable: &Path) -> Result<&Path> {
    let binaries = executable.parent().context("应用可执行文件路径无效")?;
    let contents = binaries.parent().context("应用包路径无效")?;
    let app = contents.parent().context("应用包路径无效")?;
    ensure!(
        executable
            .file_name()
            .is_some_and(|name| name == "clash-of-rust")
            && binaries.file_name().is_some_and(|name| name == "MacOS")
            && contents.file_name().is_some_and(|name| name == "Contents")
            && app.extension().is_some_and(|extension| extension == "app"),
        "请先将 DMG 中的客户端安装到应用程序目录，再使用自动更新"
    );
    Ok(app)
}

pub(super) fn ensure_installable(executable: &Path) -> Result<()> {
    let app = app_path(executable)?;
    let parent = app.parent().context("应用目录无效")?;
    let path = std::ffi::CString::new(parent.as_os_str().as_bytes())?;
    let mut stats = std::mem::MaybeUninit::<libc::statfs>::uninit();
    ensure!(
        unsafe { libc::statfs(path.as_ptr(), stats.as_mut_ptr()) } == 0,
        "无法检查应用安装目录"
    );
    ensure!(
        unsafe { stats.assume_init() }.f_flags & libc::MNT_RDONLY as u32 == 0,
        "客户端位于只读磁盘映像，请先拖入应用程序目录并从那里启动，再使用自动更新"
    );
    Ok(())
}

fn checked(command: &mut Command, operation: &str, seconds: u64) -> Result<Output> {
    let output = run_command(command, operation, Duration::from_secs(seconds))?;
    ensure!(
        output.status.success(),
        "{operation}失败：{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output)
}

fn verify_app(app: &Path, version: &str) -> Result<()> {
    for (key, expected) in [
        ("CFBundleIdentifier", "org.clashofrust.desktop"),
        ("ClashOfRustVersion", version),
    ] {
        let output = checked(
            Command::new("/usr/bin/plutil")
                .args(["-extract", key, "raw", "-o", "-"])
                .arg(app.join("Contents/Info.plist")),
            "检查更新应用信息",
            10,
        )?;
        ensure!(
            String::from_utf8_lossy(&output.stdout).trim() == expected,
            "更新应用标识或版本不匹配"
        );
    }
    checked(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(app),
        "检查更新应用签名",
        30,
    )?;
    let resources = app.join("Contents/Resources/resources");
    crate::assets::verify_geo(&resources)?;
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(resources.join("core.json"))?)?;
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    ensure!(
        record["system"].as_str() == Some("macos") && record["arch"].as_str() == Some(arch),
        "更新内核平台或架构不匹配"
    );
    ensure!(
        crate::assets::hash_file(&resources.join("mihomo"))?.0
            == record["exe_sha256"].as_str().context("内核校验记录缺失")?,
        "更新内核校验失败"
    );
    for binary in [
        app.join("Contents/MacOS/clash-of-rust"),
        app.join("Contents/MacOS/clash-tun-launcher"),
        resources.join("mihomo"),
    ] {
        let mut header = [0; 8];
        fs::File::open(binary)?.read_exact(&mut header)?;
        let cpu = u32::from_le_bytes(header[4..8].try_into()?);
        ensure!(
            header[..4] == [0xcf, 0xfa, 0xed, 0xfe]
                && cpu
                    == if cfg!(target_arch = "aarch64") {
                        0x0100000c
                    } else {
                        0x01000007
                    },
            "更新程序架构不匹配"
        );
    }
    Ok(())
}

fn wait_for_shutdown(plan: &Plan, directory: &Path) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(45);
    while parent_alive(plan.parent) {
        ensure!(
            !directory.join("cancel").exists(),
            "客户端取消了更新交接，已保留旧版本"
        );
        ensure!(
            Instant::now() < deadline,
            "客户端未能正常退出，已保留旧版本"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    ensure!(
        !directory.join("cancel").exists(),
        "已取消更新，已保留旧版本"
    );
    Ok(())
}

fn privileged_transaction(
    app: &Path,
    replacement: &Path,
    staged: &Path,
    backup: &Path,
    directory: &Path,
    parent: u32,
) -> String {
    // One authorization covers staging, the atomic swap, rollback and cleanup.
    // The GUI remains open until the validated staged app is ready to replace it.
    format!(
        "set -eu\napp={}\nstaged={}\nbackup={}\nready={}\ncancel={}\ndirectory={}\nsuccess=0\ncleanup() {{\n  if [ \"$success\" = 0 ] && [ -d \"$backup\" ] && [ ! -e \"$app\" ]; then /bin/mv \"$backup\" \"$app\"; fi\n  /bin/rm -rf \"$staged\"\n}}\ntrap cleanup EXIT\n/usr/bin/ditto {} \"$staged\"\n/usr/bin/codesign --verify --deep --strict \"$staged\"\n[ -d \"$directory\" ]\n[ ! -e \"$cancel\" ]\n/usr/bin/touch \"$ready\"\ni=0\nwhile /bin/kill -0 {parent} 2>/dev/null; do\n  [ -d \"$directory\" ] && [ ! -e \"$cancel\" ] || {{ echo '客户端取消了更新交接，已保留旧版本' >&2; exit 1; }}\n  [ \"$i\" -lt 450 ] || {{ echo '客户端未能正常退出，已保留旧版本' >&2; exit 1; }}\n  i=$((i+1))\n  /bin/sleep 0.1\ndone\n[ -d \"$directory\" ]\n[ ! -e \"$cancel\" ]\n/bin/mv \"$app\" \"$backup\"\n{}\n/bin/mv \"$staged\" \"$app\"\nsuccess=1\n/bin/rm -rf \"$backup\"\n",
        shell_quote(&app.to_string_lossy()),
        shell_quote(&staged.to_string_lossy()),
        shell_quote(&backup.to_string_lossy()),
        shell_quote(&directory.join("ready").to_string_lossy()),
        shell_quote(&directory.join("cancel").to_string_lossy()),
        shell_quote(&directory.to_string_lossy()),
        shell_quote(&replacement.to_string_lossy()),
        ":",
    )
}

pub(super) fn install(plan: &Plan, directory: &Path) -> Result<()> {
    ensure_installable(&plan.executable)?;
    ensure!(plan.parent > 1, "更新交接进程无效");
    let app = app_path(&plan.executable)?;
    let parent = app.parent().context("应用包目录无效")?;
    let mount = directory.join("volume");
    fs::create_dir_all(&mount)?;
    stage(directory, "正在挂载更新安装包…")?;
    let attached = checked(
        Command::new("/usr/bin/hdiutil")
            .args([
                "attach",
                "-readonly",
                "-nobrowse",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(&mount)
            .arg(&plan.package),
        "挂载更新 DMG",
        60,
    );
    if let Err(error) = attached {
        let _ = checked(
            Command::new("/usr/bin/hdiutil").arg("detach").arg(&mount),
            "清理未完成的更新挂载",
            30,
        );
        return Err(error);
    }
    let staged = parent.join(format!(".clash-update-{}.app", uuid::Uuid::new_v4()));
    let backup = parent.join(format!(".clash-backup-{}.app", uuid::Uuid::new_v4()));
    let result = (|| {
        let replacement = mount.join("Clash of Rust.app");
        stage(directory, "正在校验更新应用、内核与资源…")?;
        verify_app(&replacement, &plan.version)?;
        let path = std::ffi::CString::new(parent.as_os_str().as_bytes())?;
        let local = unsafe { libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) } == 0
            && fs::metadata(app)?.uid() == unsafe { libc::getuid() };
        if !local {
            stage(directory, "正在请求安装授权，请完成系统确认…")?;
            return administrator(&privileged_transaction(
                app,
                &replacement,
                &staged,
                &backup,
                directory,
                plan.parent,
            ));
        }
        stage(directory, "正在准备更新文件…")?;
        checked(
            Command::new("/usr/bin/ditto")
                .arg(&replacement)
                .arg(&staged),
            "准备更新应用",
            60,
        )?;
        verify_app(&staged, &plan.version)?;
        ensure!(
            !directory.join("cancel").exists(),
            "已取消更新，已保留旧版本"
        );
        fs::write(directory.join("ready"), b"")?;
        stage(directory, "更新已准备好，正在退出旧客户端…")?;
        wait_for_shutdown(plan, directory)?;
        stage(directory, "正在替换应用文件…")?;
        fs::rename(app, &backup).context("无法保存旧应用，已保留旧版本")?;
        let swapped = fs::rename(&staged, app);
        if let Err(error) = swapped {
            fs::rename(&backup, app).context("更新替换失败，恢复旧应用失败")?;
            return Err(error).context("更新替换失败，已恢复旧版本");
        }
        let _ = fs::remove_dir_all(&backup);
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staged);
    if result.is_ok() {
        let _ = stage(directory, "应用已更新，正在清理安装包并重启…");
    }
    let _ = checked(
        Command::new("/usr/bin/hdiutil").arg("detach").arg(&mount),
        "卸载更新 DMG",
        30,
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn update_rejects_executables_outside_a_real_app_layout() {
        assert!(
            app_path(Path::new(
                "/Applications/Test.app/Contents/MacOS/clash-of-rust"
            ))
            .is_ok()
        );
        for path in [
            "/tmp/clash-of-rust",
            "/tmp/Test/Contents/MacOS/clash-of-rust",
            "/tmp/Test.app/Contents/Other/clash-of-rust",
        ] {
            assert!(app_path(Path::new(path)).is_err());
        }
    }
}
