use super::{Mapping, Store, Value, read_config_text};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::ErrorKind,
    path::{Component, Path},
};

pub const EDIT_LIMIT: usize = 128 * 1024;

pub fn read_draft(path: &Path) -> Result<String> {
    ensure!(
        std::fs::symlink_metadata(path)?.is_file(),
        "覆写草稿不是普通文件"
    );
    read_config_text(path, EDIT_LIMIT)
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Document {
    pub text: String,
    pub enabled: bool,
    pub revision: String,
}

impl Document {
    pub fn read(store: &Store, enabled: bool) -> Result<Self> {
        let text = match read_draft(&store.root.join("merge.yaml")) {
            Ok(text) => text,
            Err(error) if missing(&error) => String::new(),
            Err(error) => return Err(error),
        };
        let revision = revision(&text);
        Ok(Self {
            text,
            enabled,
            revision,
        })
    }

    pub fn verify(&self, revision: &str, enabled: bool) -> Result<()> {
        ensure!(
            self.revision == revision && self.enabled == enabled,
            "全局覆写已被另一客户端修改，请刷新后重试"
        );
        Ok(())
    }
}

pub fn revision(text: &str) -> String {
    crate::file_hash::hex(Sha256::digest(text.as_bytes()))
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Change {
    pub text: String,
    pub enabled: bool,
    pub revision: String,
    pub expected_enabled: bool,
}

impl std::fmt::Debug for Change {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Change")
            .field("bytes", &self.text.len())
            .field("enabled", &self.enabled)
            .field("expected_enabled", &self.expected_enabled)
            .finish_non_exhaustive()
    }
}

impl Change {
    pub fn checked(&self, store: &Store, enabled: bool) -> Result<GlobalOverride> {
        let current = Document::read(store, enabled)?;
        current.verify(&self.revision, self.expected_enabled)?;
        GlobalOverride::parse(&self.text)
    }
}

fn missing(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == ErrorKind::NotFound)
}

#[derive(Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct GlobalOverride {
    rule_providers: Mapping,
    prepend_rules: Vec<String>,
    append_rules: Vec<String>,
}

impl GlobalOverride {
    pub fn applied(store: &Store, enabled: bool, builtin: bool) -> Result<Option<Self>> {
        if !enabled || builtin {
            return Ok(None);
        }
        let text = read_draft(&store.root.join("merge.yaml"))?;
        Ok(Some(Self::parse(&text)?))
    }

