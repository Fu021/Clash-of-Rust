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

    pub(super) fn diagnostic_context(&self, context: &str) -> serde_json::Value {
        let language = Language::SimplifiedChinese;
        let languages = [language, Language::English];
        let notice = if self.error { self.notice.as_str() } else { "" };
        let followups = if self.error && self.notice_visible {
            self.notice_followups.as_slice()
        } else {
            &[]
        };
        let claude_status = self.claude.status.display(language);
        let settings = Some(&self.settings);
        let network_alert = self.network_watch.notice(settings);
        let error = if self.error
            && !context.is_empty()
            && languages
                .into_iter()
                .any(|language| self.display_notice_in(language) == context)
        {
            if notice.is_empty() && self.notice_copy_failed {
                desktop_view::CopyFeedback::Failed.text(language)
            } else {
                notice
            }
        } else if self.claude.error
            && languages
                .into_iter()
                .any(|language| self.claude.status.display(language) == context)
        {
            claude_status.as_ref()
        } else if !network_alert.is_empty()
            && (context == network_alert
                || context == self.network_watch.notice_in(settings, Language::English))
        {
            network_alert.as_str()
        } else {
            [
                Some(self.fake_ip_error.as_str()),
                self.profile_editor
                    .as_ref()
                    .map(|editor| editor.error.as_str()),
                self.rule_state
                    .editor
                    .as_ref()
                    .map(|editor| editor.error.as_str()),
            ]
            .into_iter()
            .flatten()
            .filter(|error| !error.is_empty())
            .find(|error| {
                languages
                    .into_iter()
                    .any(|language| notice::localize_notice(error, true, language) == context)
            })
            .unwrap_or(context)
        };
        let network_tests = self
            .network_result
            .as_ref()
            .map(|result| result.lines(language))
            .unwrap_or_default();
        json!({
            "error":error,"notice":notice,"notice_followups":followups,"core_failure":self.core_failure,
            "notice_level":if !self.error {"status"} else if self.notice_critical {"critical"} else {"recoverable"},
            "notice_visible":self.notice_visible,
            "update":{"checking":self.updates.checking,"status":self.updates.status,
                "failure":self.updates.failure,"pending_failure":self.pending_update_failure,
                "outcome":self.updates.outcome,
                "install_pending":self.updates.install_pending,
                "installer_active":self.updates.installing.is_some()},
            "page":format!("{:?}",self.page),"running":self.snapshot.running,
            "proxy":format!("{:?}",self.snapshot.proxy_mode()),
            "network_alert":network_alert,"network_tests":network_tests,
            "network_tool":network_tools::Tool::at(self.network_tool).ok().map(|tool| tool.id()),
            "network_test_codes":self.network_codes,
            "site_results":clash_client_core::support::ip_results(&self.site_results),
            "claude_status":claude_status.as_ref(),
            "geo_status":self.geo_status,
            "events":self.issue_events,
            "desktop_startup":self.startup_diagnostics(),
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
        let context = self.diagnostic_context(context);
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
        app.prepare_initialization(None);
        app.dismiss_notice();
        let startup = app.startup_diagnostics();
        assert_eq!(startup["tray_state"], "failed");
        assert_eq!(startup["tray_error"], "当前桌面没有可用托盘宿主");
        assert!(!app.notice_visible);
        assert!(app.core_failure.is_none());
    }

    #[test]
    fn exported_context_is_independent_of_the_notice_and_report_languages() {
        let snapshot = |language| {
            let mut app = App::initial_state(Err(anyhow::anyhow!("initializing")), None);
            app.dismiss_notice();
            app.presentation_language = language;
            let _ = app.claude_event(claude_view::Event::Cancel);
            app.network_tool = network_tools::Tool::Dns.index();
            app.set_network_result(NetworkResult::Report(network_tools::Report {
                schema: 1,
                tool: network_tools::Tool::Dns,
                target: "https://example.test".into(),
                elapsed_ms: 42,
                status: "partial",
                entries: vec![network_tools::Entry {
                    label: "系统 DNS".into(),
                    detail: "外部解析器诊断原文".into(),
                    success: Some(false),
                    code: "dns_failed",
                    detail_english: Some("display-only detail".into()),
                }],
            }));
            app.show_critical("系统通知失败：无法连接桌面通知会话: D-Bus: 原文".into());
            let _ = app.update(Message::DiagnosticsExported(Ok((
                "/tmp/诊断.json".into(),
                None,
            ))));
            let displayed = app.display_notice().into_owned();
            assert_eq!(
                app.test_results[0],
                language.text("检查状态 · 部分完成 · 42 ms", "Status · Partial · 42 ms")
            );
            assert!(displayed.contains(language.text("诊断已导出", "Diagnostics exported")));
            app.presentation_language = if language == Language::English {
                Language::SimplifiedChinese
            } else {
                Language::English
            };
            let context = app.diagnostic_context(&displayed);
            assert_eq!(context["error"], context["notice"]);
            assert_eq!(
                context["notice"],
                "系统通知失败：无法连接桌面通知会话: D-Bus: 原文"
            );
            assert_eq!(context["notice_level"], "critical");
            assert_eq!(context["network_tool"], "dns");
            assert_eq!(context["network_test_codes"], json!(["dns_failed"]));
            assert_eq!(context["network_tests"][2], "系统 DNS · 外部解析器诊断原文");
            assert_eq!(context["claude_status"], "已停止检测，已获得的报告保留。");
            json!({
                "error":context["error"],"notice":context["notice"],"notice_level":context["notice_level"],
                "network_tool":context["network_tool"],"network_tests":context["network_tests"],
                "network_test_codes":context["network_test_codes"],"claude_status":context["claude_status"],
            })
        };
        assert_eq!(
            snapshot(Language::English),
            snapshot(Language::SimplifiedChinese)
        );
    }

    #[test]
    fn repeated_diagnostics_keep_visible_notice_followups() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("initializing")), None);
        app.dismiss_notice();
        app.presentation_language = Language::English;
        let primary = "界面与后台版本不一致，请重新安装完整客户端。";
        app.show_critical(primary.into());
        let first = app.diagnostic_context(&app.display_notice());

        let _ = app.update(Message::DiagnosticsExported(Ok((
            "/tmp/诊断.json".into(),
            None,
        ))));
        let _ = app.update(Message::IssueOpened(Err(
            "外部浏览器故障: diagnostic-test-credential".into(),
        )));
        let second = app.diagnostic_context(&app.display_notice());
        assert_eq!(second["error"], primary);
        assert_eq!(second["notice"], primary);
        assert_eq!(second["notice_level"], "critical");
        assert_eq!(
            second["notice_followups"],
            json!([
                "诊断已导出：/tmp/诊断.json",
                "无法打开问题反馈页面：外部浏览器故障: diagnostic-test-credential"
            ])
        );
        assert_eq!(first["notice_followups"], json!([]));

        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("settings.json"),
            br#"{"secret":"diagnostic-test-credential"}"#,
        )
        .unwrap();
        let report = clash_client_core::support::issue_report(root.path(), second, vec![]);
        assert_eq!(
            report["data"]["context"]["notice_followups"][1],
            "无法打开问题反馈页面：外部浏览器故障: [已隐藏]"
        );
        assert!(!report.to_string().contains("diagnostic-test-credential"));

        app.dismiss_notice();
        let dismissed = app.diagnostic_context("");
        assert_eq!(dismissed["notice_followups"], json!([]));
        assert!(
            dismissed["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["error"].as_str().unwrap().contains("外部浏览器故障"))
        );
    }

    #[test]
    fn desktop_validation_and_update_errors_are_exported_without_display_translation() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("initializing")), None);
        app.dismiss_notice();
        app.presentation_language = Language::English;
        app.mixed_port = "0".into();
        let _ = app.update(Message::SavePorts);
        let display = app.display_notice();
        assert_eq!(display, "Port must be an integer from 1 to 65535");
        let context = app.diagnostic_context(&display);
        assert_eq!(context["error"], "端口必须为 1–65535 的整数");
        assert_eq!(context["events"][0]["error"], context["error"]);
        assert_eq!(
            app.diagnostic_context("Enable DNS override first")["error"],
            "Enable DNS override first"
        );

        app.settings.dns_override = false;
        let _ = app.update(Message::FakeIpAdd);
        assert_eq!(
            app.diagnostic_context("Enable DNS override first")["error"],
            "请先开启 DNS 覆盖"
        );

        app.update_outcome("更新失败：安装助手无法启动，请重试".into(), true);
        let display = app.display_notice();
        assert_eq!(
            display,
            "Update failed: Could not start the installation helper; try again"
        );
        let context = app.diagnostic_context(&display);
        assert_eq!(context["error"], "更新失败：安装助手无法启动，请重试");
        assert_eq!(context["update"]["failure"], context["error"]);
        assert_eq!(context["update"]["pending_failure"], context["error"]);
    }
}
