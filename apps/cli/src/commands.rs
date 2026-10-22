mod inspection;
mod maintenance;
mod profiles;
mod rules;
use crate::{args::Args, backend, output};
use anyhow::{Context, Result, bail, ensure};
use clash_client_runtime::{
    control::{Command, Meta, Poll},
    engine::{ProxyMode, Scope},
};
use serde_json::{Value, json};

pub async fn execute(args: &Args) -> Result<()> {
    match args.path.as_str() {
        "run" => backend::run(args).await,
        "version" => output::emit(
            args,
            json!({"version":clash_client_core::VERSION,"os":std::env::consts::OS,"arch":std::env::consts::ARCH}),
        ),
        "completion" => {
            use std::io::Write;
            std::io::stdout()
                .lock()
                .write_all(crate::args::completion(&args.positional[0])?.as_bytes())?;
            Ok(())
        }
        "start" | "restart" => {
            backend::prepare_start(args).await?;
            let command = if args.path == "start" {
                Command::Start
            } else {
                Command::Restart
            };
            let _: Value = backend::request(args, command).await?;
            output::emit(args, serde_json::to_value(backend::meta(args).await?)?)
        }
        "stop" => {
            if let Some(remote) = backend::connect(args, false)
                .await
                .context("连接待停止的后台失败")?
            {
                let result: Value = remote
                    .request(Command::Shutdown)
                    .await
                    .context("停止内核失败")?;
                if result["shutdown"] == true {
                    remote
                        .wait_closed(&backend::store(args)?)
                        .await
                        .context("等待后台清理失败")?;
                }
            }
            output::emit(args, json!({"running":false}))
        }
        "status" => status(args).await,
        "mode" => {
            if let Some(mode) = args.positional.first() {
                ensure!(
                    matches!(mode.as_str(), "rule" | "global" | "direct"),
                    "模式应为 rule、global 或 direct"
                );
                let _: Value = backend::request(args, Command::Mode { mode: mode.clone() }).await?;
            }
            output::emit(
                args,
                json!({"mode":backend::meta(args).await?.settings.run_mode}),
            )
        }
        "proxy" => {
            if let Some(value) = args.positional.first() {
                let mode = match value.as_str() {
                    "off" => ProxyMode::Off,
                    "system" => ProxyMode::System,
                    "tun" => ProxyMode::Tun,
                    _ => bail!("代理状态应为 system、tun 或 off"),
                };
                if mode == ProxyMode::Tun {
                    maintenance::authorize_tun(args).await?;
                }
                let _: Value = backend::request(args, Command::Proxy { mode }).await?;
            }
            output::emit(
                args,
                json!({"proxy":backend::meta(args).await?.settings.proxy_mode}),
            )
        }
        path if path.starts_with("profile ") => profiles::execute(args).await,
        path if path.starts_with("rule ") => rules::execute(args).await,
        path if path.starts_with("node ")
            || path.starts_with("connection ")
            || path == "logs"
            || path.starts_with("logs ")
            || path == "diagnose"
            || path == "dns"
            || path.starts_with("ip-check")
            || path.starts_with("claude ") =>
        {
            inspection::execute(args).await
        }
        _ => maintenance::execute(args).await,
    }
}

async fn status(args: &Args) -> Result<()> {
    loop {
        let value = if let Some(remote) = backend::connect(args, false).await? {
            let poll: Poll = remote
                .request(Command::Snapshot {
                    scope: Scope::Home,
                    header: true,
                })
                .await?;
            let profile = poll
                .meta
                .profiles
                .iter()
                .find(|p| Some(&p.id) == poll.meta.settings.active_profile.as_ref());
            json!({"running":poll.snapshot.running,"owner":poll.meta.owner,"version":poll.meta.version,
                "profile":profile.map(|p|&p.name),"node":poll.snapshot.active_node,"mode":poll.snapshot.mode,
                "proxy":poll.meta.settings.proxy_mode,"protected":poll.meta.settings.claude_protection.is_some(),
                "connections":poll.snapshot.connection_count,"upload_rate":poll.snapshot.upload_rate,
                "download_rate":poll.snapshot.download_rate})
        } else {
            json!({"running":false,"owner":null})
        };
        output::emit(args, value)?;
        if !args.flag("watch") {
            break;
        }
        tokio::time::sleep(args.interval()?).await;
    }
    Ok(())
}

pub fn profile<'a>(
    meta: &'a Meta,
    requested: Option<&str>,
) -> Result<&'a clash_client_core::config::Profile> {
    let id = requested
        .or(meta.settings.active_profile.as_deref())
        .ok_or_else(|| anyhow::anyhow!("尚未选择订阅"))?;
    if let Some(profile) = meta.profiles.iter().find(|p| p.id == id) {
        return Ok(profile);
    }
    let mut matches = meta.profiles.iter().filter(|p| p.name == id);
    let value = matches
        .next()
        .ok_or_else(|| anyhow::anyhow!("未找到订阅：{id}"))?;
    ensure!(matches.next().is_none(), "订阅名称不唯一，请使用 ID");
    Ok(value)
}
