//! Mutations are applied to the owner's latest state while its lock is held.
use crate::{
    config::Settings,
    rule_manager::{CustomRule, Document, ProfileRules, Source},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum RuleChange {
    Add {
        raw: String,
        position: usize,
        enabled: bool,
    },
    Edit {
        rule: String,
        raw: String,
    },
    Remove {
        rule: String,
    },
    Move {
        rule: String,
        position: usize,
    },
    Enable {
        rule: String,
        enabled: bool,
    },
    Shortcut {
        index: usize,
        enabled: bool,
    },
}

pub(super) fn setting(settings: &Settings, key: &str, value: Value) -> Result<Settings> {
    ensure!(
        matches!(
            key,
            "mixed_port"
                | "secret"
                | "dark"
                | "theme_mode"
                | "accent_color"
                | "text_scale"
                | "tray_speed"
                | "update_prereleases"
                | "dns_override"
                | "ipv6"
                | "udp"
                | "ip_reputation_key"
                | "delay_interval_minutes"
                | "node_sort"
        ),
        "该设置不能直接修改；订阅、规则、模式和保护请使用对应命令"
    );
    let mut next = serde_json::to_value(settings)?;
    next[key] = value;
    let next: Settings = serde_json::from_value(next)?;
    next.validate()?;
    Ok(next)
}

pub(super) fn rule(
    document: &Document,
    draft: &mut ProfileRules,
    change: RuleChange,
) -> Result<()> {
    let (rows, _) = document.rows(draft);
    let locate = |value: &str| -> Result<usize> {
        if let Ok(number) = value.parse::<usize>() {
            ensure!(number > 0 && number <= rows.len(), "规则序号超出范围");
            return Ok(number - 1);
        }
        rows.iter()
            .position(|row| row.id == value || row.id.strip_prefix("c:") == Some(value))
            .context("未找到规则；使用 rule list 查看稳定 ID")
    };
    match change {
        RuleChange::Add {
            raw,
            position,
            enabled,
        } => {
            crate::rule_manager::parts(&raw)?;
            ensure!((1..=rows.len() + 1).contains(&position), "规则位置超出范围");
            let id = uuid::Uuid::new_v4().to_string();
            draft.custom.push(CustomRule {
                id: id.clone(),
                rule: raw,
                enabled,
            });
            draft.order = rows.iter().map(|row| row.id.clone()).collect();
            draft.order.insert(position - 1, format!("c:{id}"));
        }
        RuleChange::Move { rule, position } => {
            let at = locate(&rule)?;
            ensure!((1..=rows.len()).contains(&position), "规则位置超出范围");
            draft.order = rows.iter().map(|row| row.id.clone()).collect();
            let id = draft.order.remove(at);
            draft.order.insert(position - 1, id);
        }
        RuleChange::Shortcut { index, enabled } => {
            ensure!(index < 5 && !document.builtin, "快捷规则只适用于非内置订阅");
            draft.enabled[index] = enabled;
        }
        change => {
            let id = match &change {
                RuleChange::Edit { rule, .. }
                | RuleChange::Remove { rule }
                | RuleChange::Enable { rule, .. } => rule,
                _ => unreachable!(),
            };
            let row = &rows[locate(id)?];
            ensure!(
                row.source == Source::Custom,
                "订阅原规则只能调整顺序；快捷规则请使用 rule shortcut"
            );
            let id = row.id.strip_prefix("c:").context("自定义规则 ID 无效")?;
            let at = draft
                .custom
                .iter()
                .position(|item| item.id == id)
                .context("未找到自定义规则")?;
            match change {
                RuleChange::Edit { raw, .. } => {
                    crate::rule_manager::parts(&raw)?;
                    draft.custom[at].rule = raw;
                }
                RuleChange::Remove { .. } => {
                    draft.custom.remove(at);
                    draft.order.retain(|id| id != &row.id);
                }
                RuleChange::Enable { enabled, .. } => draft.custom[at].enabled = enabled,
                _ => unreachable!(),
            }
        }
    }
    draft.validate()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patches_keep_unrelated_and_protected_settings() {
        let settings = Settings {
            udp: false,
            ..Default::default()
        };
        let next = setting(&settings, "ipv6", Value::Bool(false)).unwrap();
        assert!(!next.udp && !next.ipv6);
        assert_eq!(next.secret, settings.secret);
        assert!(setting(&next, "claude_protection", Value::Null).is_err());
        assert!(setting(&next, "mixed_port", Value::from(0)).is_err());
        assert!(setting(&next, "controller_port", Value::from(9091)).is_err());
    }
    #[test]
    fn original_rules_are_read_only_and_new_rules_keep_order() {
        let doc =
            Document::from_raw("test", false, "proxies: []\nrules: [\"MATCH,DIRECT\"]").unwrap();
        let mut draft = ProfileRules::default();
        assert!(rule(&doc, &mut draft, RuleChange::Remove { rule: "1".into() }).is_err());
        rule(
            &doc,
            &mut draft,
            RuleChange::Add {
                raw: "DOMAIN,a.test,DIRECT".into(),
                position: 1,
                enabled: true,
            },
        )
        .unwrap();
        rule(
            &doc,
            &mut draft,
            RuleChange::Add {
                raw: "DOMAIN,b.test,REJECT".into(),
                position: 2,
                enabled: true,
            },
        )
        .unwrap();
        let rows = doc.rows(&draft).0;
        assert!(rows[0].raw.contains("a.test"));
        assert!(rows[1].raw.contains("b.test"));
        rule(
            &doc,
            &mut draft,
            RuleChange::Enable {
                rule: rows[0].id.clone(),
                enabled: false,
            },
        )
        .unwrap();
        assert!(!doc.rows(&draft).0[0].enabled);
    }
}
