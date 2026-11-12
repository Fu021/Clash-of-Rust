use serde_json::{Value, json};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(super) struct Diagnostics {
    started: Instant,
    started_unix_ms: u128,
    timeout: Duration,
    attempts: u32,
    state: &'static str,
    first_error: Option<String>,
    last_error: Option<String>,
    completed_ms: Option<u128>,
}

impl Diagnostics {
    pub(super) fn new(timeout: Duration) -> Self {
        Self {
            started: Instant::now(),
            started_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            timeout,
            attempts: 0,
            state: "waiting",
            first_error: None,
            last_error: None,
            completed_ms: None,
        }
    }

    pub(super) fn attempt(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
        self.state = "initializing";
    }

    pub(super) fn success(&mut self) {
        self.state = "created";
        self.completed_ms = Some(self.started.elapsed().as_millis());
    }

    pub(super) fn failure(&mut self, error: &str, terminal: bool) {
        let error: String = error.chars().take(512).collect();
        if self.first_error.is_none() {
            self.first_error = Some(error.clone());
        }
        self.last_error = Some(error);
        self.state = if terminal { "failed" } else { "retrying" };
        if terminal {
            self.completed_ms = Some(self.started.elapsed().as_millis());
        }
    }

    pub(super) fn snapshot(&self) -> Value {
        json!({
            "state":self.state,"attempts":self.attempts,
            "started_unix_ms":self.started_unix_ms,"timeout_ms":self.timeout.as_millis(),
            "elapsed_ms":self.completed_ms.unwrap_or_else(||self.started.elapsed().as_millis()),
            "first_error":self.first_error,"last_error":self.last_error,
            "complete":self.completed_ms.is_some(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recovered_tray_keeps_retry_evidence_and_freezes_its_completion_time() {
        let mut report = Diagnostics::new(Duration::from_secs(120));
        report.attempt();
        report.failure("host unavailable", false);
        report.attempt();
        report.success();
        let snapshot = report.snapshot();
        assert_eq!(snapshot["state"], "created");
        assert_eq!(snapshot["attempts"], 2);
        assert_eq!(snapshot["first_error"], "host unavailable");
        assert_eq!(snapshot["complete"], true);
        assert_eq!(snapshot["elapsed_ms"], report.snapshot()["elapsed_ms"]);
    }
}
