use super::{Session, core};
use anyhow::{Result, ensure};
use core::application_rules::{Rule, TargetKind};
use serde_json::json;
use std::sync::Arc;

impl Session {
    pub fn application_policy_supported(&self) -> Result<bool> {
        Ok(self
            .native
            .request(json!({"op":"application-policy-capability"}))?["supported"]
            .as_bool()
            .unwrap_or(false))
    }

    pub(crate) fn application_runtime(
        &self,
        raw: &str,
        settings: &core::config::Settings,
        profile: &str,
    ) -> Result<String> {
        let supported = if settings
            .application_rules
            .get(profile)
            .is_some_and(|rules| !rules.is_empty())
            && settings.claude_protection.is_none()
            && settings.run_mode == "rule"
        {
            self.application_policy_supported()?
        } else {
            false
        };
        core::application_rules::runtime(raw, settings, profile, supported)
    }

    pub fn save_application_rules(&mut self, profile: &str, rules: Vec<Rule>) -> Result<()> {
        self.settings.require_mutable()?;
        ensure!(
            self.settings.active_profile.as_deref() == Some(profile),
            "订阅已切换，请重新打开应用规则"
        );
        ensure!(
            self.application_policy_supported()?,
            "当前系统不支持按应用指定出口，需要 Android 10 或更新版本"
        );
        core::application_rules::validate(&rules)?;
        let mut next = self.settings.clone();
        let rules: Vec<_> = rules
            .into_iter()
            .filter(|rule| rule.target.kind != TargetKind::Follow)
            .collect();
        if rules.is_empty() {
            next.application_rules.remove(profile);
        } else {
            next.application_rules
                .insert(profile.into(), Arc::new(rules));
        }
        self.commit_settings(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NativeHost;
    use serde_json::Value;

    struct Host(bool);
    impl NativeHost for Host {
        fn request(&self, _: Value) -> Result<Value> {
            Ok(json!({"supported":self.0}))
        }
    }

    #[tokio::test]
    async fn application_rules_follow_their_profile_survive_reopen_and_are_removed_with_it() {
        let root = tempfile::tempdir().unwrap();
        let native: Arc<dyn NativeHost> = Arc::new(Host(true));
        let mut session = Session::open(root.path().into(), native.clone()).unwrap();
        session
            .save_profile(
                None,
                "Applications",
                "",
                Some("proxies: []\nrules: ['MATCH,DIRECT']\n"),
            )
            .await
            .unwrap();
        let profile = session.profiles.last().unwrap().id.clone();
        session.activate(&profile).unwrap();
        let rule = Rule {
            package: "org.example.browser".into(),
            target: core::application_rules::Target {
                kind: TargetKind::Reject,
                ..Default::default()
            },
            reject_missing: true,
        };
        session
            .save_application_rules(&profile, vec![rule.clone()])
            .unwrap();
        assert!(
            session
                .runtime(&session.settings)
                .unwrap()
                .contains("PROCESS-NAME,org.example.browser,REJECT")
        );
        let mut session = Session::open(root.path().into(), native).unwrap();
        assert_eq!(
            session.settings.application_rules[&profile].as_ref(),
            &[rule]
        );
        let default = session
            .profiles
            .iter()
            .find(|profile| profile.is_default())
            .unwrap()
            .id
            .clone();
        session.activate(&default).unwrap();
        assert!(
            !session
                .runtime(&session.settings)
                .unwrap()
                .contains("cor-application-rules")
        );
        assert!(
            session
                .save_application_rules(&profile, Vec::new())
                .is_err()
        );
        session.delete(&profile).unwrap();
        assert!(!session.settings.application_rules.contains_key(&profile));
    }

    #[tokio::test]
    async fn unsupported_system_rejects_changes_without_losing_saved_settings() {
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::open(root.path().into(), Arc::new(Host(false))).unwrap();
        let profile = session.profile().id.clone();
        let before = std::fs::read(root.path().join("settings.json")).unwrap();
        assert!(
            session
                .save_application_rules(&profile, Vec::new())
                .is_err()
        );
        assert_eq!(
            std::fs::read(root.path().join("settings.json")).unwrap(),
            before
        );
    }
}
