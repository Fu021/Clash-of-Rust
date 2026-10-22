//! User login services. No root daemon reads user-editable YAML.
use crate::{args::Args, backend, output};
use anyhow::Result;
#[cfg(unix)]
use anyhow::{Context, ensure};
use serde_json::json;
#[cfg(unix)]
use std::path::PathBuf;

pub fn execute(args: &Args) -> Result<()> {
    #[cfg(windows)]
    {
        let enabled = match args.path.as_str() {
            "service install" => Some(true),
            "service remove" => Some(false),
            _ => None,
        };
        let resources = args.get("resources").map(backend::absolute).transpose()?;
        let enabled = clash_client_runtime::platform::headless_startup(
            &backend::store(args)?.root,
            resources.as_deref(),
            enabled,
        )?;
        output::emit(
            args,
            json!({"enabled":enabled,"scope":"current-user-logon"}),
        )
    }
    #[cfg(unix)]
    {
        let (path, text) = definition(args)?;
        let existing = std::fs::read_to_string(&path).ok();
        if args.path == "service status" {
            return output::emit(
                args,
                json!({"installed":existing.is_some(),"matches_current":existing.as_deref()==Some(&text),"path":path,"scope":"current-user-logon"}),
            );
        }
        ensure!(
            existing.as_ref().is_none_or(|current| current == &text),
            "已有不同配置的 CLI 登录服务，请先从原数据目录移除它"
        );
        if args.path == "service install" {
            clash_client_core::config::atomic_write(&path, text.as_bytes())?;
            #[cfg(target_os = "linux")]
            {
                system(&["--user", "daemon-reload"])?;
                system(&["--user", "enable", "clash-of-rust-cli.service"])?;
            }
        } else {
            #[cfg(target_os = "linux")]
            if existing.is_some() {
                system(&["--user", "disable", "clash-of-rust-cli.service"])?;
            }
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            #[cfg(target_os = "linux")]
            system(&["--user", "daemon-reload"])?;
        }
        output::emit(
            args,
            json!({"installed":args.path=="service install","path":path,"effective":"next-login","scope":"current-user-logon"}),
        )
    }
}
#[cfg(target_os = "linux")]
fn system(args: &[&str]) -> Result<()> {
    let status = std::process::Command::new("systemctl")
        .args(args)
        .status()
        .context("无法调用 systemctl --user；请在有用户会话的终端配置后台自启")?;
    ensure!(status.success(), "systemctl --user 操作失败");
    Ok(())
}
#[cfg(unix)]
fn definition(args: &Args) -> Result<(PathBuf, String)> {
    let exe = std::env::current_exe()?;
    let store = backend::store(args)?;
    let mut argv = vec![
        exe.to_string_lossy().into_owned(),
        "run".into(),
        "--data-dir".into(),
        store.root.to_string_lossy().into_owned(),
    ];
    if let Some(resources) = args.get("resources") {
        argv.extend([
            "--resources".into(),
            backend::absolute(resources)?.to_string_lossy().into_owned(),
        ]);
    }
    ensure!(
        argv.iter().all(|s| !s.contains(['\0', '\n', '\r'])),
        "服务路径不能包含控制字符"
    );
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
            });
        ensure!(base.is_absolute(), "无法确定用户配置目录");
        let quote = |s: &str| {
            format!(
                "\"{}\"",
                s.replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('%', "%%")
                    .replace('$', "$$")
            )
        };
        let command = argv.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ");
        Ok((
            base.join("systemd/user/clash-of-rust-cli.service"),
            format!(
                "[Unit]\nDescription=Clash of Rust CLI\nAfter=network-online.target\n\n[Service]\nType=simple\nExecStart={command}\nTimeoutStopSec=45\n\n[Install]\nWantedBy=default.target\n"
            ),
        ))
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").context("无法确定主目录")?;
        let xml = |s: &str| {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&apos;")
        };
        let arguments = argv
            .iter()
            .map(|s| format!("<string>{}</string>", xml(s)))
            .collect::<String>();
        Ok((
            PathBuf::from(home).join("Library/LaunchAgents/org.clash-of-rust.cli.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>Label</key><string>org.clash-of-rust.cli</string><key>ProgramArguments</key><array>{arguments}</array><key>RunAtLoad</key><true/><key>ProcessType</key><string>Background</string></dict></plist>\n"
            ),
        ))
    }
}
