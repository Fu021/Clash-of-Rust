//! Home-page network overview: host addresses, exit IP, DNS and common-service
//! reachability. Local reads run without the core; probes need the local proxy.
use super::presentation::model;
use super::*;
use clash_desktop_protocol::{OverviewItem, Text as SharedString, UiData};
use clash_of_rust::overview::{self as probes, Health};

/// Resolved with the system resolver to show whether DNS works and how fast.
const DNS_HOST: &str = "github.com";
/// Results older than this are measured again when the home page is open.
const STALE: Duration = Duration::from_secs(180);
/// A node switch changes the exit; wait this long between automatic reruns.
const NODE_DEBOUNCE: Duration = Duration::from_secs(5);

type Probe<T> = Option<Result<T, String>>;

/// Made-up address of the same shape, so a hidden row keeps its width. The salt is
/// random per process: the placeholder is stable between redraws but cannot be
/// recomputed from the real address.
fn disguise(ip: std::net::IpAddr) -> String {
    use std::hash::BuildHasher;
    static SALT: std::sync::OnceLock<std::hash::RandomState> = std::sync::OnceLock::new();
    let mut state = SALT.get_or_init(Default::default).hash_one(ip) | 1;
    let mut next = move |range: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % range
    };
    match ip {
        std::net::IpAddr::V4(ip) => ip
            .octets()
            .iter()
            .map(|octet| match octet.to_string().len() {
                1 => (1 + next(9)).to_string(),
                2 => (10 + next(90)).to_string(),
                _ => (100 + next(156)).to_string(),
            })
            .collect::<Vec<_>>()
            .join("."),
        std::net::IpAddr::V6(ip) => ip
            .to_string()
            .chars()
            .map(|c| {
                if c.is_ascii_hexdigit() {
                    char::from_digit(next(16) as u32, 16).unwrap_or('0')
                } else {
                    c
                }
            })
            .collect(),
    }
}

fn mask(ips: &[std::net::IpAddr]) -> String {
    ips.iter()
        .map(|ip| disguise(*ip))
        .collect::<Vec<_>>()
        .join("  ")
}

#[derive(Default)]
pub(super) struct Overview {
    generation: u64,
    started: Option<std::time::Instant>,
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
        self.generation = self.generation.wrapping_add(1);
        self.started = None;
        self.pending = 0;
    }

    fn due(&self, running: bool, node: &str) -> bool {
        let Some(started) = self.started else {
            return true;
        };
        !self.busy()
            && (started.elapsed() > STALE
                || (running && !self.running)
                || (running && node != self.node && started.elapsed() > NODE_DEBOUNCE))
    }
}

