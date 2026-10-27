//! Ownership covers the proxy fields we write, never an entire network service.
use crate::platform::{ProxyState, ProxyValue};
use anyhow::{Result, bail, ensure};
use serde_json::{Map, Value};

const MANAGED: [&str; 11] = [
    "HTTPEnable",
    "HTTPProxy",
    "HTTPPort",
    "HTTPSEnable",
    "HTTPSProxy",
    "HTTPSPort",
    "SOCKSEnable",
    "SOCKSProxy",
    "SOCKSPort",
    "ProxyAutoConfigEnable",
    "ProxyAutoDiscoveryEnable",
];

fn properties(value: &Option<ProxyValue>) -> Result<Map<String, Value>> {
    match value {
        Some(ProxyValue::Text(text)) => match serde_json::from_str(text)? {
            Value::Object(value) => Ok(value),
            _ => bail!("invalid network proxy properties"),
        },
        None => Ok(Map::new()),
        _ => bail!("invalid network proxy properties"),
    }
}

pub(crate) fn desired(original: &ProxyState, port: u16) -> Result<ProxyState> {
    ensure!(port > 0, "invalid proxy port");
    original
        .iter()
        .map(|(id, value)| {
            let mut value = properties(value)?;
            for prefix in ["HTTP", "HTTPS", "SOCKS"] {
                value.insert(format!("{prefix}Enable"), 1.into());
                value.insert(format!("{prefix}Proxy"), "127.0.0.1".into());
                value.insert(format!("{prefix}Port"), port.into());
            }
            for key in ["ProxyAutoConfigEnable", "ProxyAutoDiscoveryEnable"] {
                value.insert(key.into(), 0.into());
            }
            Ok((
                id.clone(),
                Some(ProxyValue::Text(Value::Object(value).to_string())),
            ))
        })
        .collect()
}

fn same_field(key: &str, current: Option<&Value>, owned: Option<&Value>) -> bool {
    if key.ends_with("Enable") {
        let flag = |value: Option<&Value>| match value {
            None => Some(false),
            Some(Value::Bool(value)) => Some(*value),
            Some(Value::Number(value)) if value.as_u64() == Some(0) => Some(false),
            Some(Value::Number(value)) if value.as_u64() == Some(1) => Some(true),
            _ => None,
        };
        return flag(current).is_some_and(|value| Some(value) == flag(owned));
    }
    current == owned
}

fn matches_service(current: &Map<String, Value>, owned: &Map<String, Value>) -> bool {
    MANAGED
        .iter()
        .all(|key| owned.contains_key(*key) && same_field(key, current.get(*key), owned.get(*key)))
}

pub(crate) fn matches_managed(current: &ProxyState, owned: &ProxyState) -> Result<bool> {
    let mut shared = false;
    for (id, expected) in owned {
        let Some(actual) = current.get(id) else {
            continue;
        };
        shared = true;
        if !matches_service(&properties(actual)?, &properties(expected)?) {
            return Ok(false);
        }
    }
    Ok(shared)
}

