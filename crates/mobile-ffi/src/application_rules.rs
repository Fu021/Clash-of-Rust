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
        source: &str,
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
        core::application_rules::runtime(source, raw, settings, profile, supported)
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
            group: None,
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

    #[tokio::test]
    async fn emoji_node_bindings_use_source_identity_through_save_reload_and_subscription_updates()
    {
        let root = tempfile::tempdir().unwrap();
        let native: Arc<dyn NativeHost> = Arc::new(Host(true));
        let mut session = Session::open(root.path().into(), native.clone()).unwrap();
        let name = "🇺🇸 美国 🚀 [01].+ ✈️";
        let source = format!(
            "proxies: [{{name: '{name}', type: socks5, server: example.test, port: 1080}}]\nrules: ['MATCH,DIRECT']\n"
        );
        session
            .save_profile(None, "Unicode", "", Some(&source))
            .await
            .unwrap();
        let profile = session.profiles.last().unwrap().id.clone();
        session.activate(&profile).unwrap();
        let target = core::application_rules::targets(&source)
            .unwrap()
            .pop()
            .unwrap();
        let binding = Rule {
            package: "org.example.browser".into(),
            target,
            reject_missing: true,
            group: None,
        };
        session
            .save_application_rules(&profile, vec![binding.clone()])
            .unwrap();
        for session in [
            session,
            Session::open(root.path().into(), native.clone()).unwrap(),
        ] {
            for enabled in [false, true] {
                let mut settings = session.settings.clone();
                settings.udp = enabled;
                let runtime: yaml_serde::Value =
                    yaml_serde::from_str(&session.runtime(&settings).unwrap()).unwrap();
                let route = &runtime["cor-application-rules"]["routes"][0];
                assert_eq!(route["missing_target"].as_bool(), Some(false));
                let selector = route["target"].as_str().unwrap();
                assert_ne!(selector, "REJECT");
                assert!(
                    runtime["proxy-groups"]
                        .as_sequence()
                        .unwrap()
                        .iter()
                        .any(|group| group["name"].as_str() == Some(selector)
                            && group["proxies"][0].as_str() == Some(name))
                );
                assert_eq!(runtime["proxies"][0]["udp"].as_bool(), Some(enabled));
            }
        }
        let mut session = Session::open(root.path().into(), native).unwrap();
        for (source, missing) in [
            (source.clone(), false),
            (source.replace("1080", "1081"), true),
            (source.clone(), false),
        ] {
            session
                .save_profile(Some(&profile), "Unicode", "", Some(&source))
                .await
                .unwrap();
            let runtime: yaml_serde::Value =
                yaml_serde::from_str(&session.runtime(&session.settings).unwrap()).unwrap();
            let route = &runtime["cor-application-rules"]["routes"][0];
            assert_eq!(route["missing_target"].as_bool(), Some(missing));
            assert_eq!(route["target"].as_str() == Some("REJECT"), missing);
            assert_eq!(
                session.settings.application_rules[&profile].as_slice(),
                std::slice::from_ref(&binding)
            );
        }
    }

    #[tokio::test]
    async fn legacy_application_rules_survive_reopen_and_group_editing() {
        use super::core::application_rules::groups;
        let root = tempfile::tempdir().unwrap();
        let native: Arc<dyn NativeHost> = Arc::new(Host(true));
        let mut session = Session::open(root.path().into(), native.clone()).unwrap();
        session
            .save_profile(
                None,
                "Legacy",
                "",
                Some("proxies: []\nrules: ['MATCH,DIRECT']\n"),
            )
            .await
            .unwrap();
        let profile = session.profiles.last().unwrap().id.clone();
        session.activate(&profile).unwrap();
        let source = std::fs::read(session.store.profile_path(&profile).unwrap()).unwrap();
        let legacy: Vec<Rule> = serde_json::from_str(
            r#"[{"package":"org.example.first","target":{"kind":"direct"}},
                {"package":"org.example.second","target":{"kind":"reject"},"reject_missing":true}]"#,
        )
        .unwrap();
        session
            .save_application_rules(&profile, legacy.clone())
            .unwrap();
        drop(session);

        let mut session = Session::open(root.path().into(), native.clone()).unwrap();
        assert_eq!(
            session.settings.application_rules[&profile].as_ref(),
            &legacy
        );
        let before = groups::list(&legacy);
        assert_eq!(before.len(), 2);
        assert!(!before[0].reject_missing);
        assert!(before[1].reject_missing);
        let mut edited = before[0].clone();
        edited.packages.insert("org.example.third".into());
        let changed = groups::replace(&legacy, &edited).unwrap();
        session
            .save_application_rules(&profile, changed.clone())
            .unwrap();
        drop(session);

        let session = Session::open(root.path().into(), native).unwrap();
        assert_eq!(
            session.settings.application_rules[&profile].as_ref(),
            &changed
        );
        assert_eq!(
            groups::list(&changed)
                .iter()
                .find(|group| group.label.id == before[1].label.id),
            Some(&before[1])
        );
        assert_eq!(
            std::fs::read(session.store.profile_path(&profile).unwrap()).unwrap(),
            source
        );
    }

    #[tokio::test]
    async fn protection_cycles_restore_application_bindings_and_both_udp_preferences() {
        use super::core::claude_check::protection::{Previous, Protection};
        use std::sync::atomic::{AtomicBool, Ordering};
        #[derive(Default)]
        struct RecordingHost {
            configs: std::sync::Mutex<Vec<String>>,
            reject: AtomicBool,
        }
        impl NativeHost for RecordingHost {
            fn request(&self, value: Value) -> Result<Value> {
                if value["op"] == "reload" {
                    self.configs
                        .lock()
                        .unwrap()
                        .push(value["config"].as_str().unwrap().into());
                    ensure!(
                        !self.reject.swap(false, Ordering::AcqRel),
                        "simulated protected reload failure"
                    );
                }
                Ok(json!({"supported":true}))
            }
        }
        for enabled in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let native = Arc::new(RecordingHost::default());
            let mut session = Session::open(root.path().into(), native.clone()).unwrap();
            let source = "proxies: [{name: '🇯🇵 日本 🚀', type: socks5, server: example.test, port: 1080}]\nrules: ['MATCH,DIRECT']\n";
            session
                .save_profile(None, "Protection", "", Some(source))
                .await
                .unwrap();
            let profile = session.profiles.last().unwrap().id.clone();
            session.activate(&profile).unwrap();
            let mut settings = session.settings.clone();
            settings.udp = enabled;
            session.commit_settings(settings).unwrap();
            session
                .save_application_rules(
                    &profile,
                    vec![Rule {
                        package: "org.example.browser".into(),
                        target: core::application_rules::targets(source)
                            .unwrap()
                            .pop()
                            .unwrap(),
                        reject_missing: true,
                        group: None,
                    }],
                )
                .unwrap();
            let before = session.settings.clone();
            let original = session.runtime(&before).unwrap();
            for _ in 0..3 {
                session.running = true;
                let protection = Protection::new(
                    profile.clone(),
                    "🇯🇵 日本 🚀".into(),
                    Previous {
                        mode: before.run_mode.clone(),
                        proxy_mode: before.proxy_mode,
                        selectors: Vec::new(),
                    },
                );
                session
                    .commit_protection(Some(Arc::new(protection)), before.run_mode.clone())
                    .unwrap();
                session = Session::open(root.path().into(), native.clone()).unwrap();
                let protected_raw = session.runtime(&session.settings).unwrap();
                assert_eq!(
                    native.configs.lock().unwrap().last().unwrap(),
                    &protected_raw
                );
                let protected: yaml_serde::Value = yaml_serde::from_str(&protected_raw).unwrap();
                assert!(protected["cor-application-rules"].is_null());
                assert_eq!(protected["proxies"][0]["udp"].as_bool(), Some(true));
                assert_eq!(session.settings.udp, enabled);
                assert_eq!(
                    session.settings.application_rules[&profile],
                    before.application_rules[&profile]
                );
                let saved = std::fs::read(root.path().join("settings.json")).unwrap();
                let mut modified = session.settings.clone();
                modified.udp = !enabled;
                assert!(session.commit_settings(modified).is_err());
                assert!(
                    session
                        .save_application_rules(&profile, Vec::new())
                        .is_err()
                );
                assert_eq!(
                    std::fs::read(root.path().join("settings.json")).unwrap(),
                    saved
                );
                session.running = true;
                native.reject.store(true, Ordering::Release);
                assert!(
                    session
                        .commit_protection(None, before.run_mode.clone())
                        .is_err()
                );
                assert!(session.settings.claude_protection.is_some());
                assert_eq!(
                    std::fs::read(root.path().join("settings.json")).unwrap(),
                    saved
                );
                assert_eq!(
                    std::fs::read_to_string(session.store.runtime().join("config.yaml")).unwrap(),
                    protected_raw
                );
                assert_eq!(
                    native.configs.lock().unwrap().last().unwrap(),
                    &protected_raw
                );
                session
                    .commit_protection(None, before.run_mode.clone())
                    .unwrap();
                assert_eq!(native.configs.lock().unwrap().last().unwrap(), &original);
                session = Session::open(root.path().into(), native.clone()).unwrap();
                assert_eq!(session.runtime(&session.settings).unwrap(), original);
                assert_eq!(
                    serde_json::to_value(&session.settings).unwrap(),
                    serde_json::to_value(&before).unwrap()
                );
                assert_eq!(session.raw().unwrap(), source);
            }
        }
    }
}