    pub fn parse(text: &str) -> Result<Self> {
        ensure!(
            text.len() <= EDIT_LIMIT,
            "全局覆写不能超过 {EDIT_LIMIT} 字节"
        );
        let parsed = yaml_serde::from_str::<Option<Self>>(text)
            .map_err(|error| match error.location() {
                Some(location) => anyhow::Error::new(super::documents::SyntaxLocation {
                    line: location.line(),
                    column: location.column(),
                }),
                None => anyhow::anyhow!("全局覆写 YAML 格式错误"),
            })?
            .unwrap_or_default();
        for (name, provider) in &parsed.rule_providers {
            ensure!(
                name.as_str().is_some_and(|name| !name.trim().is_empty()),
                "全局规则集名称必须是非空字符串"
            );
            let provider = provider.as_mapping().context("全局规则集必须是配置对象")?;
            ensure!(!provider.contains_key("<<"), "全局规则集不支持 YAML 合并键");
            let kind = provider.get("type").and_then(Value::as_str);
            ensure!(
                matches!(kind, Some("http" | "file" | "inline")),
                "全局规则集类型必须是 http、file 或 inline"
            );
            ensure!(
                matches!(
                    provider.get("behavior").and_then(Value::as_str),
                    Some("domain" | "ipcidr" | "classical")
                ),
                "全局规则集 behavior 必须是 domain、ipcidr 或 classical"
            );
            if kind == Some("http") {
                let url = provider
                    .get("url")
                    .and_then(Value::as_str)
                    .context("HTTP 规则集缺少地址")?;
                ensure!(
                    reqwest::Url::parse(url)
                        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.has_host()),
                    "HTTP 规则集地址必须使用 http 或 https"
                );
            }
            if kind == Some("file") {
                ensure!(provider.contains_key("path"), "本地规则集缺少路径");
            }
            if let Some(path) = provider.get("path") {
                let path = path.as_str().context("全局规则集路径必须是字符串")?;
                ensure!(
                    !path.contains(['\\', ':', '\0'])
                        && Path::new(path).components().all(|component| {
                            matches!(component, Component::Normal(_) | Component::CurDir)
                        })
                        && Path::new(path)
                            .components()
                            .filter(|component| matches!(component, Component::Normal(_)))
                            .count()
                            >= 2,
                    "全局规则集路径必须指向运行目录下的子目录，例如 ruleset/local.yaml"
                );
            }
        }
        for (field, rules) in [
            ("prepend-rules", &parsed.prepend_rules),
            ("append-rules", &parsed.append_rules),
        ] {
            for (index, rule) in rules.iter().enumerate() {
                crate::rule_manager::parts(rule)
                    .with_context(|| format!("全局覆写 {field}[{}] 规则无效", index + 1))?;
            }
        }
        Ok(parsed)
    }

    pub(super) fn apply(&self, map: &mut Mapping) -> Result<()> {
        if self.rule_providers.is_empty()
            && self.prepend_rules.is_empty()
            && self.append_rules.is_empty()
        {
            return Ok(());
        }
        let mut providers = match map.remove("rule-providers") {
            None | Some(Value::Null) if self.rule_providers.is_empty() => None,
            None | Some(Value::Null) => Some(Mapping::new()),
            Some(Value::Mapping(providers)) => Some(providers),
            _ => anyhow::bail!("rule-providers 必须是配置对象"),
        };
        if let Some(providers) = &mut providers {
            for (name, provider) in &self.rule_providers {
                ensure!(
                    !providers.contains_key(name),
                    "全局规则集与订阅同名：{}",
                    name.as_str().unwrap_or_default()
                );
                providers.insert(name.clone(), provider.clone());
            }
        }
        let mut combined = match map.remove("rules") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Sequence(rules)) => rules,
            _ => anyhow::bail!("订阅 rules 必须是规则列表"),
        };
        let targets: BTreeSet<_> = ["proxies", "proxy-groups"]
            .into_iter()
            .filter_map(|key| map.get(key).and_then(Value::as_sequence))
            .flatten()
            .filter_map(|entry| entry.get("name").and_then(Value::as_str))
            .chain([
                "DIRECT",
                "REJECT",
                "REJECT-DROP",
                "COMPATIBLE",
                "PASS",
                "PASS-RULE",
                "GLOBAL",
            ])
            .collect();
        for (key, rules) in [
            ("prepend-rules", &self.prepend_rules),
            ("append-rules", &self.append_rules),
        ] {
            for (index, rule) in rules.iter().enumerate() {
                let parts = crate::rule_manager::parts(rule)?;
                let reference = || format!("{key}[{}]", index + 1);
                if parts.kind == "SUB-RULE" {
                    ensure!(
                        map.get("sub-rules")
                            .and_then(|value| value.get(parts.policy))
                            .is_some(),
                        "{} 引用的子规则不存在：{}",
                        reference(),
                        parts.policy
                    );
                } else {
                    ensure!(
                        targets.contains(parts.policy),
                        "{} 引用的策略不存在：{}",
                        reference(),
                        parts.policy
                    );
                }
                if parts.kind == "RULE-SET" {
                    ensure!(
                        providers
                            .as_ref()
                            .is_some_and(|providers| providers.contains_key(parts.payload)),
                        "{} 引用的规则集不存在：{}",
                        reference(),
                        parts.payload
                    );
                }
            }
        }
        let fallback = combined
            .iter()
            .position(|rule| {
                rule.as_str()
                    .and_then(|text| text.split_once(','))
                    .is_some_and(|(kind, _)| kind.trim().eq_ignore_ascii_case("MATCH"))
            })
            .unwrap_or(combined.len());
        combined.splice(
            fallback..fallback,
            self.append_rules.iter().cloned().map(Value::from),
        );
        combined.splice(..0, self.prepend_rules.iter().cloned().map(Value::from));
        if let Some(providers) = providers {
            map.insert(Value::from("rule-providers"), Value::Mapping(providers));
        }
        if !combined.is_empty() {
            map.insert(Value::from("rules"), Value::Sequence(combined));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OVERRIDE: &str = "\
rule-providers:
  local: {type: inline, behavior: domain, payload: ['+.fixture.invalid']}
prepend-rules: ['RULE-SET,local,DIRECT']
append-rules: ['DOMAIN,tail.fixture.invalid,REJECT']";

    #[test]
    fn combines_global_rules_with_the_subscription_without_touching_builtin() {
        let settings = super::super::Settings::default();
        let overrides = GlobalOverride::parse(OVERRIDE).unwrap();
        for fallback in ["", "  - MATCH,DIRECT\n"] {
            let source = format!(
                "proxies: []\nrules:\n  - DOMAIN,origin.fixture.invalid,DIRECT\n{fallback}"
            );
            for builtin in [false, true] {
                let raw = super::super::runtime_config_with_override(
                    &source,
                    &settings,
                    None,
                    builtin,
                    Some(&overrides),
                )
                .unwrap();
                let mut rules = if builtin {
                    vec!["DOMAIN,origin.fixture.invalid,DIRECT"]
                } else {
                    vec![
                        "RULE-SET,local,DIRECT",
                        "DOMAIN,origin.fixture.invalid,DIRECT",
                        "DOMAIN,tail.fixture.invalid,REJECT",
                    ]
                };
                if !fallback.is_empty() {
                    rules.push("MATCH,DIRECT");
                }
                let output: Value = yaml_serde::from_str(&raw).unwrap();
                assert_eq!(output["rules"], yaml_serde::to_value(rules).unwrap());
                assert_eq!(output["rule-providers"].get("local").is_some(), !builtin);
            }
        }
    }

    #[test]
    fn rejects_invalid_content_and_conflicting_references() {
        for input in [
            "secret: unsafe",
            "prepend-rules: [MATCH]\nprepend-rules: []",
            "rule-providers:\n  local: {type: file, behavior: domain, path: ../private.yaml}",
            "rule-providers:\n  local: {type: file, behavior: domain, path: C:\\private.yaml}",
            "rule-providers:\n  local: {type: file, behavior: domain, path: /private.yaml}",
            "rule-providers:\n  local: {type: file, behavior: domain, path: config.yaml}",
            "rule-providers:\n  local: {type: http, behavior: domain, url: file:///private.yaml}",
        ] {
            assert!(GlobalOverride::parse(input).is_err(), "{input}");
        }
        assert!(GlobalOverride::parse(&" ".repeat(EDIT_LIMIT + 1)).is_err());
        let source = "\
proxies: []
rule-providers:
  local: {type: inline, behavior: domain, payload: ['+.fixture.invalid']}";
        for input in [
            OVERRIDE,
            "append-rules: ['DOMAIN,fixture.invalid,missing']",
            "append-rules: ['RULE-SET,missing,DIRECT']",
            "append-rules: ['SUB-RULE,(DOMAIN,fixture.invalid),missing']",
        ] {
            let mut map = super::super::parse_profile(source).unwrap();
            assert!(
                GlobalOverride::parse(input)
                    .unwrap()
                    .apply(&mut map)
                    .is_err()
            );
        }
    }

    #[test]
    fn refuses_a_stale_document_and_recovers_the_committed_document() {
        let root = tempfile::tempdir().unwrap();
        let store = Store {
            root: root.path().into(),
        };
        let missing = Document::read(&store, false).unwrap();
        let change = Change {
            text: OVERRIDE.into(),
            enabled: true,
            revision: missing.revision,
            expected_enabled: false,
        };
        assert!(change.checked(&store, false).is_ok());
        assert!(change.checked(&store, true).is_err());
        let path = store.root.join("merge.yaml");
        let mut transaction = crate::profile_transaction::Transaction::new(&store.root).unwrap();
        transaction.stage(&path, change.text.as_bytes()).unwrap();
        transaction.prepare().unwrap();
        transaction.commit().unwrap();
        crate::profile_transaction::recover(&store.root).unwrap();
        assert_eq!(Document::read(&store, true).unwrap().text, OVERRIDE);
        assert!(
            GlobalOverride::applied(&store, true, false)
                .unwrap()
                .is_some()
        );
        assert!(
            GlobalOverride::applied(&store, true, true)
                .unwrap()
                .is_none()
        );
        assert!(
            GlobalOverride::applied(&store, false, false)
                .unwrap()
                .is_none()
        );
        assert!(change.checked(&store, false).is_err());
    }
}
