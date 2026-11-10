//! Home-page network overview: host addresses, exit IP, DNS and common-service
//! reachability. Local reads run without the core; probes need the local proxy.
use super::presentation::model;
use super::*;
use clash_desktop_protocol::{OverviewItem, Text as SharedString, UiData};
use clash_of_rust::overview as probes;
use futures_util::FutureExt;

/// Resolved with the system resolver to show whether DNS works and how fast.
const DNS_HOST: &str = "github.com";
/// Results older than this are measured again when the home page is open.
const STALE: Duration = Duration::from_secs(180);
/// A node switch changes the exit; wait this long between automatic reruns.
const NODE_DEBOUNCE: Duration = Duration::from_secs(5);

type Probe<T> = Option<Result<T, String>>;

#[derive(Default)]
pub(super) struct Overview {
    generation: u64,
    started: Option<std::time::Instant>,
    dirty: bool,
    aborter: Option<AbortHandle>,
    running: bool,
    node: String,
    pending: usize,
    pub(super) host: Probe<platform::HostInfo>,
    pub(super) dns: Probe<(Vec<String>, u128)>,
    pub(super) exit: Probe<ip_check::CheckResult>,
    pub(super) services: Vec<Probe<probes::Status>>,
}

impl Overview {
    pub(super) fn busy(&self) -> bool {
        self.pending > 0
    }

    /// Forget results that no longer describe the current network.
    pub(super) fn invalidate(&mut self) {
        if let Some(handle) = self.aborter.take() {
            handle.abort();
        }
        self.generation = self.generation.wrapping_add(1);
        self.pending = 0;
        self.dirty = true;
        self.host = None;
        self.dns = None;
        self.exit = None;
        self.services.clear();
    }

    fn due(&self, running: bool, node: &str) -> bool {
        let Some(started) = self.started else {
            return true;
        };
        !self.busy()
            && (started.elapsed() > STALE
                || ((self.dirty || running != self.running || (running && node != self.node))
                    && started.elapsed() >= NODE_DEBOUNCE))
    }
}

impl App {
    /// Start a run when the visible home page has none or a stale one.
    pub(super) fn overview_tick(&mut self) -> Task<Message> {
        if self.page != Page::Home || !self.visible || self.exiting {
            if self.overview.busy() {
                self.overview.invalidate();
            }
            return Task::none();
        }
        if !self.busy
            && !self.working
            && !self.overview.busy()
            && self.overview.due(
                self.snapshot.running,
                self.snapshot.active_node.as_deref().unwrap_or(""),
            )
        {
            return self.overview_refresh();
        }
        Task::none()
    }

    pub(super) fn overview_refresh(&mut self) -> Task<Message> {
        if self.page != Page::Home
            || !self.visible
            || self.busy
            || self.working
            || self.exiting
            || self.overview.busy()
        {
            return Task::none();
        }
        let running = self.snapshot.running;
        let port = self.settings.mixed_port;
        let overview = &mut self.overview;
        overview.generation = overview.generation.wrapping_add(1);
        let generation = overview.generation;
        overview.started = Some(std::time::Instant::now());
        overview.dirty = false;
        overview.running = running;
        overview.node = self.snapshot.active_node.clone().unwrap_or_default();
        overview.host = None;
        overview.dns = None;
        overview.exit = None;
        overview.services = vec![None; probes::TARGETS.len()];
        let mut tasks = vec![Task::perform(
            async {
                tokio::task::spawn_blocking(platform::host_info)
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|result| result.map_err(|e| format!("{e:#}")))
            },
            move |result| Message::OverviewHost(generation, result),
        )];
        if running {
            tasks.push(Task::perform(
                async {
                    probes::dns_lookup(DNS_HOST)
                        .await
                        .map_err(|e| e.to_string())
                },
                move |result| Message::OverviewDns(generation, result),
            ));
            tasks.push(Task::perform(
                async move {
                    ip_check::check("exit-ip", port)
                        .await
                        .map_err(|e| e.to_string())
                },
                move |result| Message::OverviewExit(generation, result),
            ));
            let checker =
                async move { probes::Checker::new(port).map_err(|error| error.to_string()) }
                    .shared();
            for (index, target) in probes::TARGETS.iter().enumerate() {
                let url = target.url;
                let checker = checker.clone();
                tasks.push(Task::perform(
                    async move {
                        checker
                            .await?
                            .check(url)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    move |result| Message::OverviewService(generation, index, result),
                ));
            }
        }
        overview.pending = tasks.len();
        let (task, handle) = Task::batch(tasks).abortable();
        overview.aborter = Some(handle);
        task
    }

    pub(super) fn overview_done(&mut self, generation: u64, store: impl FnOnce(&mut Overview)) {
        if generation != self.overview.generation {
            return;
        }
        store(&mut self.overview);
        self.overview.pending = self.overview.pending.saturating_sub(1);
        if self.overview.pending == 0 {
            self.overview.aborter = None;
        }
    }

    pub(super) fn present_overview(&self, data: &UiData) {
        let overview = &self.overview;
        let report = probes::View {
            running: self.snapshot.running,
            busy: overview.busy(),
            dirty: overview.dirty,
            measured: overview.started.is_some(),
            hidden: self.privacy_hidden,
            host: &overview.host,
            dns: &overview.dns,
            exit: &overview.exit,
            services: &overview.services,
        }
        .present();
        let row = |entry: probes::Item| OverviewItem {
            id: entry.id.into(),
            title: entry.title.into(),
            value: entry.value.into(),
            detail: entry.detail.into(),
            tone: entry.tone,
            delay: entry.delay.into(),
            delay_tone: entry.delay_tone,
            flagged: !entry.flag.is_empty(),
            flag: entry.flag.into(),
            fogged: entry.fogged,
        };
        data.set_overview_network(model(
            data.get_overview_network(),
            report.network.into_iter().map(row).collect(),
        ));
        data.set_overview_services(model(
            data.get_overview_services(),
            report.services.into_iter().map(row).collect(),
        ));
        let status = SharedString::from(report.status.as_str());
        if data.get_overview_status() != status {
            data.set_overview_status(status);
        }
        if data.get_privacy_hidden() != self.privacy_hidden {
            data.set_privacy_hidden(self.privacy_hidden);
        }
        if data.get_overview_busy() != report.busy {
            data.set_overview_busy(report.busy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalidating_a_recent_probe_keeps_the_node_debounce() {
        let mut overview = Overview {
            started: Some(std::time::Instant::now()),
            running: true,
            node: "old".into(),
            pending: 2,
            ..Default::default()
        };
        let generation = overview.generation;
        overview.invalidate();
        assert_ne!(overview.generation, generation);
        assert!(!overview.busy());
        assert!(!overview.due(true, "new"));
        overview.started = Some(std::time::Instant::now() - NODE_DEBOUNCE);
        assert!(overview.due(true, "new"));
    }

    #[test]
    fn invalidation_aborts_inflight_probes_and_discards_old_results() {
        let (handle, _registration) = AbortHandle::new_pair();
        let observer = handle.clone();
        let mut overview = Overview {
            aborter: Some(handle),
            host: Some(Ok(platform::HostInfo::default())),
            dns: Some(Ok((vec!["198.18.0.1".into()], 1))),
            pending: 2,
            ..Default::default()
        };
        overview.invalidate();
        assert!(observer.is_aborted());
        assert!(overview.host.is_none());
        assert!(overview.dns.is_none());
        assert!(overview.aborter.is_none());
        assert!(!overview.busy());
    }
}
