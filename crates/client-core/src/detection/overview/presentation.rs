use super::{Health, Status, TARGETS};
use crate::{config::Language, ip_check};
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

fn probe_error(error: &str, language: Language) -> &str {
    match error {
        "超时" => language.text("超时", "Timed out"),
        "无法连接" => language.text("无法连接", "Connection failed"),
        "请求失败" => language.text("请求失败", "Request failed"),
        "未返回地址" => language.text("未返回地址", "No address returned"),
        "检测任务中断" => language.text("检测任务中断", "Check interrupted"),
        _ => error,
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
        self.present_in(Language::SimplifiedChinese)
    }

    pub fn present_in(&self, language: Language) -> Presentation {
        let waiting = if self.busy {
            language.text("检测中…", "Checking…")
        } else {
            "—"
        };
        let lan_title = language.text("内网 IP", "Local IP");
        let dns_title = language.text("DNS 服务器", "DNS servers");
        let exit_title = language.text("出口 IP", "Exit IP");
        let lookup_title = language.text("DNS 解析", "DNS lookup");
        let start_core = language.text("启动内核后检测", "Start the core to check");
        let (lan, dns_servers) = match self.host {
            None => (
                item("lan", lan_title, waiting, "", 4),
                item("dns", dns_title, waiting, "", 4),
            ),
            Some(Err(error)) => (
                item(
                    "lan",
                    lan_title,
                    language.text("读取失败", "Read failed"),
                    probe_error(error, language),
                    3,
                ),
                item(
                    "dns",
                    dns_title,
                    language.text("读取失败", "Read failed"),
                    probe_error(error, language),
                    3,
                ),
            ),
            Some(Ok(info)) => {
                let family = match (
                    info.addresses.iter().any(IpAddr::is_ipv4),
                    info.addresses.iter().any(IpAddr::is_ipv6),
                ) {
                    (true, true) => "IPv4 + IPv6",
                    (true, false) => language.text("仅 IPv4", "IPv4 only"),
                    (false, true) => language.text("仅 IPv6", "IPv6 only"),
                    (false, false) => "",
                };
                let mut lan = item(
                    "lan",
                    lan_title,
                    language.text("未检测到", "Not detected"),
                    family,
                    2,
                );
                if !info.addresses.is_empty() {
                    lan.value = self.addresses(&info.addresses);
                    lan.tone = 0;
                    lan.fogged = self.hidden;
                }
                let mut dns = item(
                    "dns",
                    dns_title,
                    language.text("未读取到", "Not found"),
                    language.text("系统解析器配置", "System resolver configuration"),
                    2,
                );
                if !info.dns.is_empty() {
                    dns.value = self.addresses(&info.dns);
                    dns.tone = 0;
                    dns.fogged = self.hidden;
                }
                (lan, dns)
            }
        };
        let exit = match self.exit {
            _ if !self.running => item("exit", exit_title, "—", start_core, 4),
            None => item("exit", exit_title, waiting, "", 4),
            Some(Err(error)) => item(
                "exit",
                exit_title,
                language.text("检测失败", "Check failed"),
                probe_error(error, language),
                3,
            ),
            Some(Ok(result)) => match result.summary.parse::<IpAddr>() {
                Err(_) => item(
                    "exit",
                    exit_title,
                    language.text("未确认", "Unconfirmed"),
                    &result.summary,
                    2,
                ),
                Ok(address) => {
                    let country = crate::countries::localized_name(
                        crate::countries::country_text(&result.country),
                        language,
                    );
                    let flag = crate::countries::flag_emoji(&result.country);
                    let mut entry = item("exit", exit_title, &self.address(address), &country, 1)
                        .with_delay(result.millis);
                    entry.flag = flag.unwrap_or_default();
                    entry.fogged = self.hidden;
                    entry
                }
            },
        };
        let lookup = match self.dns {
            _ if !self.running => item("lookup", lookup_title, "—", start_core, 4),
            None => item("lookup", lookup_title, waiting, "", 4),
            Some(Err(error)) => item(
                "lookup",
                lookup_title,
                language.text("解析失败", "Lookup failed"),
                probe_error(error, language),
                3,
            ),
            Some(Ok((addresses, millis))) => {
                let first = addresses.first().map_or("", String::as_str);
                let fake = first.starts_with("198.18.") || first.starts_with("198.19.");
                let parsed = first.parse::<IpAddr>().ok();
                let address = parsed.map_or_else(|| first.to_owned(), |ip| self.address(ip));
                let mut entry = item(
                    "lookup",
                    lookup_title,
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
                    _ if !self.running => item(
                        target.id,
                        target.name,
                        language.text("未检测", "Not checked"),
                        language.text("内核未运行", "Core stopped"),
                        4,
                    ),
                    None => item(target.id, target.name, waiting, "", 4),
                    Some(Err(error)) => item(
                        target.id,
                        target.name,
                        probe_error(error, language),
                        language.text("未获得响应", "No response"),
                        3,
                    ),
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
                                        format!("{detail} · {}", language.text("可达", "Reachable"))
                                    } else {
                                        detail
                                    },
                                    if status.health == Health::Ok { 1 } else { 2 },
                                )
                                .with_delay(status.millis)
                            }
                            Health::Error => item(
                                target.id,
                                target.name,
                                language.text("服务异常", "Service error"),
                                &detail,
                                3,
                            ),
                        }
                    }
                }
            })
            .collect();
        let status = if self.busy {
            language.text("检测中…", "Checking…").to_owned()
        } else if !self.running {
            language
                .text(
                    "内核未运行，仅显示本机信息",
                    "Core stopped; showing local information",
                )
                .to_owned()
        } else if self.dirty {
            language
                .text("等待重新检测", "Waiting to recheck")
                .to_owned()
        } else if !self.measured {
            String::new()
        } else {
            match language {
                Language::SimplifiedChinese => format!("可访问 {reachable} / {}", TARGETS.len()),
                Language::English => format!("Reachable {reachable} / {}", TARGETS.len()),
            }
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
    fn english_overview_translates_display_without_rewriting_external_values() {
        let host = Some(Ok(HostInfo {
            addresses: vec!["192.0.2.12".parse().unwrap()],
            dns: vec![],
        }));
        let exit = Some(Ok(ip_check::CheckResult {
            state: ip_check::State::Identified,
            summary: "198.51.100.7".into(),
            country: "🇩🇪 德国".into(),
            millis: 42,
            detail: String::new(),
            target_exit: None,
        }));
        let services = [
            Some(Err("超时".into())),
            Some(Err("custom evidence".into())),
        ];
        let view = View {
            running: true,
            busy: false,
            dirty: false,
            measured: true,
            hidden: false,
            host: &host,
            dns: &None,
            exit: &exit,
            services: &services,
        };
        let english = view.present_in(Language::English);
        let chinese = view.present();
        assert_eq!(english.network[0].id, chinese.network[0].id);
        assert_eq!(english.network[0].title, "Local IP");
        assert_eq!(english.network[0].value, "192.0.2.12");
        assert_eq!(english.network[1].detail, "Germany");
        assert_eq!(english.network[1].flag, chinese.network[1].flag);
        assert_eq!(english.services[0].value, "Timed out");
        assert_eq!(english.services[1].value, "custom evidence");
        assert_eq!(english.status, "Reachable 0 / 8");
        assert_eq!(chinese.services[0].value, "超时");
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