impl App {
    /// Start a run when the visible home page has none or a stale one.
    pub(super) fn overview_tick(&mut self) -> Task<Message> {
        if self.page == Page::Home
            && self.visible
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
        if self.overview.busy() {
            return Task::none();
        }
        let running = self.snapshot.running;
        let port = self.settings.mixed_port;
        let overview = &mut self.overview;
        overview.generation = overview.generation.wrapping_add(1);
        let generation = overview.generation;
        overview.started = Some(std::time::Instant::now());
        overview.running = running;
        overview.node = self.snapshot.active_node.clone().unwrap_or_default();
        overview.host = None;
        overview.dns = None;
        overview.exit = None;
        overview.services = vec![None; probes::TARGETS.len()];
        let mut tasks = vec![
            Task::perform(
                async {
                    tokio::task::spawn_blocking(platform::host_info)
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|result| result.map_err(|e| format!("{e:#}")))
                },
                move |result| Message::OverviewHost(generation, result),
            ),
            Task::perform(
                async {
                    probes::dns_lookup(DNS_HOST)
                        .await
                        .map_err(|e| e.to_string())
                },
                move |result| Message::OverviewDns(generation, result),
            ),
        ];
        if running {
            tasks.push(Task::perform(
                async move {
                    ip_check::check("exit-ip", port)
                        .await
                        .map_err(|e| e.to_string())
                },
                move |result| Message::OverviewExit(generation, result),
            ));
            for (index, target) in probes::TARGETS.iter().enumerate() {
                let url = target.url;
                tasks.push(Task::perform(
                    async move { probes::check(url, port).await.map_err(|e| e.to_string()) },
                    move |result| Message::OverviewService(generation, index, result),
                ));
            }
        }
        overview.pending = tasks.len();
        Task::batch(tasks)
    }

    pub(super) fn overview_done(&mut self, generation: u64, store: impl FnOnce(&mut Overview)) {
        if generation != self.overview.generation {
            return;
        }
        store(&mut self.overview);
        self.overview.pending = self.overview.pending.saturating_sub(1);
    }

    pub(super) fn present_overview(&self, data: &UiData) {
        let overview = &self.overview;
        let waiting = overview.busy();
        let item = |id: &str, title: &str, value: String, detail: String, tone: i32| OverviewItem {
            id: id.into(),
            title: title.into(),
            value: value.into(),
            detail: detail.into(),
            tone,
            ..Default::default()
        };
        let checking = || "检测中…".to_owned();
        let join = |ips: &[std::net::IpAddr], separator: &str| {
            ips.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(separator)
        };

        let (lan, dns_servers) = match &overview.host {
            None if waiting => (
                item("lan", "内网 IP", checking(), String::new(), 4),
                item("dns", "DNS 服务器", checking(), String::new(), 4),
            ),
            None => (
                item("lan", "内网 IP", "—".into(), String::new(), 4),
                item("dns", "DNS 服务器", "—".into(), String::new(), 4),
            ),
            Some(Err(error)) => (
                item("lan", "内网 IP", "读取失败".into(), error.clone(), 3),
                item("dns", "DNS 服务器", "读取失败".into(), error.clone(), 3),
            ),
            Some(Ok(info)) => {
                let family = match (
                    info.addresses.iter().any(std::net::IpAddr::is_ipv4),
                    info.addresses.iter().any(std::net::IpAddr::is_ipv6),
                ) {
                    (true, true) => "IPv4 + IPv6",
                    (true, false) => "仅 IPv4",
                    (false, true) => "仅 IPv6",
                    (false, false) => "",
                };
                let show = |ips: &[std::net::IpAddr]| {
                    if self.privacy_hidden {
                        mask(ips)
                    } else {
                        join(ips, "  ")
                    }
                };
                let mut lan = item(
                    "lan",
                    "内网 IP",
                    if info.addresses.is_empty() {
                        "未检测到".into()
                    } else {
                        show(&info.addresses)
                    },
                    family.into(),
                    if info.addresses.is_empty() { 2 } else { 0 },
                );
                let mut dns = item(
                    "dns",
                    "DNS 服务器",
                    if info.dns.is_empty() {
                        "未读取到".into()
                    } else {
                        show(&info.dns)
                    },
                    "物理网卡的系统设置".into(),
                    if info.dns.is_empty() { 2 } else { 0 },
                );
                lan.fogged = self.privacy_hidden && !info.addresses.is_empty();
                dns.fogged = self.privacy_hidden && !info.dns.is_empty();
                (lan, dns)
            }
        };

        let exit = match &overview.exit {
            _ if !self.snapshot.running => {
                item("exit", "出口 IP", "—".into(), "启动内核后检测".into(), 4)
            }
            None if waiting => item("exit", "出口 IP", checking(), String::new(), 4),
            None => item("exit", "出口 IP", "—".into(), String::new(), 4),
            Some(Err(error)) => item("exit", "出口 IP", "检测失败".into(), error.clone(), 3),
            Some(Ok(result)) => {
                let (country, flag) = clash_of_rust::flags::label(&result.country);
                let mut item = item(
                    "exit",
                    "出口 IP",
                    result.summary.clone(),
                    format!("{country} · {} ms", result.millis),
                    1,
                );
                item.flagged = flag.is_some();
                item.flag = flag.unwrap_or_default();
                if self.privacy_hidden {
                    item.value = result
                        .summary
                        .parse::<std::net::IpAddr>()
                        .map_or_else(|_| "000.000.000.000".into(), |ip| mask(&[ip]))
                        .into();
                    item.fogged = true;
                }
                item
            }
        };

        let lookup = match &overview.dns {
            None if waiting => item("lookup", "DNS 解析", checking(), String::new(), 4),
            None => item("lookup", "DNS 解析", "—".into(), String::new(), 4),
            Some(Err(error)) => item("lookup", "DNS 解析", "解析失败".into(), error.clone(), 3),
            Some(Ok((addresses, millis))) => {
                let first = addresses.first().map_or("", String::as_str);
                // The core's fake-ip pool: the system resolver is answered by the core.
                let fake = first.starts_with("198.18.") || first.starts_with("198.19.");
                item(
                    "lookup",
                    "DNS 解析",
                    format!("{DNS_HOST} → {first}"),
                    format!("{millis} ms{}", if fake { " · Fake-IP" } else { "" }),
                    if *millis < 500 { 1 } else { 2 },
                )
            }
        };
        data.set_overview_network(model(
            data.get_overview_network(),
            vec![lan, exit, dns_servers, lookup],
        ));

        let running = self.snapshot.running;
        let mut reachable = 0;
        let services = probes::TARGETS
            .iter()
            .enumerate()
            .map(|(index, target)| {
                let result = overview.services.get(index).and_then(Option::as_ref);
                let (value, detail, tone) = match result {
                    _ if !running => ("未检测".into(), "内核未运行".into(), 4),
                    None if waiting => (checking(), String::new(), 4),
                    None => ("—".into(), String::new(), 4),
                    Some(Err(error)) => (error.clone(), "未获得响应".into(), 3),
                    Some(Ok(status)) => {
                        let detail = format!("HTTP {}", status.status);
                        match status.health {
                            Health::Ok => {
                                reachable += 1;
                                (
                                    format!("{} ms", status.millis),
                                    detail,
                                    if status.millis < 800 { 1 } else { 2 },
                                )
                            }
                            Health::Reachable => {
                                reachable += 1;
                                (
                                    format!("{} ms", status.millis),
                                    format!("{detail} · 可达"),
                                    2,
                                )
                            }
                            Health::Error => ("服务异常".into(), detail, 3),
                        }
                    }
                };
                item(target.id, target.name, value, detail, tone)
            })
            .collect();
        data.set_overview_services(model(data.get_overview_services(), services));
        let status = if waiting {
            "检测中…".to_owned()
        } else if !running {
            "内核未运行，仅显示本机信息".to_owned()
        } else if overview.started.is_none() {
            String::new()
        } else {
            format!("可访问 {reachable} / {}", probes::TARGETS.len())
        };
        let status = SharedString::from(status.as_str());
        if data.get_overview_status() != status {
            data.set_overview_status(status);
        }
        if data.get_privacy_hidden() != self.privacy_hidden {
            data.set_privacy_hidden(self.privacy_hidden);
        }
        if data.get_overview_busy() != waiting {
            data.set_overview_busy(waiting);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disguise_keeps_the_shape_but_not_the_address_and_is_stable() {
        let v4: std::net::IpAddr = "192.168.1.23".parse().unwrap();
        let fake = disguise(v4);
        assert_eq!(
            fake,
            disguise(v4),
            "a redraw must not change the placeholder"
        );
        let octets: Vec<_> = fake.split('.').collect();
        assert_eq!(octets.len(), 4);
        for (fake, real) in octets.iter().zip(["192", "168", "1", "23"]) {
            assert_eq!(fake.len(), real.len());
            assert!(fake.parse::<u8>().is_ok(), "{fake} must be a valid octet");
        }
        let v6: std::net::IpAddr = "2408:8000::5".parse().unwrap();
        let fake = disguise(v6);
        assert_eq!(fake.len(), "2408:8000::5".len());
        assert_eq!(fake.matches(':').count(), 3);
        assert!(fake.parse::<std::net::IpAddr>().is_ok());
        assert_eq!(
            mask(&[v4, v6]).split("  ").count(),
            2,
            "one placeholder per address"
        );
    }
}
