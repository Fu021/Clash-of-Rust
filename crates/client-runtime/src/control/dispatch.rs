use super::*;
use crate::{config::read_profile, engine::Engine};
use anyhow::Result;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::watch;

#[cfg(any(windows, target_os = "macos"))]
pub(crate) async fn prepare_update_handoff<'a, S, F, Fut, C>(
    shared: &'a Arc<Mutex<Engine>>,
    job: &crate::engine::downloads::UpdateJob,
    disconnected: &Option<watch::Receiver<bool>>,
    prereleases: bool,
    prepare: F,
    cancel: C,
) -> Result<(tokio::sync::MutexGuard<'a, Engine>, S)>
where
    F: FnOnce(u32, bool, std::path::PathBuf) -> Fut,
    Fut: std::future::Future<Output = Result<S>>,
    C: FnOnce(&S),
{
    let (gui, parent, data_dir) = {
        let engine = shared.lock().await;
        job.ensure_current(disconnected)?;
        engine.settings.require_mutable()?;
        anyhow::ensure!(
            engine.settings.update_prereleases == prereleases,
            "更新通道已改变，请重新检查更新"
        );
        let gui = super::gui_pid(&engine.store)?;
        (
            gui,
            gui.unwrap_or_else(std::process::id),
            engine.store.root.clone(),
        )
    };
    let session = prepare(parent, gui.is_some(), data_dir).await?;
    let engine = shared.lock().await;
    if let Err(error) = (|| -> Result<()> {
        job.ensure_current(disconnected)?;
        engine.settings.require_mutable()?;
        anyhow::ensure!(
            engine.settings.update_prereleases == prereleases,
            "更新通道已改变，请重新检查更新"
        );
        anyhow::ensure!(
            super::gui_pid(&engine.store)? == gui,
            "桌面客户端已变化，请重新执行更新"
        );
        Ok(())
    })() {
        cancel(&session);
        return Err(error);
    }
    Ok((engine, session))
}

pub(crate) fn meta(engine: &Engine, owner: Owner) -> Result<Meta> {
    Ok(Meta {
        view_revision: engine.view_revision(),
        measurements: engine.api.measurement_diagnostics(),
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
        engine.start_with_proxy_mode().await?;
    }
    Ok(())
}

pub(super) async fn execute(
    shared: &Arc<Mutex<Engine>>,
    operation: Command,
    owner: Owner,
    disconnected: Option<watch::Receiver<bool>>,
) -> Result<Value> {
    operation.check_protection(&shared.lock().await.settings)?;
    if matches!(&operation, Command::RecoverUnreachable) {
        return Ok(serde_json::to_value(
            Engine::recover_unreachable_cancellable(shared, disconnected).await?,
        )?);
    }
    if let Command::Delay { name, group } = &operation {
        let api = {
            let mut engine = shared.lock().await;
            start(&mut engine).await?;
            engine.api.clone()
        };
        let result = match name {
            Some(name) if *group => api.delay_group(name).await?,
            Some(name) => api.delay_node(name).await?,
            None => api.delay_all_nodes().await?,
        };
        if (name.is_none() || *group)
            && !result.is_empty()
            && result.values().all(|delay| *delay == 0)
            && let Err(error) = Engine::recover_unreachable_cancellable(shared, disconnected).await
        {
            shared
                .lock()
                .await
                .record_failure(&format!("节点状态复测未完成，已保留网络设置：{error:#}"));
        }
        return Ok(serde_json::to_value(result)?);
    }
    if matches!(&operation, Command::GeoUpdate) {
        Engine::update_geo_cancellable(shared, disconnected).await?;
        return Ok(Value::Null);
    }
    if matches!(&operation, Command::UpdateInstall) {
        return install_update(shared, disconnected, false, None).await;
    }
    if matches!(&operation, Command::UpdatePrepareTerminal) {
        return install_update(shared, disconnected, true, None).await;
    }
    if let Command::UpdateSelected {
        version,
        preview,
        terminal,
    } = &operation
    {
        return install_update(
            shared,
            disconnected,
            *terminal,
            Some((version.clone(), *preview)),
        )
        .await;
    }
    if let Command::Import {
        name,
        source,
        id,
        expect_current,
    } = &operation
    {
        Engine::import_cancellable(
            shared,
            name.clone(),
            source.clone(),
            id.clone(),
            *expect_current,
            disconnected,
        )
        .await?;
        return Ok(Value::Null);
    }
    // GUI actions already return their fresh models. Console mutations also
    // invalidate attached views, without copying full lists on every tick.
    let changes_view = matches!(
        operation,
        Command::Start
            | Command::Stop
            | Command::Restart
            | Command::RecoverNetwork
            | Command::Shutdown
            | Command::Quit
            | Command::Settings { .. }
            | Command::ConfigSet { .. }
            | Command::Ports { .. }
            | Command::RuleOverrides { .. }
            | Command::RuleChange { .. }
            | Command::SaveRules { .. }
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
    operation.check_protection(&engine.settings)?;
    if changes_view {
        engine.invalidate_views();
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
        Command::RecoverNetwork => engine.recover_network().await?,
        Command::RecoverUnreachable => unreachable!("handled before locking the owner"),
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
        Command::Import { .. } => unreachable!("handled before locking the owner"),
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
        Command::Delay { .. } => unreachable!("handled before locking the owner"),
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
        Command::GeoUpdate => unreachable!("handled before locking the owner"),
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
        Command::UpdateInstall => unreachable!("handled before locking the owner"),
        Command::UpdatePrepareTerminal => unreachable!("handled before locking the owner"),
        Command::UpdateSelected { .. } => unreachable!("handled before locking the owner"),
        Command::UpdateCommitTerminal { directory } => {
            #[cfg(target_os = "linux")]
            {
                crate::update::commit_terminal_install(&directory, &mut engine).await?;
                return Ok(json!({"shutdown":true}));
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = directory;
                anyhow::bail!("当前平台不支持 Linux 终端安装交接");
            }
        }
    }
    Ok(Value::Null)
}

async fn install_update(
    shared: &Arc<Mutex<Engine>>,
    mut disconnected: Option<watch::Receiver<bool>>,
    terminal: bool,
    expected: Option<(String, bool)>,
) -> Result<Value> {
    #[cfg(not(target_os = "linux"))]
    anyhow::ensure!(!terminal, "当前平台不支持 Linux 终端安装交接");
    crate::update::ensure_installable()?;
    let (job, download, prereleases) = Engine::download_app_update_with(
        shared,
        &mut disconnected,
        |port, prereleases, api| async move {
            let available = crate::update::check_with_prereleases(port, prereleases)
                .await?
                .ok_or_else(|| anyhow::anyhow!("当前更新通道没有可安装的版本"))?;
            available.require_selected(expected.as_ref())?;
            crate::update::download(available, port, Some(api), |_| {}).await
        },
    )
    .await?;
    #[cfg(target_os = "linux")]
    if terminal {
        let engine = shared.lock().await;
        job.ensure_current(&disconnected)?;
        engine.settings.require_mutable()?;
        anyhow::ensure!(
            engine.settings.update_prereleases == prereleases,
            "更新通道已改变，请重新检查更新"
        );
        let directory = crate::update::prepare_terminal_install(Arc::new(download), &engine)?;
        return Ok(json!({"installer":directory,"shutdown":false}));
    }
    #[cfg(any(windows, target_os = "macos"))]
    let deferred =
        cfg!(target_os = "macos") || crate::update::supports_deferred_install(download.version());
    #[cfg(any(windows, target_os = "macos"))]
    let (mut engine, session) = if deferred {
        prepare_update_handoff(
            shared,
            &job,
            &disconnected,
            prereleases,
            |parent, gui, data_dir| async move {
                tokio::task::spawn_blocking(move || {
                    crate::update::handoff_owner_deferred_at(
                        Arc::new(download),
                        parent,
                        gui,
                        Some(data_dir),
                    )
                })
                .await?
            },
            |session| session.cancel(),
        )
        .await?
    } else {
        let engine = shared.lock().await;
        job.ensure_current(&disconnected)?;
        engine.settings.require_mutable()?;
        anyhow::ensure!(
            engine.settings.update_prereleases == prereleases,
            "更新通道已改变，请重新检查更新"
        );
        let gui = super::gui_pid(&engine.store)?;
        let parent = gui.unwrap_or_else(std::process::id);
        let data_dir = engine.store.root.clone();
        let session = tokio::task::spawn_blocking(move || {
            crate::update::handoff_owner_at(
                Arc::new(download),
                parent,
                gui.is_some(),
                Some(data_dir),
            )
        })
        .await??;
        (engine, session)
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let (mut engine, session) = {
        let engine = shared.lock().await;
        job.ensure_current(&disconnected)?;
        engine.settings.require_mutable()?;
        anyhow::ensure!(
            engine.settings.update_prereleases == prereleases,
            "更新通道已改变，请重新检查更新"
        );
        let gui = super::gui_pid(&engine.store)?;
        let parent = gui.unwrap_or_else(std::process::id);
        let data_dir = engine.store.root.clone();
        let session = tokio::task::spawn_blocking(move || {
            crate::update::handoff_owner_at(
                Arc::new(download),
                parent,
                gui.is_some(),
                Some(data_dir),
            )
        })
        .await??;
        (engine, session)
    };
    #[cfg(any(windows, target_os = "macos"))]
    let was_running = engine.running();
    if let Err(error) = engine.stop().await {
        session.cancel();
        return Err(error);
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    if let Err(error) = super::request_gui_exit(&engine.store) {
        session.cancel();
        return Err(error);
    }
    #[cfg(any(windows, target_os = "macos"))]
    if let Err(error) = (if deferred { session.commit() } else { Ok(()) })
        .and_then(|()| super::request_gui_exit(&engine.store))
    {
        session.cancel();
        if was_running {
            return match engine.start_with_proxy_mode().await {
                Ok(()) => Err(error.context("更新交接失败，已重新启动内核")),
                Err(restart) => {
                    Err(error.context(format!("更新交接失败，重新启动内核也失败：{restart:#}")))
                }
            };
        }
        return Err(error.context("更新交接失败，已保留旧版本"));
    }
    Ok(json!({"shutdown":true,"installer":session.directory}))
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
