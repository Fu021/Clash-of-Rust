use super::{Health, Status, TARGETS};
use crate::ip_check;
use std::net::IpAddr;

pub type Probe<T> = Option<Result<T, String>>;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostInfo {
    pub addresses: Vec<IpAddr>,
    pub dns: Vec<IpAddr>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub value: String,
    pub detail: String,
    pub tone: i32,
    pub delay: String,
    pub delay_tone: i32,
    pub flag: String,
    pub fogged: bool,
}

pub struct Presentation {
    pub network: Vec<Item>,
    pub services: Vec<Item>,
    pub status: String,
    pub busy: bool,
    pub hidden: bool,
}

pub struct View<'a> {
    pub running: bool,
    pub busy: bool,
    pub dirty: bool,
    pub measured: bool,
    pub hidden: bool,
    pub host: &'a Probe<HostInfo>,
    pub dns: &'a Probe<(Vec<String>, u128)>,
    pub exit: &'a Probe<ip_check::CheckResult>,
    pub services: &'a [Probe<Status>],
}

fn item(id: &str, title: &str, value: &str, detail: &str, tone: i32) -> Item {
    Item {
        id: id.into(),
        title: title.into(),
        value: value.into(),
        detail: detail.into(),
        tone,
        ..Default::default()
    }
}

impl Item {
    fn with_delay(mut self, millis: u128) -> Self {
        self.delay = format!("{millis} ms");
        self.delay_tone = crate::proxy_order::response_tone(millis);
        self
    }
}

fn disguise(ip: IpAddr) -> String {
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
        IpAddr::V4(ip) => ip
            .octets()
            .iter()
            .map(|octet| {
                let (minimum, count) = match octet.to_string().len() {
                    1 => (0, 10),
                    2 => (10, 90),
                    _ => (100, 156),
                };
                (minimum + (u64::from(*octet) - minimum + 1 + next(count - 1)) % count).to_string()
            })
            .collect::<Vec<_>>()
            .join("."),
        IpAddr::V6(ip) => ip
            .to_string()
            .chars()
            .map(|symbol| {
                if let Some(digit) = symbol.to_digit(16) {
                    char::from_digit((digit + 1 + next(15) as u32) % 16, 16).unwrap_or('0')
                } else {
                    symbol
                }
            })
            .collect(),
    }
}

