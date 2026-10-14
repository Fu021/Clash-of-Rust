//! Launch a fresh user terminal without elevation or Apple Events automation.
use super::browser::validate_submit_url;
use super::system::shell_quote as quote;
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub struct Request<'a> {
    pub executable: &'a Path,
    pub submit: &'a str,
    pub directory: &'a str,
    pub offline: bool,
    pub keep_open: bool,
}
pub struct Launch {
    // Retain the private script until the report arrives or the session closes.
    _script: Option<tempfile::TempDir>,
}
impl std::fmt::Debug for Launch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AutomaticTerminalLaunch")
    }
}
fn directory(value: &str) -> Result<PathBuf> {
    let home = directories::BaseDirs::new().context("无法确定用户目录")?;
    let path = if value.trim().is_empty() {
        home.home_dir().to_owned()
    } else if let Some(tail) = value.trim().strip_prefix("~/") {
        home.home_dir().join(tail)
    } else {
        PathBuf::from(value.trim())
    };
    // Keep ordinary Windows paths: PowerShell 5 cannot Set-Location to the
    // extended \\?\ prefix returned by fs::canonicalize.
    let path = std::path::absolute(path).context("项目目录无效")?;
    ensure!(path.is_dir(), "请选择项目目录");
    Ok(path)
}
fn diagnostic(request: &Request<'_>) -> String {
    format!(
        "{} --automatic-terminal{}",
        super::system::terminal_command(request.executable, request.submit),
        if request.offline { " --offline" } else { "" }
    )
}
#[cfg(unix)]
fn script(request: &Request<'_>, directory: &Path) -> String {
    format!(
        "#!/bin/sh\ncd {} || exit 1\n{}\nresult=$?\nprintf '\\nClash of Rust: diagnostic finished (exit %s).\\n' \"$result\"\n{}exit \"$result\"\n",
        quote(&directory.to_string_lossy()),
        diagnostic(request),
        if request.keep_open {
            "printf 'Press Enter to close this window...'; read -r answer\n"
        } else {
            ""
        }
    )
}
pub async fn launch(request: Request<'_>) -> Result<Launch> {
    validate_submit_url(request.submit)?;
    ensure!(
        request.executable.is_absolute() && request.executable.is_file(),
        "应用程序路径无效"
    );
    let directory = directory(request.directory)?;
    #[cfg(windows)]
    {
        use base64::Engine as _;
        let command = format!(
            "Set-Location -LiteralPath {}; {}",
            quote(&directory.to_string_lossy()),
            diagnostic(&request)
        );
        let encoded = base64::engine::general_purpose::STANDARD.encode(
            command
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let mut command = Command::new("powershell.exe");
        command.arg("-NoLogo");
        if request.keep_open {
            command.arg("-NoExit");
        }
        // Profiles stay enabled so the new session loads the user's shell setup.
        command
            .args(["-EncodedCommand", &encoded])
            .current_dir(&directory)
            .creation_flags(0x00000010);
        command
            .spawn()
            .context("无法打开 PowerShell，可复制命令手动检测")?;
        Ok(Launch { _script: None })
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let temporary = tempfile::Builder::new()
            .prefix("clash-code-terminal-")
            .tempdir()?;
        let path = temporary.path().join(if cfg!(target_os = "macos") {
            "Claude-Code-check.command"
        } else {
            "Claude-Code-check.sh"
        });
        std::fs::write(&path, script(&request, &directory))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        #[cfg(target_os = "macos")]
        {
            let mut command = Command::new("/usr/bin/open");
            command
                .args(["-b", "com.apple.Terminal", "--"])
                .arg(&path)
                .kill_on_drop(true);
            let status = tokio::time::timeout(std::time::Duration::from_secs(15), command.status())
                .await
                .context("打开终端超时")??;
            ensure!(status.success(), "无法打开 Terminal，可复制命令手动检测");
        }
        #[cfg(target_os = "linux")]
        {
            let shell = std::env::var_os("SHELL")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute() && p.is_file())
                .unwrap_or_else(|| PathBuf::from("/bin/bash"));
            let execute = quote(&path.to_string_lossy());
            let mut opened = false;
            for terminal in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
                let mut command = Command::new(terminal);
                match terminal {
                    "gnome-terminal" => {
                        command.arg("--working-directory").arg(&directory).arg("--");
                    }
                    "konsole" => {
                        command.arg("--workdir").arg(&directory).arg("-e");
                    }
                    _ => {
                        command.arg("-e");
                    }
                }
                command
                    .arg(&shell)
                    .args(["-l", "-i", "-c", &execute])
                    .current_dir(&directory);
                match command.spawn() {
                    Ok(_) => {
                        opened = true;
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).context("无法打开终端，可复制命令手动检测"),
                }
            }
            ensure!(opened, "未找到终端程序，可复制命令手动检测");
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        anyhow::bail!("当前平台不支持自动打开终端");
        Ok(Launch {
            _script: Some(temporary),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn script_quotes_paths_and_preserves_the_requested_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("project ' $(touch unwanted) 中文");
        std::fs::create_dir(&target).unwrap();
        let output = target.join("cwd.txt");
        let executable = dir.path().join("diagnostic with ' spaces");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\npwd > {}\nprintf '%s\\n' \"$@\" > {}\n",
                quote(&output.to_string_lossy()),
                quote(&target.join("args.txt").to_string_lossy())
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let request = Request {
            executable: &executable,
            submit: "http://127.0.0.1:12345/cccccccccccccccccccccccccccccccc/report",
            directory: target.to_str().unwrap(),
            offline: true,
            keep_open: false,
        };
        let script_path = dir.path().join("test.sh");
        std::fs::write(&script_path, script(&request, &target)).unwrap();
        assert!(
            std::process::Command::new("/bin/sh")
                .arg(script_path)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(
            std::fs::read_to_string(output).unwrap().trim(),
            target.to_str().unwrap()
        );
        let args = std::fs::read_to_string(target.join("args.txt")).unwrap();
        assert!(args.contains("--automatic-terminal\n--offline"));
        assert!(!target.join("unwanted").exists());
    }
    #[test]
    fn rejects_missing_or_non_directory_project_paths() {
        let temp = tempfile::tempdir().unwrap();
        assert!(directory(temp.path().join("missing").to_str().unwrap()).is_err());
        let file = temp.path().join("file");
        std::fs::write(&file, "test").unwrap();
        assert!(directory(file.to_str().unwrap()).is_err());
    }
}
