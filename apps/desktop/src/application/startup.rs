use serde_json::{Value, json};
use std::{collections::VecDeque, path::PathBuf, time::Instant};

const EVENT_LIMIT: usize = 96;

pub(super) struct Trace {
    started: Instant,
    identity: Value,
    events: VecDeque<Value>,
    discarded: usize,
    status: Value,
    sender: Option<tokio::sync::watch::Sender<Value>>,
    write_error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl Trace {
    pub(super) fn new(background: bool, update_restart: bool) -> Self {
        Self {
            started: Instant::now(),
            identity: json!({
                "schema":1,"kind":"desktop_startup","session_id":uuid::Uuid::new_v4().to_string(),
                "started_at":clash_client_core::support::timestamp(),
                "pid":std::process::id(),"version":clash_of_rust::VERSION,
                "background_requested":background,"update_restart":update_restart,
                "display_present":std::env::var_os("DISPLAY").is_some(),
                "wayland_display_present":std::env::var_os("WAYLAND_DISPLAY").is_some(),
                "session_bus_address_present":std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some(),
                "xdg_runtime_dir_present":std::env::var_os("XDG_RUNTIME_DIR").is_some(),
                "desktop_environment":desktop_environment(
                    std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref()),
            }),
            events: VecDeque::new(),
            discarded: 0,
            status: Value::Null,
            sender: None,
            write_error: Default::default(),
        }
    }

    pub(super) fn record(&mut self, event: &'static str, detail: Value) {
        if self.events.len() == EVENT_LIMIT {
            self.events.pop_front();
            self.discarded += 1;
        }
        self.events.push_back(json!({
            "at":clash_client_core::support::timestamp(),
            "elapsed_ms":self.started.elapsed().as_millis(),"event":event,"detail":detail,
        }));
        self.publish();
    }

    pub(super) fn status(&mut self, source: &'static str, status: Value) {
        if self.status != status {
            self.status = status.clone();
            self.record("state_changed", json!({"source":source,"state":status}));
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) fn environment(&mut self, report: Value) {
        self.identity["linux_at_startup"] = report;
        self.publish();
    }

    pub(super) fn snapshot(&self) -> Value {
        let mut report = self.identity.clone();
        report["status"] = self.status.clone();
        report["events"] = json!(self.events);
        report["discarded_events"] = json!(self.discarded);
        report["persistence_error"] = json!(*self.write_error.lock().unwrap());
        report
    }

    fn publish(&self) {
        if let Some(sender) = &self.sender {
            sender.send_replace(self.snapshot());
        }
    }

    pub(super) fn persist(&mut self, root: PathBuf) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (sender, mut receiver) = tokio::sync::watch::channel(self.snapshot());
        self.sender = Some(sender);
        let write_error = self.write_error.clone();
        runtime.spawn(async move {
            let mut first = true;
            loop {
                let report = receiver.borrow_and_update().clone();
                let root = root.clone();
                let result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                    let current = root.join("desktop-startup.json");
                    if first && let Some(bytes) = previous_record(&current) {
                        clash_of_rust::config::atomic_write(
                            &root.join("desktop-startup.previous.json"),
                            &bytes,
                        )?;
                    }
                    clash_of_rust::config::atomic_write(
                        &current,
                        &serde_json::to_vec_pretty(&report)?,
                    )
                })
                .await;
                *write_error.lock().unwrap() = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(_)) => Some("startup_record_write_failed".into()),
                    Err(_) => Some("startup_record_worker_failed".into()),
                };
                first = false;
                if receiver.changed().await.is_err() {
                    break;
                }
            }
        });
    }
}

fn desktop_environment(value: Option<&str>) -> &'static str {
    for name in ["COSMIC", "GNOME", "KDE", "XFCE", "LXQT", "MATE", "CINNAMON"] {
        if value.is_some_and(|value| {
            value
                .split(':')
                .any(|component| component.trim().eq_ignore_ascii_case(name))
        }) {
            return name;
        }
    }
    "unknown"
}

fn previous_record(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    if !std::fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(256 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 256 * 1024 {
        return None;
    }
    let report: Value = serde_json::from_slice(&bytes).ok()?;
    (report["schema"] == 1 && report["kind"] == "desktop_startup").then_some(bytes)
}

impl super::App {
    pub(super) fn trace_startup(&mut self, source: &'static str) {
        self.startup_trace.status(
            source,
            json!({
                "window_visible":self.visible,"window_opening":self.window_opening,
                "core_running":self.snapshot.running,"core_ready":self.engine.is_some(),
                "tray_state":if self.tray.is_some() {"created"}
                    else if self.tray_startup.is_some() {"initializing"}
                    else if self.tray_failure.is_some() {"failed"} else {"not_started"},
                "exiting":self.exiting,
                "tray_initialization":self.tray_startup_report,
                "tray_error":self.tray_failure,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_identity_uses_known_labels_without_exporting_arbitrary_environment_values() {
        for (value, expected) in [
            (Some("pop:COSMIC"), "COSMIC"),
            (Some("COSMIC"), "COSMIC"),
            (Some("ubuntu:GNOME"), "GNOME"),
            (Some("plasma:KDE"), "KDE"),
            (Some("private-session-token"), "unknown"),
            (None, "unknown"),
        ] {
            assert_eq!(desktop_environment(value), expected);
        }
    }

    #[test]
    fn repeated_state_polls_do_not_fill_the_trace_and_explicit_reopen_reasons_survive() {
        let mut trace = Trace::new(true, false);
        for _ in 0..1000 {
            trace.status("tick", json!({"window_visible":false}));
        }
        assert_eq!(trace.snapshot()["events"].as_array().unwrap().len(), 1);
        trace.record("show_requested", json!({"source":"tray_timeout"}));
        trace.status("show_window", json!({"window_visible":true}));
        let report = trace.snapshot();
        assert_eq!(report["background_requested"], true);
        assert_eq!(report["events"][1]["detail"]["source"], "tray_timeout");
        assert_eq!(report["status"]["window_visible"], true);
        assert!(report.get("session_bus_address").is_none());
    }

    #[test]
    fn startup_history_is_bounded_and_reports_discarded_events() {
        let mut trace = Trace::new(false, false);
        for index in 0..200 {
            trace.record("show_requested", json!({"sequence":index}));
        }
        let report = trace.snapshot();
        assert_eq!(report["events"].as_array().unwrap().len(), EVENT_LIMIT);
        assert_eq!(report["discarded_events"], 200 - EVENT_LIMIT);
        assert_eq!(report["events"][EVENT_LIMIT - 1]["detail"]["sequence"], 199);
    }

    #[tokio::test]
    async fn startup_evidence_is_written_without_an_export_and_keeps_the_previous_session() {
        let root = tempfile::tempdir().unwrap();
        let previous = json!({"schema":1,"kind":"desktop_startup","session_id":"previous"});
        std::fs::write(
            root.path().join("desktop-startup.json"),
            previous.to_string(),
        )
        .unwrap();
        let mut trace = Trace::new(true, false);
        trace.record("process_start", json!({"window_visible":false}));
        trace.persist(root.path().to_owned());
        trace.record("show_requested", json!({"source":"tray_failed"}));
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let report = previous_record(&root.path().join("desktop-startup.json"))
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            if report
                .as_ref()
                .is_some_and(|report| report["events"][1]["detail"]["source"] == "tray_failed")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "startup evidence was not persisted"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let archived: Value = serde_json::from_slice(
            &std::fs::read(root.path().join("desktop-startup.previous.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(archived, previous);
    }
}