impl View<'_> {
    fn address(&self, ip: IpAddr) -> String {
        if self.hidden {
            disguise(ip)
        } else {
            ip.to_string()
        }
    }

    fn addresses(&self, ips: &[IpAddr]) -> String {
        ips.iter()
            .map(|ip| self.address(*ip))
            .collect::<Vec<_>>()
            .join("  ")
    }

    pub fn present(&self) -> Presentation {
        let waiting = if self.busy { "检测中…" } else { "—" };
        let (lan, dns_servers) = match self.host {
            None => (
                item("lan", "内网 IP", waiting, "", 4),
                item("dns", "DNS 服务器", waiting, "", 4),
            ),
            Some(Err(error)) => (
                item("lan", "内网 IP", "读取失败", error, 3),
                item("dns", "DNS 服务器", "读取失败", error, 3),
            ),
            Some(Ok(info)) => {
                let family = match (
                    info.addresses.iter().any(IpAddr::is_ipv4),
                    info.addresses.iter().any(IpAddr::is_ipv6),
                ) {
                    (true, true) => "IPv4 + IPv6",
                    (true, false) => "仅 IPv4",
                    (false, true) => "仅 IPv6",
                    (false, false) => "",
                };
                let mut lan = item("lan", "内网 IP", "未检测到", family, 2);
                if !info.addresses.is_empty() {
                    lan.value = self.addresses(&info.addresses);
                    lan.tone = 0;
                    lan.fogged = self.hidden;
                }
                let mut dns = item("dns", "DNS 服务器", "未读取到", "系统解析器配置", 2);
                if !info.dns.is_empty() {
                    dns.value = self.addresses(&info.dns);
                    dns.tone = 0;
                    dns.fogged = self.hidden;
                }
                (lan, dns)
            }
        };
        let exit = match self.exit {
            _ if !self.running => item("exit", "出口 IP", "—", "启动内核后检测", 4),
            None => item("exit", "出口 IP", waiting, "", 4),
            Some(Err(error)) => item("exit", "出口 IP", "检测失败", error, 3),
            Some(Ok(result)) => match result.summary.parse::<IpAddr>() {
                Err(_) => item("exit", "出口 IP", "未确认", &result.summary, 2),
                Ok(address) => {
                    let country = crate::countries::country_text(&result.country);
                    let flag = crate::countries::flag_emoji(&result.country);
                    let mut entry = item("exit", "出口 IP", &self.address(address), &country, 1)
                        .with_delay(result.millis);
                    entry.flag = flag.unwrap_or_default();
                    entry.fogged = self.hidden;
                    entry
                }
            },
        };
        let lookup = match self.dns {
            _ if !self.running => item("lookup", "DNS 解析", "—", "启动内核后检测", 4),
            None => item("lookup", "DNS 解析", waiting, "", 4),
            Some(Err(error)) => item("lookup", "DNS 解析", "解析失败", error, 3),
            Some(Ok((addresses, millis))) => {
                let first = addresses.first().map_or("", String::as_str);
                let fake = first.starts_with("198.18.") || first.starts_with("198.19.");
                let parsed = first.parse::<IpAddr>().ok();
                let address = parsed.map_or_else(|| first.to_owned(), |ip| self.address(ip));
                let mut entry = item(
                    "lookup",
                    "DNS 解析",
                    &format!("github.com → {address}"),
                    if fake { "Fake-IP" } else { "" },
                    1,
                )
                .with_delay(*millis);
                entry.fogged = self.hidden && parsed.is_some();
                entry
            }
        };
        let mut reachable = 0;
        let services = TARGETS
            .iter()
            .enumerate()
            .map(|(index, target)| {
                let result = self.services.get(index).and_then(Option::as_ref);
                match result {
                    _ if !self.running => item(target.id, target.name, "未检测", "内核未运行", 4),
                    None => item(target.id, target.name, waiting, "", 4),
                    Some(Err(error)) => item(target.id, target.name, error, "未获得响应", 3),
                    Some(Ok(status)) => {
                        let detail = format!("HTTP {}", status.status);
                        match status.health {
                            Health::Ok | Health::Reachable => {
                                reachable += 1;
                                item(
                                    target.id,
                                    target.name,
                                    &format!("{} ms", status.millis),
                                    &if status.health == Health::Reachable {
                                        format!("{detail} · 可达")
                                    } else {
                                        detail
                                    },
                                    if status.health == Health::Ok { 1 } else { 2 },
                                )
                                .with_delay(status.millis)
                            }
                            Health::Error => item(target.id, target.name, "服务异常", &detail, 3),
                        }
                    }
                }
            })
            .collect();
        let status = if self.busy {
            "检测中…".to_owned()
        } else if !self.running {
            "内核未运行，仅显示本机信息".to_owned()
        } else if self.dirty {
            "等待重新检测".to_owned()
        } else if !self.measured {
            String::new()
        } else {
            format!("可访问 {reachable} / {}", TARGETS.len())
        };
        Presentation {
            network: vec![lan, exit, dns_servers, lookup],
            services,
            status,
            busy: self.busy,
            hidden: self.hidden,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_time_color_does_not_override_service_health() {
        let services = [
            Some(Ok(Status {
                health: Health::Ok,
                status: 200,
                millis: 925,
            })),
            Some(Ok(Status {
                health: Health::Reachable,
                status: 403,
                millis: 128,
            })),
            Some(Ok(Status {
                health: Health::Error,
                status: 502,
                millis: 30,
            })),
        ];
        let report = View {
            running: true,
            busy: false,
            dirty: false,
            measured: true,
            hidden: false,
            host: &None,
            dns: &None,
            exit: &None,
            services: &services,
        }
        .present();
        assert_eq!(
            (report.services[0].tone, report.services[0].delay_tone),
            (1, 3)
        );
        assert_eq!(
            (report.services[1].tone, report.services[1].delay_tone),
            (2, 1)
        );
        assert!(report.services[1].detail.contains("可达"));
        assert_eq!(report.services[2].tone, 3);
        assert!(report.services[2].delay.is_empty());
    }

    #[test]
    fn privacy_changes_the_address_before_either_host_receives_the_display_model() {
        let host = Some(Ok(HostInfo {
            addresses: vec!["192.168.1.23".parse().unwrap()],
            dns: vec!["1.1.1.1".parse().unwrap()],
        }));
        let dns = Some(Ok((vec!["198.18.0.18".into()], 12)));
        let view = View {
            running: true,
            busy: false,
            dirty: false,
            measured: true,
            hidden: true,
            host: &host,
            dns: &dns,
            exit: &None,
            services: &[],
        };
        let report = view.present();
        for (index, address) in [(0, "192.168.1.23"), (2, "1.1.1.1"), (3, "198.18.0.18")] {
            assert!(report.network[index].fogged);
            assert!(!report.network[index].value.contains(address));
        }
        assert!(report.network[3].detail.contains("Fake-IP"));
        assert_eq!(report.network, view.present().network);
    }

    #[test]
    fn disguise_keeps_shape_and_never_returns_the_original_address() {
        for text in [
            "0.0.0.0",
            "192.168.1.23",
            "255.255.255.255",
            "::1",
            "2408:8000::5",
        ] {
            let address = text.parse().unwrap();
            let hidden = disguise(address);
            assert_ne!(hidden, text);
            assert_eq!(hidden, disguise(address));
            assert_eq!(hidden.len(), text.len());
            assert!(hidden.parse::<IpAddr>().is_ok());
        }
    }
}
