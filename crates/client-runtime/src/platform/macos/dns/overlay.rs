//! A temporary view of service DNS; no SCPreferences or VPN settings are written.
use super::ResolverLease;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) type Services = BTreeMap<String, Value>;
const ADDRESS_FIELDS: &[&str] = &["ServerAddresses", "ServerPort"];
const MATCH_FIELDS: &[&str] = &["SupplementalMatchDomains", "SupplementalMatchOrders"];

fn copy_fields(target: &mut Value, source: &Value, fields: &[&str]) {
    for field in fields {
        if let Some(value) = source.get(*field) {
            target[*field] = value.clone();
        } else {
            target.as_object_mut().unwrap().remove(*field);
        }
    }
}

pub(super) fn restored(current: &Value, entry: &ResolverLease) -> Option<Value> {
    let original = entry.original.as_ref()?;
    if !current.is_object() {
        return Some(current.clone());
    }
    let mut result = current.clone();
    // Keep external edits, including a later change of the DNS upstream. The
    // address and port form one setting and must be restored together.
    for fields in [ADDRESS_FIELDS, MATCH_FIELDS] {
        if fields
            .iter()
            .all(|field| current.get(*field) == entry.owned.get(*field))
        {
            copy_fields(&mut result, original, fields);
        }
    }
    Some(result)
}

pub(super) fn originals(current: &Services, previous: &[ResolverLease], prefix: &str) -> Services {
    current
        .iter()
        .filter(|(key, _)| !key.starts_with(prefix))
        .map(|(key, value)| {
            let original = previous
                .iter()
                .find(|entry| entry.key == *key)
                .and_then(|entry| restored(value, entry))
                .unwrap_or_else(|| value.clone());
            (key.clone(), original)
        })
        .collect()
}

fn names(value: &Value, field: &str) -> Vec<String> {
    value[field]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|domain| !domain.is_empty() && *domain != ".")
        .map(str::to_owned)
        .collect()
}

pub(super) fn plan(sources: &Services, prefix: &str, port: u16) -> Result<Vec<ResolverLease>> {
    use crate::claude_check::protection::{
        ASSOCIATED_DOMAINS, DIAGNOSTIC_DNS_SUFFIXES, DIAGNOSTIC_HOSTS, OWNED_DOMAINS,
    };
    ensure!(sources.len() <= 128, "系统 DNS 服务超过安全上限");
    let domains = std::iter::once("")
        .chain(OWNED_DOMAINS.iter().copied())
        .chain(ASSOCIATED_DOMAINS.iter().copied())
        .chain(DIAGNOSTIC_HOSTS.iter().copied())
        .chain(DIAGNOSTIC_DNS_SUFFIXES.iter().copied())
        .collect::<Vec<_>>();
    let mut result = vec![ResolverLease {
        key: format!("{prefix}/DNS"),
        original: None,
        owned: json!({
            "ServerAddresses": ["127.0.0.1"],
            "ServerPort": port,
            "SearchOrder": 1,
            "SupplementalMatchDomains": domains,
            "SupplementalMatchDomainsNoSearch": 1,
            "InterfaceName": "*",
        }),
    }];
    for (index, (key, original)) in sources.iter().enumerate() {
        ensure!(original.is_object(), "系统 DNS 服务格式无效");
        if original["ServerAddresses"]
            .as_array()
            .is_none_or(Vec::is_empty)
        {
            continue;
        }
        let matches = original["SupplementalMatchDomains"].as_array();
        let root = matches.is_none_or(|domains| {
            domains.is_empty() || domains.iter().any(|domain| domain == "" || domain == ".")
        });
        let private = names(original, "SupplementalMatchDomains");
        let search = names(original, "SearchDomains");
        ensure!(
            private
                .iter()
                .chain(search.iter().filter(|_| root))
                .all(|domain| {
                    !crate::claude_check::protection::protected_dns_split(
                        &domain.trim_end_matches('.').to_ascii_lowercase(),
                    )
                }),
            "系统分域 DNS 与保护范围重叠，无法可靠阻止 DNS 回退，未启用保护"
        );
        if !root {
            continue;
        }
        for (kind, domains, searchable) in [("private", private, false), ("search", search, true)] {
            if domains.is_empty() {
                continue;
            }
            let mut owned = original.clone();
            // configd generates resolver identities/flags; a new supplemental
            // service must derive its own identity while retaining its interface.
            owned
                .as_object_mut()
                .unwrap()
                .retain(|key, _| !key.starts_with("__"));
            owned["SupplementalMatchDomains"] = json!(domains);
            owned
                .as_object_mut()
                .unwrap()
                .remove("SupplementalMatchOrders");
            if searchable {
                owned["SupplementalMatchDomainsNoSearch"] = json!(0);
            }
            result.push(ResolverLease {
                key: format!("{prefix}.{kind}.{index}/DNS"),
                original: None,
                owned,
            });
        }
        let mut owned = original.clone();
        owned["ServerAddresses"] = json!(["127.0.0.1"]);
        owned["ServerPort"] = port.into();
        if matches.is_some() {
            owned["SupplementalMatchDomains"] = json!([""]);
            owned
                .as_object_mut()
                .unwrap()
                .remove("SupplementalMatchOrders");
        }
        result.push(ResolverLease {
            key: key.clone(),
            original: Some(original.clone()),
            owned,
        });
    }
    Ok(result)
}

