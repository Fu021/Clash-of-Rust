use anyhow::{Context, Result, ensure};
use clash_client_core::{
    api::{Proxies, Proxy, Text},
    config::{self, Settings, Store},
};
use std::collections::BTreeMap;
use std::io::Read;
use yaml_serde::Value;

pub(super) fn saved(store: &Store) -> Result<Proxies> {
    let settings = match std::fs::File::open(store.root.join("settings.json")) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 1024 * 1024, "本地设置文件过大");
            serde_json::from_slice::<Settings>(&bytes).context("本地设置格式错误")?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Settings::default(),
        Err(error) => return Err(error).context("无法读取本地设置"),
    };
    let Some(id) = settings.active_profile else {
        return Ok(Proxies::default());
    };
    from_yaml(&config::read_profile(&store.profile_path(&id)?)?)
}

fn entries(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_sequence().into_iter().flatten()
}

fn names(value: &Value) -> impl Iterator<Item = Text> {
    entries(value).filter_map(Value::as_str).map(Text::from)
}

fn insert(proxies: &mut Proxies, node: &Value) -> Result<Text> {
    let name = node["name"].as_str().context("节点或策略组缺少名称")?;
    ensure!(!name.is_empty(), "节点或策略组名称为空");
    let kind = node["type"].as_str().unwrap_or("");
    let kind = match kind {
        "select" => "Selector",
        "url-test" => "URLTest",
        "fallback" => "Fallback",
        "load-balance" => "LoadBalance",
        "relay" => "Relay",
        other => other,
    };
    let name = Text::from(name);
    proxies
        .proxies
        .entry(name.clone())
        .or_insert_with(|| Proxy {
            kind: kind.into(),
            ..Default::default()
        });
    Ok(name)
}

fn from_yaml(raw: &str) -> Result<Proxies> {
    let root: Value = yaml_serde::from_str(raw).context("本地订阅 YAML 格式错误")?;
    let mut proxies = Proxies::default();
    for (name, kind) in [("DIRECT", "Direct"), ("REJECT", "Reject")] {
        proxies.proxies.insert(
            name.into(),
            Proxy {
                kind: kind.into(),
                ..Default::default()
            },
        );
    }
    for node in entries(&root["proxies"]) {
        insert(&mut proxies, node)?;
    }
    let mut providers = BTreeMap::new();
    if let Some(mapping) = root["proxy-providers"].as_mapping() {
        for (name, provider) in mapping {
            if provider["type"].as_str() != Some("inline") {
                continue;
            }
            let mut members = Vec::new();
            for node in entries(&provider["payload"]) {
                members.push(insert(&mut proxies, node)?);
            }
            if let Some(name) = name.as_str() {
                providers.insert(name, members);
            }
        }
    }
    for group in entries(&root["proxy-groups"]) {
        let name = insert(&mut proxies, group)?;
        let mut members: Vec<_> = names(&group["proxies"]).collect();
        for provider in names(&group["use"]) {
            if let Some(nodes) = providers.get(provider.as_str()) {
                members.extend(nodes.iter().cloned());
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        members.retain(|member| seen.insert(member.clone()));
        proxies.proxies.get_mut(name.as_str()).unwrap().all = members;
    }
    Ok(proxies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_catalog_preserves_unicode_and_does_not_invent_live_state() {
        let proxies = from_yaml(
            r#"proxies: [{name: '🇺🇸 节点 /1', type: anytls}]
proxy-providers:
  Inline: {type: inline, payload: [{name: '🚀 二', type: socks5}]}
  Remote: {type: http, url: 'https://example.com/private'}
proxy-groups:
  - {name: PROXY, type: select, proxies: ['🇺🇸 节点 /1', DIRECT], use: [Inline, Remote]}
"#,
        )
        .unwrap();
        let group = &proxies.proxies["PROXY"];
        assert_eq!(group.kind.as_str(), "Selector");
        assert_eq!(
            group.all.iter().map(Text::as_str).collect::<Vec<_>>(),
            ["🇺🇸 节点 /1", "DIRECT", "🚀 二"]
        );
        assert!(proxies.proxies.values().all(|node| node.delay.is_none()));
        assert!(group.now.is_empty());
        let output = serde_json::to_string(&proxies).unwrap();
        assert!(!output.contains("private"));
    }

    #[test]
    fn missing_data_directory_remains_absent() {
        let temporary = tempfile::tempdir().unwrap();
        let store = Store {
            root: temporary.path().join("absent"),
        };
        assert!(saved(&store).unwrap().proxies.is_empty());
        assert!(!store.root.exists());
    }

    #[test]
    fn malformed_saved_settings_are_not_replaced() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("settings.json");
        std::fs::write(&path, b"broken").unwrap();
        let store = Store {
            root: temporary.path().to_owned(),
        };
        assert!(saved(&store).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
        assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 1);
    }
}
