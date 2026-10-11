//! The header represents the default route, since rule mode can route individual
//! connections through different groups. Resolve groups to their selected leaf.
use crate::{api::Proxies, config::parse_profile, rule_manager::parts};
use anyhow::Result;
use serde_yaml::Value;

pub const MAX_DEPTH: usize = 16;
pub fn root(raw: &str) -> Result<String> {
    let map = parse_profile(raw)?;
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
/// Use the already loaded proxy page without fetching it a second time.
pub fn from_snapshot(root: &str, proxies: &Proxies) -> Option<String> {
    let mut name = root;
    let mut visited = Vec::new();
    for _ in 0..MAX_DEPTH {
        if terminal(name) {
            return Some(name.into());
        }
        if visited.contains(&name) {
            return None;
        }
        visited.push(name);
        let proxy = proxies.proxies.get(name)?;
        if proxy.now.as_str().is_empty() {
            return Some(name.into());
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
            "Cycle": {"type":"Selector", "now":"Cycle"}
        }}))
        .unwrap();
        assert_eq!(
            from_snapshot("GLOBAL", &proxies).as_deref(),
            Some("日本节点")
        );
        assert!(from_snapshot("Cycle", &proxies).is_none());
        assert!(from_snapshot("Missing", &proxies).is_none());
    }
}
