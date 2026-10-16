//! Platform-neutral mobile session. JNI, VpnService and Slint stay in their hosts.
use anyhow::{Context, Result, ensure};
pub use clash_client_core as core;
use core::{
    api::{Api, Connections, Proxies},
    config::{Profile, Settings, Store, atomic_write},
    rule_manager::{CustomRule, Document, ProfileRules},
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Instant};

pub trait NativeHost: Send + Sync + 'static {
    fn request(&self, value: Value) -> Result<Value>;
}

pub struct Session {
    pub store: Store,
    pub settings: Settings,
    pub profiles: Vec<Profile>,
    pub api: Api,
    pub running: bool,
    pub vpn: bool,
    pub proxies: Proxies,
    pub connections: Connections,
    pub sampled: Option<Instant>,
    pub draft: ProfileRules,
    pub native: Arc<dyn NativeHost>,
}

impl Session {
    pub fn open(root: PathBuf, native: Arc<dyn NativeHost>) -> Result<Self> {
        let store = Store::at(root)?;
        core::profile_transaction::recover(&store.root)?;
        let mut settings = store.load_settings()?;
        let mut profiles = store.profiles()?;
        if profiles.is_empty() {
            let id = uuid::Uuid::new_v4().to_string();
            atomic_write(
                &store.profile_path(&id)?,
                include_bytes!("../../../resources/default.yaml"),
            )?;
            profiles.push(Profile {
                id: id.clone(),
                name: "默认直连".into(),
                source: String::new(),
                updated: 0,
                usage: None,
            });
            settings.active_profile = Some(id);
            store.save_profiles(&profiles)?;
            store.save_settings(&settings)?;
        }
        ensure!(
            profiles
                .iter()
                .any(|p| Some(&p.id) == settings.active_profile.as_ref()),
            "当前订阅已丢失"
        );
        let id = settings.active_profile.as_deref().context("尚未选择订阅")?;
        let draft = ProfileRules::for_profile(&settings, id, false)
            .as_ref()
            .clone();
        let api = Api::new(&settings)?;
        Ok(Self {
            store,
            settings,
            profiles,
            api,
            running: false,
            vpn: false,
            proxies: Proxies::default(),
            connections: Connections::default(),
            sampled: None,
            draft,
            native,
        })
    }
    pub fn profile(&self) -> &Profile {
        self.profiles
            .iter()
            .find(|p| Some(&p.id) == self.settings.active_profile.as_ref())
            .expect("validated profile")
    }
    pub fn raw(&self) -> Result<String> {
        core::config::read_profile(&self.store.profile_path(&self.profile().id)?)
    }
    pub fn document(&self) -> Result<Document> {
        Document::from_raw(&self.profile().id, false, &self.raw()?)
    }
    pub fn runtime(&self, settings: &Settings) -> Result<String> {
        let id = settings.active_profile.as_deref().context("尚未选择订阅")?;
        let raw = core::config::read_profile(&self.store.profile_path(id)?)?;
        let rules = ProfileRules::for_profile(settings, id, false);
        android_safe_config(&core::config::runtime_config_with_rules(
            &raw,
            settings,
            Some(&rules),
            false,
        )?)
    }
    pub fn initialize(&mut self) -> Result<()> {
        let raw = self.runtime(&self.settings)?;
        self.native
            .request(json!({"op":"initialize","config":raw,"ipv6":self.settings.ipv6}))?;
        atomic_write(&self.store.runtime().join("config.yaml"), raw.as_bytes())?;
        self.running = true;
        Ok(())
    }
    pub fn commit_settings(&mut self, settings: Settings) -> Result<()> {
        settings.validate()?;
        let raw = self.runtime(&settings)?;
        let old_raw = self.runtime(&self.settings)?;
        if self.running {
            self.native
                .request(json!({"op":"reload","config":raw,"ipv6":settings.ipv6}))?;
        }
        let mut tx = core::profile_transaction::Transaction::new(&self.store.root)?;
        tx.stage(
            &self.store.root.join("settings.json"),
            &serde_json::to_vec_pretty(&settings)?,
        )?;
        tx.stage(&self.store.runtime().join("config.yaml"), raw.as_bytes())?;
        tx.prepare()?;
        if let Err(error) = tx.commit() {
            if self.running {
                self.native
                    .request(json!({"op":"reload","config":old_raw,"ipv6":self.settings.ipv6}))
                    .context("配置保存失败，内核恢复也失败")?;
            }
            return Err(error);
        }
        self.api = Api::new(&settings)?;
        self.settings = settings;
        Ok(())
    }
    pub async fn refresh(&mut self, page: i32) -> Result<()> {
        let state = self.native.request(json!({"op":"state"}))?;
        self.running = state["running"].as_bool().unwrap_or(false);
        self.vpn = state["vpn"].as_bool().unwrap_or(false);
        if !self.running {
            return Ok(());
        }
        if page == 1 || page == 0 {
            self.proxies = self.api.get("proxies").await?;
        } else {
            self.proxies = Proxies::default();
        }
        if page == 0 || page == 3 || page == 8 {
            let mut next: Connections = self.api.get("connections").await?;
            if let Some(at) = self.sampled {
                core::connection::sample_rates(
                    &mut next.connections,
                    &self.connections.connections,
                    at.elapsed(),
                );
            }
            self.sampled = Some(Instant::now());
            self.connections = next;
        } else {
            self.connections = Connections::default();
            self.sampled = None;
        }
        Ok(())
    }
    pub async fn save_profile(
        &mut self,
        id: Option<&str>,
        name: &str,
        source: &str,
        content: Option<&str>,
    ) -> Result<()> {
        ensure!(!name.trim().is_empty(), "订阅名称不能为空");
        let (raw, usage) = if let Some(raw) = content {
            (raw.to_owned(), None)
        } else {
            let url = core::probe::validate_url(source)?;
            core::subscription::download_routes(
                &url,
                &core::network::routes(self.running.then_some(self.settings.mixed_port), None),
            )
            .await?
        };
        core::config::parse_profile(&raw)?;
        let id = id
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let mut profiles = self.profiles.clone();
        let profile = Profile {
            id: id.clone(),
            name: name.trim().into(),
            source: source.into(),
            updated: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
            usage,
        };
        if let Some(old) = profiles.iter_mut().find(|p| p.id == id) {
            *old = profile;
        } else {
            profiles.push(profile);
        }
        ensure!(
            self.settings.active_profile.as_deref() != Some(&id),
            "当前使用中的订阅请先切换后再编辑"
        );
        let mut tx = core::profile_transaction::Transaction::new(&self.store.root)?;
        tx.stage(&self.store.profile_path(&id)?, raw.as_bytes())?;
        tx.stage(
            &self.store.root.join("profiles.json"),
            &serde_json::to_vec_pretty(&profiles)?,
        )?;
        tx.prepare()?;
        tx.commit()?;
        self.profiles = profiles;
        Ok(())
    }
    pub fn activate(&mut self, id: &str) -> Result<()> {
        ensure!(
            self.settings.claude_protection.is_none(),
            "请先关闭 Claude 保护"
        );
        ensure!(self.profiles.iter().any(|p| p.id == id), "订阅不存在");
        let mut next = self.settings.clone();
        next.active_profile = Some(id.into());
        self.commit_settings(next)?;
        self.draft = ProfileRules::for_profile(&self.settings, id, false)
            .as_ref()
            .clone();
        Ok(())
    }
    pub fn delete(&mut self, id: &str) -> Result<()> {
        ensure!(
            self.settings.active_profile.as_deref() != Some(id),
            "请先切换到其他订阅再删除"
        );
        let profiles: Vec<_> = self
            .profiles
            .iter()
            .filter(|p| p.id != id)
            .cloned()
            .collect();
        ensure!(profiles.len() != self.profiles.len(), "订阅不存在");
        let mut tx = core::profile_transaction::Transaction::new(&self.store.root)?;
        tx.stage_remove(&self.store.profile_path(id)?)?;
        tx.stage(
            &self.store.root.join("profiles.json"),
            &serde_json::to_vec_pretty(&profiles)?,
        )?;
        tx.prepare()?;
        tx.commit()?;
        self.profiles = profiles;
        Ok(())
    }
    pub fn save_rule(&mut self, selected: &str, raw: &str, position: usize) -> Result<()> {
        core::rule_manager::parts(raw)?;
        let id = if self.draft.custom.iter().any(|r| r.id == selected) {
            selected.to_owned()
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        let rule = CustomRule {
            id: id.clone(),
            rule: raw.into(),
            enabled: true,
        };
        if let Some(old) = self.draft.custom.iter_mut().find(|r| r.id == id) {
            *old = rule;
        } else {
            self.draft.custom.push(rule);
        }
        let (rows, _) = self.document()?.rows(&self.draft);
        let mut order: Vec<_> = rows
            .into_iter()
            .map(|r| r.id)
            .filter(|key| key != &id)
            .collect();
        let index = position.saturating_sub(1).min(order.len());
        order.insert(index, id);
        self.draft.order = order;
        self.draft.validate()
    }
    pub fn apply_rules(&mut self) -> Result<()> {
        let mut next = self.settings.clone();
        next.rule_profiles
            .insert(self.profile().id.clone(), Arc::new(self.draft.clone()));
        self.commit_settings(next)
    }
}

/// OS routing belongs to VpnService, never to subscription YAML or Linux ioctls.
pub fn android_safe_config(raw: &str) -> Result<String> {
    let mut value: serde_yaml::Value = serde_yaml::from_str(raw)?;
    let tun = value["tun"].as_mapping_mut().context("TUN 配置无效")?;
    tun.clear();
    tun.insert("enable".into(), false.into());
    value["dns"]["listen"] = "127.0.0.1:0".into();
    value["profile"]["store-selected"] = true.into();
    value["profile"]["store-fake-ip"] = true.into();
    Ok(serde_yaml::to_string(&value)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake;
    impl NativeHost for Fake {
        fn request(&self, _: Value) -> Result<Value> {
            Ok(json!({"ok":true}))
        }
    }
    #[test]
    fn mobile_never_inherits_subscription_owned_routes_or_controller() {
        let mut settings = Settings::default();
        settings.ipv6 = false;
        let raw = "proxies: []\nrules: [MATCH,DIRECT]\ntun:\n  enable: true\n  auto-route: true\n  file-descriptor: 42\nexternal-controller: 0.0.0.0:1\n";
        let raw = core::config::runtime_config(raw, &settings).unwrap();
        let value: serde_yaml::Value =
            serde_yaml::from_str(&android_safe_config(&raw).unwrap()).unwrap();
        assert_eq!(value["tun"].as_mapping().unwrap().len(), 1);
        assert_eq!(value["tun"]["enable"], false);
        assert_eq!(value["ipv6"], false);
        assert_eq!(
            value["external-controller"],
            format!("127.0.0.1:{}", settings.controller_port)
        );
        assert_eq!(value["secret"], settings.secret);
    }
    #[test]
    fn mobile_data_is_explicit_and_invalid_sources_do_not_replace_it() {
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::open(root.path().into(), Arc::new(Fake)).unwrap();
        let before = session.raw().unwrap();
        assert!(session.activate("missing").is_err());
        assert_eq!(session.raw().unwrap(), before);
        assert!(session.delete(&session.profile().id.clone()).is_err());
        assert!(session.save_rule("", "", 1).is_err());
    }
}
