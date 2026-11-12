//! One network watcher belongs to the GUI/CLI owner, independent of its windows.
use crate::{
    engine::Engine,
    platform::network::{self, Snapshot},
};
use std::{
    sync::Weak,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, watch};

#[derive(Default)]
pub(crate) struct Monitor {
    baseline: Option<Snapshot>,
    pending: Option<Snapshot>,
    retry_at: Option<Instant>,
    failures: u32,
}

impl Monitor {
    pub(crate) fn seed(&mut self, routes: Option<Snapshot>) {
        *self = Self {
            baseline: routes,
            ..Default::default()
        };
    }

    pub(crate) fn observe(&mut self, routes: Snapshot, now: Instant) -> bool {
        #[cfg(target_os = "linux")]
        let routes = Snapshot {
            ipv6: routes.ipv6.or_else(|| {
                self.baseline
                    .as_ref()
                    .and_then(|baseline| baseline.ipv6.clone())
            }),
            ..routes
        };
        if self.baseline.is_none() || routes.is_empty() {
            self.seed(Some(routes));
            return false;
        }
        if self.baseline.as_ref() == Some(&routes) {
            self.pending = None;
            self.retry_at = None;
            self.failures = 0;
            return false;
        }
        if self.pending.as_ref() != Some(&routes) {
            self.pending = Some(routes);
            self.retry_at = None;
            self.failures = 0;
            return false;
        }
        // Two matching observations avoid refreshing during DHCP/route churn.
        self.retry_at.is_none_or(|deadline| now >= deadline)
    }

    pub(crate) fn finish(&mut self, succeeded: bool, now: Instant) {
        if succeeded {
            self.baseline = self.pending.take();
            self.retry_at = None;
            self.failures = 0;
        } else {
            self.failures = self.failures.saturating_add(1);
            let seconds = 30 * (1u64 << self.failures.min(4));
            self.retry_at = Some(now + Duration::from_secs(seconds.min(300)));
        }
    }
}

pub(super) fn start(
    engine: Weak<Mutex<Engine>>,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut reported = false;
        loop {
            tokio::select! {
                _ = tick.tick() => {},
                _ = shutdown.changed() => break,
            }
            if *shutdown.borrow() {
                break;
            }
            let Some(shared) = engine.upgrade() else {
                break;
            };
            let Some(pid) = shared.lock().await.network_core() else {
                reported = false;
                continue;
            };
            // Native reads and route-table allocation happen outside the owner lock.
            let routes = tokio::task::spawn_blocking(network::recovery_snapshot).await;
            if *shutdown.borrow() {
                break;
            }
            let mut owner = shared.lock().await;
            if owner.network_core() != Some(pid) {
                continue; // Discard a sample captured for a previous core process.
            }
            match routes {
                Ok(Ok(routes)) => {
                    reported = false;
                    owner.network_changed(routes).await;
                }
                _ if !reported => {
                    owner.record_failure("读取默认路由失败，网络恢复监控将在下次重试");
                    reported = true;
                }
                _ => {}
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::network::Route;

    fn online(gateway: &str) -> Snapshot {
        Snapshot {
            routes: vec![Route {
                interface: "physical".into(),
                gateway: gateway.into(),
            }],
            #[cfg(target_os = "linux")]
            ipv6: None,
        }
    }

    fn offline() -> Snapshot {
        let mut snapshot = online("a");
        snapshot.routes.clear();
        snapshot
    }

    #[test]
    fn offline_start_recovers_once_after_the_route_settles() {
        let mut monitor = Monitor::default();
        monitor.seed(Some(offline()));
        let now = Instant::now();
        assert!(!monitor.observe(online("a"), now));
        assert!(monitor.observe(online("a"), now));
        monitor.finish(true, now);
        assert!(!monitor.observe(online("a"), now));
        assert!(!monitor.observe(offline(), now));
        assert!(!monitor.observe(online("a"), now));
        assert!(monitor.observe(online("a"), now));
    }

    #[test]
    fn initial_online_unknown_or_unstable_network_does_not_refresh() {
        let mut monitor = Monitor::default();
        let now = Instant::now();
        assert!(!monitor.observe(online("a"), now));
        assert!(!monitor.observe(online("a"), now));
        assert!(!monitor.observe(online("b"), now));
        assert!(!monitor.observe(online("c"), now));
        assert!(monitor.observe(online("c"), now));
        monitor.finish(true, now);
        assert!(!monitor.observe(online("c"), now));
    }

    #[test]
    fn failure_keeps_the_pending_route_and_backs_off_until_success() {
        let mut monitor = Monitor::default();
        monitor.seed(Some(online("a")));
        let now = Instant::now();
        assert!(!monitor.observe(online("b"), now));
        assert!(monitor.observe(online("b"), now));
        monitor.finish(false, now);
        assert!(!monitor.observe(online("b"), now + Duration::from_secs(59)));
        assert!(monitor.observe(online("b"), now + Duration::from_secs(60)));
        monitor.finish(true, now);
        assert!(!monitor.observe(online("b"), now + Duration::from_secs(300)));
        monitor.seed(Some(online("b")));
        assert!(!monitor.observe(online("b"), now));
    }

    #[test]
    fn a_route_reverting_or_a_stopped_core_cancels_pending_recovery() {
        let mut monitor = Monitor::default();
        monitor.seed(Some(online("a")));
        let now = Instant::now();
        assert!(!monitor.observe(online("b"), now));
        assert!(!monitor.observe(online("a"), now));
        assert!(!monitor.observe(online("b"), now));
        monitor.seed(None);
        assert!(!monitor.observe(online("b"), now));
        assert!(!monitor.observe(online("b"), now));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ipv6_arrival_loss_and_address_changes_refresh_once_after_stabilizing() {
        use crate::platform::host_network::ipv6;
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("eth0")).unwrap();
        std::fs::write(root.path().join("eth0/flags"), "0x1").unwrap();
        let mut absent = online("a");
        absent.ipv6 = Some(ipv6::parse("", root.path()).unwrap());
        let mut present = absent.clone();
        present.ipv6 = Some(
            ipv6::parse(
                "20010db8000000000000000000000001 02 40 00 80 eth0\n",
                root.path(),
            )
            .unwrap(),
        );
        let mut changed = present.clone();
        changed.ipv6 = Some(
            ipv6::parse(
                "20010db8000000000000000000000002 02 40 00 80 eth0\n",
                root.path(),
            )
            .unwrap(),
        );
        let now = Instant::now();
        let mut monitor = Monitor::default();
        monitor.seed(Some(absent.clone()));
        for next in [present, changed, absent] {
            assert!(!monitor.observe(next.clone(), now));
            assert!(monitor.observe(next.clone(), now));
            monitor.finish(true, now);
            assert!(!monitor.observe(next, now));
        }
        let mut unreadable = online("a");
        unreadable.ipv6 = None;
        assert!(!monitor.observe(unreadable, now));
    }
}
