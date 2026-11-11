use super::*;
use serde_json::json;

impl App {
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
            "page":format!("{:?}",self.page),"running":self.snapshot.running,
            "proxy":format!("{:?}",self.snapshot.proxy_mode()),
            "network_alert":self.network_watch.notice(),"network_tests":self.test_results,
            "site_results":clash_client_core::support::ip_results(&self.site_results),
            "claude_status":self.claude.status,
            "geo_status":self.geo_status,
            "events":self.issue_events,
        });
        Task::perform(
            async move {
                let routes = platform::host_network::observe().await;
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
