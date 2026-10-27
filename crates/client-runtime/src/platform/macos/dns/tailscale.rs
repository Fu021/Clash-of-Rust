//! Read native resolver scopes, including /etc/resolver and Network Extensions.
use crate::claude_check::protection::SplitDns;
use anyhow::{Result, ensure};
use std::{collections::BTreeMap, net::IpAddr, process::Command};

#[derive(Default)]
struct Resolver {
    domains: Vec<String>,
    servers: Vec<IpAddr>,
    interface: Option<String>,
}

pub struct Snapshot {
    pub split_dns: Vec<SplitDns>,
    pub interface: Option<String>,
}

fn parse(text: &str) -> Snapshot {
    fn append(
        resolver: Resolver,
        result: &mut BTreeMap<String, SplitDns>,
        fallback_interface: &mut Option<String>,
    ) {
        let servers: Vec<_> = resolver
            .servers
            .into_iter()
            .filter(|ip| {
                matches!(
                    ip.to_string().as_str(),
                    "100.100.100.100" | "fd7a:115c:a1e0::53"
                )
            })
            .collect();
        if !servers.is_empty() && resolver.interface.is_some() {
            *fallback_interface = resolver.interface.clone();
        }
        for domain in resolver.domains {
            let split = SplitDns {
                domain: domain.trim_end_matches('.').to_ascii_lowercase(),
                servers: servers.clone(),
                interface: resolver.interface.clone(),
            };
            if split.valid() {
                result
                    .entry(split.domain.clone())
                    .and_modify(|old| {
                        if old.interface.is_none() && split.interface.is_some() {
                            *old = split.clone();
                        }
                    })
                    .or_insert(split);
            }
        }
    }
    let mut result = BTreeMap::new();
    let mut resolver = Resolver::default();
    let mut fallback_interface = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("resolver #") || line.starts_with("DNS configuration") {
            append(
                std::mem::take(&mut resolver),
                &mut result,
                &mut fallback_interface,
            );
        } else if let Some((key, value)) = line.split_once(" : ") {
            let value = value.trim();
            match key.trim() {
                "domain" => resolver.domains.push(value.into()),
                key if key.starts_with("search domain[") => resolver.domains.push(value.into()),
                key if key.starts_with("nameserver[") => {
                    if let Ok(ip) = value.parse() {
                        resolver.servers.push(ip);
                    }
                }
                "if_index" => {
                    resolver.interface = value
                        .split_once('(')
                        .map(|(_, name)| name.trim_end_matches(')').into());
                }
                _ => {}
            }
        }
    }
    append(resolver, &mut result, &mut fallback_interface);
    for resolver in result.values_mut() {
        if resolver.interface.is_none() {
            resolver.interface = fallback_interface.clone();
        }
    }
    Snapshot {
        split_dns: result.into_values().take(128).collect(),
        interface: fallback_interface,
    }
}

pub fn snapshot() -> Result<Snapshot> {
    let output = Command::new("/usr/sbin/scutil").arg("--dns").output()?;
    ensure!(output.status.success(), "无法读取 macOS 分域 DNS 配置");
    ensure!(output.stdout.len() <= 1024 * 1024, "macOS DNS 配置过大");
    Ok(parse(std::str::from_utf8(&output.stdout)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_magic_and_split_dns_without_promoting_the_vpn_default() {
        let text = "DNS configuration\n\
resolver #1\n search domain[0] : tail-fixture.ts.net\n nameserver[0] : 100.100.100.100\n nameserver[1] : fd7a:115c:a1e0::53\n if_index : 22 (utun4)\n\
resolver #2\n nameserver[0] : 192.0.2.53\n if_index : 14 (en0)\n\
resolver #3\n domain : tail-fixture.ts.net.\n nameserver[0] : 100.100.100.100\n if_index : 22 (utun4)\n\
resolver #4\n domain : corp.fixture.test\n nameserver[0] : 100.100.100.100\n\
resolver #5\n domain : local\n options : mdns\n\
DNS configuration (for scoped queries)\n\
resolver #1\n nameserver[0] : 192.0.2.53\n if_index : 14 (en0)\n";
        let snapshot = parse(text);
        assert_eq!(snapshot.interface.as_deref(), Some("utun4"));
        let parsed = snapshot.split_dns;
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].domain, "corp.fixture.test");
        assert_eq!(parsed[0].interface.as_deref(), Some("utun4"));
        assert_eq!(parsed[1].domain, "tail-fixture.ts.net");
        assert_eq!(parsed[1].servers.len(), 2);
        assert_eq!(parsed[1].interface.as_deref(), Some("utun4"));
        let global =
            parse("resolver #1\n nameserver[0] : 100.100.100.100\n if_index : 22 (utun4)\n");
        assert!(global.split_dns.is_empty());
        assert_eq!(global.interface.as_deref(), Some("utun4"));
    }
}
