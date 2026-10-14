//! A reversible runtime overlay. Preferences and subscription sources stay intact.
use crate::config::Settings;
use crate::proxy::ProxyMode;
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
pub const DOH: &[&str] = &["https://1.1.1.1/dns-query", "https://8.8.8.8/dns-query"];
pub const BOOTSTRAP: &[&str] = &[
    "https://223.5.5.5/dns-query#DIRECT",
    "https://223.6.6.6/dns-query#DIRECT",
];

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
    pub route: String,
    pub previous: Previous,
}
impl Protection {
    pub fn new(profile_id: String, node: String, previous: Previous) -> Self {
        Self {
            profile_id,
            node,
            route: format!("COR-Claude-{}", uuid::Uuid::new_v4().simple()),
            previous,
        }
    }
    pub fn validate(&self) -> Result<()> {
        uuid::Uuid::parse_str(&self.profile_id)?;
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
    let key = Value::from("proxy-groups");
    let groups = map
        .entry(key)
        .or_insert(Value::Sequence(vec![]))
        .as_sequence_mut()
        .ok_or_else(|| anyhow::anyhow!("订阅代理组格式无效"))?;
    ensure!(
        !groups
            .iter()
            .any(|g| g.get("name").and_then(Value::as_str) == Some(&protection.route)),
        "订阅包含同名的保护线路"
    );
    groups.push(serde_yaml::to_value(
        serde_json::json!({"name": protection.route, "type":"select", "proxies":[protection.node]}),
    )?);

    let original = map
        .remove(Value::from("rules"))
        .unwrap_or(Value::Sequence(vec![]));
    let Value::Sequence(original) = original else {
        anyhow::bail!("订阅规则格式无效");
    };
    let mut rules = Vec::with_capacity(original.len() + 40);
    // The higher-priority overlay replaces only equal predicates in the runtime
    // copy. The subscription and user rule identities are never rewritten.
    for domain in OWNED_DOMAINS.iter().chain(ASSOCIATED_DOMAINS) {
        rules.push(Value::from(format!(
            "DOMAIN-SUFFIX,{domain},{}",
            protection.route
        )));
    }
    for keyword in KEYWORDS {
        rules.push(Value::from(format!(
            "DOMAIN-KEYWORD,{keyword},{}",
            protection.route
        )));
    }
    for host in DIAGNOSTIC_HOSTS {
        rules.push(Value::from(format!("DOMAIN,{host},{}", protection.route)));
    }
    for (kind, payload) in [
        ("IP-CIDR", "160.79.104.0/23"),
        ("IP-CIDR6", "2607:6bc0::/48"),
    ] {
        rules.push(Value::from(format!(
            "{kind},{payload},{},no-resolve",
            protection.route
        )));
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
    map.insert(Value::from("mode"), Value::from("rule"));
    map.insert(Value::from("ipv6"), Value::from(true));

    let dns = map
        .get_mut(Value::from("dns"))
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| anyhow::anyhow!("DNS 配置缺失"))?;
    for key in [
        "nameserver-policy",
        "proxy-server-nameserver-policy",
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
    dns.insert(
        Value::from("default-nameserver"),
        serde_yaml::to_value(BOOTSTRAP)?,
    );
    dns.insert(
        Value::from("proxy-server-nameserver"),
        serde_yaml::to_value(BOOTSTRAP)?,
    );
    // Private names retain local resolution. These explicit exceptions are
    // shown to the user; no public failure falls back to the system resolver.
    dns.insert(Value::from("nameserver-policy"), serde_yaml::from_str::<Mapping>("'+.lan': system\n'+.local': system\nlocalhost: system\n'+.home.arpa': system\n'+.in-addr.arpa': system\n'+.ip6.arpa': system\n")?.into());
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

#[cfg(test)]
mod tests {
    use super::*;
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
        let raw = "proxies: [{name: 'node,&', type: socks5, server: 127.0.0.1, port: 1080}]\ndns: {enable: false, nameserver: [9.9.9.9], fallback: [8.8.8.8], nameserver-policy: {'+.google.com': 114.114.114.114}, direct-nameserver: [system], fake-ip-filter: ['*.lan']}\ntun: {route-address: [203.0.113.0/24], dns-hijack: []}\nrules: ['DOMAIN-SUFFIX,claude.ai,DIRECT', 'MATCH,DIRECT']\n";
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
        assert_eq!(map["mode"], "rule");
        assert_eq!(map["ipv6"], true);
        assert_eq!(map["tun"]["dns-hijack"].as_sequence().unwrap().len(), 2);
        assert!(map["tun"]["route-address"].is_null());
        assert!(map["dns"]["fallback"].is_null());
        assert!(map["dns"]["direct-nameserver"].is_null());
        assert!(map["dns"]["nameserver-policy"]["+.google.com"].is_null());
        let rules = map["rules"].as_sequence().unwrap();
        assert_eq!(
            rules
                .iter()
                .filter(|v| v.as_str().unwrap().starts_with("DOMAIN-SUFFIX,claude.ai,"))
                .count(),
            1
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
