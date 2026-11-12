use super::{Rule, Target, TargetKind, validate};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub label: Label,
    pub packages: BTreeSet<String>,
    pub target: Target,
    pub reject_missing: bool,
}

impl Default for Group {
    fn default() -> Self {
        Self {
            label: Label {
                id: uuid::Uuid::new_v4().to_string(),
                name: String::new(),
            },
            packages: BTreeSet::new(),
            target: Target::default(),
            reject_missing: true,
        }
    }
}

pub fn validate_label(label: &Label) -> Result<()> {
    ensure!(
        !label.id.is_empty()
            && label.id.len() <= 80
            && label
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "应用规则标识无效"
    );
    ensure!(
        !label.name.trim().is_empty()
            && label.name == label.name.trim()
            && label.name.chars().count() <= 40
            && !label.name.chars().any(char::is_control),
        "规则名称需为 1～40 个字符，不含换行"
    );
    Ok(())
}

pub fn identity(rule: &Rule) -> String {
    rule.group
        .as_ref()
        .map(|label| label.id.clone())
        .unwrap_or_else(|| {
            let content = serde_json::to_vec(&(&rule.target, rule.reject_missing))
                .expect("application target is serializable");
            format!("legacy-{}", crate::file_hash::hex(Sha256::digest(content)))
        })
}

pub fn list(rules: &[Rule]) -> Vec<Group> {
    let mut result: Vec<Group> = Vec::new();
    let mut names: BTreeSet<_> = rules
        .iter()
        .filter_map(|rule| rule.group.as_ref())
        .map(|label| label.name.to_lowercase())
        .collect();
    for rule in rules {
        let id = identity(rule);
        if let Some(group) = result.iter_mut().find(|group| group.label.id == id) {
            group.packages.insert(rule.package.clone());
        } else {
            result.push(Group {
                label: rule.group.clone().unwrap_or_else(|| {
                    let mut index = result.len() + 1;
                    let mut name = format!("应用规则 {index}");
                    while names.contains(&name.to_lowercase()) {
                        index += 1;
                        name = format!("应用规则 {index}");
                    }
                    names.insert(name.to_lowercase());
                    Label { id, name }
                }),
                packages: BTreeSet::from([rule.package.clone()]),
                target: rule.target.clone(),
                reject_missing: rule.reject_missing,
            });
        }
    }
    result
}

pub fn remove(rules: &[Rule], id: &str) -> Vec<Rule> {
    rules
        .iter()
        .filter(|rule| identity(rule) != id)
        .cloned()
        .collect()
}

