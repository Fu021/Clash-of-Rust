use anyhow::{Context, Result, ensure};
use clash_mobile_runtime::{Session, core};
use core::application_rules::{Rule, Target, TargetKind};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeSet;

#[derive(Default, Deserialize)]
struct Application {
    name: String,
    package: String,
    uid: u32,
}

#[derive(Default)]
pub(super) struct Editor {
    profile: String,
    profile_name: String,
    supported: bool,
    applications: Vec<Application>,
    targets: Vec<Target>,
    query: String,
    scope: String,
    selected: BTreeSet<String>,
    revision: i32,
    form: (String, String, bool),
    source_hash: String,
}

#[derive(Clone, Default)]
pub(super) struct View {
    pub profile: String,
    pub supported: bool,
    pub applications: Vec<String>,
    pub targets: Vec<String>,
    pub rules: Vec<(String, String, String, String)>,
    pub revision: i32,
    pub form: (String, String, bool),
    pub status: String,
}

fn label(application: &Application) -> String {
    format!(
        "{} · {}",
        application.name.replace(char::is_control, " "),
        application.package
    )
}

fn captured(scope: &str, selected: &BTreeSet<String>, package: &str) -> bool {
    match scope {
        "only" => selected.contains(package),
        "exclude" => !selected.contains(package),
        _ => true,
    }
}

impl Editor {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub async fn request(
        &mut self,
        session: &mut Session,
        action: &str,
        value: &str,
    ) -> Result<String> {
        if action == "close" {
            self.clear();
            return Ok(String::new());
        }
        if action == "open" || action == "refresh" {
            #[derive(Deserialize)]
            struct Catalog {
                applications: Vec<Application>,
                mode: String,
                selected: BTreeSet<String>,
            }
            let catalog: Catalog =
                serde_json::from_value(session.native.request(json!({"op":"applications-list"}))?)?;
            let raw = session.raw()?;
            let mut targets = core::application_rules::targets(&raw)?;
            if session.running {
                let sources = session.api.application_provider_targets().await?;
                for (provider, names) in sources {
                    for name in names {
                        targets.push(Target {
                            kind: TargetKind::Node,
                            name,
                            provider: provider.clone(),
                            ..Default::default()
                        });
                    }
                }
            }
            self.profile = session.profile().id.clone();
            self.source_hash =
                core::file_hash::hash_file(&session.store.profile_path(&self.profile)?)?.0;
            self.profile_name = session.profile().name.clone();
            self.supported = session.application_policy_supported()?;
            self.applications = catalog.applications;
            self.scope = catalog.mode;
            self.selected = catalog.selected;
            self.targets = targets;
            self.query.clear();
            self.form = (String::new(), "跟随配置".into(), false);
            self.revision += 1;
            return Ok(String::new());
        }
        if action == "query" {
            self.query = value.into();
            return Ok(String::new());
        }
        ensure!(
            session.settings.active_profile.as_deref() == Some(self.profile.as_str()),
            "订阅已切换，请重新打开应用规则"
        );
        if action == "edit" {
            let rule = session
                .settings
                .application_rules
                .get(&self.profile)
                .and_then(|rules| rules.iter().find(|rule| rule.package == value))
                .context("应用规则不存在")?;
            let application = self
                .applications
                .iter()
                .find(|application| application.package == value);
            self.form = (
                application
                    .map(label)
                    .unwrap_or_else(|| rule.package.clone()),
                rule.target.label(),
                rule.reject_missing,
            );
            self.revision += 1;
            return Ok(String::new());
        }
        session.settings.require_mutable()?;
        ensure!(
            self.supported,
            "当前系统不支持按应用指定出口，需要 Android 10 或更新版本"
        );
        ensure!(
            core::file_hash::hash_file(&session.store.profile_path(&self.profile)?)?.0
                == self.source_hash,
            "订阅内容已更新，请重新打开应用规则"
        );
        let mut rules = session
            .settings
            .application_rules
            .get(&self.profile)
            .map(|rules| rules.as_ref().clone())
            .unwrap_or_default();
        if action == "delete" {
            ensure!(
                rules.iter().any(|rule| rule.package == value),
                "应用规则不存在"
            );
            rules.retain(|rule| rule.package != value);
        } else {
            ensure!(action == "save", "应用规则操作无效");
            let mut parts = value.splitn(3, '\n');
            let package_label = parts.next().context("请选择应用")?;
            let target_label = parts.next().context("请选择出站")?;
            let reject_missing = match parts.next() {
                Some("true") => true,
                Some("false") => false,
                _ => anyhow::bail!("目标失效策略无效"),
            };
            let application = self
                .applications
                .iter()
                .find(|application| label(application) == package_label)
                .context("应用清单已改变，请重新选择")?;
            let target = self
                .targets
                .iter()
                .find(|target| target.label() == target_label)
                .context("目标已失效，请重新选择")?
                .clone();
            ensure!(
                target.kind == TargetKind::Follow
                    || captured(&self.scope, &self.selected, &application.package),
                "此应用未进入 VPN，请先调整应用分流范围"
            );
            for other in &self.applications {
                if other.uid == application.uid && other.package != application.package {
                    if let Some(rule) = rules.iter().find(|rule| rule.package == other.package) {
                        ensure!(
                            rule.target == target && rule.reject_missing == reject_missing,
                            "这些应用共享系统身份，需要指定相同出站"
                        );
                    }
                }
            }
            rules.retain(|rule| rule.package != application.package);
            if target.kind != TargetKind::Follow {
                rules.push(Rule {
                    package: application.package.clone(),
                    target,
                    reject_missing,
                });
            }
        }
        session.save_application_rules(&self.profile, rules)?;
        self.form = (String::new(), "跟随配置".into(), false);
        self.revision += 1;
        if session.vpn {
            if let Err(error) = session
                .native
                .request(json!({"op":"applications-reconnect"}))
            {
                return Ok(format!("应用规则已保存，但 VPN 重连未发起：{error:#}"));
            }
            Ok("应用规则已保存，正在重新连接 VPN".into())
        } else {
            Ok("应用规则已保存，下次连接时生效".into())
        }
    }

