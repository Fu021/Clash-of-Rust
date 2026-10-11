//! Per-subscription edits. Original YAML is never rewritten; identities survive
//! subscription updates without copying the subscription's rule text into settings.
use crate::config::{SUBSCRIPTION_RULES, Settings, parse_profile};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileRules {
    pub enabled: [bool; 5],
    pub custom: Vec<CustomRule>,
    /// Only persisted after an explicit reorder. Contains identities, not rule text.
    pub order: Vec<String>,
}
impl ProfileRules {
    pub fn for_profile(settings: &Settings, id: &str, builtin: bool) -> Arc<Self> {
        settings.rule_profiles.get(id).cloned().unwrap_or_else(|| {
            Arc::new(Self {
                enabled: if builtin {
                    [false; 5]
                } else {
                    settings.rule_overrides.enabled
                },
                ..Self::default()
            })
        })
    }
    pub fn validate(&self) -> Result<()> {
        if self.custom.len() > 1000 {
            bail!("自定义规则不能超过 1000 条");
        }
        let mut ids = BTreeSet::new();
        for rule in &self.custom {
            uuid::Uuid::parse_str(&rule.id).context("自定义规则 ID 无效")?;
            if !ids.insert(&rule.id) {
                bail!("自定义规则 ID 重复");
            }
            // Disabled rules still need valid syntax; the core validates enabled
            // rules and referenced policies/providers before committing a draft.
            parts(&rule.rule)?;
        }
        if self.order.iter().any(|id| id.len() > 80) {
            bail!("规则排序标识无效");
        }
        let unique: BTreeSet<_> = self.order.iter().collect();
        if unique.len() != self.order.len() {
            bail!("规则排序不能包含重复项");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomRule {
    pub id: String,
    pub rule: String,
    pub enabled: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Custom,
    Shortcut,
    Subscription,
}
impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Custom => "自定义",
            Self::Shortcut => "快捷规则",
            Self::Subscription => "订阅",
        })
    }
}
#[derive(Debug, Clone)]
pub struct Row {
    pub id: String,
    pub raw: Arc<str>,
    pub source: Source,
    pub enabled: bool,
    pub duplicate: bool,
}
#[derive(Debug, Clone)]
pub struct Document {
    pub profile_id: String,
    pub builtin: bool,
    pub original: Vec<Row>,
    pub policies: Vec<String>,
    pub included: [bool; 5],
}
impl Document {
    pub fn from_raw(id: &str, builtin: bool, raw: &str) -> Result<Self> {
        let map = parse_profile(raw)?;
        let original = original_rows(&map)?;
        let included = std::array::from_fn(|i| {
            original
                .iter()
                .any(|row| same_shortcut(&row.raw, SUBSCRIPTION_RULES[i]))
        });
        let mut policies: BTreeSet<String> = ["DIRECT", "REJECT", "REJECT-DROP", "PASS"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        for key in ["proxy-groups", "proxies"] {
            if let Some(Value::Sequence(items)) = map.get(Value::from(key)) {
                for item in items {
                    if let Some(name) = item.get("name").and_then(Value::as_str) {
                        policies.insert(name.to_owned());
                    }
                }
            }
        }
        Ok(Self {
            profile_id: id.to_owned(),
            builtin,
            original,
            policies: policies.into_iter().collect(),
            included,
        })
    }
    pub fn rows(&self, draft: &ProfileRules) -> (Vec<Row>, Vec<String>) {
        let (rows, mut warnings) = compose(&self.original, draft, self.builtin);
        let mut missing = BTreeSet::new();
        for custom in draft.custom.iter().filter(|r| r.enabled) {
            if let Ok(parts) = parts(&custom.rule)
                && parts.kind != "SUB-RULE"
                && !self.policies.iter().any(|p| p == parts.policy)
            {
                missing.insert(parts.policy);
            }
        }
        for policy in missing {
            warnings.push(format!(
                "订阅中未找到策略“{policy}”，请确认名称或改选策略；应用时会再次校验。"
            ));
        }
        (rows, warnings)
    }
}

pub struct Parts<'a> {
    pub kind: &'a str,
    pub payload: &'a str,
    pub policy: &'a str,
    pub no_resolve: bool,
}
/// Split only top-level commas, so logical rules remain editable in text mode.
pub fn parts(raw: &str) -> Result<Parts<'_>> {
    if raw.len() > 8192 || raw.contains(['\n', '\r', '\0']) {
        bail!("规则应为单行文本，且不超过 8192 字节");
    }
    let raw = raw.trim();
    let mut fields = Vec::new();
    let logical = ["AND,", "OR,", "NOT,", "SUB-RULE,"]
        .iter()
        .any(|prefix| raw.starts_with(prefix));
    let mut depth = 0usize;
    let mut begin = 0;
    for (index, ch) in raw.char_indices() {
        match ch {
            '(' if logical => depth += 1,
            ')' if logical => {
                depth = depth.checked_sub(1).context("规则括号不匹配")?;
            }
            ',' if depth == 0 => {
                fields.push((begin, index));
                begin = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        bail!("规则括号不匹配");
    }
    fields.push((begin, raw.len()));
    let field = |i: usize| raw[fields[i].0..fields[i].1].trim();
    let kind = field(0);
    if kind.is_empty()
        || !kind
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c == b'-' || c.is_ascii_digit())
    {
        bail!("规则类型无效，应使用 DOMAIN、GEOIP、MATCH 等大写类型");
    }
    let no_resolve = fields.len() > 2 && field(fields.len() - 1).eq_ignore_ascii_case("no-resolve");
    let count = fields.len() - usize::from(no_resolve);
    if (kind == "MATCH" && count != 2) || (kind != "MATCH" && count != 3) {
        bail!("规则格式应为 类型,内容,策略（MATCH 为 MATCH,策略）");
    }
    let policy = field(count - 1);
    if policy.is_empty() {
        bail!("请选择规则策略");
    }
    let payload = if count == 3 { field(1) } else { "" };
    if count == 3 && payload.is_empty() {
        bail!("规则内容不能为空");
    }
    Ok(Parts {
        kind,
        payload,
        policy,
        no_resolve,
    })
}
fn original_rows(map: &Mapping) -> Result<Vec<Row>> {
    let previous = match map.get(Value::from("rules")) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Sequence(rules)) => rules,
        _ => bail!("订阅 rules 必须是规则列表"),
    };
    let mut occurrences = BTreeMap::<String, usize>::new();
    previous
        .iter()
        .map(|value| {
            let raw = value.as_str().context("订阅规则必须是字符串")?;
            let digest = Sha256::digest(raw.trim().as_bytes());
            let hash: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
            let occurrence = occurrences.entry(hash.clone()).or_default();
            let id = format!("s:{hash}:{occurrence}");
            *occurrence += 1;
            Ok(Row {
                id,
                raw: Arc::from(raw),
                source: Source::Subscription,
                enabled: true,
                duplicate: false,
            })
        })
        .collect()
}
pub fn same_shortcut(a: &str, b: &str) -> bool {
    let mut a = a.split(',').map(str::trim);
    let mut b = b.split(',').map(str::trim);
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => {}
            _ => return false,
        }
    }
}
fn compose(original: &[Row], draft: &ProfileRules, builtin: bool) -> (Vec<Row>, Vec<String>) {
    let mut rows: Vec<Row> = Vec::with_capacity(original.len() + draft.custom.len() + 5);
    let mut warnings = Vec::new();
    let mut duplicates = 0;
    for rule in &draft.custom {
        let shortcut = SUBSCRIPTION_RULES
            .iter()
            .find(|shortcut| same_shortcut(&rule.rule, shortcut));
        let duplicate = rule.enabled
            && original.iter().chain(rows.iter()).any(|row: &Row| {
                row.enabled
                    && (row.raw.trim() == rule.rule.trim()
                        || shortcut.is_some_and(|shortcut| same_shortcut(&row.raw, shortcut)))
            });
        duplicates += usize::from(duplicate);
        rows.push(Row {
            id: format!("c:{}", rule.id),
            raw: Arc::from(rule.rule.as_str()),
            source: Source::Custom,
            enabled: rule.enabled && !duplicate,
            duplicate,
        });
    }
    if duplicates > 0 {
        warnings.push(format!(
            "有 {duplicates} 条自定义规则已包含在订阅或其他规则中，保留原规则，不重复加入。"
        ));
    }
    for (i, raw) in SUBSCRIPTION_RULES.iter().enumerate() {
        if !builtin
            && draft.enabled[i]
            && !original
                .iter()
                .chain(rows.iter().filter(|row| row.enabled))
                .any(|row| same_shortcut(&row.raw, raw))
        {
            rows.push(Row {
                id: format!("q:{i}"),
                raw: Arc::from(*raw),
                source: Source::Shortcut,
                enabled: true,
                duplicate: false,
            });
        }
    }
    rows.extend_from_slice(original);
    if !draft.order.is_empty() {
        let mut remaining: BTreeMap<String, Row> =
            rows.into_iter().map(|row| (row.id.clone(), row)).collect();
        rows = Vec::with_capacity(remaining.len());
        let mut missing = 0;
        for id in &draft.order {
            if let Some(row) = remaining.remove(id) {
                rows.push(row);
            } else if id.starts_with("s:") {
                missing += 1;
            }
        }
        if missing > 0 {
            warnings.push(format!("订阅更新后有 {missing} 条原规则已删除或更改；旧排序引用已跳过，可恢复订阅顺序重新整理。"));
        }
        // Bucket new rows before a surviving original neighbour. This is
        // O(n log n), including a completely replaced, very large subscription.
        let positions: BTreeMap<_, _> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.as_str(), i))
            .collect();
        let fallback = rows
            .iter()
            .position(|r| r.raw.trim_start().starts_with("MATCH,"))
            .unwrap_or(rows.len());
        let mut buckets: BTreeMap<usize, (Vec<Row>, Vec<Row>)> = BTreeMap::new();
        let mut next = fallback;
        for original in original.iter().rev() {
            if let Some(&at) = positions.get(original.id.as_str()) {
                next = at;
            } else if let Some(row) = remaining.remove(&original.id) {
                buckets.entry(next).or_default().1.push(row);
            }
        }
        for (_, upstream) in buckets.values_mut() {
            upstream.reverse();
        }
        let custom_at = rows
            .iter()
            .position(|r| r.source != Source::Custom)
            .unwrap_or(rows.len());
        for rule in &draft.custom {
            if let Some(row) = remaining.remove(&format!("c:{}", rule.id)) {
                buckets.entry(custom_at).or_default().0.push(row);
            }
        }
        let shortcut_at = rows
            .iter()
            .position(|r| r.source == Source::Subscription)
            .unwrap_or(rows.len());
        for i in 0..5 {
            if let Some(row) = remaining.remove(&format!("q:{i}")) {
                buckets.entry(shortcut_at).or_default().0.push(row);
            }
        }
        let mut merged = Vec::with_capacity(
            rows.len()
                + buckets
                    .values()
                    .map(|(a, b)| a.len() + b.len())
                    .sum::<usize>(),
        );
        for (at, row) in rows.into_iter().enumerate() {
            if let Some((extra, upstream)) = buckets.remove(&at) {
                merged.extend(extra);
                merged.extend(upstream);
            }
            merged.push(row);
        }
        for (extra, upstream) in buckets.into_values() {
            merged.extend(extra);
            merged.extend(upstream);
        }
        rows = merged;
    }
    if let Some(at) = rows
        .iter()
        .position(|r| r.enabled && r.raw.trim_start().starts_with("MATCH,"))
    {
        let hidden = rows[at + 1..].iter().filter(|r| r.enabled).count();
        if hidden > 0 {
            warnings.push(format!(
                "MATCH 之后有 {hidden} 条规则不会被匹配，请检查顺序。"
            ));
        }
    }
    (rows, warnings)
}
pub fn apply(map: &mut Mapping, draft: &ProfileRules, builtin: bool) -> Result<()> {
    draft.validate()?;
    let original = original_rows(map)?;
    let (rows, _) = compose(&original, draft, builtin);
    map.insert(
        Value::from("rules"),
        Value::Sequence(
            rows.into_iter()
                .filter(|r| r.enabled)
                .map(|r| Value::from(r.raw.as_ref()))
                .collect(),
        ),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(rules: &[&str]) -> Document {
        let raw =
            serde_yaml::to_string(&serde_json::json!({"proxies": [], "rules": rules})).unwrap();
        Document::from_raw("test", false, &raw).unwrap()
    }
    fn custom(raw: &str, enabled: bool) -> CustomRule {
        CustomRule {
            id: uuid::Uuid::new_v4().to_string(),
            rule: raw.into(),
            enabled,
        }
    }
    #[test]
    fn defaults_and_existing_shortcuts_preserve_subscription() {
        let doc = document(&["GEOSITE, CN ,direct", "MATCH,DIRECT"]);
        assert!(doc.included[3]);
        let draft = ProfileRules {
            enabled: [true; 5],
            ..Default::default()
        };
        let (rows, _) = doc.rows(&draft);
        assert_eq!(rows.len(), 6);
        assert_eq!(&*rows[4].raw, "GEOSITE, CN ,direct");
        assert_eq!(doc.rows(&ProfileRules::default()).0.len(), 2);
    }
    #[test]
    fn disabled_custom_rules_do_not_reach_runtime() {
        let mut map = parse_profile("proxies: []\nrules: [\"MATCH,DIRECT\"]").unwrap();
        let draft = ProfileRules {
            custom: vec![custom("DOMAIN,example.com,REJECT", false)],
            ..Default::default()
        };
        apply(&mut map, &draft, false).unwrap();
        assert_eq!(map[Value::from("rules")].as_sequence().unwrap().len(), 1);
    }
    #[test]
    fn updates_rebase_and_new_rules_stay_before_fallback() {
        let doc = document(&[
            "DOMAIN,a.test,DIRECT",
            "DOMAIN,b.test,REJECT",
            "MATCH,DIRECT",
        ]);
        let draft = ProfileRules {
            order: [1, 0, 2].map(|i| doc.original[i].id.clone()).to_vec(),
            ..Default::default()
        };
        let updated = document(&[
            "DOMAIN,a.test,DIRECT",
            "DOMAIN,new.test,DIRECT",
            "MATCH,DIRECT",
        ]);
        let (rows, warnings) = updated.rows(&draft);
        assert_eq!(&*rows[1].raw, "DOMAIN,new.test,DIRECT");
        assert_eq!(&*rows[2].raw, "MATCH,DIRECT");
        assert_eq!(warnings.len(), 1);
    }
    #[test]
    fn duplicate_subscription_rules_have_distinct_stable_ids() {
        let a = document(&[
            "DOMAIN,a.test,DIRECT",
            "DOMAIN,a.test,DIRECT",
            "MATCH,DIRECT",
        ]);
        let b = document(&[
            "DOMAIN,new.test,DIRECT",
            "DOMAIN,a.test,DIRECT",
            "DOMAIN,a.test,DIRECT",
            "MATCH,DIRECT",
        ]);
        assert_ne!(a.original[0].id, a.original[1].id);
        assert_eq!(a.original[0].id, b.original[1].id);
        assert_eq!(a.original[1].id, b.original[2].id);
    }
    #[test]
    fn parser_preserves_logical_rules_and_rejects_multiline() {
        let p = parts("AND,((DOMAIN,a.test),(NETWORK,TCP)),DIRECT").unwrap();
        assert_eq!(p.policy, "DIRECT");
        assert_eq!(
            parts(r"DOMAIN-REGEX,[(]example[)],DIRECT").unwrap().payload,
            "[(]example[)]"
        );
        assert_eq!(p.payload, "((DOMAIN,a.test),(NETWORK,TCP))");
        assert!(parts("DOMAIN,a.test,DIRECT\nMATCH,REJECT").is_err());
        assert!(parts("MATCH,").is_err());
        assert!(parts("DOMAIN,a.test,DIRECT,wrong").is_err());
    }
    #[test]
    fn profile_preferences_are_isolated_and_upgrade_keeps_old_switches() {
        let mut settings = Settings::default();
        settings.rule_overrides.enabled[0] = true;
        assert!(ProfileRules::for_profile(&settings, "old", false).enabled[0]);
        settings
            .rule_profiles
            .insert("edited".into(), Arc::new(ProfileRules::default()));
        assert!(!ProfileRules::for_profile(&settings, "edited", false).enabled[0]);
        assert!(ProfileRules::for_profile(&settings, "other", false).enabled[0]);
        assert_eq!(
            ProfileRules::for_profile(&settings, "default", true).enabled,
            [false; 5]
        );
    }
    #[test]
    fn upstream_duplicates_are_not_reinserted_and_shortcuts_keep_case_compatibility() {
        let doc = document(&[
            "GEOSITE, CN ,DIRECT",
            "DOMAIN,a.test,DIRECT",
            "MATCH,DIRECT",
        ]);
        let draft = ProfileRules {
            enabled: [false, false, false, true, false],
            custom: vec![
                custom("GEOSITE,CN,DIRECT", true),
                custom("DOMAIN,a.test,DIRECT", true),
            ],
            ..Default::default()
        };
        let (rows, warnings) = doc.rows(&draft);
        assert_eq!(rows.iter().filter(|r| r.enabled).count(), 3);
        assert!(rows[..2].iter().all(|r| r.duplicate));
        assert_eq!(warnings.len(), 1);
    }
}
