use super::*;
use serde_json::json;

impl App {
    fn startup_diagnostics(&self) -> serde_json::Value {
        let session_type = std::env::var("XDG_SESSION_TYPE").ok();
        json!({
            "background_requested":background_start(),
            "update_restart":update_restart(),
            "window_visible":self.visible,
            "tray_state":if self.tray.is_some() {"created"}
                else if self.tray_startup.is_some() {"initializing"}
                else if self.tray_failure.is_some() {"failed"}
                else {"not_started"},
            "tray_error":self.tray_failure,
            "tray_initialization":self.tray_startup.as_ref()
                .map(tray::Startup::diagnostics).unwrap_or_else(||self.tray_startup_report.clone()),
            "history":self.startup_trace.snapshot(),
            "session_type":match session_type.as_deref() {
                Some("wayland") => "wayland",
                Some("x11") => "x11",
                Some("tty") => "tty",
                _ => "unknown",
            },
        })
    }

    pub(super) fn export_diagnostics(&mut self, context: &str) -> Task<Message> {
        self.prepare_diagnostics(context, false)
    }

    pub(super) fn prepare_diagnostics(&mut self, context: &str, reporting: bool) -> Task<Message> {
        if self.diagnosing {
            return Task::none();
        }
        self.diagnosing = true;
        let engine = self.engine.clone();
        let context = json!({
            "error":context,"notice":self.notice,"core_failure":self.core_failure,
            "notice_level":if !self.error {"status"} else if self.notice_critical {"critical"} else {"recoverable"},
            "notice_visible":self.notice_visible,
            "update":{"checking":self.updates.checking,"status":self.updates.status,
                "failure":self.updates.failure,"outcome":self.updates.outcome,
                "install_pending":self.updates.install_pending,
                "installer_active":self.updates.installing.is_some()},
            "page":format!("{:?}",self.page),"running":self.snapshot.running,
            "proxy":format!("{:?}",self.snapshot.proxy_mode()),
            "network_alert":self.network_watch.notice(Some(&self.settings)),"network_tests":self.test_results,
            "network_tool":network_tools::Tool::at(self.network_tool).ok().map(|tool| tool.id()),
            "network_test_codes":self.network_codes,
            "site_results":clash_client_core::support::ip_results(&self.site_results),
            "claude_status":self.claude.status,
            "geo_status":self.geo_status,
            "events":self.issue_events,
            "desktop_startup":self.startup_diagnostics(),
        });
        Task::perform(
            async move {
                let routes = platform::host_network::diagnose(
                    platform::host_network::HolderScope::CurrentUser,
                )
                .await;
                let engine = if let Some(engine) = engine {
                    tokio::time::timeout(Duration::from_secs(5), engine.lock_owned())
                        .await
                        .ok()
                } else {
                    None
                };
                tokio::task::spawn_blocking(move || -> anyhow::Result<(String, Option<String>)> {
                    let root = match &engine {
                        Some(engine) => engine.store.root.clone(),
                        None => Store::default_root()?,
                    };
                    let logs = match &engine {
                        Some(engine) => engine.log_snapshot().map_err(|error| format!("{error:#}")),
                        None => Err("后台未就绪或读取超时".into()),
                    };
                    let measurements = engine
                        .as_ref()
                        .map(|engine| engine.api.measurement_diagnostics())
                        .unwrap_or_default();
                    drop(engine);
                    let mut context = context;
                    context["desktop_startup"]["autostart"] = match platform::autostart_enabled() {
                        Ok(enabled) => json!({"state":"collected","enabled":enabled}),
                        Err(error) => json!({"state":"unavailable","error":format!("{error:#}")}),
                    };
                    #[cfg(target_os = "linux")]
                    {
                        context["desktop_startup"]["linux"] =
                            platform::desktop_startup_diagnostics();
                    }
                    context["measurements"] = json!(measurements);
                    if let Err(error) = &logs {
                        context["log_error"] = json!(error);
                    }
                    let mut report = clash_client_core::support::issue_report(
                        &root,
                        context,
                        logs.unwrap_or_default(),
                    );
                    report["data"]["host_routes"] = routes;
                    let path = root.join("diagnostics").join(format!(
                        "cor-diagnose-{}.json",
                        chrono::Utc::now().format("%Y%m%d-%H%M%S%.3f")
                    ));
                    let bytes = serde_json::to_vec_pretty(&report)?;
                    clash_of_rust::config::atomic_write(&path, &bytes)?;
                    let url =
                        reporting.then(|| clash_client_core::support::issue_draft_url(&report));
                    Ok((path.display().to_string(), url))
                })
                .await
                .map_err(|error| format!("{error:#}"))?
                .map_err(|error| format!("{error:#}"))
            },
            Message::DiagnosticsExported,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dismissing_a_tray_warning_keeps_the_failure_in_diagnostics() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("initializing")), None);
        app.tray_failure = Some("当前桌面没有可用托盘宿主".into());
        app.prepare_initialization();
        app.dismiss_notice();
        let startup = app.startup_diagnostics();
        assert_eq!(startup["tray_state"], "failed");
        assert_eq!(startup["tray_error"], "当前桌面没有可用托盘宿主");
        assert!(!app.notice_visible);
        assert!(app.core_failure.is_none());
    }
}
