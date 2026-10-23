//! A reversible runtime overlay. Preferences and subscription sources stay intact.
use crate::config::Settings;
use crate::model::ProxyMode;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

pub const OWNED_DOMAINS: &[&str] = &[
    "anthropic.com",
    "claude.ai",
    "claude.com",
    "claude.app",
    "claudeusercontent.com",
    "claudemcpcontent.com",
];
// Candidate/associated domains are routed conservatively, but are never
// attributed to a Claude process merely because another application uses them.
pub const ASSOCIATED_DOMAINS: &[&str] = &[
    "clau.de",
    "claudemcpclient.com",
    "anthropic.auth0.com",
    "anthropic-com.ghost.io",
    "anthropic.com.cdn.cloudflare.net",
    "servd-anthropic-website.b-cdn.net",
    "sentry.io",
    "statsigapi.net",
    "datadoghq.com",
    "browser-intake-us5-datadoghq.com",
    "intercom.io",
    "intercomcdn.com",
    "cdn.usefathom.com",
];
pub const KEYWORDS: &[&str] = &["datadog", "sentry", "sift"];
pub const DIAGNOSTIC_HOSTS: &[&str] = &[
    "api.ipify.org",
    "api6.ipify.org",
    "api.ip.sb",
    "api.ipapi.is",
    "www.cloudflare.com",
    "speed.cloudflare.com",
    "api.globalping.io",
    "rdap.org",
    "arin.net",
    "ripe.net",
    "apnic.net",
    "lacnic.net",
    "afrinic.net",
    "registro.br",
    "nic.ad.jp",
    "nic.or.kr",
    "stun.cloudflare.com",
    "stun.l.google.com",
];
pub const DOH: &[&str] = &[
    "https://1.1.1.1/dns-query",
    "https://8.8.8.8/dns-query",
    "https://[2606:4700:4700::1111]/dns-query",
    "https://[2001:4860:4860::8888]/dns-query",
];
pub const BOOTSTRAP: &[&str] = &[
    "https://223.5.5.5/dns-query#DIRECT",
    "https://223.6.6.6/dns-query#DIRECT",
    // Node bootstrap must survive a blocked resolver and either IP family.
    // Application/public DNS still uses only the pinned protection route.
    "https://1.0.0.1/dns-query#DIRECT",
    "https://[2606:4700:4700::1111]/dns-query#DIRECT",
];