pub fn replace(rules: &[Rule], group: &Group) -> Result<Vec<Rule>> {
    validate_label(&group.label)?;
    ensure!(!group.packages.is_empty(), "请至少选择一个应用");
    ensure!(
        group.target.kind != TargetKind::Follow,
        "请选择规则组的出口"
    );
    ensure!(
        !list(rules)
            .iter()
            .any(|existing| existing.label.id != group.label.id
                && existing.label.name.to_lowercase() == group.label.name.to_lowercase()),
        "规则名称已存在，请换一个名称"
    );
    let mut result = remove(rules, &group.label.id);
    ensure!(
        !result
            .iter()
            .any(|rule| group.packages.contains(&rule.package)),
        "部分应用已属于其他规则组，请先编辑原规则组"
    );
    result.extend(group.packages.iter().map(|package| Rule {
        package: package.clone(),
        target: group.target.clone(),
        reject_missing: group.reject_missing,
        group: Some(group.label.clone()),
    }));
    validate(&result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(name: &str, packages: &[&str]) -> Group {
        Group {
            label: Label {
                id: name.into(),
                name: name.into(),
            },
            packages: packages.iter().map(|package| (*package).into()).collect(),
            target: Target {
                kind: TargetKind::Direct,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn group_edit_round_trips_and_removes_deselected_applications_atomically() {
        let first = group("浏览器", &["org.example.first", "org.example.second"]);
        let mut first = Group {
            label: Label {
                id: "browser".into(),
                ..first.label
            },
            ..first
        };
        let rules = replace(&[], &first).unwrap();
        let serialized = serde_json::to_vec(&rules).unwrap();
        let restored: Vec<Rule> = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(list(&restored), vec![first.clone()]);
        first.packages.remove("org.example.second");
        first.label.name = "日常浏览".into();
        let next = replace(&restored, &first).unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].group.as_ref().unwrap().name, "日常浏览");
        assert!(remove(&next, "browser").is_empty());
    }

    #[test]
    fn conflicts_empty_groups_and_inconsistent_metadata_are_rejected_without_mutation() {
        let first = group("first", &["org.example.first"]);
        let saved = replace(&[], &first).unwrap();
        let before = saved.clone();
        assert!(replace(&saved, &group("second", &["org.example.first"])).is_err());
        assert!(replace(&saved, &group("first", &[])).is_err());
        let mut duplicate = group("second", &["org.example.second"]);
        duplicate.label.name = "FIRST".into();
        assert!(replace(&saved, &duplicate).is_err());
        assert_eq!(saved, before);
        let mut inconsistent = saved.clone();
        let mut other = saved[0].clone();
        other.package = "org.example.other".into();
        other.reject_missing = false;
        inconsistent.push(other);
        assert!(validate(&inconsistent).is_err());
    }

    #[test]
    fn legacy_rules_group_by_target_without_changing_their_fallback() {
        let old: Vec<Rule> = serde_json::from_str(
            r#"[{"package":"org.example.first","target":{"kind":"direct"},"reject_missing":false},
                {"package":"org.example.second","target":{"kind":"direct"},"reject_missing":false}]"#,
        )
        .unwrap();
        let groups = list(&old);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].packages.len(), 2);
        assert!(!groups[0].reject_missing);
        let named = replace(&old, &groups[0]).unwrap();
        assert_eq!(list(&named), groups);
        let mut settings = crate::config::Settings::default();
        settings
            .application_rules
            .insert("profile".into(), std::sync::Arc::new(old));
        let raw = "proxies: []\nrules: ['MATCH,DIRECT']\n";
        let before = super::super::runtime(raw, &settings, "profile", true).unwrap();
        settings
            .application_rules
            .insert("profile".into(), std::sync::Arc::new(named));
        assert_eq!(
            super::super::runtime(raw, &settings, "profile", true).unwrap(),
            before
        );
        assert!(Group::default().reject_missing);
    }

    #[test]
    fn labels_reject_blank_oversized_and_multiline_names() {
        let mut label = Label {
            id: "browser".into(),
            name: "浏".repeat(40),
        };
        assert!(validate_label(&label).is_ok());
        for name in [
            "".into(),
            " ".into(),
            " 前后空格 ".into(),
            "多\n行".into(),
            "浏".repeat(41),
        ] {
            label.name = name;
            assert!(validate_label(&label).is_err());
        }
        label.name = "浏览器".into();
        label.id = "../browser".into();
        assert!(validate_label(&label).is_err());
    }

    #[test]
    fn legacy_groups_do_not_reuse_existing_names_or_change_saved_rules() {
        let mut existing = group("named", &["org.example.named"]);
        existing.label.name = "应用规则 1".into();
        let mut saved = replace(&[], &existing).unwrap();
        saved.push(Rule {
            package: "org.example.legacy".into(),
            target: Target {
                kind: TargetKind::Direct,
                ..Default::default()
            },
            reject_missing: false,
            group: None,
        });
        let before = saved.clone();
        let groups = list(&saved);
        assert_eq!(groups.len(), 2);
        assert_ne!(groups[0].label.name, groups[1].label.name);
        assert!(!groups[1].reject_missing);
        assert_eq!(saved, before);
    }

    #[test]
    fn editing_the_first_legacy_group_keeps_other_legacy_groups_and_their_names() {
        let saved: Vec<Rule> = serde_json::from_str(
            r#"[{"package":"org.example.first","target":{"kind":"direct"},"reject_missing":false},
                {"package":"org.example.second","target":{"kind":"reject"},"reject_missing":true}]"#,
        )
        .unwrap();
        let before = list(&saved);
        assert_eq!(before.len(), 2);
        let mut first = before[0].clone();
        first.packages.insert("org.example.third".into());
        let updated = replace(&saved, &first).unwrap();
        let after = list(&updated);
        assert_eq!(after.len(), 2);
        assert_eq!(
            after.iter().find(|group| group.label.id == first.label.id),
            Some(&first)
        );
        assert_eq!(
            after
                .iter()
                .find(|group| group.label.id == before[1].label.id),
            Some(&before[1])
        );
    }
}
