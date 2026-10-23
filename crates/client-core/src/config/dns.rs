//! DNS overrides shared by desktop, CLI and Android. Exceptions change answers,
//! not outbound policies: a real address must still obey the user's routing rules.
use super::{Mapping, Settings, Value};
use anyhow::{Context, Result, bail};

/// Shared fallback used when a host needs DNS but the subscription provides none.
pub const DEFAULT_DNS_NAMESERVERS: [&str; 2] = [
    "https://doh.pub/dns-query",
    "https://dns.alidns.com/dns-query",
];

// Keep this list limited to local names and protocols/clients that need real
// addresses (bound/bypass sockets, peer discovery, STUN and connectivity probes).
// References: tailscale.com/docs/reference/dns-in-tailscale;
// docs.netbird.io/about-netbird/ports-and-firewalls;
// developers.cloudflare.com/cloudflare-one/team-and-resources/devices/cloudflare-one-client/deployment/firewall;
// docs.syncthing.net/users/firewall;
// github.com/rustdesk/hbb_common/blob/master/src/config.rs.
pub(super) const REAL_IP_DOMAINS: &[&str] = &[
    // Local and reverse lookup namespaces.
    "localhost",
    "+.localhost",
    "+.lan",
    "+.local",
    "+.home.arpa",
    "+.in-addr.arpa",
    "+.ip6.arpa",
    // Overlay/VPN control, relay and peer names.
    "+.tailscale.com",
    "+.tailscale.io",
    "+.ts.net",
    "+.netbird.io",
    "+.netbird.cloud",
    "+.cloudflareclient.com",
    "+.devices.cloudflare.com",
    "+.devices.fed.cloudflare.com",
    // Public remote-connection and peer-discovery services.
    "+.rustdesk.com",
    "+.syncthing.net",
    // Clash domain-set wildcards occupy whole labels; do not use `stun.+`.
    "+.stun.*.*",
    "+.stun.*.*.*",
    "+.stun.*.*.*.*",
    "+.turn.*.*",
    "+.turn.*.*.*",
    "+.turn.*.*.*.*",
    // Common system time sources.
    "+.pool.ntp.org",
    "ntp.ubuntu.com",
    "ntp.aliyun.com",
    "time.windows.com",
    "time.apple.com",
    "time.android.com",
    "time.google.com",
    "time.cloudflare.com",
    // OS and WARP connectivity probes, without excluding entire vendor domains.
    "+.msftncsi.com",
    "+.msftconnecttest.com",
    "connectivitycheck.gstatic.com",
    "connectivitycheck.android.com",
    "captive.apple.com",
    "connectivity-check.ubuntu.com",
    "nmcheck.gnome.org",
    "cloudflareportal.com",
    "cloudflareok.com",
    "cloudflarecp.com",
];

/// Returns whether this application supplied the DNS baseline. Other provider
/// DNS fields remain intact; IPv6 independently follows the user's preference.
pub(super) fn apply(map: &mut Mapping, settings: &Settings) -> Result<bool> {
    let mut dns = match map.remove(Value::from("dns")) {
        None | Some(Value::Null) => Mapping::new(),
        Some(Value::Mapping(dns)) => dns,
        _ => bail!("订阅 dns 必须是配置对象"),
    };
    let managed = settings.dns_override || dns.is_empty();
    if managed {
        dns.insert(Value::from("enable"), Value::from(true));
        dns.insert(Value::from("enhanced-mode"), Value::from("fake-ip"));
        dns.insert(
            Value::from("nameserver"),
            serde_yaml::to_value(DEFAULT_DNS_NAMESERVERS)?,
        );
    }
    dns.insert(Value::from("ipv6"), Value::from(settings.ipv6));
    map.insert(Value::from("dns"), Value::Mapping(dns));
    Ok(managed)
}