pub(super) fn quarantine(sources: &Services, prefix: &str, port: u16) -> Vec<ResolverLease> {
    let mut result = plan(&Services::new(), prefix, port).expect("empty DNS plan");
    for (key, original) in sources {
        if original["ServerAddresses"]
            .as_array()
            .is_none_or(Vec::is_empty)
        {
            continue;
        }
        let mut owned = original.clone();
        owned["ServerAddresses"] = json!(["127.0.0.1"]);
        owned["ServerPort"] = port.into();
        result.push(ResolverLease {
            key: key.clone(),
            original: Some(original.clone()),
            owned,
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    const PREFIX: &str = "State:/Network/Service/org.clashofrust.protection.501";
    #[test]
    fn separates_private_dns_and_replaces_every_default_without_changing_preferences() {
        let vpn = json!({
            "ServerAddresses": ["100.100.100.100", "fd7a:115c:a1e0::53"],
            "ServerPort": 53,
            "InterfaceName": "utun4",
            "SupplementalMatchDomains": ["", "tail.fixture.ts.net", "corp.test"],
            "SupplementalMatchOrders": [100600,100601,100602],
            "SupplementalMatchDomainsNoSearch": 1,
            "SearchDomains": ["tail.fixture.ts.net"],
            "__CONFIGURATION_ID__": "Supplemental: original",
        });
        let physical = json!({"ServerAddresses":["192.168.1.1"],"SearchDomains":["lan"]});
        let split =
            json!({"ServerAddresses":["10.0.0.53"],"SupplementalMatchDomains":["private.test"]});
        let sources = Services::from([
            ("physical".into(), physical.clone()),
            ("split".into(), split.clone()),
            ("vpn".into(), vpn.clone()),
        ]);
        let plan = plan(&sources, PREFIX, 25353).unwrap();
        assert!(!plan.iter().any(|entry| entry.key == "split"));
        for entry in plan.iter().filter(|entry| entry.original.is_some()) {
            assert_eq!(entry.owned["ServerAddresses"], json!(["127.0.0.1"]));
            assert_eq!(entry.owned["ServerPort"], 25353);
            assert_eq!(restored(&entry.owned, entry), entry.original);
        }
        let private = plan
            .iter()
            .find(|entry| entry.key.contains(".private."))
            .unwrap();
        assert_eq!(private.owned["InterfaceName"], "utun4");
        assert_eq!(private.owned["ServerAddresses"], vpn["ServerAddresses"]);
        assert_eq!(private.owned["ServerPort"], 53);
        assert!(private.owned.get("__CONFIGURATION_ID__").is_none());
        assert_eq!(
            private.owned["SupplementalMatchDomains"],
            json!(["tail.fixture.ts.net", "corp.test"])
        );
        let search = plan
            .iter()
            .find(|entry| entry.key.contains(".search.") && entry.owned["InterfaceName"] == "utun4")
            .unwrap();
        assert_eq!(search.owned["SupplementalMatchDomainsNoSearch"], 0);
        assert_eq!(
            search.owned["SupplementalMatchDomains"],
            json!(["tail.fixture.ts.net"])
        );
        assert_eq!(sources["physical"], physical);
        assert_eq!(sources["vpn"], vpn);
        assert_eq!(sources["split"], split);
    }

    #[test]
    fn refresh_and_restore_keep_external_edits_and_do_not_resurrect_removed_services() {
        let original =
            json!({"ServerAddresses":["100.100.100.100"],"SearchDomains":["old.ts.net"]});
        let first = plan(
            &Services::from([("vpn".into(), original.clone())]),
            PREFIX,
            25353,
        )
        .unwrap();
        let entry = first.iter().find(|entry| entry.key == "vpn").unwrap();
        let mut edited = entry.owned.clone();
        edited["SearchDomains"] = json!(["new.ts.net"]);
        let restored = restored(&edited, entry).unwrap();
        assert_eq!(restored["ServerAddresses"], original["ServerAddresses"]);
        assert_eq!(restored["SearchDomains"], json!(["new.ts.net"]));
        let current = Services::from([
            ("vpn".into(), edited),
            (format!("{PREFIX}/DNS"), first[0].owned.clone()),
        ]);
        let sources = originals(&current, &first, PREFIX);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources["vpn"], restored);
        assert!(originals(&Services::new(), &first, PREFIX).is_empty());
        let external = json!({"ServerAddresses":["10.0.0.53"],"ServerPort":5353});
        let sources = originals(
            &Services::from([("vpn".into(), external.clone())]),
            &first,
            PREFIX,
        );
        assert_eq!(sources["vpn"], external);
    }

    #[test]
    fn conflicting_reconnect_is_quarantined_and_can_be_restored() {
        let sources = Services::from([(
            "vpn".into(),
            json!({
                "ServerAddresses":["100.100.100.100"],
                "SupplementalMatchDomains":["","claude.ai"],
                "InterfaceName":"utun4",
            }),
        )]);
        assert!(plan(&sources, PREFIX, 25353).is_err());
        let quarantined = quarantine(&sources, PREFIX, 25353);
        let entry = quarantined.iter().find(|entry| entry.key == "vpn").unwrap();
        assert_eq!(entry.owned["ServerAddresses"], json!(["127.0.0.1"]));
        assert_eq!(
            entry.owned["SupplementalMatchDomains"],
            json!(["", "claude.ai"])
        );
        assert_eq!(restored(&entry.owned, entry), Some(sources["vpn"].clone()));
        let split_only = Services::from([(
            "vpn".into(),
            json!({
                "ServerAddresses":["192.0.2.53"],
                "SupplementalMatchDomains":["com"],
            }),
        )]);
        assert!(plan(&split_only, PREFIX, 25353).is_err());
        let quarantine = quarantine(&split_only, PREFIX, 25353);
        assert_eq!(quarantine[1].owned["ServerAddresses"], json!(["127.0.0.1"]));
    }
}
