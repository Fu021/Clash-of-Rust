//! Serialized engine commands; no window, toolkit or drawing types.
use super::*;

#[derive(Debug, Clone)]
pub(super) enum Action {
    Refresh,
    Start,
    SavePorts(u16),
    SaveRules(String, ProfileRules, Arc<ProfileRules>),
    LoadRules(String),
    Appearance(ThemeMode),
    Accent(AccentColor),
    TextScale(u16),
    TraySpeed(bool),
    UpdatePrereleases(bool),
    DnsOverride(bool),
    Ipv6(bool),
    Udp(bool),
    ClaudeProtection(bool),
    ReputationKey(Option<String>),
    Autostart(bool),
    DelayInterval(u32),
    NodeSort(NodeSort),
    Import(String, String),
    EditProfile(String, String, String),
    RefreshTray,
    Activate(String),
    UpdateProfile(String),
    DeleteProfile(String),
    OpenProfiles,
    Mode(String),
    ProxyMode(ProxyMode),
    Select(String, String),
    Delay(String),
    Close(String),
    CloseAll,
    ClearLogs,
    ExportLogs,
    UpdateGeo,
}

impl Action {
    /// Preview only local appearance; persistence still uses the serialized engine queue.
    pub(super) fn preview_appearance(&self, settings: &mut Settings) -> bool {
        match self {
            Self::Appearance(mode) => settings.theme_mode = Some(*mode),
            Self::Accent(color) => settings.accent_color = *color,
            Self::TextScale(scale) => settings.text_scale = *scale,
            _ => return false,
        }
        true
    }
}

#[derive(Debug, Clone)]
pub(super) struct Reply {
    pub(super) view_revision: u64,
    pub(super) remote: bool,
    pub(super) profile_edit: bool,
    pub(super) profile_import: bool,
    pub(super) scope: Scope,
    pub(super) start_requested: bool,
    pub(super) settings: Settings,
    pub(super) profiles: Option<Vec<Profile>>,
    pub(super) snapshot: Option<Snapshot>,
    pub(super) rule_document: Option<Arc<rule_manager::Document>>,
    pub(super) rules_applied: bool,
    pub(super) notice: Result<String, String>,
    pub(super) running: bool,
    pub(super) geo_status: String,
    pub(super) autostart: bool,
    pub(super) protection_changed: bool,
    pub(super) tray_nodes: Option<Option<tray::Nodes>>,
}

pub(super) struct ViewContext {
    pub(super) revision: u64,
    pub(super) rule_profile: Option<String>,
    pub(super) active_profile: Option<String>,
    pub(super) protected: bool,
    pub(super) editing_rules: bool,
}

impl ViewContext {
    fn refresh_rule(
        &self,
        changed: bool,
        scope: Scope,
        active: Option<&str>,
        exists: impl Fn(&str) -> bool,
    ) -> Option<String> {
        if !changed || scope != Scope::Rules {
            return None;
        }
        let selected = self.rule_profile.as_deref();
        if self.editing_rules && selected.is_some_and(|id| !exists(id)) {
            return None;
        }
        if (!self.editing_rules && selected == self.active_profile.as_deref())
            || selected.is_none_or(|id| !exists(id))
        {
            active.map(str::to_owned)
        } else {
            self.rule_profile.clone()
        }
    }
}