pub(super) fn fake_ip_exceptions(map: &mut Mapping, extra: &[&str]) -> Result<()> {
    let dns = map
        .get_mut(Value::from("dns"))
        .and_then(Value::as_mapping_mut)
        .context("DNS 配置缺失")?;
    if dns
        .get(Value::from("enhanced-mode"))
        .and_then(Value::as_str)
        != Some("fake-ip")
    {
        return Ok(());
    }
    let mode = dns
        .get(Value::from("fake-ip-filter-mode"))
        .filter(|mode| !mode.is_null())
        .map(|mode| mode.as_str().context("fake-ip-filter-mode 必须是字符串"))
        .transpose()?
        .unwrap_or("blacklist")
        .to_owned();
    let mut filters = match dns.get(Value::from("fake-ip-filter")) {
        None | Some(Value::Null) => vec![],
        Some(Value::Sequence(filters)) => filters.clone(),
        _ => bail!("fake-ip-filter 必须是列表"),
    };
    for filter in &filters {
        filter.as_str().context("fake-ip-filter 条目必须是字符串")?;
    }
    let mut providers = None;
    match mode.as_str() {
        "blacklist" => {
            for domain in REAL_IP_DOMAINS.iter().chain(extra) {
                if !filters
                    .iter()
                    .any(|filter| filter.as_str().unwrap().eq_ignore_ascii_case(domain))
                {
                    filters.push(Value::from(*domain));
                }
            }
        }
        "rule" | "whitelist" => {
            // A whitelist match means fake-IP. Appending the exceptions there
            // would do the opposite of what we need. An ordered real-IP prefix
            // takes precedence; native domain-set matching preserves old globs.
            let mut rules = exception_rules(extra);
            if mode == "whitelist" {
                let mut domain_patterns = vec![];
                for filter in &filters {
                    let filter = filter.as_str().unwrap();
                    let reference = filter.split_once(':');
                    let kind = reference.and_then(|(kind, _)| {
                        if kind.eq_ignore_ascii_case("geosite") {
                            Some("GEOSITE")
                        } else if kind.eq_ignore_ascii_case("rule-set") {
                            Some("RULE-SET")
                        } else {
                            None
                        }
                    });
                    if let Some(kind) = kind {
                        // Match mihomo's geosite:/rule-set: list syntax.
                        for name in reference.unwrap().1.split(':').next().unwrap().split(',') {
                            rules.push(Value::from(format!("{kind},{name},fake-ip")));
                        }
                    } else {
                        domain_patterns.push(Value::from(filter));
                    }
                }
                if !domain_patterns.is_empty() {
                    providers = Some(domain_patterns);
                    // Resolved below after releasing the mutable DNS borrow.
                }
                rules.push(Value::from("MATCH,real-ip"));
            } else {
                filters.retain(|filter| {
                    !rules.iter().any(|rule| {
                        rule.as_str()
                            .unwrap()
                            .eq_ignore_ascii_case(filter.as_str().unwrap())
                    })
                });
                rules.extend(filters);
            }
            filters = rules;
            dns.insert(Value::from("fake-ip-filter-mode"), Value::from("rule"));
        }
        _ => bail!("不支持的 fake-ip-filter-mode：{mode}"),
    }
    dns.insert(Value::from("fake-ip-filter"), Value::Sequence(filters));
    if let Some(patterns) = providers {
        let name = whitelist_provider(map, patterns)?;
        let filters = map["dns"]["fake-ip-filter"]
            .as_sequence_mut()
            .expect("DNS filters were just constructed");
        filters.insert(
            filters.len() - 1,
            Value::from(format!("RULE-SET,{name},fake-ip")),
        );
    }
    Ok(())
}

fn whitelist_provider(map: &mut Mapping, patterns: Vec<Value>) -> Result<String> {
    let providers = map
        .entry(Value::from("rule-providers"))
        .or_insert(Value::Mapping(Mapping::new()));
    if providers.is_null() {
        *providers = Value::Mapping(Mapping::new());
    }
    let providers = providers
        .as_mapping_mut()
        .context("rule-providers 必须是配置对象")?;
    let mut name = "COR-FakeIP-Whitelist".to_owned();
    let mut suffix = 0;
    while providers.contains_key(Value::from(name.clone())) {
        suffix += 1;
        name = format!("COR-FakeIP-Whitelist-{suffix}");
    }
    let mut provider = Mapping::new();
    provider.insert(Value::from("type"), Value::from("inline"));
    provider.insert(Value::from("behavior"), Value::from("domain"));
    provider.insert(Value::from("payload"), Value::Sequence(patterns));
    providers.insert(Value::from(name.clone()), Value::Mapping(provider));
    Ok(name)
}

