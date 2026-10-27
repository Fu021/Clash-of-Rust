//! Read native resolver scopes, including /etc/resolver and Network Extensions.
use crate::claude_check::protection::SplitDns;
use anyhow::{Result, ensure};
use std::{collections::BTreeMap, net::IpAddr, process::Command};

#[derive(Default)]
struct Resolver {
    domains: Vec<String>,
    match_domains: Vec<String>,
    servers: Vec<IpAddr>,
    interface: Option<String>,
}

pub(crate) struct Snapshot {
    pub split_dns: Vec<SplitDns>,
    pub interface: Option<String>,
    pub protected_conflict: bool,
}

fn parse(text: &str) -> Result<Snapshot> {
    fn append(
        resolver: Resolver,
        result: &mut BTreeMap<String, SplitDns>,
        fallback_interface: &mut Option<String>,
        protected_conflict: &mut bool,
    ) -> Result<()> {
        if !resolver.servers.is_empty() {
            *protected_conflict |= resolver.match_domains.iter().any(|domain| {
                let domain = domain.trim_end_matches('.').to_ascii_lowercase();
                !domain.is_empty() && crate::claude_check::protection::protected_dns_split(&domain)
            });
        }
        let mut servers = Vec::new();
        for ip in resolver.servers {
            if matches!(
                ip.to_string().as_str(),
                "100.100.100.100" | "fd7a:115c:a1e0::53"
            ) && !servers.contains(&ip)
            {
                servers.push(ip);
            }
        }
        if !servers.is_empty() && resolver.interface.is_some() {
            *fallback_interface = resolver.interface.clone();
        }
        if servers.is_empty() {
            return Ok(());
        }
        for domain in resolver.domains {
            if domain.is_empty() || domain == "." {
                continue;
            }
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            let split = SplitDns {
                domain,
                servers: servers.clone(),
                interface: resolver.interface.clone(),
            };
            ensure!(split.valid(), "Tailscale 分域 DNS 格式无效，无法安全接管");
            result
                .entry(split.domain.clone())
                .and_modify(|old| {
                    if old.interface.is_none() && split.interface.is_some() {
                        *old = split.clone();
                    }
                })
                .or_insert(split);
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    let mut resolver = Resolver::default();
    let mut fallback_interface = None;
    let mut protected_conflict = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("resolver #") || line.starts_with("DNS configuration") {
            append(
                std::mem::take(&mut resolver),
                &mut result,
                &mut fallback_interface,
                &mut protected_conflict,
            )?;
        } else if let Some((key, value)) = line.split_once(" : ") {
            let value = value.trim();
            match key.trim() {
                "domain" => {
                    resolver.domains.push(value.into());
                    resolver.match_domains.push(value.into());
                }
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
    append(
        resolver,
        &mut result,
        &mut fallback_interface,
        &mut protected_conflict,
    )?;
    for resolver in result.values_mut() {
        if resolver.interface.is_none() {
            resolver.interface = fallback_interface.clone();
        }
    }
    ensure!(result.len() <= 128, "Tailscale 分域 DNS 超过安全上限");
    Ok(Snapshot {
        split_dns: result.into_values().collect(),
        interface: fallback_interface,
        protected_conflict,
    })
}

pub(crate) fn snapshot() -> Result<Snapshot> {
    let output = Command::new("/usr/sbin/scutil").arg("--dns").output()?;
    ensure!(output.status.success(), "无法读取 macOS 分域 DNS 配置");
    ensure!(output.stdout.len() <= 1024 * 1024, "macOS DNS 配置过大");
    parse(std::str::from_utf8(&output.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_magic_and_split_dns_without_promoting_the_vpn_default() {
        let text = "DNS configuration\n\
resolver #1\n search domain[0] : tail-fixture.ts.net\n nameserver[0] : 100.100.100.100\n nameserver[1] : 100.100.100.100\n nameserver[2] : fd7a:115c:a1e0::53\n if_index : 22 (utun4)\n\
resolver #2\n nameserver[0] : 192.0.2.53\n if_index : 14 (en0)\n\
resolver #3\n domain : tail-fixture.ts.net.\n nameserver[0] : 100.100.100.100\n if_index : 22 (utun4)\n\
resolver #4\n domain : corp.fixture.test\n nameserver[0] : 100.100.100.100\n\
resolver #5\n domain : local\n options : mdns\n\
DNS configuration (for scoped queries)\n\
resolver #1\n nameserver[0] : 192.0.2.53\n if_index : 14 (en0)\n";
        let snapshot = parse(text).unwrap();
        assert_eq!(snapshot.interface.as_deref(), Some("utun4"));
        assert!(!snapshot.protected_conflict);
        let parsed = snapshot.split_dns;
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].domain, "corp.fixture.test");
        assert_eq!(parsed[0].interface.as_deref(), Some("utun4"));
        assert_eq!(parsed[1].domain, "tail-fixture.ts.net");
        assert_eq!(parsed[1].servers.len(), 2);
        assert_eq!(parsed[1].interface.as_deref(), Some("utun4"));
        let global =
            parse("resolver #1\n nameserver[0] : 100.100.100.100\n if_index : 22 (utun4)\n")
                .unwrap();
        assert!(global.split_dns.is_empty());
        assert_eq!(global.interface.as_deref(), Some("utun4"));
    }

    #[test]
    fn rejects_too_many_private_domains_instead_of_silently_dropping_them() {
        let text = (0..129)
            .map(|index| {
                format!(
                    "resolver #{index}\n domain : host{index}.fixture.test\n nameserver[0] : 100.100.100.100\n if_index : 22 (utun4)\n"
                )
            })
            .collect::<String>();
        assert!(parse(&text).is_err());
    }

    #[test]
    fn keeps_single_label_private_domains_and_rejects_unrepresentable_ones() {
        let text = "resolver #1\n domain : internal\n nameserver[0] : 100.100.100.100\n if_index : 22 (utun4)\n";
        let snapshot = parse(text).unwrap();
        assert_eq!(snapshot.split_dns[0].domain, "internal");
        let invalid = "resolver #1\n domain : invalid domain\n nameserver[0] : 100.100.100.100\n";
        assert!(parse(invalid).is_err());
    }

    #[test]
    fn detects_protected_matches_for_all_dns_servers_not_only_quad100() {
        let other_vpn = "resolver #1\n domain : CLAUDE.AI.\n nameserver[0] : 192.0.2.53\n if_index : 22 (utun4)\n";
        assert!(parse(other_vpn).unwrap().protected_conflict);
        let parent = "resolver #1\n domain : COM.\n nameserver[0] : 2001:db8::53\n";
        assert!(parse(parent).unwrap().protected_conflict);
        let search_only =
            "resolver #1\n search domain[0] : claude.ai\n nameserver[0] : 192.0.2.53\n";
        assert!(!parse(search_only).unwrap().protected_conflict);
        let no_servers = "resolver #1\n domain : claude.ai\n options : mdns\n";
        assert!(!parse(no_servers).unwrap().protected_conflict);
        let private = "resolver #1\n domain : corp.example\n nameserver[0] : 192.0.2.53\n";
        assert!(!parse(private).unwrap().protected_conflict);
    }
}