    pub fn view(&self, session: &Session) -> View {
        if self.profile.is_empty() {
            return View::default();
        }
        let query = self.query.to_lowercase();
        let mut applications: Vec<_> = self
            .applications
            .iter()
            .filter(|application| {
                query.is_empty()
                    || application.name.to_lowercase().contains(&query)
                    || application.package.to_lowercase().contains(&query)
            })
            .take(80)
            .map(label)
            .collect();
        if !self.form.0.is_empty() && !applications.contains(&self.form.0) {
            applications.insert(0, self.form.0.clone());
        }
        let targets = self.targets.iter().map(Target::label).collect();
        let rules = session
            .settings
            .application_rules
            .get(&self.profile)
            .into_iter()
            .flat_map(|rules| rules.iter())
            .map(|rule| {
                let application = self
                    .applications
                    .iter()
                    .find(|application| application.package == rule.package);
                let status = if application.is_none() {
                    "应用未安装"
                } else if !captured(&self.scope, &self.selected, &rule.package) {
                    "未进入 VPN"
                } else if !self.supported {
                    "系统不支持"
                } else if session.settings.run_mode != "rule" {
                    "仅规则模式生效"
                } else if session.settings.claude_protection.is_some() {
                    "Claude 保护期间暂停"
                } else if !self.targets.contains(&rule.target) {
                    if rule.reject_missing {
                        "目标失效 · 阻止连接"
                    } else {
                        "目标失效 · 跟随配置"
                    }
                } else if !session.vpn {
                    "等待连接 VPN"
                } else {
                    "已应用 · 新连接生效"
                };
                (
                    rule.package.clone(),
                    application
                        .map(|application| application.name.clone())
                        .unwrap_or_else(|| rule.package.clone()),
                    rule.target.label(),
                    status.into(),
                )
            })
            .collect();
        View {
            profile: self.profile_name.clone(),
            supported: self.supported,
            applications,
            targets,
            rules,
            revision: self.revision,
            form: self.form.clone(),
            status: if !self.supported {
                "按应用指定出口需要 Android 10 或更新版本，现有应用范围分流仍可使用。".into()
            } else if session.settings.active_profile.as_deref() != Some(&self.profile) {
                "订阅已切换，请重新打开应用规则。".into()
            } else if session.settings.run_mode != "rule" {
                "应用规则仅在规则模式生效。".into()
            } else {
                "只作用于进入 VPN 的应用；未指定的应用跟随配置。".into()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_outbound_never_overrides_excluded_scope() {
        let selected = BTreeSet::from(["org.example.app".into()]);
        assert!(captured("all", &selected, "org.example.other"));
        assert!(captured("only", &selected, "org.example.app"));
        assert!(!captured("only", &selected, "org.example.other"));
        assert!(!captured("exclude", &selected, "org.example.app"));
        assert!(captured("exclude", &selected, "org.example.other"));
    }
}