fn exception_rules(extra: &[&str]) -> Vec<Value> {
    REAL_IP_DOMAINS
        .iter()
        .chain(extra)
        .map(|domain| {
            let rule = if domain.contains('*') {
                // Generated compatibility patterns only. Subscription patterns
                // use mihomo's native domain provider, not a translated glob.
                let (prefix, suffix) = domain
                    .strip_prefix("+.")
                    .map_or(("^", *domain), |suffix| ("^([^.]+\\.)*", suffix));
                let labels: Vec<_> = suffix
                    .split('.')
                    .map(|label| {
                        if label == "*" {
                            "[^.]+".to_owned()
                        } else {
                            regex::escape(label)
                        }
                    })
                    .collect();
                format!("DOMAIN-REGEX,{prefix}{}$,real-ip", labels.join("\\."))
            } else if let Some(suffix) = domain.strip_prefix("+.") {
                format!("DOMAIN-SUFFIX,{suffix},real-ip")
            } else {
                format!("DOMAIN,{domain},real-ip")
            };
            Value::from(rule)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(dns: &str) -> Mapping {
        serde_yaml::from_str(&format!("dns:\n{dns}")).unwrap()
    }

    fn filters(map: &Mapping) -> Vec<&str> {
        map["dns"]["fake-ip-filter"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect()
    }

    #[test]
    fn blacklist_keeps_existing_exceptions_and_never_grows_on_reload() {
        let mut map = config(
            "  enhanced-mode: fake-ip\n  nameserver-policy: {corp.example: system}\n  fake-ip-filter: [CUSTOM.example, '+.TAILSCALE.COM']\n",
        );
        let before = map["dns"]["nameserver-policy"].clone();
        fake_ip_exceptions(&mut map, &[]).unwrap();
        let entries = filters(&map);
        assert_eq!(&entries[..2], &["CUSTOM.example", "+.TAILSCALE.COM"]);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.eq_ignore_ascii_case("+.tailscale.com"))
                .count(),
            1
        );
        assert!(entries.contains(&"+.netbird.io"));
        assert!(entries.contains(&"+.devices.cloudflare.com"));
        assert!(entries.contains(&"time.cloudflare.com"));
        assert_eq!(map["dns"]["nameserver-policy"], before);
        let once = map.clone();
        fake_ip_exceptions(&mut map, &[]).unwrap();
        assert_eq!(map, once);
    }

    #[test]
    fn whitelist_preserves_native_globs_and_references_after_real_ip_exceptions() {
        let mut map = config(
            "  enhanced-mode: fake-ip\n  fake-ip-filter-mode: whitelist\n  fake-ip-filter: ['*', '+.corp.example', 'GEOSITE:cn,private', 'rule-set:existing']\n",
        );
        let existing: Value = serde_yaml::from_str(
            "existing: {type: inline, behavior: domain, payload: ['+.vendor.example']}\nCOR-FakeIP-Whitelist: {type: inline, behavior: domain, payload: ['user.example']}\n",
        )
        .unwrap();
        map.insert(Value::from("rule-providers"), existing.clone());
        fake_ip_exceptions(&mut map, &[]).unwrap();
        assert_eq!(map["dns"]["fake-ip-filter-mode"], "rule");
        let entries = filters(&map);
        let exception = entries
            .iter()
            .position(|line| *line == "DOMAIN-SUFFIX,tailscale.com,real-ip")
            .unwrap();
        let original = entries
            .iter()
            .position(|line| *line == "RULE-SET,COR-FakeIP-Whitelist-1,fake-ip")
            .unwrap();
        assert!(exception < original);
        assert!(entries.contains(&"GEOSITE,cn,fake-ip"));
        assert!(entries.contains(&"GEOSITE,private,fake-ip"));
        assert!(entries.contains(&"RULE-SET,existing,fake-ip"));
        assert_eq!(entries.last(), Some(&"MATCH,real-ip"));
        assert_eq!(
            map["rule-providers"]["COR-FakeIP-Whitelist-1"]["payload"],
            serde_yaml::to_value(["*", "+.corp.example"]).unwrap()
        );
        for name in ["existing", "COR-FakeIP-Whitelist"] {
            assert_eq!(map["rule-providers"][name], existing[name]);
        }
        let once = map.clone();
        fake_ip_exceptions(&mut map, &[]).unwrap();
        assert_eq!(map, once);
    }

    #[test]
    fn empty_whitelist_remains_real_ip_by_default_without_creating_a_provider() {
        let mut map = config(
            "  enhanced-mode: fake-ip\n  fake-ip-filter-mode: whitelist\n  fake-ip-filter: []\n",
        );
        fake_ip_exceptions(&mut map, &[]).unwrap();
        assert_eq!(filters(&map).last(), Some(&"MATCH,real-ip"));
        assert!(map.get("rule-providers").is_none());
    }

    #[test]
    fn rule_mode_exceptions_precede_user_catch_all_without_reordering_other_rules() {
        let mut map = config(
            "  enhanced-mode: fake-ip\n  fake-ip-filter-mode: rule\n  fake-ip-filter: ['DOMAIN,own.example,real-ip', 'MATCH,fake-ip', 'DOMAIN-SUFFIX,tailscale.com,real-ip']\n",
        );
        fake_ip_exceptions(&mut map, &["+.bash.ws"]).unwrap();
        let entries = filters(&map);
        assert_eq!(
            &entries[entries.len() - 2..],
            &["DOMAIN,own.example,real-ip", "MATCH,fake-ip"]
        );
        assert!(entries.contains(&"DOMAIN-SUFFIX,bash.ws,real-ip"));
        assert_eq!(
            entries
                .iter()
                .filter(|entry| **entry == "DOMAIN-SUFFIX,tailscale.com,real-ip")
                .count(),
            1
        );
        let once = map.clone();
        fake_ip_exceptions(&mut map, &["+.bash.ws"]).unwrap();
        assert_eq!(map, once);
    }

    #[test]
    fn generated_stun_and_turn_patterns_do_not_match_normal_websites() {
        let rules = exception_rules(&[]);
        let patterns: Vec<_> = rules
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|line| line.strip_prefix("DOMAIN-REGEX,"))
            .map(|line| regex::Regex::new(line.strip_suffix(",real-ip").unwrap()).unwrap())
            .collect();
        for host in [
            "stun.cloudflare.com",
            "stun.l.google.com",
            "one.two.stun.example.com",
            "turn.region.example.net",
        ] {
            assert!(
                patterns.iter().any(|pattern| pattern.is_match(host)),
                "{host}"
            );
        }
        for host in ["claude.ai", "www.google.com", "notstun.example.com"] {
            assert!(
                !patterns.iter().any(|pattern| pattern.is_match(host)),
                "{host}"
            );
        }
    }

    #[test]
    fn redir_host_does_not_need_fake_ip_exceptions() {
        let mut map = config(
            "  enhanced-mode: redir-host\n  fake-ip-filter-mode: whitelist\n  fake-ip-filter: ['*']\n",
        );
        let before = map.clone();
        fake_ip_exceptions(&mut map, &[]).unwrap();
        assert_eq!(map, before);
    }

    #[test]
    fn invalid_filter_settings_fail_instead_of_silently_changing_dns_behaviour() {
        for tail in [
            "  fake-ip-filter: invalid\n",
            "  fake-ip-filter: [1]\n",
            "  fake-ip-filter-mode: false\n",
            "  fake-ip-filter-mode: unknown\n",
        ] {
            let mut map = config(&format!("  enhanced-mode: fake-ip\n{tail}"));
            assert!(fake_ip_exceptions(&mut map, &[]).is_err(), "{tail}");
        }
    }
}
