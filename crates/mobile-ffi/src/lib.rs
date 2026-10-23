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
    fn notify(&self, title: &str, message: &str, interactive: bool) -> Result<()> {
        ensure!(
            title.len() <= 128
                && message.len() <= 2048
                && !title.contains('\0')
                && !message.contains('\0'),
            "系统通知内容无效"
        );
        self.request(
            json!({"op":"notify","title":title,"message":message,"request_permission":interactive}),
        )?;
        Ok(())
    }
}

/// Native status objects may contain an empty error; only actual failures fail a request.
pub fn native_response(value: Value) -> Result<Value> {
    if let Some(message) = value
        .get("error")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        anyhow::bail!("{message}");
    }
    Ok(value)
}

/// Controller I/O runs outside the presentation loop. A host can cancel an old
/// snapshot when navigating or replacing the controller without losing UI events.
pub struct Snapshot {
    proxies: Option<Proxies>,
    connections: Option<Connections>,
    stats: core::api::ConnectionStats,
    at: Instant,
}
impl Snapshot {
    pub async fn fetch(api: &Api, page: i32, refresh_route: bool) -> Result<Self> {
        let proxies = async {
            if refresh_route || matches!(page, 0 | 1) {
                api.get("proxies").await.map(Some)
            } else {
                Ok(None)
            }
        };
        let connections = async {
            if matches!(page, 3 | 8) {
                let connections: Connections = api.get("connections").await?;
                let stats = core::api::ConnectionStats {
                    count: connections.connections.len(),
                    upload_total: connections.upload_total,
                    download_total: connections.download_total,
                };
                Ok::<_, anyhow::Error>((Some(connections), stats))
            } else {
                Ok((None, api.get("connections").await?))
            }
        };
        let (proxies, (connections, stats)) = tokio::try_join!(proxies, connections)?;
        Ok(Self {
            proxies,
            connections,
            stats,
            at: Instant::now(),
        })
    }
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
    connection_sampled: Option<Instant>,
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
            connection_sampled: None,
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
    pub fn document(&self) -> &Document {
        &self.rule_document
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
        let native_running = self.native.request(json!({"op":"state"}))?["running"] == true;
        let controller = if native_running {
            None
        } else {
            Some(core::config::reserve_controller(
                self.settings.controller_port,
                self.settings.mixed_port,
            )?)
        };
        if let Some(listener) = &controller {
            let port = listener.local_addr()?.port();
            if port != self.settings.controller_port {
                let mut settings = self.settings.clone();
                settings.controller_port = port;
                self.store.save_settings(&settings)?;
                self.api = Api::new(&settings)?;
                self.settings = settings;
            }
        }
        let raw = self.runtime(&self.settings)?;
        drop(controller);
        self.native.request(
            json!({"op":"initialize","config":raw,"ipv6":effective_ipv6(&self.settings)}),
        )?;
        atomic_write(&self.store.runtime().join("config.yaml"), raw.as_bytes())?;
        self.running = true;
        self.route = core::active_route::root(&raw)?;
        if self.settings.claude_protection.is_some() {
            self.native.request(json!({"op":"protection-sync"}))?;
        }
        Ok(())
    }
    pub fn commit_settings(&mut self, mut settings: Settings) -> Result<()> {
        if settings.mixed_port == settings.controller_port {
            settings.controller_port = core::config::reserve_controller(0, settings.mixed_port)?
                .local_addr()?
                .port();
        }
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
        let protection_changed = self.settings.claude_protection != settings.claude_protection;
        if protection_changed
            && self.settings.claude_protection.is_some()
            && let Err(error) = self.native.request(json!({"op":"protection-pause"}))
        {
            tx.rollback()?;
            return Err(error);
        }
        if self.running
            && let Err(error) = self
                .native
                .request(json!({"op":"reload","config":raw,"ipv6":effective_ipv6(&settings)}))
        {
            tx.rollback()?;
            self.native
                .request(
                    json!({"op":"reload","config":old_raw,"ipv6":effective_ipv6(&self.settings)}),
                )
                .context("内核拒绝配置后恢复失败")?;
            if protection_changed {
                self.native.request(json!({"op":"protection-sync"}))?;
            }
            return Err(error);
        }
        if let Err(error) = tx.commit() {
            tx.rollback().context("配置写入失败，磁盘恢复也失败")?;
            if self.running {
                self.native
                    .request(json!({"op":"reload","config":old_raw,"ipv6":effective_ipv6(&self.settings)}))
                    .context("配置保存失败，内核恢复也失败")?;
            }
            if protection_changed {
                self.native.request(json!({"op":"protection-sync"}))?;
            }
            return Err(error);
        }
        self.api = Api::new(&settings)?;
        self.route = core::active_route::root(&raw)?;
        self.settings = settings;
        if protection_changed {
            self.native.request(json!({"op":"protection-sync"}))?;
        }
        Ok(())
    }
    pub fn observe_native(&mut self) -> Result<()> {
        let state = self.native.request(json!({"op":"state"}))?;
        self.running = state["running"].as_bool().unwrap_or(false);
        self.vpn = state["vpn"].as_bool().unwrap_or(false);
        self.platform = state;
        if !self.running {
            self.clear_snapshot();
        }
        Ok(())
    }
    pub fn clear_snapshot(&mut self) {
        self.proxies = Proxies::default();
        self.connections = Connections::default();
        self.count = 0;
        self.rates = (0, 0);
        self.sampled = None;
        self.connection_sampled = None;
    }
    /// The home page represents the default outbound route. Do not infer its
    /// group from alphabetical order or from a leaf shared by several groups.
    pub fn active_group(&self) -> Option<&str> {
        let name = core::active_route::start(&self.settings.run_mode, &self.route);
        self.proxies
            .proxies
            .get(name)
            .filter(|group| !group.all.is_empty())
            .map(|_| name)
    }
    pub fn apply_snapshot(&mut self, snapshot: Snapshot) {
        if let Some(proxies) = snapshot.proxies {
            self.proxies = proxies;
            let root = core::active_route::start(&self.settings.run_mode, &self.route);
            if let Some(node) = self
                .settings
                .claude_protection
                .as_ref()
                .map(|p| p.node.clone())
                .or_else(|| core::active_route::from_snapshot(root, &self.proxies))
            {
                self.node = node;
            }
        } else {
            self.proxies = Proxies::default();
        }
        if let Some(previous) = self.sampled {
            let elapsed = snapshot.at.duration_since(previous).as_secs_f64();
            if elapsed > 0.0 {
                self.rates = (
                    (snapshot
                        .stats
                        .upload_total
                        .saturating_sub(self.connections.upload_total) as f64
                        / elapsed) as u64,
                    (snapshot
                        .stats
                        .download_total
                        .saturating_sub(self.connections.download_total)
                        as f64
                        / elapsed) as u64,
                );
            }
        }
        if let Some(mut next) = snapshot.connections {
            if let Some(previous) = self.connection_sampled {
                core::connection::sample_rates(
                    &mut next.connections,
                    &self.connections.connections,
                    snapshot.at.duration_since(previous),
                );
            }
            self.connections = next;
            self.connection_sampled = Some(snapshot.at);
        } else {
            // A bounded first screen stays available during navigation. Do not
            // retain a full busy connection table on unrelated mobile pages.
            self.connections.connections.truncate(50);
            self.connections.connections.shrink_to_fit();
        }
        self.count = snapshot.stats.count;
        self.connections.upload_total = snapshot.stats.upload_total;
        self.connections.download_total = snapshot.stats.download_total;
        self.sampled = Some(snapshot.at);
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
                .request(json!({"op":"reload","config":raw,"ipv6":effective_ipv6(&self.settings)}))
        {
            tx.rollback()?;
            if let Some(raw) = &old_raw {
                self.native.request(
                    json!({"op":"reload","config":raw,"ipv6":effective_ipv6(&self.settings)}),
                )?;
            }
            return Err(error);
        }
        if let Err(error) = tx.commit() {
            tx.rollback()?;
            if self.running
                && let Some(raw) = old_raw
            {
                self.native.request(
                    json!({"op":"reload","config":raw,"ipv6":effective_ipv6(&self.settings)}),
                )?;
            }
            return Err(error);
        }
        self.profiles = profiles;
        if let Some(raw) = new_runtime {
            self.route = core::active_route::root(&raw)?;
            self.node.clear();
            self.clear_snapshot();
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
        // Keep an unapplied rules draft attached to its original subscription.
        // Otherwise the rule page follows the newly active subscription.
        let follow =
            self.draft == *ProfileRules::for_profile(&self.settings, &self.rule_profile, false);
        let document = if follow {
            Some(Document::from_raw(
                id,
                false,
                &core::config::read_profile(&self.store.profile_path(id)?)?,
            )?)
        } else {
            None
        };
        self.commit_settings(next)?;
        self.node.clear();
        self.clear_snapshot();
        if let Some(document) = document {
            self.rule_profile = id.to_owned();
            self.rule_document = document;
            self.draft = ProfileRules::for_profile(&self.settings, id, false)
                .as_ref()
                .clone();
        }
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
        let (rows, _) = self.document().rows(&self.draft);
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

pub fn effective_ipv6(settings: &Settings) -> bool {
    settings.ipv6 || settings.claude_protection.is_some()
}

/// Both mobile hosts reuse the desktop overlay, provider resolution and guard.
pub async fn prepare_protection(
    api: &Api,
    settings: &Settings,
    raw: &str,
    vpn: bool,
) -> Result<core::claude_check::protection::Protection> {
    use crate::core::claude_check::protection::{Previous, Protection};
    let proxies: Proxies = api.get("proxies").await?;
    let root = core::active_route::root(raw)?;
    let mode = if settings.run_mode == "direct" {
        "rule"
    } else {
        &settings.run_mode
    };
    let mut node = core::active_route::start(mode, &root).to_owned();
    let mut visited = std::collections::BTreeSet::new();
    let mut provider = None;
    let mut route_groups = std::collections::BTreeMap::new();
    loop {
        ensure!(
            mode != "global" || node != "DIRECT",
            "当前全局出口为 DIRECT，请先选择代理节点再开启 Claude 保护"
        );
        ensure!(
            !core::active_route::terminal(&node)
                && visited.len() < core::active_route::MAX_DEPTH
                && visited.insert(node.clone()),
            "请先选择具体代理节点"
        );
        match api.proxy_nodes(&node).await {
            Ok(proxy) if !proxy.now.is_empty() => {
                if matches!(proxy.kind.as_str(), "URLTest" | "Fallback" | "LoadBalance") {
                    route_groups.insert(node.clone(), proxy.now.to_string());
                }
                node = proxy.now.to_string();
            }
            Ok(proxy) => {
                ensure!(
                    proxy.all.is_empty()
                        && !matches!(
                            proxy.kind.as_str(),
                            "" | "Direct"
                                | "Reject"
                                | "Pass"
                                | "Compatible"
                                | "Selector"
                                | "URLTest"
                                | "Fallback"
                                | "LoadBalance"
                        ),
                    "请选择具体代理节点"
                );
                break;
            }
            Err(_) => {
                let (source, kind) = api
                    .provider_leaf(&node)
                    .await?
                    .context("当前代理节点不存在")?;
                ensure!(
                    !matches!(
                        kind.as_str(),
                        "Direct"
                            | "Reject"
                            | "Pass"
                            | "Selector"
                            | "URLTest"
                            | "Fallback"
                            | "LoadBalance"
                    ),
                    "请选择具体代理节点"
                );
                provider = Some(source);
                break;
            }
        }
    }
    let mut protection = Protection::new(
        settings.active_profile.clone().context("请先选择订阅")?,
        node,
        Previous {
            mode: settings.run_mode.clone(),
            proxy_mode: if vpn {
                core::model::ProxyMode::Tun
            } else {
                core::model::ProxyMode::Off
            },
            selectors: proxies
                .proxies
                .iter()
                .filter(|(_, p)| p.kind.as_str() == "Selector" && !p.now.is_empty())
                .map(|(name, p)| (name.to_string(), p.now.to_string()))
                .collect(),
        },
    );
    protection.provider = provider;
    protection.capture_automatic_groups(&proxies);
    protection.locked_groups.extend(route_groups);
    protection.validate()?;
    Ok(protection)
}

/// OS routing belongs to VpnService, never to subscription YAML or Linux ioctls.
pub fn android_safe_config(raw: &str) -> Result<String> {
    let mut value: serde_yaml::Value = serde_yaml::from_str(raw)?;
    let tun = value["tun"].as_mapping_mut().context("TUN 配置无效")?;
    tun.clear();
    tun.insert("enable".into(), false.into());
    // VpnService supplies our TUN address as the system DNS. Disabling the
    // controller's DNS service would leave every VPN client without a resolver.
    // Keep subscription upstreams/policies when the user's DNS override is off.
    let dns = value["dns"].as_mapping_mut().context("DNS 配置无效")?;
    dns.insert("enable".into(), true.into());
    dns.insert("listen".into(), "127.0.0.1:0".into());
    if dns
        .get("nameserver")
        .is_none_or(|v| v.is_null() || v.as_sequence().is_some_and(Vec::is_empty))
    {
        dns.insert(
            "nameserver".into(),
            serde_yaml::to_value(core::config::DEFAULT_DNS_NAMESERVERS)?,
        );
    }
    value["profile"]["store-selected"] = true.into();
    value["profile"]["store-fake-ip"] = true.into();
    Ok(serde_yaml::to_string(&value)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn protection_checks_the_actual_global_chain_and_rejects_nested_direct() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (mode, global, nested, allowed) in [
            ("global", "DIRECT", "Node", false),
            ("global", "Main", "DIRECT", false),
            ("global", "Main", "Node", true),
            ("rule", "DIRECT", "Node", true),
        ] {
            let proxies = json!({
                "GLOBAL":{"type":"Selector","now":global,"all":["DIRECT","Main"]},
                "Main":{"type":"URLTest","now":nested,"all":["DIRECT","Node"]},
                "DIRECT":{"type":"Direct"},
                "Node":{"type":"Trojan"}
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut chunk = [0; 512];
                        let count = stream.read(&mut chunk).await.unwrap();
                        assert!(count > 0 && request.len() + count <= 8192);
                        request.extend_from_slice(&chunk[..count]);
                    }
                    let request = String::from_utf8_lossy(&request);
                    assert!(request.starts_with("GET "));
                    let path = request.split_whitespace().nth(1).unwrap();
                    let body = if path == "/proxies" {
                        json!({"proxies":proxies}).to_string()
                    } else {
                        proxies[path.strip_prefix("/proxies/").unwrap()].to_string()
                    };
                    stream.write_all(format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(), body
                    ).as_bytes()).await.unwrap();
                }
            });
            let settings = Settings {
                active_profile: Some(uuid::Uuid::new_v4().to_string()),
                run_mode: mode.into(),
                controller_port: port,
                ..Default::default()
            };
            let result = prepare_protection(
                &Api::new(&settings).unwrap(),
                &settings,
                "proxies: []\nrules: ['MATCH,Main']\n",
                false,
            )
            .await;
            server.abort();
            if allowed {
                let saved = result.unwrap();
                assert_eq!(saved.node, "Node");
                assert_eq!(saved.run_mode(), mode);
                assert_eq!(
                    saved.locked_groups.get("Main").map(String::as_str),
                    Some("Node")
                );
            } else {
                assert!(result.unwrap_err().to_string().contains("DIRECT"));
                assert!(settings.claude_protection.is_none());
            }
        }
    }

