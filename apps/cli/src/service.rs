//! User login services. No root daemon reads user-editable YAML.
use crate::{args::Args, backend, output};
use anyhow::Result;
#[cfg(unix)]
use anyhow::{Context, ensure};
use serde_json::json;
#[cfg(unix)]
use std::path::PathBuf;
mod status;

pub async fn execute(args: &Args) -> Result<()> {
    #[cfg(windows)]
    {
        let enabled = match args.path.as_str() {
            "service install" => Some(true),
            "service remove" => Some(false),
            _ => None,
        };
        let resources = args.get("resources").map(backend::absolute).transpose()?;
        let enabled = clash_client_runtime::platform::headless_startup(
            &backend::data_root(args)?,
            resources.as_deref(),
            enabled,
        )?;
        let mut value = json!({"enabled":enabled,"scope":"current-user-logon"});
        if args.path == "service status" {
            value["installed"] = serde_json::Value::Null;
            value["active"] = serde_json::Value::Null;
            value["main_pid"] = serde_json::Value::Null;
            value = status::enrich(args, value).await;
        }
        output::emit(args, value)
    }
    #[cfg(unix)]
    {
        let (path, text) = definition(args)?;
        if args.path == "service status" {
            let mut value = status::configuration(&path, &text);
            match status::query().await {
                Ok(state) => value
                    .as_object_mut()
                    .unwrap()
                    .extend(state.as_object().unwrap().clone()),
                Err(error) => {
                    value["enabled"] = serde_json::Value::Null;
                    value["active"] = serde_json::Value::Null;
                    value["main_pid"] = serde_json::Value::Null;
                    value["service_error"] = json!(format!("{error:#}"));
                }
            }
            return output::emit(args, status::enrich(args, value).await);
        }
        let existing = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("无法读取 CLI 登录自启配置"),
        };
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
    let data = backend::data_root(args)?;
    #[cfg(target_os = "linux")]
    {
        let resources = args.get("resources").map(backend::absolute).transpose()?;
        let service = clash_client_runtime::platform::cli_service::definition(
            &exe,
            &data,
            resources.as_deref(),
        )?;
        Ok((
            clash_client_runtime::platform::cli_service::path()?,
            service,
        ))
    }
    #[cfg(target_os = "macos")]
    {
        let mut argv = vec![
            exe.to_string_lossy().into_owned(),
            "run".into(),
            "--data-dir".into(),
            data.to_string_lossy().into_owned(),
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
