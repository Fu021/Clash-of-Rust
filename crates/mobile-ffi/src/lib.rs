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
    pub count: usize,
    pub rates: (u64, u64),
    pub platform: Value,
    pub node: String,
    pub rule_profile: String,
    rule_document: Document,
    route: String,
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
            profiles.push(Profile {
                id: id.clone(),
                name: "默认直连".into(),
                source: String::new(),
                updated: 0,
                usage: None,
            });
            settings.active_profile = Some(id.clone());
            let mut tx = core::profile_transaction::Transaction::new(&store.root)?;
            tx.stage(
                &store.profile_path(&id)?,
                include_bytes!("../../../resources/default.yaml"),
            )?;
            tx.stage(
                &store.root.join("profiles.json"),
                &serde_json::to_vec_pretty(&profiles)?,
            )?;
            tx.stage(
                &store.root.join("settings.json"),
                &serde_json::to_vec_pretty(&settings)?,
            )?;
            tx.prepare()?;
            if let Err(error) = tx.commit() {
                tx.rollback()?;
                return Err(error);
            }
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
        let document = Document::from_raw(
            id,
            false,
            &core::config::read_profile(&store.profile_path(id)?)?,
        )?;
        let rule_profile = id.to_owned();
        let route =
            core::active_route::root(&core::config::read_profile(&store.profile_path(id)?)?)?;
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
            count: 0,
            rates: (0, 0),
            platform: json!({}),
            node: String::new(),
            rule_profile,
            rule_document: document,
            route,
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
        Ok(self.rule_document.clone())
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
        self.route = core::active_route::root(&raw)?;
        Ok(())
    }
    pub fn commit_settings(&mut self, settings: Settings) -> Result<()> {
        settings.validate()?;
        let raw = self.runtime(&settings)?;
        let old_raw = self.runtime(&self.settings)?;
        let mut tx = core::profile_transaction::Transaction::new(&self.store.root)?;
        tx.stage(
            &self.store.root.join("settings.json"),
            &serde_json::to_vec_pretty(&settings)?,
        )?;
        tx.stage(&self.store.runtime().join("config.yaml"), raw.as_bytes())?;
        tx.prepare()?;
        if self.running
            && let Err(error) = self
                .native
                .request(json!({"op":"reload","config":raw,"ipv6":settings.ipv6}))
        {
            tx.rollback()?;
            self.native
                .request(json!({"op":"reload","config":old_raw,"ipv6":self.settings.ipv6}))
                .context("内核拒绝配置后恢复失败")?;
            return Err(error);
        }
        if let Err(error) = tx.commit() {
            tx.rollback().context("配置写入失败，磁盘恢复也失败")?;
            if self.running {
                self.native
                    .request(json!({"op":"reload","config":old_raw,"ipv6":self.settings.ipv6}))
                    .context("配置保存失败，内核恢复也失败")?;
            }
            return Err(error);
        }
        self.api = Api::new(&settings)?;
        self.route = core::active_route::root(&raw)?;
        self.settings = settings;
        Ok(())
    }
    pub async fn refresh(&mut self, page: i32) -> Result<()> {
        let state = self.native.request(json!({"op":"state"}))?;
        self.running = state["running"].as_bool().unwrap_or(false);
        self.vpn = state["vpn"].as_bool().unwrap_or(false);
        self.platform = state;
        if !self.running {
            return Ok(());
        }
        if page == 1 || page == 0 {
            self.proxies = self.api.get("proxies").await?;
            let root = core::active_route::start(&self.settings.run_mode, &self.route);
            if let Some(node) = core::active_route::from_snapshot(root, &self.proxies) {
                self.node = node;
            }
        } else {
            self.proxies = Proxies::default();
        }
        let at = Instant::now();
        let old_up = self.connections.upload_total;
        let old_down = self.connections.download_total;
        if page == 3 || page == 8 {
            let mut next: Connections = self.api.get("connections").await?;
            if let Some(at) = self.sampled {
                core::connection::sample_rates(
                    &mut next.connections,
                    &self.connections.connections,
                    at.elapsed(),
                );
            }
            self.count = next.connections.len();
            self.connections = next;
        } else {
            let stats: core::api::ConnectionStats = self.api.get("connections").await?;
            self.count = stats.count;
            self.connections = Connections {
                upload_total: stats.upload_total,
                download_total: stats.download_total,
                connections: vec![],
            };
        }
        if let Some(previous) = self.sampled {
            let elapsed = at.duration_since(previous).as_secs_f64();
            if elapsed > 0.0 {
                self.rates = (
                    (self.connections.upload_total.saturating_sub(old_up) as f64 / elapsed) as u64,
                    (self.connections.download_total.saturating_sub(old_down) as f64 / elapsed)
                        as u64,
                );
            }
        }
        self.sampled = Some(at);
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
        ensure!(
            !self
                .profiles
                .iter()
                .any(|p| p.name == name.trim() && Some(p.id.as_str()) != id),
            "订阅名称已存在"
        );
        let (raw, usage) = if let Some(raw) = content {
            (raw.to_owned(), None)
        } else if source.is_empty() {
            let id = id.context("本地配置需要导入文件")?;
            (
                core::config::read_profile(&self.store.profile_path(id)?)?,
                None,
            )
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
        let active = self.settings.active_profile.as_deref() == Some(&id);
        let old_raw = active.then(|| self.runtime(&self.settings)).transpose()?;
        let new_runtime = if active {
            let rules = ProfileRules::for_profile(&self.settings, &id, false);
            Some(android_safe_config(
                &core::config::runtime_config_with_rules(
                    &raw,
                    &self.settings,
                    Some(&rules),
                    false,
                )?,
            )?)
        } else {
            None
        };
        let mut tx = core::profile_transaction::Transaction::new(&self.store.root)?;
        tx.stage(&self.store.profile_path(&id)?, raw.as_bytes())?;
        tx.stage(
            &self.store.root.join("profiles.json"),
            &serde_json::to_vec_pretty(&profiles)?,
        )?;
        if let Some(raw) = &new_runtime {
            tx.stage(&self.store.runtime().join("config.yaml"), raw.as_bytes())?;
        }
        tx.prepare()?;
        if self.running
            && let Some(raw) = &new_runtime
            && let Err(error) = self
                .native
                .request(json!({"op":"reload","config":raw,"ipv6":self.settings.ipv6}))
        {
            tx.rollback()?;
            if let Some(raw) = &old_raw {
                self.native
                    .request(json!({"op":"reload","config":raw,"ipv6":self.settings.ipv6}))?;
            }
            return Err(error);
        }
        if let Err(error) = tx.commit() {
            tx.rollback()?;
            if self.running
                && let Some(raw) = old_raw
            {
                self.native
                    .request(json!({"op":"reload","config":raw,"ipv6":self.settings.ipv6}))?;
            }
            return Err(error);
        }
        self.profiles = profiles;
        if let Some(raw) = new_runtime {
            self.route = core::active_route::root(&raw)?;
        }
        if self.rule_profile == id {
            self.rule_document = Document::from_raw(&id, false, &raw)?;
        }
        Ok(())
    }
    pub async fn update_profile(&mut self, id: &str) -> Result<()> {
        let profile = self
            .profiles
            .iter()
            .find(|p| p.id == id)
            .context("订阅不存在")?
            .clone();
        ensure!(!profile.source.is_empty(), "本地配置没有更新地址");
        self.save_profile(Some(id), &profile.name, &profile.source, None)
            .await
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
        self.node.clear();
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
        let mut settings = self.settings.clone();
        settings.rule_profiles.remove(id);
        let mut tx = core::profile_transaction::Transaction::new(&self.store.root)?;
        tx.stage(
            &self.store.root.join("settings.json"),
            &serde_json::to_vec_pretty(&settings)?,
        )?;
        tx.stage_remove(&self.store.profile_path(id)?)?;
        tx.stage(
            &self.store.root.join("profiles.json"),
            &serde_json::to_vec_pretty(&profiles)?,
        )?;
        tx.prepare()?;
        if let Err(error) = tx.commit() {
            tx.rollback()?;
            return Err(error);
        }
        self.profiles = profiles;
        self.settings = settings;
        if self.rule_profile == id {
            self.draft = ProfileRules::default();
            self.rule_profile = self.profile().id.clone();
            self.draft = ProfileRules::for_profile(&self.settings, &self.rule_profile, false)
                .as_ref()
                .clone();
            self.rule_document = Document::from_raw(&self.rule_profile, false, &self.raw()?)?;
        }
        Ok(())
    }
    pub fn select_rules(&mut self, id: &str) -> Result<()> {
        ensure!(self.profiles.iter().any(|p| p.id == id), "订阅不存在");
        let saved = ProfileRules::for_profile(&self.settings, &self.rule_profile, false);
        ensure!(self.draft == *saved, "请先应用或放弃当前规则修改");
        self.rule_document = Document::from_raw(
            id,
            false,
            &core::config::read_profile(&self.store.profile_path(id)?)?,
        )?;
        self.rule_profile = id.to_owned();
        self.draft = ProfileRules::for_profile(&self.settings, id, false)
            .as_ref()
            .clone();
        Ok(())
    }
    pub fn save_rule(&mut self, selected: &str, raw: &str, position: usize) -> Result<()> {
        core::rule_manager::parts(raw)?;
        let selected = selected.strip_prefix("c:").unwrap_or(selected);
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
        let key = format!("c:{id}");
        let (rows, _) = self.document()?.rows(&self.draft);
        let mut order: Vec<_> = rows
            .into_iter()
            .map(|r| r.id)
            .filter(|old| old != &key)
            .collect();
        let index = position.saturating_sub(1).min(order.len());
        order.insert(index, key);
        self.draft.order = order;
        self.draft.validate()
    }
    pub fn apply_rules(&mut self) -> Result<()> {
        let mut next = self.settings.clone();
        next.rule_profiles
            .insert(self.rule_profile.clone(), Arc::new(self.draft.clone()));
        if next.active_profile.as_deref() == Some(&self.rule_profile) {
            self.commit_settings(next)
        } else {
            self.store.save_settings(&next)?;
            self.settings = next;
            Ok(())
        }
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
        let settings = Settings {
            ipv6: false,
            ..Settings::default()
        };
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
    #[test]
    fn failed_native_reload_restores_saved_settings_and_runtime() {
        struct RejectIpv4;
        impl NativeHost for RejectIpv4 {
            fn request(&self, value: Value) -> Result<Value> {
                ensure!(
                    value["op"] != "reload" || value["ipv6"] != false,
                    "simulated native reload failure"
                );
                Ok(json!({"ok":true}))
            }
        }
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::open(root.path().into(), Arc::new(RejectIpv4)).unwrap();
        session.initialize().unwrap();
        let before = std::fs::read(session.store.runtime().join("config.yaml")).unwrap();
        let mut next = session.settings.clone();
        next.ipv6 = false;
        assert!(session.commit_settings(next).is_err());
        assert!(session.settings.ipv6);
        assert!(session.store.load_settings().unwrap().ipv6);
        assert_eq!(
            std::fs::read(session.store.runtime().join("config.yaml")).unwrap(),
            before
        );
        assert!(!core::profile_transaction::recover(&session.store.root).unwrap());
    }
}