    #[test]
    fn normal_native_status_is_not_an_error_and_vpn_errors_do_not_hide_the_core() {
        for value in [
            json!({"running":true,"error":""}),
            json!({"running":true,"vpn_error":"VPN denied"}),
        ] {
            assert_eq!(native_response(value).unwrap()["running"], true);
        }
        assert!(native_response(json!({"error":"Failed to initialize"})).is_err());
    }
    struct Fake;
    impl NativeHost for Fake {
        fn request(&self, _: Value) -> Result<Value> {
            Ok(json!({"ok":true}))
        }
    }
    #[test]
    fn active_group_follows_the_route_and_mode_not_the_first_group_or_shared_leaf() {
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::open(root.path().into(), Arc::new(Fake)).unwrap();
        session.settings.run_mode = "rule".into();
        session.route = "Main".into();
        session.proxies = serde_json::from_value(json!({"proxies": {
            "AAA unused": {"type":"Selector", "now":"Leaf", "all":["Leaf"]},
            "GLOBAL": {"type":"Selector", "now":"Main", "all":["Main", "Leaf"]},
            "Main": {"type":"Selector", "now":"Auto", "all":["Auto", "Leaf"]},
            "Auto": {"type":"URLTest", "now":"Leaf", "all":["Leaf"]},
            "Leaf": {"type":"Trojan"}
        }}))
        .unwrap();
        assert_eq!(session.active_group(), Some("Main"));
        session.settings.run_mode = "global".into();
        assert_eq!(session.active_group(), Some("GLOBAL"));
        session.settings.run_mode = "direct".into();
        assert_eq!(session.active_group(), None);
        session.settings.run_mode = "rule".into();
        session.route = "Leaf".into();
        assert_eq!(session.active_group(), None);
        session.route = "Missing".into();
        assert_eq!(session.active_group(), None);
        session.route = "Main".into();
        assert_eq!(session.active_group(), Some("Main"));
        session.clear_snapshot();
        assert_eq!(session.active_group(), None);
    }
    #[test]
    fn logo_and_guard_notifications_share_a_validated_native_request() {
        struct Recorder(std::sync::Mutex<Vec<Value>>);
        impl NativeHost for Recorder {
            fn request(&self, value: Value) -> Result<Value> {
                self.0.lock().unwrap().push(value);
                Ok(json!({"ok":true}))
            }
        }
        let host = Recorder(Default::default());
        host.notify("Clash of Rust", "喵!", true).unwrap();
        host.notify("Claude 保护", "异常连接已关闭", false).unwrap();
        assert!(host.notify("invalid\0title", "body", false).is_err());
        let requests = host.0.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|request| request["op"] == "notify"));
        assert_eq!(requests[0]["request_permission"], true);
        assert_eq!(requests[1]["request_permission"], false);
    }
    #[tokio::test]
    async fn mobile_protection_survives_reopen_and_restores_network_preferences() {
        use crate::core::claude_check::protection::{Previous, Protection};
        let root = tempfile::tempdir().unwrap();
        let native = Arc::new(Fake);
        let mut session = Session::open(root.path().into(), native.clone()).unwrap();
        session.save_profile(None, "测试订阅", "", Some(
            "proxies: [{name: Test, type: socks5, server: 127.0.0.1, port: 1080}]\nrules: ['MATCH,Test']\n"
        )).await.unwrap();
        let profile = session
            .profiles
            .iter()
            .find(|p| p.name == "测试订阅")
            .unwrap()
            .id
            .clone();
        session.activate(&profile).unwrap();
        let mut before = session.settings.clone();
        before.ipv6 = false;
        before.udp = false;
        before.dns_override = false;
        session.commit_settings(before.clone()).unwrap();
        let raw = session.runtime(&before).unwrap();
        let mut next = before.clone();
        next.claude_protection = Some(Arc::new(Protection::new(
            profile,
            "Test".into(),
            Previous {
                mode: before.run_mode.clone(),
                proxy_mode: core::model::ProxyMode::Off,
                selectors: Default::default(),
            },
        )));
        session.commit_settings(next).unwrap();
        assert!(effective_ipv6(&session.settings));
        assert!(!session.settings.ipv6 && !session.settings.udp && !session.settings.dns_override);
        let active: serde_yaml::Value =
            serde_yaml::from_str(&session.runtime(&session.settings).unwrap()).unwrap();
        assert_eq!(active["ipv6"], true);
        assert_eq!(active["tun"]["enable"], false);
        let mut reopened = Session::open(root.path().into(), native).unwrap();
        assert!(reopened.settings.claude_protection.is_some());
        reopened.commit_settings(before).unwrap();
        assert!(!effective_ipv6(&reopened.settings));
        assert_eq!(reopened.runtime(&reopened.settings).unwrap(), raw);
        assert!(!core::profile_transaction::recover(&reopened.store.root).unwrap());
    }
    #[test]
    fn mobile_never_inherits_subscription_owned_routes_or_controller() {
        let settings = Settings {
            ipv6: false,
            ..Settings::default()
        };
        let raw = "proxies: []\nrules: ['MATCH,DIRECT']\ntun:\n  enable: true\n  auto-route: true\n  file-descriptor: 42\nexternal-controller: 0.0.0.0:1\n";
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
    #[tokio::test]
    async fn switching_profiles_follows_active_rules_without_losing_a_draft() {
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::open(root.path().into(), Arc::new(Fake)).unwrap();
        let original = session.profile().id.clone();
        session
            .save_profile(
                None,
                "另一订阅",
                "",
                Some("proxies: []\nrules: ['DOMAIN,example.test,DIRECT', 'MATCH,DIRECT']\n"),
            )
            .await
            .unwrap();
        let next = session
            .profiles
            .iter()
            .find(|p| p.name == "另一订阅")
            .unwrap()
            .id
            .clone();
        session.activate(&next).unwrap();
        assert_eq!(session.rule_profile, next);
        assert!(
            session
                .document()
                .original
                .iter()
                .any(|r| r.raw.contains("example.test"))
        );
        session
            .save_rule("", "DOMAIN-SUFFIX,draft.test,DIRECT", 1)
            .unwrap();
        let draft = session.draft.clone();
        session.activate(&original).unwrap();
        assert_eq!(session.profile().id, original);
        assert_eq!(session.rule_profile, next);
        assert_eq!(session.draft, draft);
        assert!(session.select_rules(&original).is_err());
    }
    #[test]
    fn background_stats_keep_a_bounded_first_screen_and_correct_rate_interval() {
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::open(root.path().into(), Arc::new(Fake)).unwrap();
        let at = Instant::now();
        let connections = |upload| Connections {
            upload_total: upload,
            connections: (0..80)
                .map(|id| core::api::Connection {
                    id: id.to_string().into(),
                    upload,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        session.apply_snapshot(Snapshot {
            proxies: None,
            connections: Some(connections(10)),
            stats: core::api::ConnectionStats {
                count: 80,
                upload_total: 10,
                download_total: 0,
            },
            at,
        });
        session.apply_snapshot(Snapshot {
            proxies: None,
            connections: None,
            stats: core::api::ConnectionStats {
                count: 90,
                upload_total: 110,
                download_total: 0,
            },
            at: at + std::time::Duration::from_secs(1),
        });
        assert_eq!(session.count, 90);
        assert_eq!(session.connections.connections.len(), 50);
        assert_eq!(session.rates.0, 100);
        session.apply_snapshot(Snapshot {
            proxies: None,
            connections: Some(connections(210)),
            stats: core::api::ConnectionStats {
                count: 80,
                upload_total: 210,
                download_total: 0,
            },
            at: at + std::time::Duration::from_secs(2),
        });
        assert_eq!(
            session.connections.connections[0].rates.unwrap().upload,
            100
        );
        session.clear_snapshot();
        assert_eq!(session.count, 0);
        assert!(session.connections.connections.is_empty());
        assert_eq!(session.rates, (0, 0));
    }
    #[test]
    fn vpn_dns_stays_available_with_override_off_and_subscription_dns_disabled() {
        let settings = Settings {
            dns_override: false,
            ..Settings::default()
        };
        for upstreams in ["", "  nameserver: []\n", "  nameserver: [9.9.9.9]\n"] {
            let raw = format!(
                "proxies: []\nrules: ['MATCH,DIRECT']\ndns:\n  enable: false\n  nameserver-policy: {{example.test: 1.1.1.1}}\n{upstreams}"
            );
            let raw = core::config::runtime_config(&raw, &settings).unwrap();
            let value: serde_yaml::Value =
                serde_yaml::from_str(&android_safe_config(&raw).unwrap()).unwrap();
            assert_eq!(value["dns"]["enable"], true);
            assert_eq!(value["dns"]["nameserver-policy"]["example.test"], "1.1.1.1");
            let expected = if upstreams.contains("9.9.9.9") {
                serde_yaml::to_value(["9.9.9.9"]).unwrap()
            } else {
                serde_yaml::to_value(core::config::DEFAULT_DNS_NAMESERVERS).unwrap()
            };
            assert_eq!(value["dns"]["nameserver"], expected);
            assert!(!settings.dns_override);
        }
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
