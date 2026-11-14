//! The header represents the default route, since rule mode can route individual
//! connections through different groups. Resolve groups to their selected leaf.
use crate::{api::Proxies, rule_manager::parts};
use anyhow::Result;
use yaml_serde::Value;

pub const MAX_DEPTH: usize = 16;
pub fn root(raw: &str) -> Result<String> {
    // This is the validated runtime configuration, whose size limit is higher
    // than the original subscription's after serialization and rule additions.
    let map: yaml_serde::Mapping = yaml_serde::from_str(raw)?;
    if let Some(Value::Sequence(rules)) = map.get(Value::from("rules")) {
        for rule in rules.iter().filter_map(Value::as_str) {
            if let Ok(rule) = parts(rule)
                && rule.kind == "MATCH"
            {
                return Ok(rule.policy.to_owned());
            }
        }
    }
    if let Some(Value::Sequence(groups)) = map.get(Value::from("proxy-groups")) {
        for group in groups {
            if let Some(name) = group.get("name").and_then(Value::as_str) {
                return Ok(name.to_owned());
            }
        }
    }
    Ok("DIRECT".into())
}
pub fn start<'a>(mode: &str, default: &'a str) -> &'a str {
    match mode {
        "direct" => "DIRECT",
        "global" => "GLOBAL",
        _ => default,
    }
}
pub fn terminal(name: &str) -> bool {
    matches!(name, "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS")
}
pub fn group<'a>(mode: &str, default: &'a str, proxies: &Proxies) -> Option<&'a str> {
    let name = start(mode, default);
    proxies
        .proxies
        .get(name)
        .filter(|group| !group.all.is_empty())
        .map(|_| name)
}
/// Use the already loaded proxy page without fetching it a second time.
pub fn from_snapshot(root: &str, proxies: &Proxies) -> Option<String> {
    snapshot_path(root, proxies).and_then(|path| path.last().map(|name| (*name).to_owned()))
}

pub fn path_from_snapshot(root: &str, proxies: &Proxies) -> Option<Vec<String>> {
    snapshot_path(root, proxies).map(|path| path.into_iter().map(str::to_owned).collect())
}

fn snapshot_path<'a>(root: &'a str, proxies: &'a Proxies) -> Option<Vec<&'a str>> {
    let mut name = root;
    let mut visited = Vec::new();
    for _ in 0..MAX_DEPTH {
        if terminal(name) {
            visited.push(name);
            return Some(visited);
        }
        if visited.contains(&name) {
            return None;
        }
        visited.push(name);
        let Some(proxy) = proxies.proxies.get(name) else {
            // A valid group's selected provider member is absent from this map.
            return (visited.len() > 1).then_some(visited);
        };
        if proxy.now.as_str().is_empty() {
            return Some(visited);
        }
        name = proxy.now.as_str();
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mode_and_first_fallback_determine_the_default_route() {
        let raw = "proxies: []\nproxy-groups: [{name: Secondary}]\nrules: [\"DOMAIN,a.test,DIRECT\", \"MATCH,Primary\", \"MATCH,DIRECT\"]";
        assert_eq!(root(raw).unwrap(), "Primary");
        assert_eq!(start("direct", "Primary"), "DIRECT");
        assert_eq!(start("global", "Primary"), "GLOBAL");
        assert_eq!(
            root("proxies: []\nproxy-groups: [{name: First}, {name: Second}]").unwrap(),
            "First"
        );
        assert_eq!(root("proxies: []").unwrap(), "DIRECT");
    }
    #[test]
    fn nested_groups_resolve_to_leaf_and_cycles_do_not_hang() {
        let proxies: Proxies = serde_json::from_value(serde_json::json!({"proxies": {
            "GLOBAL": {"type":"Selector", "now":"Main"},
            "Main": {"type":"Selector", "now":"Auto"},
            "Auto": {"type":"URLTest", "now":"日本节点"},
            "日本节点": {"type":"Trojan"},
            "Cycle": {"type":"Selector", "now":"Cycle"},
            "Provider": {"type":"Selector", "now":"provider leaf", "all":["provider leaf"]}
        }}))
        .unwrap();
        assert_eq!(
            from_snapshot("GLOBAL", &proxies).as_deref(),
            Some("日本节点")
        );
        assert_eq!(
            path_from_snapshot("GLOBAL", &proxies).unwrap(),
            ["GLOBAL", "Main", "Auto", "日本节点"]
        );
        assert_eq!(
            path_from_snapshot("Provider", &proxies).unwrap(),
            ["Provider", "provider leaf"]
        );
        assert_eq!(path_from_snapshot("DIRECT", &proxies).unwrap(), ["DIRECT"]);
        assert!(path_from_snapshot("Cycle", &proxies).is_none());
        assert!(path_from_snapshot("Missing", &proxies).is_none());
        assert!(from_snapshot("Cycle", &proxies).is_none());
        assert!(from_snapshot("Missing", &proxies).is_none());
        assert_eq!(
            from_snapshot("Provider", &proxies).as_deref(),
            Some("provider leaf")
        );
    }

    #[test]
    fn runtime_route_accepts_configs_larger_than_the_subscription_limit() {
        let mut raw = String::from("proxies: []\nrules: [\"MATCH,Primary\"]\n# ");
        raw.extend(std::iter::repeat_n('x', 10 * 1024 * 1024));
        assert_eq!(root(&raw).unwrap(), "Primary");
    }
}
