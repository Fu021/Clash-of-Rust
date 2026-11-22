pub(crate) mod inspection;
mod maintenance;
mod profiles;
mod rules;
use crate::{args::Args, backend, output};
use anyhow::{Context, Result, bail, ensure};
use clash_client_runtime::{
    control::{Command, Meta, Poll},
    engine::{ProxyMode, Scope},
};
pub(crate) use maintenance::install_selected_update;
use serde_json::{Value, json};

pub async fn execute(args: &Args) -> Result<()> {
    if args.flag("all-users") && args.path == "profile list" {
        return output::emit(args, crate::users::profiles(args)?);
    }
    match args.path.as_str() {
        "ui" => crate::tui::run(args).await,
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
        "restart" if args.flag("network-only") => {
            output::confirm(
                args,
                args.language().text(
                    "刷新内核出站与 DNS 会关闭当前内核的旧连接，可能中断其他用户和程序；继续？",
                    "Refreshing core routes and DNS closes existing connections and may interrupt other users and applications. Continue?",
                ),
            )
            .await?;
            let remote = backend::connect(args, false)
                .await?
                .context(args.language().text(
                "当前用户后台未运行；网络刷新不会自动启动内核",
                "The current user's backend is not running; a network refresh will not start it",
            ))?;
            let _: Value = remote
                .request(Command::RecoverNetwork)
                .await
                .context(args.language().text(
                "网络刷新请求失败；请检查后台状态，旧版后台可能不支持此操作",
                "Network refresh failed; check the backend, as older versions may not support it",
            ))?;
            output::emit(args, json!({"network_refreshed":true}))
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
            if let Some(remote) =
                backend::connect(args, false)
                    .await
                    .context(args.language().text(
                        "连接待停止的后台失败",
                        "Failed to connect to the backend to stop it",
                    ))?
            {
                let result: Value = remote.request(Command::Shutdown).await.context(
                    args.language()
                        .text("停止内核失败", "Failed to stop the core"),
                )?;
                if result["shutdown"] == true {
                    remote.wait_closed(&backend::store(args)?).await.context(
                        args.language()
                            .text("等待后台清理失败", "Backend cleanup did not complete"),
                    )?;
                }
            }
            let routes = crate::host_network::with_proxy_context(
                crate::host_network::observe(args).await,
                Some(ProxyMode::Off),
            );
            output::emit(args, json!({"running":false,"host_routes":routes}))
        }
        "status" => status(args).await,
        "mode" => {
            if let Some(mode) = args.positional.first() {
                ensure!(
                    matches!(mode.as_str(), "rule" | "global" | "direct"),
                    "{}",
                    args.language().text(
                        "模式应为 rule、global 或 direct",
                        "Mode must be rule, global or direct"
                    )
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
                    _ => bail!(
                        "{}",
                        args.language().text(
                            "代理状态应为 system、tun 或 off",
                            "Proxy mode must be system, tun or off"
                        )
                    ),
                };
                if mode == ProxyMode::Tun {
                    maintenance::authorize_tun(args).await?;
                }
                let _: Value = backend::request(args, Command::Proxy { mode }).await?;
            }
            let poll = current_snapshot(args, Scope::Other, false).await?;
            let proxy = poll
                .as_ref()
                .filter(|poll| poll.snapshot.running)
                .map_or(ProxyMode::Off, |poll| poll.snapshot.proxy_mode());
            let routes = crate::host_network::with_settings_context(
                crate::host_network::observe(args).await,
                Some(proxy),
                poll.as_ref().map(|poll| &poll.meta.settings),
            );
            output::emit(args, json!({"proxy":proxy,"host_routes":routes}))
        }
        path if path.starts_with("profile ") => profiles::execute(args).await,
        path if path.starts_with("rule ") => rules::execute(args).await,
        path if path.starts_with("node ")
            || path.starts_with("connection ")
            || path == "logs"
            || path.starts_with("logs ")
            || path == "diagnose"
            || path == "dns"
            || path.starts_with("network ")
            || path.starts_with("ip-check")
            || path.starts_with("claude ") =>
        {
            inspection::execute(args).await
        }
        _ => maintenance::execute(args).await,
    }
}

async fn current_snapshot(args: &Args, scope: Scope, header: bool) -> Result<Option<Poll>> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let Some(remote) = backend::connect(args, false).await? else {
            return Ok(None);
        };
        remote
            .request(Command::Snapshot { scope, header })
            .await
            .map(Some)
    })
    .await
    .context(args.language().text(
        "读取当前用户后台状态超过 5 秒",
        "Reading the current user's backend status exceeded 5 seconds",
    ))?
}

async fn status(args: &Args) -> Result<()> {
    let mut routes = crate::host_network::observe(args).await;
    let mut observed_at = std::time::Instant::now();
    loop {
        if observed_at.elapsed() >= std::time::Duration::from_secs(5) {
            routes = crate::host_network::observe(args).await;
            observed_at = std::time::Instant::now();
        }
        let mut failure = None;
        let mut network_settings = None;
        let mut value = if let Some(poll) = match current_snapshot(args, Scope::Home, true).await {
            Ok(poll) => poll,
            Err(error) => {
                failure = Some(error);
                None
            }
        } {
            let profile = poll
                .meta
                .profiles
                .iter()
                .find(|p| Some(&p.id) == poll.meta.settings.active_profile.as_ref());
            let value = json!({"running":poll.snapshot.running,"owner":poll.meta.owner,"version":poll.meta.version,
                "profile":profile.map(|p|&p.name),"node":poll.snapshot.active_node,
                "route":poll.snapshot.active_route,"mode":poll.snapshot.mode,
                "proxy":poll.snapshot.proxy_mode(),"protected":poll.meta.settings.claude_protection.is_some(),
                "connections":poll.snapshot.connection_count,"upload_rate":poll.snapshot.upload_rate,
                "download_rate":poll.snapshot.download_rate});
            network_settings = Some(poll.meta.settings);
            value
        } else {
            json!({"running":false,"owner":null})
        };
        let proxy = if let Some(error) = &failure {
            value["running"] = Value::Null;
            value["backend_error"] = json!(format!("{error:#}"));
            None
        } else if value["running"] == true {
            serde_json::from_value(value["proxy"].clone()).ok()
        } else {
            Some(ProxyMode::Off)
        };
        value["host_routes"] = crate::host_network::with_settings_context(
            routes.clone(),
            proxy,
            network_settings.as_ref(),
        );
        output::emit(args, value)?;
        if !args.flag("watch") {
            if let Some(error) = failure {
                return Err(error);
            }
            break;
        }
        tokio::time::sleep(args.interval()?).await;
    }
    Ok(())
}

pub fn profile<'a>(
    meta: &'a Meta,
    requested: Option<&str>,
    language: clash_client_core::config::Language,
) -> Result<&'a clash_client_core::config::Profile> {
    let id = requested
        .or(meta.settings.active_profile.as_deref())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{}",
                language.text("尚未选择订阅", "No subscription is selected")
            )
        })?;
    if let Some(profile) = meta.profiles.iter().find(|p| p.id == id) {
        return Ok(profile);
    }
    let mut matches = meta.profiles.iter().filter(|p| p.name == id);
    let value = matches.next().ok_or_else(|| {
        anyhow::anyhow!(
            "{}{id}",
            language.text("未找到订阅：", "Subscription not found: ")
        )
    })?;
    ensure!(
        matches.next().is_none(),
        "{}",
        language.text(
            "订阅名称不唯一，请使用 ID",
            "The subscription name is ambiguous; use its ID"
        )
    );
    Ok(value)
}