// Only node/DNS-server bootstrap uses these resolvers. Public domain lookups
// always use DOH through the pinned node below. Replacing a working private,
// ISP or subscription resolver with fixed public HTTPS IPs can make the node
// itself unresolvable (and therefore break every proxied DNS request).
fn bootstrap(dns: &mut Mapping, system_dns: &[String]) -> Result<()> {
    let nonempty = |value: &Value| match value {
        Value::String(text) => !text.trim().is_empty(),
        Value::Sequence(items) => !items.is_empty(),
        _ => false,
    };
    let explicit_node_dns = dns
        .get(Value::from("proxy-server-nameserver"))
        .is_some_and(nonempty);
    // Without a dedicated node resolver, mihomo previously used the general
    // DNS policy for node names too. Preserve split DNS for that purpose only.
    if !explicit_node_dns
        && !dns.contains_key(Value::from("proxy-server-nameserver-policy"))
        && let Some(policy) = dns.get(Value::from("nameserver-policy")).cloned()
    {
        dns.insert(Value::from("proxy-server-nameserver-policy"), policy);
    }
    let original = dns
        .get(Value::from("proxy-server-nameserver"))
        .filter(|value| nonempty(value))
        .or_else(|| {
            dns.get(Value::from("nameserver"))
                .filter(|value| nonempty(value))
        })
        .or_else(|| {
            dns.get(Value::from("default-nameserver"))
                .filter(|value| nonempty(value))
        })
        .cloned()
        .unwrap_or(serde_yaml::to_value(BOOTSTRAP)?);
    // A bootstrap lookup must never depend on the node it is trying to find.
    // Keep resolver transports and options, but remove proxy-group selection.
    fn direct(value: Value, system_dns: &[String]) -> Result<Value> {
        match value {
            Value::String(text) => {
                let (address, options) = text.split_once('#').unwrap_or((&text, ""));
                let options: Vec<_> = options
                    .split('&')
                    .filter(|option| option.contains('='))
                    .collect();
                // System/DHCP resolvers have their own native routing. Ordinary
                // IP, UDP, TCP, DoT and DoH all support the #DIRECT parameter.
                if address == "system"
                    || address.starts_with("system://")
                    || address.starts_with("dhcp://")
                {
                    // macOS takes over system DNS for Safari. A dynamic system
                    // resolver would then recurse through the protected node.
                    if (address == "system" || address.starts_with("system://"))
                        && !system_dns.is_empty()
                    {
                        return serde_yaml::to_value(
                            system_dns
                                .iter()
                                .map(|ip| {
                                    let ip: std::net::IpAddr =
                                        ip.parse().expect("validated original DNS");
                                    format!("udp://{}#DIRECT", std::net::SocketAddr::new(ip, 53))
                                })
                                .collect::<Vec<_>>(),
                        )
                        .map_err(Into::into);
                    }
                    return Ok(Value::String(address.to_owned()));
                }
                // Appending #DIRECT before normalizing a bare IPv6 literal
                // prevents mihomo's pure-IP parser from adding the brackets.
                let address = match address.parse::<std::net::IpAddr>() {
                    Ok(std::net::IpAddr::V6(ip)) => format!("udp://[{ip}]"),
                    _ => address.to_owned(),
                };
                let mut routed = format!("{address}#DIRECT");
                for option in options {
                    routed.push('&');
                    routed.push_str(option);
                }
                Ok(Value::String(routed))
            }
            Value::Sequence(items) => {
                let mut routed = Vec::with_capacity(items.len());
                for item in items {
                    match direct(item, system_dns)? {
                        Value::Sequence(values) => routed.extend(values),
                        value => routed.push(value),
                    }
                }
                Ok(Value::Sequence(routed))
            }
            _ => anyhow::bail!("节点 DNS 必须是地址或地址列表"),
        }
    }
    dns.insert(
        Value::from("proxy-server-nameserver"),
        direct(original, system_dns)?,
    );
    if let Some(policy) = dns.get_mut(Value::from("proxy-server-nameserver-policy"))
        && let Some(policy) = policy.as_mapping_mut()
    {
        for value in policy.values_mut() {
            *value = direct(value.clone(), system_dns)?;
        }
    }
    if let Some(default) = dns
        .get(Value::from("default-nameserver"))
        .filter(|value| nonempty(value))
        .cloned()
    {
        dns.insert(
            Value::from("default-nameserver"),
            direct(default, system_dns)?,
        );
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Previous {
    pub mode: String,
    pub proxy_mode: ProxyMode,
    pub selectors: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Protection {
    pub profile_id: String,
    pub node: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    // macOS local-name resolution must keep the pre-takeover resolvers rather
    // than feeding the new system DNS addresses back into the core itself.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub system_dns: Vec<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub locked_groups: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub fixed_groups: std::collections::BTreeMap<String, String>,
    pub route: String,
    pub previous: Previous,
}
impl Protection {
    /// Preserve global routing; a direct-mode start still needs the rule overlay.
    pub fn run_mode(&self) -> &'static str {
        if self.previous.mode == "global" {
            "global"
        } else {
            "rule"
        }
    }

    pub fn new(profile_id: String, node: String, previous: Previous) -> Self {
        Self {
            profile_id,
            node,
            provider: None,
            system_dns: vec![],
            locked_groups: Default::default(),
            fixed_groups: Default::default(),
            route: format!("COR-Claude-{}", uuid::Uuid::new_v4().simple()),
            previous,
        }
    }
    pub fn validate(&self) -> Result<()> {
        uuid::Uuid::parse_str(&self.profile_id)?;
        ensure!(
            self.fixed_groups
                .iter()
                .all(|(group, node)| self.locked_groups.contains_key(group)
                    && !node.is_empty()
                    && node.len() <= 512
                    && !node.chars().any(char::is_control)),
            "Claude 保护的自动组恢复记录无效"
        );
        ensure!(
            self.locked_groups.len() <= 4096
                && self
                    .locked_groups
                    .iter()
                    .all(|(group, node)| !group.is_empty()
                        && !node.is_empty()
                        && group.len() <= 512
                        && node.len() <= 512
                        && !group.chars().any(char::is_control)
                        && !node.chars().any(char::is_control)),
            "Claude 保护自动组锁定记录无效"
        );
        ensure!(
            self.system_dns.len() <= 16
                && self
                    .system_dns
                    .iter()
                    .all(|s| s.parse::<std::net::IpAddr>().is_ok()),
            "Claude 保护原 DNS 记录无效"
        );
        ensure!(
            !self.node.is_empty()
                && self.node.len() <= 512
                && !self.node.chars().any(char::is_control),
            "Claude 保护节点无效"
        );
        ensure!(
            !crate::active_route::terminal(&self.node),
            "请先选择可用的代理节点"
        );
        ensure!(
            self.provider.as_ref().is_none_or(|name| !name.is_empty()
                && name.len() <= 512
                && !name.chars().any(char::is_control)),
            "Claude 保护的代理集合无效"
        );
        ensure!(
            self.route.starts_with("COR-Claude-")
                && self.route.len() == 43
                && self
                    .route
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "Claude 保护线路无效"
        );
        ensure!(
            matches!(self.previous.mode.as_str(), "rule" | "global" | "direct"),
            "Claude 保护恢复模式无效"
        );
        ensure!(
            self.previous.selectors.len() <= 4096
                && self
                    .previous
                    .selectors
                    .iter()
                    .all(|(a, b)| a.len() <= 512 && b.len() <= 512),
            "Claude 保护恢复记录过大"
        );
        Ok(())
    }
    pub fn capture_automatic_groups(&mut self, proxies: &crate::api::Proxies) {
        self.fixed_groups = proxies
            .proxies
            .iter()
            .filter(|(_, p)| {
                matches!(p.kind.as_str(), "URLTest" | "Fallback")
                    && !p.now.is_empty()
                    && !p.fixed.is_empty()
            })
            .map(|(name, p)| (name.to_string(), p.fixed.to_string()))
            .collect();
        self.locked_groups = proxies
            .proxies
            .iter()
            .filter(|(_, proxy)| {
                matches!(proxy.kind.as_str(), "URLTest" | "Fallback" | "LoadBalance")
                    && !proxy.now.is_empty()
            })
            .map(|(name, proxy)| (name.to_string(), proxy.now.to_string()))
            .collect();
    }
}

fn exact_group_pattern(name: &str) -> String {
    // mihomo separates multiple filters with backticks, including literal names.
    format!("^{}$", regex::escape(name).replace('`', r"\x60"))
}

pub fn owned_host(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    OWNED_DOMAINS
        .iter()
        .any(|suffix| domain_matches(host, suffix))
}
pub fn associated_host(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    ASSOCIATED_DOMAINS
        .iter()
        .any(|suffix| domain_matches(host, suffix))
        || KEYWORDS
            .iter()
            .any(|keyword| host.to_ascii_lowercase().contains(keyword))
}
pub fn anthropic_ip(input: &str) -> bool {
    match input.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            let bytes = ip.octets();
            bytes[..2] == [160, 79] && matches!(bytes[2], 104 | 105)
        }
        Ok(std::net::IpAddr::V6(ip)) => ip.octets()[..6] == [0x26, 7, 0x6b, 0xc0, 0, 0],
        _ => false,
    }
}
fn domain_matches(host: &str, suffix: &str) -> bool {
    host.eq_ignore_ascii_case(suffix)
        || (host.len() > suffix.len()
            && host.as_bytes()[host.len() - suffix.len() - 1] == b'.'
            && host.as_bytes()[host.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes()))
}

