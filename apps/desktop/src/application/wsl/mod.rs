//! Native WSL control surface. Effects run on the worker, never the UI thread.
use super::*;
use clash_client_core::wsl::{self as contract, Backend, Operation, Report, WslSettings};
use clash_client_ui::{DetailItem, UiData, WslDistributionItem, models::ViewModels};

#[cfg(any(windows, test))]
mod payload;
mod runner;

#[derive(Debug, Clone)]
pub(super) enum Event {
    Input(String, String),
    Action(String, String),
    RestoreConfirmed(WslSettings),
    Finished(Operation, Box<Result<Report, Failure>>, String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App::initial_state(Err(anyhow::anyhow!("isolated WSL state test")), None)
    }

    fn request(action: contract::Action) -> contract::Request {
        contract::Request {
            action,
            execute: action.mutating(),
            backend: Backend::WireGuard,
            profile_path: r"C:\Private\egress.json".into(),
            ..Default::default()
        }
    }

    fn report(action: contract::Action, outcome: &str) -> Report {
        Report::parse(
            &serde_json::json!({
                "schema_version":1, "operation_id":uuid::Uuid::new_v4().to_string(),
                "generation":1, "action":action, "outcome":outcome,
                "protection":"unknown", "phase":"failed", "backend":"WireGuard",
                "message":"", "distributions":[], "checks":[], "logs":[]
            })
            .to_string(),
        )
        .unwrap()
    }

    #[test]
    fn failed_mutation_report_locks_and_readonly_failures_do_not_unlock() {
        let mut app = app();
        let ticket = app
            .wsl
            .evidence
            .begin(request(contract::Action::Start))
            .unwrap();
        let _ = app.wsl_event(Event::Finished(
            ticket,
            Box::new(Ok(report(contract::Action::Start, "failed"))),
            String::new(),
        ));
        assert!(app.wsl.inspection_required);
        let ticket = app
            .wsl
            .evidence
            .begin(request(contract::Action::Doctor))
            .unwrap();
        let _ = app.wsl_event(Event::Finished(
            ticket,
            Box::new(Err(Failure::Runner(runner::Error::Unavailable))),
            String::new(),
        ));
        assert!(app.wsl.inspection_required);
        let ticket = app
            .wsl
            .evidence
            .begin(request(contract::Action::Status))
            .unwrap();
        let _ = app.wsl_event(Event::Finished(
            ticket,
            Box::new(Ok(report(contract::Action::Status, "succeeded"))),
            String::new(),
        ));
        assert!(
            app.wsl.inspection_required,
            "the synthetic status uses a failed phase and cannot settle recovery"
        );
    }

    #[test]
    fn rejected_mutation_report_locks_without_retaining_evidence() {
        let mut app = app();
        let ticket = app
            .wsl
            .evidence
            .begin(request(contract::Action::Start))
            .unwrap();
        let _ = app.wsl_event(Event::Finished(
            ticket,
            Box::new(Ok(report(contract::Action::Stop, "succeeded"))),
            String::new(),
        ));
        assert!(app.wsl.inspection_required);
        assert!(!app.wsl.evidence.busy());
        assert!(app.wsl.evidence.report().is_none());
    }

    #[test]
    fn expired_mutation_callback_cannot_lock_a_newer_readonly_operation() {
        let mut app = app();
        let old = app
            .wsl
            .evidence
            .begin(request(contract::Action::Start))
            .unwrap();
        app.wsl
            .evidence
            .fail(&old, contract::Error::HelperFailed)
            .unwrap();
        let current = app
            .wsl
            .evidence
            .begin(request(contract::Action::Status))
            .unwrap();
        let _ = app.wsl_event(Event::Finished(
            old,
            Box::new(Ok(report(contract::Action::Start, "failed"))),
            String::new(),
        ));
        assert!(!app.wsl.inspection_required);
        assert!(app.wsl.evidence.busy());
        app.wsl
            .evidence
            .fail(&current, contract::Error::HelperFailed)
            .unwrap();
    }