/// Return a patch for surviving services that still use our proxy. Merge into
/// their latest dictionaries so new exceptions and unrelated settings survive.
pub(crate) fn restoration(
    current: &ProxyState,
    original: &ProxyState,
    owned: &ProxyState,
) -> Result<ProxyState> {
    let mut patch = ProxyState::new();
    for (id, expected) in owned {
        let (Some(actual), Some(previous)) = (current.get(id), original.get(id)) else {
            continue;
        };
        let mut value = properties(actual)?;
        if !matches_service(&value, &properties(expected)?) {
            continue;
        }
        let previous_properties = properties(previous)?;
        for key in MANAGED {
            match previous_properties.get(key) {
                Some(previous) => {
                    value.insert(key.into(), previous.clone());
                }
                None => {
                    value.remove(key);
                }
            }
        }
        let restored = if previous.is_none() && value.is_empty() {
            None
        } else {
            Some(ProxyValue::Text(Value::Object(value).to_string()))
        };
        if &restored != actual {
            patch.insert(id.clone(), restored);
        }
    }
    Ok(patch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state(value: Value) -> ProxyState {
        ProxyState::from([("wifi".into(), Some(ProxyValue::Text(value.to_string())))])
    }
    fn value(state: &ProxyState, id: &str) -> Value {
        Value::Object(properties(&state[id]).unwrap())
    }

    #[test]
    fn unrelated_fields_and_service_changes_do_not_hide_an_enabled_proxy() {
        let original = state(json!({"ExceptionsList": ["*.local"]}));
        let owned = desired(&original, 7890).unwrap();
        let mut changed = value(&owned, "wifi");
        changed["ExceptionsList"] = json!(["*.local", "new.example"]);
        changed["ExcludeSimpleHostnames"] = 1.into();
        let mut current = state(changed);
        current.insert("new-vpn".into(), None);
        assert!(matches_managed(&current, &owned).unwrap());
        assert!(!matches_managed(&ProxyState::new(), &owned).unwrap());
        assert!(!matches_managed(&current, &ProxyState::new()).unwrap());
        let mut with_removed = owned.clone();
        with_removed.insert("removed".into(), owned["wifi"].clone());
        assert!(matches_managed(&current, &with_removed).unwrap());
    }

    #[test]
    fn actual_route_changes_are_not_reported_as_ours_or_overwritten() {
        let original = state(json!({}));
        let owned = desired(&original, 7890).unwrap();
        for key in MANAGED {
            let mut changed = value(&owned, "wifi");
            changed[key] = match key {
                "HTTPProxy" | "HTTPSProxy" | "SOCKSProxy" => "external.example".into(),
                "ProxyAutoConfigEnable" | "ProxyAutoDiscoveryEnable" => 1.into(),
                _ => 0.into(),
            };
            let current = state(changed);
            assert!(!matches_managed(&current, &owned).unwrap(), "{key}");
            assert!(
                restoration(&current, &original, &owned).unwrap().is_empty(),
                "{key}"
            );
        }
    }

    #[test]
    fn disabled_flags_accept_native_missing_and_boolean_representations() {
        let original = state(json!({}));
        let owned = desired(&original, 7890).unwrap();
        let mut changed = value(&owned, "wifi");
        changed
            .as_object_mut()
            .unwrap()
            .remove("ProxyAutoConfigEnable");
        changed["ProxyAutoDiscoveryEnable"] = false.into();
        changed["HTTPEnable"] = true.into();
        assert!(matches_managed(&state(changed.clone()), &owned).unwrap());
        changed["HTTPEnable"] = Value::Null;
        assert!(!matches_managed(&state(changed), &owned).unwrap());
    }

    #[test]
    fn restoration_preserves_later_exceptions_pac_urls_and_new_services() {
        let original = state(json!({
            "HTTPEnable": 0, "HTTPPort": 80, "ProxyAutoConfigEnable": 1,
            "ProxyAutoConfigURLString": "https://old.example/proxy.pac",
            "ExceptionsList": ["*.local"]
        }));
        let owned = desired(&original, 7890).unwrap();
        let mut changed = value(&owned, "wifi");
        changed["ExceptionsList"] = json!(["new.example"]);
        changed["ProxyAutoConfigURLString"] = "https://new.example/proxy.pac".into();
        let mut current = state(changed);
        current.insert("new-vpn".into(), None);
        let patch = restoration(&current, &original, &owned).unwrap();
        assert_eq!(patch.len(), 1);
        let restored = value(&patch, "wifi");
        assert_eq!(restored["HTTPEnable"], 0);
        assert_eq!(restored["HTTPPort"], 80);
        assert_eq!(restored["ProxyAutoConfigEnable"], 1);
        assert_eq!(restored["ExceptionsList"], json!(["new.example"]));
        assert_eq!(
            restored["ProxyAutoConfigURLString"],
            "https://new.example/proxy.pac"
        );
        assert!(restored.get("HTTPProxy").is_none());
        assert!(restored.get("SOCKSEnable").is_none());
    }

    #[test]
    fn restore_surviving_owned_services_without_recreating_removed_or_foreign_ones() {
        let original = ProxyState::from([
            ("wifi".into(), None),
            ("removed".into(), None),
            ("external".into(), None),
        ]);
        let owned = desired(&original, 7890).unwrap();
        let mut current = owned.clone();
        current.remove("removed");
        current.insert("external".into(), None);
        assert!(!matches_managed(&current, &owned).unwrap());
        let patch = restoration(&current, &original, &owned).unwrap();
        assert_eq!(patch, ProxyState::from([("wifi".into(), None)]));
        let mut changed = value(&owned, "wifi");
        changed["ExceptionsList"] = json!(["keep.example"]);
        current.insert("wifi".into(), state(changed)["wifi"].clone());
        let patch = restoration(&current, &original, &owned).unwrap();
        assert_eq!(
            value(&patch, "wifi"),
            json!({"ExceptionsList":["keep.example"]})
        );
    }
}