pub(super) async fn execute(
    engine: Arc<Mutex<Engine>>,
    action: Action,
    scope: Scope,
    header: bool,
    context: ViewContext,
) -> Reply {
    let profile_edit = matches!(&action, Action::EditProfile(..));
    let profile_import = matches!(&action, Action::Import(..));
    let refresh_tray = matches!(
        action,
        Action::RefreshTray
            | Action::Start
            | Action::ClaudeProtection(_)
            | Action::Activate(_)
            | Action::DeleteProfile(_)
            | Action::Mode(_)
            | Action::Select(..)
            | Action::SaveRules(..)
    );
    let start_requested = matches!(action, Action::Start);
    let include_profiles = !matches!(action, Action::Refresh | Action::LoadRules(_));
    let rule_id = match &action {
        Action::LoadRules(id) | Action::SaveRules(id, _, _) => Some(id.clone()),
        _ => None,
    };
    let saving_rules = matches!(action, Action::SaveRules(..));
    let protection_requested = matches!(action, Action::ClaudeProtection(_));
    let mut engine = engine.lock().await;
    let result: anyhow::Result<String> = async {
        match action {
            Action::Refresh => Ok(String::new()),
            Action::RefreshTray => Ok(String::new()),
            Action::Start => {
                if !engine.running() {
                    engine.start().await?;
                    engine.restore_proxy_mode().await?;
                }
                Ok("内核已启动".into())
            }
            Action::SavePorts(mixed) => {
                engine.save_ports(mixed).await?;
                Ok("端口已保存，运行中的内核已自动重启".into())
            }
            Action::LoadRules(_) => Ok(String::new()),
            Action::SaveRules(id, rules, saved) => {
                engine
                    .save_profile_rules_checked(&id, rules, Some(rule_manager::revision(&saved)?))
                    .await?;
                Ok(if engine.settings.active_profile.as_deref() == Some(&id) {
                    "规则已保存并应用；新连接按新规则匹配"
                } else {
                    "规则已保存，启用该订阅时生效"
                }
                .into())
            }
            Action::Appearance(theme_mode) => {
                let settings = Settings {
                    theme_mode: Some(theme_mode),
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("外观已自动保存".into())
            }
            Action::Accent(accent_color) => {
                let settings = Settings {
                    accent_color,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("强调色已自动保存".into())
            }
            Action::TextScale(text_scale) => {
                let settings = Settings {
                    text_scale,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("字号已自动保存".into())
            }
            Action::TraySpeed(tray_speed) => {
                let settings = Settings {
                    tray_speed,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("托盘显示已保存".into())
            }
            Action::UpdatePrereleases(update_prereleases) => {
                let settings = Settings {
                    update_prereleases,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("更新通道已保存".into())
            }
            Action::DnsOverride(dns_override) => {
                let settings = Settings {
                    dns_override,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("DNS 覆盖设置已保存并应用".into())
            }
            Action::Ipv6(ipv6) => {
                let settings = Settings {
                    ipv6,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("IPv6 设置已保存并应用".into())
            }
            Action::Udp(udp) => {
                let settings = Settings {
                    udp,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("UDP 设置已保存并应用".into())
            }
            Action::ClaudeProtection(enabled) => {
                engine.claude_protection(enabled).await?;
                Ok(if enabled {
                    "Claude 保护已启用，正在复测"
                } else {
                    "Claude 保护已关闭，原网络状态已恢复"
                }
                .into())
            }
            Action::ReputationKey(ip_reputation_key) => {
                let settings = Settings {
                    ip_reputation_key,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("检测服务设置已保存；密钥不会写入报告".into())
            }
            Action::Autostart(enabled) => {
                tokio::task::spawn_blocking(move || platform::set_autostart(enabled)).await??;
                Ok(if enabled {
                    "已开启开机自启，将在后台启动"
                } else {
                    "已关闭开机自启"
                }
                .into())
            }
            Action::DelayInterval(minutes) => {
                let settings = Settings {
                    delay_interval_minutes: minutes,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("定时测速间隔已保存".into())
            }
            Action::NodeSort(node_sort) => {
                let settings = Settings {
                    node_sort,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok("节点排序已保存".into())
            }
            Action::DeleteProfile(id) => {
                engine.delete_profile(&id).await?;
                Ok("订阅已删除；使用中的订阅会先切换到默认直连".into())
            }
            Action::OpenProfiles => {
                platform::open_directory(&engine.store.root.join("profiles"))?;
                Ok("已打开订阅配置目录".into())
            }
            Action::Import(name, source) => {
                engine.import(name, source, None).await?;
                Ok("配置已导入".into())
            }
            Action::EditProfile(id, name, source) => {
                engine.edit_profile(&id, &name, &source)?;
                Ok("订阅信息已保存；更改来源后，点击更新可重新载入配置。".into())
            }
            Action::Activate(id) => {
                engine.activate(id).await?;
                Ok("当前配置已切换".into())
            }
            Action::UpdateProfile(id) => {
                let profile = engine
                    .profiles
                    .iter()
                    .find(|p| p.id == id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("配置不存在"))?;
                engine
                    .import(profile.name, profile.source, Some(profile.id))
                    .await?;
                Ok("订阅已更新".into())
            }
            Action::Mode(mode) => {
                engine.select_mode(&mode).await?;
                Ok("运行模式已切换".into())
            }
            Action::ProxyMode(mode) => {
                engine.select_proxy_mode(mode).await?;
                Ok(match mode {
                    ProxyMode::System => "系统代理已开启",
                    ProxyMode::Tun => "TUN 模式已开启",
                    ProxyMode::Off => "代理模式已关闭",
                }
                .into())
            }
            Action::Select(group, node) => {
                engine.select_node(&group, &node).await?;
                Ok(format!("已选择 {node}"))
            }
            Action::Delay(node) => {
                let delay = engine.api.delay(&node).await?;
                Ok(format!("{node} · {delay} ms"))
            }
            Action::Close(id) => {
                engine.api.close_connection(&id).await?;
                Ok("连接已关闭".into())
            }
            Action::CloseAll => {
                engine.api.close_connections().await?;
                Ok("连接已全部关闭".into())
            }
            Action::ClearLogs => {
                engine.clear_logs()?;
                Ok("日志已清空".into())
            }
            Action::ExportLogs => {
                let path = engine.export_logs()?;
                Ok(format!("日志已导出到 {}", path.display()))
            }
            Action::UpdateGeo => {
                engine.update_geo().await?;
                Ok("GeoIP、GeoSite、Country、ASN 数据已全部更新".into())
            }
        }
    }
    .await;
    let rules_applied = saving_rules && result.is_ok();
    let protection_changed = protection_requested && result.is_ok();
    let mut notice = result.map_err(|e| format!("{e:#}"));
    // The editor uses the original YAML, not the lossy core rule API. Avoid
    // retaining a second complete rule list on this page.
    let poll_scope = if scope == Scope::Rules {
        Scope::Other
    } else {
        scope
    };
    let snapshot = match engine.poll_with_header(poll_scope, header).await {
        Ok(snapshot) => Some(snapshot),
        Err(e) => {
            if notice.as_ref().is_ok_and(|s| s.is_empty()) {
                notice = Err(format!("{e:#}"));
            }
            None
        }
    };
    // A remote poll refreshes metadata as well. Compare revisions afterwards,
    // so console edits do not need an unrelated GUI action to become visible.
    let external_change = context.revision != engine.view_revision();
    let protection_changed =
        protection_changed || context.protected != engine.settings.claude_protection.is_some();
    let rule_id = rule_id.or_else(|| {
        context.refresh_rule(
            external_change,
            scope,
            engine.settings.active_profile.as_deref(),
            |id| engine.profiles.iter().any(|profile| profile.id == id),
        )
    });
    let rule_document = rule_id.and_then(|id| match engine.rule_document(&id) {
        Ok(doc) => Some(Arc::new(doc)),
        Err(e) => {
            notice = Err(format!("{e:#}"));
            None
        }
    });
    if let Err(error) = &notice {
        engine.record_failure(error);
    }
    let tray_nodes = if refresh_tray || external_change {
        Some(engine.tray_nodes().await.unwrap_or(None))
    } else {
        None
    };
    Reply {
        view_revision: engine.view_revision(),
        remote: engine.is_remote(),
        profile_edit,
        profile_import,
        tray_nodes,
        scope,
        start_requested,
        settings: engine.settings.clone(),
        profiles: (include_profiles || external_change).then(|| engine.profiles.clone()),
        snapshot,
        rule_document,
        rules_applied,
        notice,
        running: engine.running(),
        geo_status: engine.geo_manifest.version.clone(),
        autostart: platform::autostart_enabled().unwrap_or(false),
        protection_changed,
    }
}

#[cfg(test)]
mod synchronization_tests {
    use super::*;

    #[test]
    fn console_profile_changes_follow_the_active_profile_without_discarding_drafts() {
        let mut context = ViewContext {
            revision: 1,
            rule_profile: Some("old".into()),
            active_profile: Some("old".into()),
            protected: false,
            editing_rules: false,
        };
        assert_eq!(
            context
                .refresh_rule(true, Scope::Rules, Some("new"), |_| true)
                .as_deref(),
            Some("new")
        );
        assert!(
            context
                .refresh_rule(false, Scope::Rules, Some("new"), |_| true)
                .is_none()
        );
        context.editing_rules = true;
        assert_eq!(
            context
                .refresh_rule(true, Scope::Rules, Some("new"), |_| true)
                .as_deref(),
            Some("old")
        );
        assert!(
            context
                .refresh_rule(true, Scope::Rules, Some("new"), |id| id != "old")
                .is_none()
        );
    }
}
