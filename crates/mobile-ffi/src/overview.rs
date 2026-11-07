use crate::{NativeHost, core};
use core::overview::{HostInfo, Presentation, Probe, Status, View};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::mpsc, task::JoinHandle};

const STALE: Duration = Duration::from_secs(180);
const DEBOUNCE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub running: bool,
    pub port: u16,
    pub route: String,
}

pub struct Input {
    pub context: Context,
    pub visible: bool,
    pub idle: bool,
}

enum ResultEvent {
    Host(Result<HostInfo, String>),
    Dns(Result<(Vec<String>, u128), String>),
    Exit(Result<core::ip_check::CheckResult, String>),
    Service(usize, Result<Status, String>),
    Finished,
}

pub struct Event {
    generation: u64,
    value: ResultEvent,
}

#[derive(Default)]
pub struct Monitor {
    context: Option<Context>,
    started: Option<Instant>,
    generation: u64,
    job: Option<JoinHandle<()>>,
    pending: usize,
    dirty: bool,
    hidden: bool,
    host: Probe<HostInfo>,
    dns: Probe<(Vec<String>, u128)>,
    exit: Probe<core::ip_check::CheckResult>,
    services: Vec<Probe<Status>>,
}

impl Drop for Monitor {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            job.abort();
        }
    }
}

impl Monitor {
    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    pub fn toggle_privacy(&mut self) {
        self.hidden = !self.hidden;
    }

    fn invalidate(&mut self) {
        if let Some(job) = self.job.take() {
            job.abort();
        }
        self.generation = self.generation.wrapping_add(1);
        self.pending = 0;
        self.dirty = true;
        self.host = None;
        self.dns = None;
        self.exit = None;
        self.services.clear();
    }

    pub fn tick(
        &mut self,
        input: Input,
        native: Arc<dyn NativeHost>,
        send: mpsc::Sender<Event>,
        force: bool,
    ) {
        if self.context.as_ref() != Some(&input.context) {
            self.invalidate();
            self.context = Some(input.context.clone());
        }
        if !input.visible {
            if self.busy() {
                self.invalidate();
            }
            return;
        }
        if !input.idle || self.busy() {
            return;
        }
        let due = self.started.is_none_or(|started| {
            started.elapsed() >= STALE || (self.dirty && started.elapsed() >= DEBOUNCE)
        });
        if !force && !due {
            return;
        }
        self.invalidate();
        self.started = Some(Instant::now());
        self.dirty = false;
        let generation = self.generation;
        let running = input.context.running;
        let port = input.context.port;
        self.pending = if running {
            3 + core::overview::TARGETS.len()
        } else {
            1
        };
        self.services = vec![None; core::overview::TARGETS.len()];
        self.job = Some(tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            tasks.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    native
                        .request(serde_json::json!({"op":"network-info"}))
                        .and_then(|value| serde_json::from_value(value).map_err(Into::into))
                        .map_err(|error| format!("{error:#}"))
                })
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result);
                ResultEvent::Host(result)
            });
            if running {
                tasks.spawn(async {
                    ResultEvent::Dns(
                        core::overview::dns_lookup("github.com")
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
                tasks.spawn(async move {
                    ResultEvent::Exit(
                        core::ip_check::check("exit-ip", port)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
                for (index, target) in core::overview::TARGETS.iter().enumerate() {
                    tasks.spawn(async move {
                        ResultEvent::Service(
                            index,
                            core::overview::check(target.url, port)
                                .await
                                .map_err(|error| error.to_string()),
                        )
                    });
                }
            }
            while let Some(result) = tasks.join_next().await {
                let Ok(value) = result else { continue };
                if send.send(Event { generation, value }).await.is_err() {
                    break;
                }
            }
            let _ = send
                .send(Event {
                    generation,
                    value: ResultEvent::Finished,
                })
                .await;
        }));
    }

    pub fn apply(&mut self, event: Event) {
        if event.generation != self.generation || !self.busy() {
            return;
        }
        match event.value {
            ResultEvent::Finished => {
                self.host.get_or_insert_with(|| Err("检测任务中断".into()));
                if self.context.as_ref().is_some_and(|context| context.running) {
                    self.dns.get_or_insert_with(|| Err("检测任务中断".into()));
                    self.exit.get_or_insert_with(|| Err("检测任务中断".into()));
                    for slot in &mut self.services {
                        slot.get_or_insert_with(|| Err("检测任务中断".into()));
                    }
                }
                self.pending = 0;
            }
            ResultEvent::Host(result) => self.host = Some(result),
            ResultEvent::Dns(result) => self.dns = Some(result),
            ResultEvent::Exit(result) => self.exit = Some(result),
            ResultEvent::Service(index, result) => {
                if let Some(slot) = self.services.get_mut(index) {
                    *slot = Some(result);
                }
            }
        }
        self.pending = self.pending.saturating_sub(1);
        if !self.busy() {
            self.job = None;
        }
    }

    pub fn present(&self, running: bool) -> Presentation {
        View {
            running,
            busy: self.busy(),
            dirty: self.dirty,
            measured: self.started.is_some(),
            hidden: self.hidden,
            host: &self.host,
            dns: &self.dns,
            exit: &self.exit,
            services: &self.services,
        }
        .present()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Host(AtomicUsize);
    impl NativeHost for Host {
        fn request(&self, value: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            assert_eq!(value["op"], "network-info");
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::json!({"addresses":["192.168.1.9"],"dns":["1.1.1.1"]}))
        }
    }

    fn input(visible: bool) -> Input {
        Input {
            context: Context {
                running: false,
                port: 0,
                route: "default".into(),
            },
            visible,
            idle: true,
        }
    }

    #[tokio::test]
    async fn stopped_core_reads_only_host_and_reuses_results_until_stale() {
        let native = Arc::new(Host::default());
        let (send, mut receive) = mpsc::channel(16);
        let mut monitor = Monitor::default();
        monitor.tick(input(true), native.clone(), send.clone(), false);
        let event = tokio::time::timeout(Duration::from_secs(2), receive.recv())
            .await
            .unwrap()
            .unwrap();
        monitor.apply(event);
        assert!(!monitor.busy());
        assert_eq!(native.0.load(Ordering::SeqCst), 1);
        let view = monitor.present(false);
        assert_eq!(view.network[0].value, "192.168.1.9");
        assert!(view.services.iter().all(|row| row.value == "未检测"));
        assert!(monitor.dns.is_none());
        monitor.tick(input(true), native.clone(), send, false);
        assert!(!monitor.busy());
        monitor.toggle_privacy();
        let view = monitor.present(false);
        assert_ne!(view.network[0].value, "192.168.1.9");
        assert!(view.network[0].fogged);
    }

    #[tokio::test]
    async fn leaving_home_aborts_and_late_results_cannot_repopulate_the_model() {
        let native = Arc::new(Host::default());
        let (send, _receive) = mpsc::channel(16);
        let mut monitor = Monitor::default();
        monitor.tick(input(true), native.clone(), send.clone(), false);
        let generation = monitor.generation;
        monitor.tick(input(false), native.clone(), send.clone(), false);
        assert!(!monitor.busy());
        assert!(monitor.job.is_none());
        monitor.apply(Event {
            generation,
            value: ResultEvent::Host(Ok(HostInfo::default())),
        });
        assert!(monitor.host.is_none());
        monitor.tick(input(true), native.clone(), send.clone(), false);
        assert!(!monitor.busy());
        monitor.started = Some(Instant::now() - DEBOUNCE);
        monitor.tick(input(true), native, send, false);
        assert!(monitor.busy());
    }
}