pub fn apply(map: &mut Mapping, settings: &Settings) -> Result<()> {
    let Some(protection) = &settings.claude_protection else {
        return Ok(());
    };
    protection.validate()?;
    ensure!(
        settings.active_profile.as_deref() == Some(&protection.profile_id),
        "请先关闭 Claude 保护再切换订阅"
    );
    let group = if let Some(provider) = &protection.provider {
        ensure!(
            map.get(Value::from("proxy-providers"))
                .and_then(Value::as_mapping)
                .is_some_and(|providers| providers.contains_key(Value::from(provider.clone()))),
            "保护节点的代理集合已不存在，请关闭保护后重新选择节点"
        );
        serde_json::json!({"name":protection.route,"type":"select","hidden":true,"empty-fallback":"REJECT","use":[provider],"filter":exact_group_pattern(&protection.node)})
    } else {
        serde_json::json!({"name":protection.route,"type":"select","hidden":true,"empty-fallback":"REJECT","proxies":[protection.node]})
    };
    let key = Value::from("proxy-groups");
    let groups = map
        .entry(key)
        .or_insert(Value::Sequence(vec![]))
        .as_sequence_mut()
        .ok_or_else(|| anyhow::anyhow!("订阅代理组格式无效"))?;
    for group in groups.iter_mut() {
        let Some(selected) = group
            .get("name")
            .and_then(Value::as_str)
            .and_then(|name| protection.locked_groups.get(name))
        else {
            continue;
        };
        if !matches!(
            group.get("type").and_then(Value::as_str),
            Some("url-test" | "fallback" | "load-balance")
        ) {
            continue;
        }
        let group = group
            .as_mapping_mut()
            .ok_or_else(|| anyhow::anyhow!("自动组配置无效"))?;
        group.insert("type".into(), "select".into());
        group.insert("filter".into(), exact_group_pattern(selected).into());
        // Raw members skip mihomo's positive provider filter. This exclusion
        // also locks them, while preserving the original provider ownership.
        group.insert(
            "exclude-filter".into(),
            format!(
                "^(?!{})",
                exact_group_pattern(selected).trim_start_matches('^')
            )
            .into(),
        );
        group.remove(Value::from("exclude-type"));
        group.insert("empty-fallback".into(), "REJECT".into());
        group.insert("interval".into(), 0.into());
        group.insert("lazy".into(), true.into());
    }
    ensure!(
        !groups
            .iter()
            .any(|g| g.get("name").and_then(Value::as_str) == Some(&protection.route)),
        "订阅包含同名的保护线路"
    );
    groups.push(serde_yaml::to_value(&group)?);
    if protection.run_mode() == "global" {
        // A one-member GLOBAL survives reloads/cold starts without trusting the
        // core's selection cache. The original group remains in the source YAML
        // and its previous selection is restored when protection is disabled.
        groups.retain(|group| group.get("name").and_then(Value::as_str) != Some("GLOBAL"));
        let mut global = group;
        global["name"] = "GLOBAL".into();
        global["hidden"] = false.into();
        groups.push(serde_yaml::to_value(global)?);
    }

    let original = map
        .remove(Value::from("rules"))
        .unwrap_or(Value::Sequence(vec![]));
    let Value::Sequence(original) = original else {
        anyhow::bail!("订阅规则格式无效");
    };
    let mut rules = Vec::with_capacity(original.len() + 100);
    // The higher-priority overlay replaces only equal predicates in the runtime
    // copy. The subscription and user rule identities are never rewritten.
    for domain in OWNED_DOMAINS.iter().chain(ASSOCIATED_DOMAINS) {
        route_or_reject(&mut rules, "DOMAIN-SUFFIX", domain, protection, false);
    }
    for keyword in KEYWORDS {
        route_or_reject(&mut rules, "DOMAIN-KEYWORD", keyword, protection, false);
    }
    for host in DIAGNOSTIC_HOSTS {
        route_or_reject(&mut rules, "DOMAIN", host, protection, false);
    }
    for (kind, payload) in [
        ("IP-CIDR", "160.79.104.0/23"),
        ("IP-CIDR6", "2607:6bc0::/48"),
    ] {
        route_or_reject(&mut rules, kind, payload, protection, true);
    }
    let overlay_count = rules.len();
    for original in original {
        let duplicate = original
            .as_str()
            .and_then(|s| crate::rule_manager::parts(s).ok())
            .is_some_and(|old| {
                rules[..overlay_count]
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(|s| crate::rule_manager::parts(s).ok())
                    .any(|new| {
                        new.kind == old.kind && new.payload.eq_ignore_ascii_case(old.payload)
                    })
            });
        if !duplicate {
            rules.push(original);
        }
    }
    map.insert(Value::from("rules"), Value::Sequence(rules));
    map.insert(Value::from("mode"), Value::from(protection.run_mode()));
    map.insert(Value::from("ipv6"), Value::from(true));

    // Keep a known DNS/proxy hostname authoritative. Subscription force-sniff
    // settings can otherwise replace claude.ai with a shared CDN/ECH outer
    // name before domain rules run. Bare-IP clients still need HTTP/SNI/QUIC
    // discovery, e.g. when reusing a browser's pre-protection DNS cache.
    map.insert(
        Value::from("sniffer"),
        serde_yaml::from_str::<Value>(
            "enable: true\nforce-dns-mapping: false\nparse-pure-ip: true\noverride-destination: false\nsniff:\n  HTTP: {ports: [80, 8080-8880], override-destination: false}\n  TLS: {ports: [443, 8443], override-destination: false}\n  QUIC: {ports: [443, 8443], override-destination: false}\nforce-domain: []\nskip-domain: ['+.lan', '+.local', localhost, '+.home.arpa']\n",
        )?,
    );

    let dns = map
        .get_mut(Value::from("dns"))
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| anyhow::anyhow!("DNS 配置缺失"))?;
    bootstrap(dns, &protection.system_dns)?;
    for key in [
        "nameserver-policy",
        "fallback",
        "fallback-filter",
        "direct-nameserver",
        "direct-nameserver-follow-policy",
        "prefer-h3",
    ] {
        dns.remove(Value::from(key));
    }
    dns.insert(Value::from("enable"), Value::from(true));
    dns.insert(Value::from("ipv6"), Value::from(true));
    dns.insert(Value::from("enhanced-mode"), Value::from("fake-ip"));
    dns.insert(Value::from("respect-rules"), Value::from(false));
    dns.insert(
        Value::from("nameserver"),
        serde_yaml::to_value(
            DOH.iter()
                .map(|s| format!("{s}#{}", protection.route))
                .collect::<Vec<_>>(),
        )?,
    );
    // Private names retain local resolution. These explicit exceptions are
    // shown to the user; no public failure falls back to the system resolver.
    dns.insert(Value::from("nameserver-policy"), serde_yaml::from_str::<Mapping>("'+.lan': system\n'+.local': system\nlocalhost: system\n'+.home.arpa': system\n'+.in-addr.arpa': system\n'+.ip6.arpa': system\n")?.into());
    #[cfg(target_os = "macos")]
    if !protection.system_dns.is_empty() {
        let local: Vec<_> = protection
            .system_dns
            .iter()
            .map(|ip| {
                let ip: std::net::IpAddr = ip.parse().expect("validated original DNS");
                format!("udp://{}#DIRECT", std::net::SocketAddr::new(ip, 53))
            })
            .collect();
        let policy = dns
            .get_mut(Value::from("nameserver-policy"))
            .unwrap()
            .as_mapping_mut()
            .unwrap();
        for value in policy.values_mut() {
            *value = serde_yaml::to_value(&local)?;
        }
        // config::dns adds local/probe real-IP exceptions after this overlay,
        // preserving blacklist, whitelist and ordered rule semantics.
    }
    let tun = map
        .get_mut(Value::from("tun"))
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| anyhow::anyhow!("TUN 配置缺失"))?;
    tun.insert(Value::from("strict-route"), Value::from(true));
    tun.insert(
        Value::from("dns-hijack"),
        serde_yaml::to_value(["any:53", "tcp://any:53"])?,
    );
    for key in [
        "route-address",
        "inet4-route-address",
        "inet6-route-address",
        "route-address-set",
        "route-exclude-address",
        "inet4-route-exclude-address",
        "inet6-route-exclude-address",
        "route-exclude-address-set",
        "include-interface",
        "exclude-interface",
        "include-uid",
        "include-uid-range",
        "exclude-uid",
        "exclude-uid-range",
    ] {
        tun.remove(Value::from(key));
    }
    Ok(())
}

