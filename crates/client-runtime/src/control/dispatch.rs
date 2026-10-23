use super::*;
use crate::{config::read_profile, engine::Engine};
use anyhow::Result;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) fn meta(engine: &Engine, owner: Owner) -> Result<Meta> {
    Ok(Meta {
        view_revision: engine.view_revision(),
        settings: engine.settings.clone(),
        profiles: engine.profiles.clone(),
        geo: engine.geo_manifest.clone(),
        running: engine.running(),
        version: crate::VERSION.into(),
        owner,
        executable: std::env::current_exe()?,
    })
}

async fn start(engine: &mut Engine) -> Result<()> {
    if !engine.running() {
        engine.start().await?;
        if let Err(error) = engine.restore_proxy_mode().await {
            let _ = engine.stop().await;
            return Err(error);
        }
    }
    Ok(())
}

pub(super) async fn execute(
    shared: &Arc<Mutex<Engine>>,
    operation: Command,
    owner: Owner,
) -> Result<Value> {
    // GUI actions already return their fresh models. Console mutations also
    // invalidate attached views, without copying full lists on every tick.
    let changes_view = matches!(
        operation,
        Command::Start
            | Command::Stop
            | Command::Restart
            | Command::Shutdown
            | Command::Quit
            | Command::Settings { .. }
            | Command::ConfigSet { .. }
            | Command::Ports { .. }
            | Command::RuleOverrides { .. }
            | Command::RuleChange { .. }
            | Command::SaveRules { .. }
            | Command::Import { .. }
            | Command::EditProfile { .. }
            | Command::Activate { .. }
            | Command::DeleteProfile { .. }
            | Command::Select { .. }
            | Command::Mode { .. }
            | Command::Proxy { .. }
            | Command::Protect { .. }
            | Command::Delay { .. }
    );
    let mut engine = shared.lock().await;
    if changes_view {
        engine.view_revision = engine.view_revision.wrapping_add(1);
    }
    match operation {
        Command::Meta => return Ok(serde_json::to_value(meta(&engine, owner)?)?),
        Command::Snapshot { scope, header } => {
            if scope == Scope::Logs {
                engine.retain_external_logs();
            }
            let snapshot = engine.poll_with_header(scope, header).await?;
            return Ok(serde_json::to_value(Poll {
                meta: meta(&engine, owner)?,
                snapshot,
            })?);
        }
        Command::Start => start(&mut engine).await?,
        Command::Stop => engine.stop().await?,
        Command::Restart => {
            engine.stop().await?;
            start(&mut engine).await?;
        }
        Command::Shutdown => {
            engine.stop().await?;
            return Ok(
                json!({"shutdown": owner == Owner::Daemon && super::gui_pid(&engine.store)?.is_none()}),
            );
        }
        Command::Quit => {
            engine.stop().await?;
            return Ok(json!({"shutdown":owner==Owner::Daemon}));
        }
        Command::Settings { settings, expected } => {
            anyhow::ensure!(
                expected.is_none_or(|expected| serde_json::to_value(&engine.settings)
                    .is_ok_and(|current| current == expected)),
                "设置已被另一客户端修改，请刷新后重试"
            );
            engine.save_settings(settings).await?;
        }
        Command::RuleOverrides { overrides } => engine.save_rule_overrides(overrides).await?,
        Command::ConfigSet { key, value } => {
            if key == "mixed_port" {
                let port: u16 = serde_json::from_value(value)?;
                anyhow::ensure!(port != 0, "代理端口必须非零");
                engine.save_ports(port).await?;
            } else {
                let settings = super::mutations::setting(&engine.settings, &key, value)?;
                engine.save_settings(settings).await?;
            }
        }
        Command::RuleChange { id, change } => {
            let document = engine.rule_document(&id)?;
            let mut draft = (*crate::rule_manager::ProfileRules::for_profile(
                &engine.settings,
                &id,
                document.builtin,
            ))
            .clone();
            super::mutations::rule(&document, &mut draft, change)?;
            engine.save_profile_rules(&id, draft).await?;
        }
        Command::Ports { mixed } => engine.save_ports(mixed).await?,
        Command::Import { name, source, id } => engine.import(name, source, id).await?,
        Command::EditProfile { id, name, source } => engine.edit_profile(&id, &name, &source)?,
        Command::Activate { id } => engine.activate(id).await?,
        Command::DeleteProfile { id } => engine.delete_profile(&id).await?,
        Command::RuleDocument { id } => {
            return Ok(serde_json::to_value(engine.rule_document(&id)?)?);
        }
        Command::SaveRules {
            id,
            rules,
            expected,
        } => {
            engine
                .save_profile_rules_checked(&id, rules, expected)
                .await?
        }
        Command::Select { group, node } => {
            start(&mut engine).await?;
            engine.select_node(&group, &node).await?;
        }
        Command::Mode { mode } => {
            if engine.running() {
                engine.select_mode(&mode).await?;
            } else {
                let settings = Settings {
                    run_mode: mode,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
            }
        }
        Command::Proxy { mode } => {
            if mode == crate::engine::ProxyMode::Off && !engine.running() {
                anyhow::ensure!(
                    engine.settings.claude_protection.is_none(),
                    "请先关闭 Claude 保护"
                );
                engine.proxy_mode(mode).await?;
                let settings = Settings {
                    proxy_mode: mode,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
            } else {
                start(&mut engine).await?;
                engine.select_proxy_mode(mode).await?;
            }
        }
        Command::Protect { enabled } => {
            if enabled {
                start(&mut engine).await?;
            }
            engine.claude_protection(enabled).await?;
        }
        Command::TrayNodes => return Ok(serde_json::to_value(engine.tray_nodes().await?)?),
        Command::Delay { name, group } => {
            start(&mut engine).await?;
            let result = match name {
                Some(name) if group => engine.api.delay_group(&name).await?,
                Some(name) => engine.api.delay_node(&name).await?,
                None => engine.api.delay_all_nodes().await?,
            };
            return Ok(serde_json::to_value(result)?);
        }
        Command::Close { id } => {
            if let Some(id) = id {
                engine.api.close_connection(&id).await?;
            } else {
                engine.api.close_connections().await?;
            }
        }
        Command::Logs { clear } => {
            if clear {
                engine.clear_logs()?;
            }
            engine.retain_external_logs();
            let snapshot = engine.poll(Scope::Logs).await?;
            return Ok(serde_json::to_value(snapshot.logs)?);
        }
        Command::ExportLogs => return Ok(serde_json::to_value(engine.export_logs()?)?),
        Command::RecordFailure { message } => engine.record_failure(&message),
        Command::GeoUpdate => engine.update_geo().await?,
        Command::GeoInstall { path } => engine.install_geo(&path).await?,
        Command::CheckConfig => {
            engine.check_effective_config().await?;
            return Ok(json!({"valid":true}));
        }
        Command::EffectiveConfig => return Ok(json!({"yaml":engine.effective_config()?})),
        Command::Notify { title, message } => {
            crate::platform::notification::show(&title, &message).await?
        }
        Command::Autostart { enabled } => {
            if let Some(enabled) = enabled {
                crate::platform::set_autostart(enabled)?;
            }
            return Ok(json!({"enabled":crate::platform::autostart_enabled()?}));
        }
        Command::UpdateInstall => {
            crate::update::ensure_installable()?;
            let port = engine.running().then_some(engine.settings.mixed_port);
            let available =
                crate::update::check_with_prereleases(port, engine.settings.update_prereleases)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("当前更新通道没有可安装的版本"))?;
            let download = Arc::new(
                crate::update::download(available, port, Some(engine.api.clone()), |_| {}).await?,
            );
            let gui = super::gui_pid(&engine.store)?;
            let parent = gui.unwrap_or_else(std::process::id);
            let session = tokio::task::spawn_blocking(move || {
                crate::update::handoff_owner(download, parent, gui.is_some())
            })
            .await??;
            if let Err(error) = engine.stop().await {
                session.cancel();
                return Err(error);
            }
            if let Err(error) = super::request_gui_exit(&engine.store) {
                session.cancel();
                return Err(error);
            }
            return Ok(json!({"shutdown":true,"installer":session.directory}));
        }
    }
    Ok(Value::Null)
}

impl Engine {
    pub fn effective_config(&self) -> Result<String> {
        let id = self
            .settings
            .active_profile
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("尚未选择订阅"))?;
        let raw = read_profile(&self.store.profile_path(id)?)?;
        self.effective_profile(&raw, id)
    }
    pub async fn check_effective_config(&self) -> Result<()> {
        self.check_payload(&self.effective_config()?).await
    }
}
