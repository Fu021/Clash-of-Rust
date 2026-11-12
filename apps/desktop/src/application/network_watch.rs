use super::*;
use platform::host_network;
use serde_json::Value;
use std::time::Instant;

#[derive(Default)]
pub(super) struct Watch {
    routes: Value,
    proxy: Option<ProxyMode>,
    revision: u64,
    pending: bool,
    checked_at: Option<Instant>,
}

impl Watch {
    pub(super) fn invalidate(&mut self) {
        self.routes = Value::Null;
        self.checked_at = None;
        self.revision = self.revision.wrapping_add(1);
    }

    pub(super) fn proxy(&mut self, proxy: Option<ProxyMode>) {
        if self.proxy != proxy {
            self.proxy = proxy;
            self.invalidate();
        }
    }

    pub(super) fn begin(&mut self) -> Option<u64> {
        if self.pending
            || self
                .checked_at
                .is_some_and(|time| time.elapsed() < Duration::from_secs(30))
        {
            return None;
        }
        self.pending = true;
        self.checked_at = Some(Instant::now());
        Some(self.revision)
    }

    pub(super) fn finish(&mut self, revision: u64, routes: Value) {
        self.pending = false;
        if self.revision == revision {
            self.routes = routes;
        }
    }

    pub(super) fn notice(&self, settings: Option<&Settings>) -> String {
        host_network::notice(&host_network::with_settings_context(
            self.routes.clone(),
            self.proxy,
            settings,
        ))
        .unwrap_or_default()
    }
}

impl App {
    pub(super) fn observe_network(&mut self) -> Task<Message> {
        if !cfg!(target_os = "linux") || !self.visible || self.exiting {
            return Task::none();
        }
        let Some(revision) = self.network_watch.begin() else {
            return Task::none();
        };
        Task::perform(host_network::observe(), move |routes| {
            Message::HostRoutes(revision, routes)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn routes() -> Value {
        json!({
            "scope":"current_network_namespace",
            "ipv4":{"state":"observed","interface":"Meta","kind":"tun"},
            "tun_holders":{"interfaces":{"Meta":[{"uid":0,"pid":42}]}}
        })
    }

    #[test]
    fn route_observation_is_bounded_and_discards_results_before_a_proxy_change() {
        let mut watch = Watch::default();
        let revision = watch.begin().unwrap();
        assert!(watch.begin().is_none());
        watch.proxy(Some(ProxyMode::Off));
        watch.finish(revision, routes());
        assert!(watch.notice(None).is_empty());
        let revision = watch.begin().unwrap();
        watch.finish(revision, routes());
        assert!(watch.notice(None).contains("外部 TUN"));
        assert!(watch.notice(None).contains("root · PID 42"));
        assert!(watch.begin().is_none());
        watch.proxy(Some(ProxyMode::Tun));
        assert!(watch.notice(None).is_empty());
        assert!(watch.begin().is_some());
    }

    #[test]
    fn external_tun_notice_is_removed_after_a_confirmed_route_change() {
        let mut watch = Watch::default();
        watch.proxy(Some(ProxyMode::Off));
        let revision = watch.begin().unwrap();
        watch.finish(revision, routes());
        assert!(!watch.notice(None).is_empty());
        watch.checked_at = None;
        let revision = watch.begin().unwrap();
        watch.finish(
            revision,
            json!({
                "scope":"current_network_namespace",
                "ipv4":{"state":"observed","interface":"eth0","kind":"other"}
            }),
        );
        assert!(watch.notice(None).is_empty());
    }

    #[test]
    fn changing_ipv6_preference_reclassifies_the_existing_sample() {
        let mut watch = Watch::default();
        watch.proxy(Some(ProxyMode::Tun));
        let revision = watch.begin().unwrap();
        watch.finish(
            revision,
            json!({
                "scope":"current_network_namespace",
                "ipv4":{"state":"observed","interface":"Meta","kind":"tun"},
                "ipv6":{"state":"observed","interface":"wlp3s0","kind":"other"}
            }),
        );
        let mut settings = Settings {
            ipv6: true,
            ..Default::default()
        };
        assert!(!watch.notice(Some(&settings)).is_empty());
        settings.ipv6 = false;
        assert!(watch.notice(Some(&settings)).is_empty());
        settings.ipv6 = true;
        assert!(!watch.notice(Some(&settings)).is_empty());
    }

    #[test]
    fn an_ipv6_warning_tracks_uplink_arrival_and_disappearance() {
        let mut watch = Watch::default();
        watch.proxy(Some(ProxyMode::Tun));
        let settings = Settings::default();
        for state in ["unavailable", "available", "unavailable"] {
            watch.checked_at = None;
            let revision = watch.begin().unwrap();
            watch.finish(
                revision,
                json!({
                    "scope":"current_network_namespace",
                    "ipv4":{"state":"observed","interface":"Meta","kind":"tun"},
                    "ipv6":{"state":"observed","interface":"eth0","kind":"other"},
                    "ipv6_uplink":{"state":state}
                }),
            );
            assert_eq!(
                watch.notice(Some(&settings)).is_empty(),
                state == "unavailable"
            );
        }
    }
}