    #[test]
    fn late_core_initialization_preserves_an_active_wsl_ticket_or_edited_draft() {
        let mut state = State::new(WslSettings::default());
        let saved = WslSettings {
            backend: Backend::WireGuard,
            ..Default::default()
        };
        state.load_preferences(saved.clone());
        assert_eq!(state.draft.backend, Backend::WireGuard);
        let ticket = state
            .evidence
            .begin(request(contract::Action::Start))
            .unwrap();
        state.load_preferences(WslSettings::default());
        assert!(state.evidence.busy());
        assert_eq!(state.draft.backend, Backend::WireGuard);
        state
            .evidence
            .fail(&ticket, contract::Error::HelperFailed)
            .unwrap();
        state.edit("profile-path", r"C:\Private\edited.json".into());
        state.load_preferences(saved);
        assert_eq!(state.draft.profile_path, r"C:\Private\edited.json");
    }

    #[test]
    fn fresh_status_report_is_the_explicit_recovery_event() {
        let mut app = app();
        app.wsl.inspection_required = true;
        let ticket = app
            .wsl
            .evidence
            .begin(request(contract::Action::Status))
            .unwrap();
        let raw = serde_json::json!({
            "schema_version":1, "operation_id":uuid::Uuid::new_v4().to_string(),
            "generation":1, "action":"Status", "outcome":"succeeded",
            "protection":"unknown", "phase":"installed_closed", "backend":"WireGuard",
            "message":"inspected", "distributions":[], "checks":[], "logs":[]
        });
        let _ = app.wsl_event(Event::Finished(
            ticket,
            Box::new(Ok(Report::parse(&raw.to_string()).unwrap())),
            String::new(),
        ));
        assert!(!app.wsl.inspection_required);
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Failure {
    Runner(runner::Error),
    Contract(contract::Error),
    Preferences,
}
impl Failure {
    fn message(self) -> String {
        match self {
            Self::Runner(error) => error.message().into(),
            Self::Contract(error) => error.to_string(),
            Self::Preferences => "无法保存 WSL 连接偏好；管理操作未执行。".into(),
        }
    }
}

pub(super) struct State {
    pub(super) draft: WslSettings,
    pub(super) evidence: contract::State,
    message: String,
    administrator_command: String,
    // A timeout cannot establish that Linux descendants have exited. Permit
    // only read-only inspection until a new broker report has been accepted.
    inspection_required: bool,
}
impl State {
    pub(super) fn new(draft: WslSettings) -> Self {
        Self {
            draft,
            evidence: contract::State::default(),
            message: "尚未检查；完整冷启动验收未完成，当前不声明已建立保护。".into(),
            administrator_command: String::new(),
            inspection_required: false,
        }
    }

    pub(super) fn load_preferences(&mut self, draft: WslSettings) {
        if self.evidence.generation == 0 && !self.evidence.busy() {
            self.draft = draft;
        }
    }

    fn edit(&mut self, key: &str, value: String) {
        if self.evidence.busy() {
            return;
        }
        self.administrator_command.clear();
        if let Err(error) = self.evidence.invalidate() {
            self.message = error.to_string();
            return;
        }
        if value.len() > 1024 || value.chars().any(char::is_control) {
            self.message = "输入无效；旧代次证据已废弃。".into();
            return;
        }
        match key {
            "backend" => match value.as_str() {
                "Tailscale" => self.draft.backend = Backend::Tailscale,
                "WireGuard" => self.draft.backend = Backend::WireGuard,
                _ => {
                    self.message = "后端无效；旧代次证据已废弃。".into();
                    return;
                }
            },
            "endpoint-ipv4" => self.draft.endpoint_ipv4 = value,
            "endpoint-port" => self.draft.endpoint_port = value,
            "distro" => self.draft.distro = value,
            "profile-path" => self.draft.profile_path = value,
            _ => {
                self.message = "未知配置字段；旧代次证据已废弃。".into();
                return;
            }
        }
        self.message = "配置已编辑；旧代次证据已废弃，需重新检查。".into();
    }
}

impl App {
    pub(super) fn wsl_event(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Finished(ticket, result, command) => {
                let accepted = match *result {
                    Ok(report) => {
                        let failed = report.outcome == contract::Outcome::Failed;
                        let accepted = self.wsl.evidence.apply(&ticket, report);
                        if ticket.request.execute
                            && (failed || accepted.is_err())
                            && accepted != Err(contract::Error::StaleOperation)
                        {
                            self.wsl.inspection_required = true;
                        }
                        accepted
                    }
                    Err(error) => {
                        let accepted = self
                            .wsl
                            .evidence
                            .fail(&ticket, contract::Error::HelperFailed);
                        if accepted.is_ok() {
                            self.wsl.message = error.message();
                            self.wsl.inspection_required |= ticket.request.execute
                                && matches!(
                                    error,
                                    Failure::Runner(runner::Error::Timeout | runner::Error::Output)
                                        | Failure::Contract(_)
                                );
                        }
                        accepted
                    }
                };
                // Stale callbacks never overwrite a newer operation's status,
                // administrator instructions, busy state, or list generation.
                if let Err(error) = accepted {
                    if error != contract::Error::StaleOperation {
                        self.wsl.message = error.to_string();
                    }
                    return Task::none();
                }
                self.wsl.administrator_command = command;
                if let Some(report) = self.wsl.evidence.report() {
                    self.wsl.message.clone_from(&report.message);
                    // Status is the explicit read-only recovery event. It may
                    // clear the interlock only after a fresh, parsed broker
                    // report says that no mutation is active.
                    if self.wsl.evidence.inspection_is_settled() {
                        self.wsl.inspection_required = false;
                    }
                }
            }
            Event::Input(key, value) => {
                if !self.exiting && self.confirmation.is_none() {
                    self.wsl.edit(&key, value);
                }
            }
            Event::Action(action, mode) => {
                if self.exiting || self.wsl.evidence.busy() || self.confirmation.is_some() {
                    return Task::none();
                }
                let action = match action.as_str() {
                    "Doctor" => contract::Action::Doctor,
                    "Status" => contract::Action::Status,
                    "Install" => contract::Action::Install,
                    "Start" => contract::Action::Start,
                    "Stop" => contract::Action::Stop,
                    "Test" => contract::Action::Test,
                    "Rollback" => contract::Action::Rollback,
                    "Uninstall" => contract::Action::Uninstall,
                    _ => {
                        let _ = self.wsl.evidence.invalidate();
                        self.wsl.message = "未知 WSL 操作；未执行。".into();
                        return Task::none();
                    }
                };
                if mode == "restore-confirm" && action == contract::Action::Rollback {
                    return self.ask(
                        "恢复 WSL 原网络并撤销本轮保护？",
                        "仅在全部 WSL 发行版已手动停止时恢复本轮快照。恢复后 WSL 可能恢复直接联网；Docker Desktop 数据将保留。取消将保留保护。".into(),
                        "恢复原网络",
                        desktop_view::Pending::WslRestore(self.wsl.draft.clone()),
                    );
                }
                if !matches!(mode.as_str(), "preview" | "execute") {
                    let _ = self.wsl.evidence.invalidate();
                    self.wsl.message = "执行模式无效；未执行。".into();
                    return Task::none();
                }
                let request = self.wsl.draft.request(action, mode == "execute");
                return self.start_wsl_request(request);
            }
            Event::RestoreConfirmed(draft) => {
                if self.exiting || self.wsl.evidence.busy() || draft != self.wsl.draft {
                    return Task::none();
                }
                let mut request = draft.request(contract::Action::Rollback, true);
                request.restore_original_network = true;
                return self.start_wsl_request(request);
            }
        }
        Task::none()
    }

    fn start_wsl_request(&mut self, request: contract::Request) -> Task<Message> {
        if !cfg!(windows) {
            let _ = self.wsl.evidence.invalidate();
            self.wsl.message = runner::Error::Unsupported.message().into();
            return Task::none();
        }
        if self.wsl.inspection_required
            && !matches!(
                request.action,
                contract::Action::Doctor | contract::Action::Status
            )
        {
            self.wsl.message = "上次管理进程状态未知；本会话仅允许 Doctor/状态。请在管理员终端核实任务已退出并定向恢复后重新打开客户端。".into();
            return Task::none();
        }
        let ticket = match self.wsl.evidence.begin(&request) {
            Ok(ticket) => ticket,
            Err(error) => {
                self.wsl.message = error.to_string();
                return Task::none();
            }
        };
        self.wsl.administrator_command.clear();
        self.wsl.message = "正在执行明确请求的操作；完整冷启动验收仍未完成。".into();
        let settings = self.wsl.draft.clone();
        let engine = self.engine.clone();
        Task::perform(
            async move {
                let arguments = match request.arguments() {
                    Ok(arguments) => arguments
                        .into_iter()
                        .map(std::ffi::OsString::from)
                        .collect::<Vec<_>>(),
                    Err(error) => return (ticket, Err(Failure::Contract(error)), String::new()),
                };
                // Persist preferences on the worker under the existing engine lock;
                // preserve all unrelated settings and never regenerate network config.
                if let Some(engine) = engine {
                    let mut engine = engine.lock().await;
                    let mut saved = engine.settings.clone();
                    saved.wsl = settings;
                    if engine.store.save_settings(&saved).is_err() {
                        return (ticket, Err(Failure::Preferences), String::new());
                    }
                    engine.settings = saved;
                }
                let command = if request.execute {
                    runner::administrator_command(arguments.clone()).await
                } else {
                    String::new()
                };
                let result = runner::run(arguments)
                    .await
                    .map_err(Failure::Runner)
                    .and_then(|raw| Report::parse(&raw).map_err(Failure::Contract));
                (ticket, result, command)
            },
            |(ticket, result, command)| {
                Message::Wsl(Event::Finished(ticket, Box::new(result), command))
            },
        )
    }

    pub(super) fn present_wsl(&self, data: &UiData<'_>, models: &mut ViewModels) {
        let state = &self.wsl;
        data.set_wsl_supported(cfg!(windows));
        data.set_wsl_busy(state.evidence.busy());
        data.set_wsl_backend(state.draft.backend.as_str().into());
        data.set_wsl_endpoint_ipv4(state.draft.endpoint_ipv4.as_str().into());
        data.set_wsl_endpoint_port(state.draft.endpoint_port.as_str().into());
        data.set_wsl_distro(state.draft.distro.as_str().into());
        data.set_wsl_profile_path(state.draft.profile_path.as_str().into());
        data.set_wsl_message(state.message.as_str().into());
        data.set_wsl_admin_command(state.administrator_command.as_str().into());
        data.set_wsl_generation(
            state
                .evidence
                .service_generation()
                .unwrap_or(0)
                .to_string()
                .into(),
        );
        data.set_wsl_protection(state.evidence.protection().as_str().into());
        let report = state.evidence.report();
        data.set_wsl_status(
            if state.evidence.last_error.is_some() {
                "failed"
            } else {
                report.map_or("not_run", |r| r.outcome.as_str())
            }
            .into(),
        );
        data.set_wsl_phase(
            if state.evidence.busy() {
                "running"
            } else {
                report.map_or("idle", |r| r.phase.as_str())
            }
            .into(),
        );
        let connected = report.is_some_and(|report| match report.backend {
            Backend::Tailscale => report.tailscale_connected(),
            Backend::WireGuard => report.check_passed("wireguard_connected"),
        });
        data.set_wsl_tunnel(
            if connected {
                "已验证连接"
            } else {
                "unknown · 未验证"
            }
            .into(),
        );
        data.set_wsl_exit(
            if state.evidence.german_exit_verified() {
                "DE · IPv4 已验证"
            } else {
                "unknown · 德国 IPv4 未验证"
            }
            .into(),
        );
        data.set_wsl_functional(
            if state.evidence.functional_ready() {
                "verified"
            } else {
                "Blocked · 完整验收未完成"
            }
            .into(),
        );
        if !self.visible || self.page != Page::Wsl {
            models.clear_wsl_results();
            return;
        }
        let distributions = report.map_or_else(Vec::new, |report| {
            report
                .distributions
                .iter()
                .map(|d| WslDistributionItem {
                    name: d.name.as_str().into(),
                    state: d.state.as_str().into(),
                    coverage: d.coverage.as_str().into(),
                })
                .collect()
        });
        let checks = report.map_or_else(Vec::new, |report| {
            report
                .checks
                .iter()
                .map(|check| DetailItem {
                    label: check.id.as_str().into(),
                    value: format!("{} · {}", check.result, check.detail).into(),
                })
                .collect()
        });
        let logs = report.map_or_else(Vec::new, |report| {
            report
                .logs
                .iter()
                .map(|line| line.as_str().into())
                .collect()
        });
        // Model generation uses the local epoch: input edits clear evidence even
        // while the broker's independent millisecond generation is unchanged.
        let _ = models.set_wsl_results(state.evidence.generation, distributions, checks, logs);
    }
}