fn route_or_reject(
    rules: &mut Vec<Value>,
    kind: &str,
    payload: &str,
    protection: &Protection,
    no_resolve: bool,
) {
    // Mihomo SKIPS a matching rule if the selected adapter does not support
    // UDP. Without an adjacent fallback, QUIC can reach subscription DIRECT
    // rules or even the core's implicit DIRECT default (with an empty rule).
    // Preserve working UDP; reject only when the protected route is skipped.
    let suffix = if no_resolve { ",no-resolve" } else { "" };
    for route in [protection.route.as_str(), "REJECT"] {
        rules.push(Value::from(format!("{kind},{payload},{route}{suffix}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protected_mode_and_global_pin_survive_restart_without_rewriting_the_source() {
        let raw = "proxies: [{name: Node, type: socks5, server: 127.0.0.1, port: 1080}]\nproxy-groups: [{name: GLOBAL, type: select, proxies: [DIRECT, Node]}]\nrules: ['MATCH,DIRECT']\n";
        for mode in ["rule", "global", "direct"] {
            let id = uuid::Uuid::new_v4().to_string();
            let mut settings = Settings {
                active_profile: Some(id.clone()),
                run_mode: mode.into(),
                ..Default::default()
            };
            let original = crate::config::runtime_config(raw, &settings).unwrap();
            settings.claude_protection = Some(std::sync::Arc::new(Protection::new(
                id,
                "Node".into(),
                Previous {
                    mode: mode.into(),
                    proxy_mode: ProxyMode::Off,
                    selectors: vec![("GLOBAL".into(), "Node".into())],
                },
            )));
            let protected = crate::config::runtime_config(raw, &settings).unwrap();
            let map: Value = serde_yaml::from_str(&protected).unwrap();
            assert_eq!(
                map["mode"],
                if mode == "global" { "global" } else { "rule" }
            );
            let globals: Vec<_> = map["proxy-groups"]
                .as_sequence()
                .unwrap()
                .iter()
                .filter(|group| group["name"] == "GLOBAL")
                .collect();
            assert_eq!(globals.len(), 1);
            let members = globals[0]["proxies"].as_sequence().unwrap();
            assert_eq!(members.len(), if mode == "global" { 1 } else { 2 });
            if mode == "global" {
                assert_eq!(members[0], "Node");
                assert_eq!(globals[0]["empty-fallback"], "REJECT");
            }
            let mut reopened: Settings =
                serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
            assert_eq!(
                crate::config::runtime_config(raw, &reopened).unwrap(),
                protected
            );
            reopened.claude_protection = None;
            assert_eq!(
                crate::config::runtime_config(raw, &reopened).unwrap(),
                original
            );
        }
    }

    #[test]
    fn automatic_groups_stay_pinned_after_restart_and_restore_on_disable() {
        let id = uuid::Uuid::new_v4().to_string();
        let raw = "proxies: [{name: 'node.[1]`x', type: socks5, server: 127.0.0.1, port: 1080}, {name: Other, type: socks5, server: 127.0.0.1, port: 1081}]\nproxy-providers: {Feed: {type: file, path: providers/feed.yaml}}\nproxy-groups: [{name: Auto, type: url-test, url: 'https://health.example/check', proxies: ['node.[1]`x', Other], use: [Feed], tolerance: 80, interval: 300}]\nrules: ['MATCH,Auto']\n";
        let mut settings = Settings {
            active_profile: Some(id.clone()),
            ..Default::default()
        };
        let before = crate::config::runtime_config(raw, &settings).unwrap();
        let proxies = serde_json::from_value::<crate::api::Proxies>(serde_json::json!({"proxies":{
            "Auto":{"type":"URLTest","now":"node.[1]`x","fixed":"Other","all":["node.[1]`x","Other"]},
            "Main":{"type":"Selector","now":"Auto","all":["Auto"]}}}))
        .unwrap();
        let mut protection = Protection::new(
            id,
            "node.[1]`x".into(),
            Previous {
                mode: "rule".into(),
                proxy_mode: ProxyMode::Off,
                selectors: vec![],
            },
        );
        protection.capture_automatic_groups(&proxies);
        assert_eq!(protection.locked_groups.len(), 1);
        settings.claude_protection = Some(std::sync::Arc::new(protection));
        let mut restored: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert_eq!(
            restored.claude_protection.as_ref().unwrap().fixed_groups["Auto"],
            "Other"
        );
        let protected: Value =
            serde_yaml::from_str(&crate::config::runtime_config(raw, &restored).unwrap()).unwrap();
        let group = &protected["proxy-groups"][0];
        assert_eq!(group["type"], "select");
        assert_eq!(group["empty-fallback"], "REJECT");
        assert_eq!(group["filter"], r"^node\.\[1\]\x60x$");
        assert_eq!(group["exclude-filter"], r"^(?!node\.\[1\]\x60x$)");
        assert_eq!(group["use"][0], "Feed");
        assert_eq!(group["proxies"][1], "Other");
        restored.claude_protection = None;
        assert_eq!(
            crate::config::runtime_config(raw, &restored).unwrap(),
            before
        );
    }
    #[test]
    fn bootstrap_preserves_working_node_resolvers_without_a_proxy_dependency() {
        let mut dns: Mapping = serde_yaml::from_str("nameserver: ['https://192.0.2.2/dns-query#Main']\ndefault-nameserver: ['192.0.2.53']\nproxy-server-nameserver: ['tls://192.0.2.54#Main&ecs=192.0.2.1', 'system']\nproxy-server-nameserver-policy: {'+.node.test': 'udp://192.0.2.55#Main'}\n").unwrap();
        bootstrap(&mut dns, &[]).unwrap();
        assert_eq!(
            dns["proxy-server-nameserver"],
            serde_yaml::to_value(["tls://192.0.2.54#DIRECT&ecs=192.0.2.1", "system"]).unwrap()
        );
        assert_eq!(
            dns["proxy-server-nameserver-policy"]["+.node.test"],
            "udp://192.0.2.55#DIRECT"
        );
        assert_eq!(dns["default-nameserver"][0], "192.0.2.53#DIRECT");
        dns.remove(Value::from("proxy-server-nameserver"));
        dns.remove(Value::from("default-nameserver"));
        bootstrap(&mut dns, &[]).unwrap();
        assert_eq!(
            dns["proxy-server-nameserver"][0],
            "https://192.0.2.2/dns-query#DIRECT"
        );
        // Missing default-nameserver must retain the core's normal bootstrap.
        // Otherwise a previously working DoH hostname now depends on the same
        // unreachable fixed HTTPS IPs that caused this regression.
        assert!(!dns.contains_key(Value::from("default-nameserver")));
        let mut dns: Mapping =
            serde_yaml::from_str("default-nameserver: ['2001:db8::53']\n").unwrap();
        bootstrap(&mut dns, &[]).unwrap();
        assert_eq!(
            dns["proxy-server-nameserver"][0],
            "udp://[2001:db8::53]#DIRECT"
        );
        assert_eq!(dns["default-nameserver"][0], "udp://[2001:db8::53]#DIRECT");
        let mut dns: Mapping = serde_yaml::from_str(
            "proxy-server-nameserver: [system]\ndefault-nameserver: [system]\n",
        )
        .unwrap();
        bootstrap(&mut dns, &["192.0.2.53".into(), "2001:db8::53".into()]).unwrap();
        let captured = serde_yaml::to_value([
            "udp://192.0.2.53:53#DIRECT",
            "udp://[2001:db8::53]:53#DIRECT",
        ])
        .unwrap();
        assert_eq!(dns["proxy-server-nameserver"], captured);
        assert_eq!(dns["default-nameserver"], captured);
        let mut dns: Mapping = serde_yaml::from_str("nameserver: ['https://192.0.2.2/dns-query']\nnameserver-policy: {'+.node.test': '192.0.2.53'}\n").unwrap();
        bootstrap(&mut dns, &[]).unwrap();
        assert_eq!(
            dns["proxy-server-nameserver-policy"]["+.node.test"],
            "192.0.2.53#DIRECT"
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_dns_overlay_keeps_local_names_out_of_the_new_system_resolver() {
        for mode in ["blacklist", "whitelist", "rule"] {
            let id = uuid::Uuid::new_v4().to_string();
            let raw = format!(
                "proxies: [{{name: Node, type: socks5, server: 127.0.0.1, port: 1080}}]\ndns: {{fake-ip-filter-mode: {mode}, fake-ip-filter: []}}\nrules: ['MATCH,DIRECT']\n"
            );
            let mut settings = Settings {
                active_profile: Some(id.clone()),
                ..Default::default()
            };
            let before = crate::config::runtime_config(&raw, &settings).unwrap();
            let mut protection = Protection::new(
                id,
                "Node".into(),
                Previous {
                    mode: "rule".into(),
                    proxy_mode: ProxyMode::Off,
                    selectors: vec![],
                },
            );
            protection.system_dns = vec!["192.168.1.1".into(), "fd00::1".into()];
            settings.claude_protection = Some(std::sync::Arc::new(protection));
            let value: Value =
                serde_yaml::from_str(&crate::config::runtime_config(&raw, &settings).unwrap())
                    .unwrap();
            assert_eq!(
                value["dns"]["nameserver-policy"]["+.lan"],
                serde_yaml::to_value(["udp://192.168.1.1:53#DIRECT", "udp://[fd00::1]:53#DIRECT"])
                    .unwrap()
            );
            let probe = if value["dns"]["fake-ip-filter-mode"] == "rule" {
                "DOMAIN-SUFFIX,bash.ws,real-ip"
            } else {
                "+.bash.ws"
            };
            assert!(
                value["dns"]["fake-ip-filter"]
                    .as_sequence()
                    .unwrap()
                    .contains(&Value::from(probe))
            );
            let mut restarted: Settings =
                serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
            restarted.claude_protection = None;
            assert_eq!(
                crate::config::runtime_config(&raw, &restarted).unwrap(),
                before
            );
        }
    }
    #[test]
    fn provider_node_is_pinned_by_exact_name_without_rewriting_source() {
        let id = uuid::Uuid::new_v4().to_string();
        let raw = "proxy-providers: {Feed: {type: file, path: providers/feed.yaml}}\nproxy-groups: [{name: Main, type: select, use: [Feed]}]\nrules: ['MATCH,Main']\n";
        let mut settings = Settings {
            active_profile: Some(id.clone()),
            ..Default::default()
        };
        let before = crate::config::runtime_config(raw, &settings).unwrap();
        let mut protection = Protection::new(
            id,
            "Node (A)+".into(),
            Previous {
                mode: "rule".into(),
                proxy_mode: ProxyMode::Off,
                selectors: vec![],
            },
        );
        protection.provider = Some("Feed".into());
        settings.claude_protection = Some(std::sync::Arc::new(protection));
        let value: Value =
            serde_yaml::from_str(&crate::config::runtime_config(raw, &settings).unwrap()).unwrap();
        let group = value["proxy-groups"].as_sequence().unwrap().last().unwrap();
        assert_eq!(group["use"][0], "Feed");
        assert_eq!(group["hidden"], true);
        assert_eq!(group["empty-fallback"], "REJECT");
        let filter = regex::Regex::new(group["filter"].as_str().unwrap()).unwrap();
        assert!(filter.is_match("Node (A)+"));
        assert!(!filter.is_match("Node AAA") && !filter.is_match("Other Node (A)+"));
        std::sync::Arc::make_mut(settings.claude_protection.as_mut().unwrap())
            .previous
            .mode = "global".into();
        let value: Value =
            serde_yaml::from_str(&crate::config::runtime_config(raw, &settings).unwrap()).unwrap();
        let global = value["proxy-groups"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|group| group["name"] == "GLOBAL")
            .unwrap();
        assert_eq!(value["mode"], "global");
        assert_eq!(global["use"][0], "Feed");
        assert_eq!(global["empty-fallback"], "REJECT");
        assert!(
            regex::Regex::new(global["filter"].as_str().unwrap())
                .unwrap()
                .is_match("Node (A)+")
        );
        let mut restarted: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert_eq!(restarted.claude_protection, settings.claude_protection);
        restarted.claude_protection = None;
        assert_eq!(
            crate::config::runtime_config(raw, &restarted).unwrap(),
            before
        );
    }
    #[test]
    fn overlay_does_not_change_preferences_and_can_be_removed_exactly() {
        let id = uuid::Uuid::new_v4().to_string();
        let mut settings = Settings {
            active_profile: Some(id.clone()),
            dns_override: false,
            ipv6: false,
            run_mode: "global".into(),
            proxy_mode: ProxyMode::System,
            ..Default::default()
        };
        let raw = "proxies: [{name: 'node,&', type: socks5, server: 127.0.0.1, port: 1080}]\ndns: {enable: false, nameserver: [9.9.9.9], fallback: [8.8.8.8], nameserver-policy: {'+.google.com': 114.114.114.114}, direct-nameserver: [system], fake-ip-filter: ['*.lan']}\ntun: {route-address: [203.0.113.0/24], dns-hijack: []}\nsniffer: {enable: false, force-dns-mapping: true, parse-pure-ip: false, force-domain: ['+.claude.ai'], skip-domain: ['+.*']}\nrules: ['DOMAIN-SUFFIX,claude.ai,DIRECT', 'MATCH,DIRECT']\n";
        let before = crate::config::runtime_config(raw, &settings).unwrap();
        settings.claude_protection = Some(std::sync::Arc::new(Protection::new(
            id,
            "node,&".into(),
            Previous {
                mode: "global".into(),
                proxy_mode: ProxyMode::System,
                selectors: vec![],
            },
        )));
        let saved = serde_json::to_vec(&settings).unwrap();
        let protected = crate::config::runtime_config(raw, &settings).unwrap();
        let map: Value = serde_yaml::from_str(&protected).unwrap();
        assert_eq!(map["mode"], "global");
        assert_eq!(map["ipv6"], true);
        assert_eq!(map["sniffer"]["enable"], true);
        assert_eq!(map["sniffer"]["parse-pure-ip"], true);
        assert_eq!(map["sniffer"]["force-dns-mapping"], false);
        assert_eq!(map["sniffer"]["override-destination"], false);
        assert!(
            map["sniffer"]["force-domain"]
                .as_sequence()
                .unwrap()
                .is_empty()
        );
        for protocol in ["HTTP", "TLS", "QUIC"] {
            assert!(
                !map["sniffer"]["sniff"][protocol]["ports"]
                    .as_sequence()
                    .unwrap()
                    .is_empty()
            );
        }
        {
            let route = settings.claude_protection.as_ref().unwrap().route.as_str();
            let addresses: Vec<std::net::IpAddr> = map["dns"]["nameserver"]
                .as_sequence()
                .unwrap()
                .iter()
                .map(|value| {
                    let url = reqwest::Url::parse(value.as_str().unwrap()).unwrap();
                    assert_eq!(url.scheme(), "https");
                    assert_eq!(url.fragment(), Some(route));
                    url.host_str()
                        .unwrap()
                        .trim_matches(['[', ']'])
                        .parse()
                        .unwrap()
                })
                .collect();
            assert!(addresses.iter().any(std::net::IpAddr::is_ipv4));
            assert!(addresses.iter().any(std::net::IpAddr::is_ipv6));
        }
        assert!(map["dns"]["default-nameserver"].is_null());
        assert_eq!(map["dns"]["proxy-server-nameserver"][0], "9.9.9.9#DIRECT");
        assert_eq!(map["tun"]["dns-hijack"].as_sequence().unwrap().len(), 2);
        assert!(map["tun"]["route-address"].is_null());
        assert!(map["dns"]["fallback"].is_null());
        assert!(map["dns"]["direct-nameserver"].is_null());
        assert!(map["dns"]["nameserver-policy"]["+.google.com"].is_null());
        let rules = map["rules"].as_sequence().unwrap();
        let overlay: Vec<_> = rules
            .iter()
            .map(|v| v.as_str().unwrap())
            .take_while(|rule| *rule != "MATCH,DIRECT")
            .collect();
        assert_eq!(overlay.len() % 2, 0);
        assert!(!overlay.is_empty());
        for pair in overlay.as_chunks::<2>().0 {
            assert_eq!(
                pair[0].replace(
                    &settings.claude_protection.as_ref().unwrap().route,
                    "REJECT"
                ),
                pair[1],
                "every protected predicate needs an immediate fail-closed fallback"
            );
        }
        assert_eq!(
            overlay
                .iter()
                .filter(|rule| rule.starts_with("DOMAIN-SUFFIX,claude.ai,"))
                .count(),
            2
        );
        assert!(
            !rules
                .iter()
                .any(|v| v.as_str().unwrap().contains("category-ntp"))
        );
        assert_eq!(serde_json::to_vec(&settings).unwrap(), saved);
        // The restart loads the same restoration record, rather than taking
        // a new snapshot of the already protected state.
        let mut restarted: Settings = serde_json::from_slice(&saved).unwrap();
        assert_eq!(restarted.claude_protection, settings.claude_protection);
        restarted.claude_protection = None;
        assert_eq!(
            crate::config::runtime_config(raw, &restarted).unwrap(),
            before
        );
        assert!(!restarted.dns_override && !restarted.ipv6);
        assert_eq!(restarted.run_mode, "global");
        assert_eq!(restarted.proxy_mode, ProxyMode::System);
    }
    #[test]
    fn owned_and_shared_services_are_distinguished() {
        assert!(owned_host("bridge.claudeusercontent.com"));
        assert!(owned_host("preview.claude.app."));
        assert!(!owned_host("claude.ai.example.org"));
        assert!(!owned_host("sentry.io"));
        assert!(associated_host("shared.sentry.io"));
        assert!(associated_host("example-datadog.test"));
    }
    #[test]
    fn interrupted_enable_and_disable_recover_the_correct_persisted_snapshot() {
        use crate::config::{Store, atomic_write};
        use crate::profile_transaction::{Transaction, recover};
        for initially_enabled in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let store = Store::at(temp.path().to_owned()).unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let protection = std::sync::Arc::new(Protection::new(
                id.clone(),
                "node".into(),
                Previous {
                    mode: "global".into(),
                    proxy_mode: ProxyMode::System,
                    selectors: vec![("Main".into(), "original".into())],
                },
            ));
            let original = Settings {
                active_profile: Some(id),
                claude_protection: initially_enabled.then_some(protection.clone()),
                ..Default::default()
            };
            store.save_settings(&original).unwrap();
            atomic_write(&store.runtime().join("config.yaml"), b"old runtime\n").unwrap();
            let mut candidate = original.clone();
            candidate.claude_protection = (!initially_enabled).then_some(protection);
            let mut transaction = Transaction::new(&store.root).unwrap();
            transaction
                .stage(
                    &store.root.join("settings.json"),
                    &serde_json::to_vec(&candidate).unwrap(),
                )
                .unwrap();
            transaction
                .stage(&store.runtime().join("config.yaml"), b"new runtime\n")
                .unwrap();
            transaction.prepare().unwrap();
            // Crash after a partial write, before the transaction is finished.
            atomic_write(
                &store.root.join("settings.json"),
                &serde_json::to_vec(&candidate).unwrap(),
            )
            .unwrap();
            atomic_write(&store.runtime().join("config.yaml"), b"new runtime\n").unwrap();
            drop(transaction);
            assert!(recover(&store.root).unwrap());
            assert_eq!(
                serde_json::to_value(store.load_settings().unwrap()).unwrap(),
                serde_json::to_value(original).unwrap()
            );
            assert_eq!(
                std::fs::read(store.runtime().join("config.yaml")).unwrap(),
                b"old runtime\n"
            );
            assert!(!recover(&store.root).unwrap());
        }
    }
}
