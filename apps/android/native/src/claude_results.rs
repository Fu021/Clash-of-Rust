use crate::messages;
use clash_mobile_runtime::core::{claude_check::Report, config::Language};
use std::sync::{
    Mutex,
    atomic::{AtomicI64, Ordering},
};
use tokio::sync::oneshot;

pub(crate) struct BrowserLaunches {
    sequence: AtomicI64,
    failed: Mutex<Option<(i64, oneshot::Sender<()>)>>,
}

impl BrowserLaunches {
    pub(crate) const fn new() -> Self {
        Self {
            sequence: AtomicI64::new(0),
            failed: Mutex::new(None),
        }
    }

    pub(crate) fn register(&self) -> BrowserLaunch<'_> {
        let ticket = self
            .sequence
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        let (send, failure) = oneshot::channel();
        *self
            .failed
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some((ticket, send));
        BrowserLaunch {
            launches: self,
            ticket,
            failure,
        }
    }

    pub(crate) fn open_failed(&self, ticket: i64) {
        if let Some(failed) = self.take(ticket) {
            let _ = failed.send(());
        }
    }

    fn take(&self, ticket: i64) -> Option<oneshot::Sender<()>> {
        let mut pending = self
            .failed
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if pending
            .as_ref()
            .is_some_and(|(active, _)| *active == ticket)
        {
            pending.take().map(|(_, send)| send)
        } else {
            None
        }
    }
}

pub(crate) struct BrowserLaunch<'a> {
    launches: &'a BrowserLaunches,
    pub(crate) ticket: i64,
    pub(crate) failure: oneshot::Receiver<()>,
}

impl Drop for BrowserLaunch<'_> {
    fn drop(&mut self) {
        self.launches.take(self.ticket);
    }
}

#[derive(Default)]
pub(crate) struct Results {
    network: Option<Report>,
    browser: Option<Report>,
    report: Option<Report>,
}

impl Results {
    pub(crate) fn begin(&mut self) {
        self.network = None;
        self.browser = None;
    }

    pub(crate) fn network(&mut self, report: Report) {
        self.network = Some(report);
        self.refresh();
    }

    pub(crate) fn browser(&mut self, report: Report) {
        self.browser = Some(report);
        self.refresh();
    }

    pub(crate) fn report(&self) -> Option<&Report> {
        self.report.as_ref()
    }

    fn refresh(&mut self) {
        self.report = match (&self.network, &self.browser) {
            (Some(network), Some(browser)) => {
                let mut report = network.clone();
                report.signals.extend_from_slice(&browser.signals);
                Some(report)
            }
            (network, browser) => network.as_ref().or(browser.as_ref()).cloned(),
        };
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum BrowserStatus {
    #[default]
    Idle,
    Sampling,
    Complete,
    OpenFailed,
    Unavailable,
    TimedOut,
    Stopped,
}

impl BrowserStatus {
    pub(crate) fn retryable(self) -> bool {
        matches!(
            self,
            Self::OpenFailed | Self::Unavailable | Self::TimedOut | Self::Stopped
        )
    }

    pub(crate) fn render(self, language: Language) -> &'static str {
        match self {
            Self::Idle => "",
            Self::Sampling => messages::claude_browser_sampling(language),
            Self::Complete => messages::claude_browser_complete(language),
            Self::OpenFailed => messages::claude_browser_open_failed(language),
            Self::Unavailable => messages::claude_browser_unavailable(language),
            Self::TimedOut => messages::claude_browser_timed_out(language),
            Self::Stopped => messages::claude_browser_stopped(language),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_mobile_runtime::core::claude_check::{Environment, Group, Signal};

    fn report(environment: Environment, title: &str) -> Report {
        let mut report = Report::new(environment, "fixture");
        report
            .signals
            .push(Signal::unknown(Group::Exit, title, "fixture", "fixture"));
        report
    }

    #[test]
    fn network_is_available_before_browser_and_retries_replace_only_browser_data() {
        let mut results = Results::default();
        results.network(report(Environment::Core, "network"));
        assert_eq!(results.report().unwrap().signals.len(), 1);
        results.browser(report(Environment::Web, "first browser"));
        results.browser(report(Environment::Web, "retry"));
        let report = results.report().unwrap();
        assert_eq!(report.signals.len(), 2);
        assert_eq!(report.signals[0].title, "network");
        assert_eq!(report.signals[1].title, "retry");
    }

    #[test]
    fn a_new_check_preserves_the_previous_display_until_fresh_results_arrive() {
        let mut results = Results::default();
        results.network(report(Environment::Core, "previous network"));
        results.browser(report(Environment::Web, "previous browser"));
        results.begin();
        assert_eq!(results.report().unwrap().signals.len(), 2);
        results.browser(report(Environment::Web, "fresh browser"));
        assert_eq!(results.report().unwrap().signals.len(), 1);
        results.network(report(Environment::Core, "fresh network"));
        let report = results.report().unwrap();
        assert_eq!(report.signals[0].title, "fresh network");
        assert_eq!(report.signals[1].title, "fresh browser");
    }

    #[test]
    fn browser_failures_can_retry_and_follow_the_current_language() {
        assert!(!BrowserStatus::Idle.retryable());
        assert!(!BrowserStatus::Sampling.retryable());
        assert!(!BrowserStatus::Complete.retryable());
        for status in [
            BrowserStatus::OpenFailed,
            BrowserStatus::Unavailable,
            BrowserStatus::TimedOut,
            BrowserStatus::Stopped,
        ] {
            assert!(status.retryable());
            assert_ne!(
                status.render(Language::English),
                status.render(Language::SimplifiedChinese)
            );
        }
    }

    #[test]
    fn only_the_active_browser_launch_receives_a_failure() {
        let launches = BrowserLaunches::new();
        let previous = launches.register();
        let previous_ticket = previous.ticket;
        let mut current = launches.register();
        drop(previous);
        launches.open_failed(previous_ticket);
        assert_eq!(
            current.failure.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        );
        launches.open_failed(current.ticket);
        assert!(current.failure.try_recv().is_ok());
    }
}
