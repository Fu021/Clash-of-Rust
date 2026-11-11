use crate::config::{Settings, parse_profile};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use yaml_serde::{Mapping, Value};

pub const LIMIT: usize = 256;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    #[default]
    Follow,
    Direct,
    Reject,
    Group,
    Node,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Target {
    pub kind: TargetKind,
    pub name: String,
    pub provider: String,
    pub fingerprint: String,
}

impl Target {
    pub fn label(&self) -> String {
        match self.kind {
            TargetKind::Follow => "跟随配置".into(),
            TargetKind::Direct => "直连".into(),
            TargetKind::Reject => "拦截".into(),
            TargetKind::Group => format!("策略组 · {}", self.name),
            TargetKind::Node if self.provider.is_empty() => format!("节点 · {}", self.name),
            TargetKind::Node => format!("节点 · {} · {}", self.name, self.provider),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub package: String,
    pub target: Target,
    #[serde(default)]
    pub reject_missing: bool,
}

pub fn validate(rules: &[Rule]) -> Result<()> {
    ensure!(rules.len() <= LIMIT, "应用规则最多 {LIMIT} 条");
    let mut packages = BTreeSet::new();
    for rule in rules {
        ensure!(
            !rule.package.is_empty()
                && rule.package.len() <= 255
                && rule.package.split('.').all(|part| {
                    !part.is_empty()
                        && part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                }),
            "应用包名无效"
        );
        ensure!(packages.insert(&rule.package), "同一应用只能有一条应用规则");
        ensure!(
            rule.target.name.len() <= 512
                && rule.target.provider.len() <= 512
                && !rule.target.name.chars().any(char::is_control)
                && !rule.target.provider.chars().any(char::is_control),
            "应用规则目标无效"
        );
        ensure!(
            !matches!(rule.target.kind, TargetKind::Group | TargetKind::Node)
                || !rule.target.name.is_empty(),
            "请选择策略组或节点"
        );
        ensure!(
            rule.target.fingerprint.is_empty()
                || (rule.target.fingerprint.len() == 64
                    && rule
                        .target
                        .fingerprint
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit())),
            "节点身份无效"
        );
    }
    Ok(())
}

fn fingerprint(value: &Value) -> Result<String> {
    let mut value = serde_json::to_value(value)?;
    if let Some(map) = value.as_object_mut() {
        map.remove("name");
    }
    Ok(crate::file_hash::hex(Sha256::digest(serde_json::to_vec(
        &value,
    )?)))
}

pub fn targets(raw: &str) -> Result<Vec<Target>> {
    let map = parse_profile(raw)?;
    targets_from_profile(&map)
}

fn targets_from_profile(map: &Mapping) -> Result<Vec<Target>> {
    let mut result = vec![
        Target::default(),
        Target {
            kind: TargetKind::Direct,
            ..Default::default()
        },
        Target {
            kind: TargetKind::Reject,
            ..Default::default()
        },
    ];
    for (key, kind) in [
        ("proxy-groups", TargetKind::Group),
        ("proxies", TargetKind::Node),
    ] {
        for item in map
            .get(Value::from(key))
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
        {
            if let Some(name) = item.get("name").and_then(Value::as_str) {
                result.push(Target {
                    kind: kind.clone(),
                    name: name.into(),
                    fingerprint: if kind == TargetKind::Node {
                        fingerprint(item)?
                    } else {
                        String::new()
                    },
                    ..Default::default()
                });
            }
        }
    }
    Ok(result)
}

pub fn target_present(raw: &str, target: &Target) -> Result<bool> {
    let map = parse_profile(raw)?;
    let candidates = targets_from_profile(&map)?;
    Ok(target_in_profile(&map, &candidates, target))
}

fn target_in_profile(map: &Mapping, candidates: &[Target], target: &Target) -> bool {
    if !target.provider.is_empty() {
        return map
            .get(Value::from("proxy-providers"))
            .and_then(|providers| providers.get(&target.provider))
            .is_some();
    }
    candidates.iter().any(|candidate| {
        candidate.kind == target.kind
            && candidate.name == target.name
            && (target.fingerprint.is_empty() || candidate.fingerprint == target.fingerprint)
    })
}

pub fn runtime(raw: &str, settings: &Settings, profile: &str, supported: bool) -> Result<String> {
    let Some(rules) = settings
        .application_rules
        .get(profile)
        .filter(|rules| !rules.is_empty())
    else {
        return Ok(raw.into());
    };
    if !supported || settings.claude_protection.is_some() || settings.run_mode != "rule" {
        return Ok(raw.into());
    }
    validate(rules)?;
    let profile = parse_profile(raw)?;
    let candidates = targets_from_profile(&profile)?;
    let mut value = Value::Mapping(profile);
    let mut routes = Vec::new();
    let mut predicates = Vec::new();
    let mut generated = BTreeSet::new();
    for rule in rules.iter() {
        if rule.target.kind == TargetKind::Follow {
            continue;
        }
        let present = target_in_profile(
            value.as_mapping().context("配置无效")?,
            &candidates,
            &rule.target,
        );
        let target = if !present {
            if rule.reject_missing {
                "REJECT".into()
            } else {
                String::new()
            }
        } else {
            match rule.target.kind {
                TargetKind::Direct => "DIRECT".into(),
                TargetKind::Reject => "REJECT".into(),
                TargetKind::Follow => String::new(),
                TargetKind::Node | TargetKind::Group => {
                    let name = format!(
                        "cor-application-{}",
                        &crate::file_hash::hex(Sha256::digest(serde_json::to_vec(&(
                            &rule.target,
                            rule.reject_missing,
                        ))?))[..16]
                    );
                    let groups = value
                        .as_mapping_mut()
                        .context("配置无效")?
                        .entry(Value::from("proxy-groups"))
                        .or_insert_with(|| Value::Sequence(Vec::new()))
                        .as_sequence_mut()
                        .context("策略组配置无效")?;
                    if generated.contains(&name) {
                        name
                    } else {
                        ensure!(
                            !groups
                                .iter()
                                .any(|group| group.get("name").and_then(Value::as_str)
                                    == Some(name.as_str())),
                            "应用内部策略组名称冲突"
                        );
                        let mut selector = serde_json::json!({
                            "name": name, "type":"select", "hidden":true,
                            "empty-fallback":if rule.reject_missing { "REJECT" } else { "PASS" }
                        });
                        if rule.target.provider.is_empty() {
                            selector["proxies"] = serde_json::json!([rule.target.name]);
                        } else {
                            selector["use"] = serde_json::json!([rule.target.provider]);
                            selector["filter"] = serde_json::json!(format!(
                                "^{}$",
                                regex::escape(&rule.target.name)
                            ));
                        }
                        groups.push(yaml_serde::to_value(selector)?);
                        generated.insert(name.clone());
                        name
                    }
                }
            }
        };
        if !target.is_empty() {
            predicates.push(Value::from(format!(
                "PROCESS-NAME,{},{target}",
                rule.package
            )));
            predicates.push(Value::from(format!("PROCESS-NAME,{},REJECT", rule.package)));
        }
        predicates.push(Value::from(format!(
            "PROCESS-NAME,cor-reject:{},REJECT",
            rule.package
        )));
        routes.push(serde_json::json!({"package":rule.package,"target":target,"reject_missing":rule.reject_missing}));
    }
    if let Some(Value::Sequence(original)) = value
        .as_mapping_mut()
        .context("配置无效")?
        .remove(Value::from("rules"))
    {
        predicates.extend(original);
    }
    value["rules"] = Value::Sequence(predicates);
    value["find-process-mode"] = "strict".into();
    value["cor-application-rules"] =
        yaml_serde::to_value(serde_json::json!({"schema":1,"routes":routes}))?;
    Ok(yaml_serde::to_string(&value)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(target: Target, reject_missing: bool) -> Settings {
        let mut settings = Settings::default();
        settings.application_rules.insert(
            "profile".into(),
            std::sync::Arc::new(vec![Rule {
                package: "org.example.browser".into(),
                target,
                reject_missing,
            }]),
        );
        settings
    }

    #[test]
    fn missing_and_changed_nodes_obey_explicit_fallback_without_invalid_references() {
        let raw = "proxies: [{name: node, type: socks5, server: example.test, port: 1080}]\n";
        let target = targets(raw).unwrap().pop().unwrap();
        for reject in [false, true] {
            let settings = settings(target.clone(), reject);
            let changed = raw.replace("1080", "1081");
            let runtime: Value =
                yaml_serde::from_str(&runtime(&changed, &settings, "profile", true).unwrap())
                    .unwrap();
            assert_eq!(
                runtime["cor-application-rules"]["routes"][0]["target"].as_str(),
                Some(if reject { "REJECT" } else { "" })
            );
        }
    }

    #[test]
    fn unsupported_devices_modes_and_other_profiles_do_not_enable_rules() {
        let raw = "proxies: []\n";
        let mut settings = settings(
            Target {
                kind: TargetKind::Direct,
                ..Default::default()
            },
            false,
        );
        assert_eq!(runtime(raw, &settings, "profile", false).unwrap(), raw);
        assert_eq!(runtime(raw, &settings, "other", true).unwrap(), raw);
        settings.run_mode = "global".into();
        assert_eq!(runtime(raw, &settings, "profile", true).unwrap(), raw);
    }

    #[test]
    fn provider_nodes_are_exact_hidden_selectors_with_explicit_missing_policy() {
        let raw = "proxies: []\nproxy-providers: {feed: {type: file, path: feed.yaml}}\n";
        let settings = settings(
            Target {
                kind: TargetKind::Node,
                name: "node.[1]".into(),
                provider: "feed".into(),
                ..Default::default()
            },
            false,
        );
        let result: Value =
            yaml_serde::from_str(&runtime(raw, &settings, "profile", true).unwrap()).unwrap();
        assert_eq!(
            result["proxy-groups"][0]["filter"].as_str(),
            Some(r"^node\.\[1\]$")
        );
        assert_eq!(
            result["proxy-groups"][0]["empty-fallback"].as_str(),
            Some("PASS")
        );
        assert_eq!(result["proxy-groups"][0]["hidden"].as_bool(), Some(true));
    }

    #[test]
    fn duplicate_packages_and_control_characters_are_rejected() {
        let mut rule = Rule {
            package: "org.example.app".into(),
            target: Target::default(),
            reject_missing: false,
        };
        assert!(validate(&[rule.clone(), rule.clone()]).is_err());
        rule.target.name = "bad\nname".into();
        assert!(validate(&[rule]).is_err());
    }

    #[test]
    fn applications_with_the_same_target_share_a_selector_without_uid_conflicts() {
        let raw = "proxies: [{name: node, type: socks5, server: example.test, port: 1080}]\n";
        let target = targets(raw).unwrap().pop().unwrap();
        let mut saved = settings(target, false);
        let mut second = saved.application_rules["profile"][0].clone();
        second.package = "org.example.second".into();
        std::sync::Arc::make_mut(saved.application_rules.get_mut("profile").unwrap()).push(second);
        let output: Value =
            yaml_serde::from_str(&runtime(raw, &saved, "profile", true).unwrap()).unwrap();
        assert_eq!(output["proxy-groups"].as_sequence().unwrap().len(), 1);
        assert_eq!(
            output["cor-application-rules"]["routes"][0]["target"],
            output["cor-application-rules"]["routes"][1]["target"]
        );
    }

    #[test]
    fn protection_suspends_application_routes_and_rejects_changes_bundled_with_appearance() {
        use crate::protection::{Previous, Protection};
        let raw = "proxies: []\nrules: ['MATCH,DIRECT']\n";
        let mut before = settings(
            Target {
                kind: TargetKind::Reject,
                ..Default::default()
            },
            true,
        );
        before.claude_protection = Some(std::sync::Arc::new(Protection::new(
            "profile".into(),
            "node".into(),
            Previous {
                mode: "rule".into(),
                proxy_mode: crate::model::ProxyMode::Tun,
                selectors: Default::default(),
            },
        )));
        assert_eq!(runtime(raw, &before, "profile", true).unwrap(), raw);
        let mut appearance = before.clone();
        appearance.theme_mode = Some(crate::config::ThemeMode::Light);
        appearance.accent_color = crate::config::AccentColor::Blue;
        appearance.text_scale = 125;
        appearance.tray_speed = !before.tray_speed;
        assert!(before.require_settings_change(&appearance).is_ok());
        for key in ["ipv6", "fake_ip", "udp", "update_prereleases"] {
            let mut value = serde_json::to_value(&appearance).unwrap();
            value[key] = (!value[key].as_bool().unwrap()).into();
            let next = serde_json::from_value(value).unwrap();
            assert!(before.require_settings_change(&next).is_err(), "{key}");
        }
        appearance.application_rules.clear();
        assert!(before.require_settings_change(&appearance).is_err());
    }
}
