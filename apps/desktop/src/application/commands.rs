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
    SetLanguage(LanguagePreference),
    UpdatePrereleases(bool),
    DnsOverride(bool),
    FakeIp(bool),
    FakeIpAdd(Vec<String>),
    FakeIpRemove(String),
    FakeIpRestoreDefaults,
    Ipv6(bool),
    Udp(bool),
    ClaudeProtection(bool),
    Autostart(bool),
    DelayInterval(u32),
    SubscriptionInterval(u32),
    UpdateInterval(u32),
    NodeSort(NodeSort),
    GroupPagination(bool),
    ProfilePagination(bool),
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
}

impl Action {
    pub(super) fn allowed_during_protection(&self) -> bool {
        matches!(
            self,
            Self::Refresh
                | Self::Start
                | Self::Appearance(_)
                | Self::Accent(_)
                | Self::TextScale(_)
                | Self::TraySpeed(_)
                | Self::SetLanguage(_)
                | Self::NodeSort(_)
                | Self::GroupPagination(_)
                | Self::ProfilePagination(_)
                | Self::LoadRules(_)
                | Self::RefreshTray
                | Self::OpenProfiles
                | Self::Delay(_)
                | Self::ClearLogs
                | Self::ExportLogs
                | Self::ClaudeProtection(false)
        )
    }

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
    pub(super) profile_updated: bool,
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
    language: Language,
) -> Reply {
    let profile_edit = matches!(&action, Action::EditProfile(..));
    let profile_import = matches!(&action, Action::Import(..));
    let updating_profile = matches!(&action, Action::UpdateProfile(_));
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
    let import_result = match &action {
        Action::Import(name, source) => Some(
            Engine::import_shared(&engine, name.clone(), source.clone(), None)
                .await
                .map(|()| language.text("配置已导入", "Configuration imported").into()),
        ),
        Action::UpdateProfile(id) => {
            let profile = engine
                .lock()
                .await
                .profiles
                .iter()
                .find(|profile| &profile.id == id)
                .cloned();
            Some(match profile {
                Some(profile) => {
                    Engine::import_shared(&engine, profile.name, profile.source, Some(profile.id))
                        .await
                        .map(|()| language.text("订阅已更新", "Subscription updated").into())
                }
                None => Err(anyhow::anyhow!("配置不存在")),
            })
        }
        _ => None,
    };
    let mut engine = engine.lock().await;
    if matches!(
        &action,
        Action::Start
            | Action::SavePorts(_)
            | Action::DnsOverride(_)
            | Action::FakeIp(_)
            | Action::FakeIpAdd(_)
            | Action::FakeIpRemove(_)
            | Action::FakeIpRestoreDefaults
            | Action::Ipv6(_)
            | Action::Udp(_)
            | Action::ClaudeProtection(_)
            | Action::SaveRules(..)
            | Action::Activate(_)
            | Action::DeleteProfile(_)
            | Action::Mode(_)
            | Action::ProxyMode(_)
            | Action::Select(..)
    ) {
        engine.invalidate_views();
    }
    let result: anyhow::Result<String> = async {
        if !action.allowed_during_protection() {
            engine.settings.require_mutable()?;
        }
        match action {
            Action::Refresh => Ok(String::new()),
            Action::RefreshTray => Ok(String::new()),
            Action::Start => {
                if !engine.running() {
                    engine.start_with_proxy_mode().await?;
                }
                Ok(language.text("内核已启动", "Core started").into())
            }
            Action::SavePorts(mixed) => {
                engine.save_ports(mixed).await?;
                Ok(language
                    .text(
                        "端口已保存，运行中的内核已自动重启",
                        "Port saved; the running core was restarted",
                    )
                    .into())
            }
            Action::LoadRules(_) => Ok(String::new()),
            Action::SaveRules(id, rules, saved) => {
                engine
                    .save_profile_rules_checked(&id, rules, Some(rule_manager::revision(&saved)?))
                    .await?;
                Ok(if engine.settings.active_profile.as_deref() == Some(&id) {
                    language.text(
                        "规则已保存并应用；新连接按新规则匹配",
                        "Rules saved and applied to new connections",
                    )
                } else {
                    language.text(
                        "规则已保存，启用该订阅时生效",
                        "Rules saved; they apply when this subscription is active",
                    )
                }
                .into())
            }
            Action::Appearance(theme_mode) => {
                let settings = Settings {
                    theme_mode: Some(theme_mode),
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(language.text("外观已自动保存", "Appearance saved").into())
            }
            Action::Accent(accent_color) => {
                let settings = Settings {
                    accent_color,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(language.text("强调色已自动保存", "Accent color saved").into())
            }
            Action::TextScale(text_scale) => {
                let settings = Settings {
                    text_scale,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(language.text("字号已自动保存", "Text size saved").into())
            }
            Action::TraySpeed(tray_speed) => {
                let settings = Settings {
                    tray_speed,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(language.text("托盘显示已保存", "Tray display saved").into())
            }
            Action::SetLanguage(language) => {
                if engine.settings.language != language {
                    let settings = Settings {
                        language,
                        ..engine.settings.clone()
                    };
                    engine.save_settings(settings).await?;
                }
                Ok(String::new())
            }
            Action::UpdatePrereleases(update_prereleases) => {
                let settings = Settings {
                    update_prereleases,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(language.text("更新通道已保存", "Update channel saved").into())
            }
            Action::DnsOverride(dns_override) => {
                // DNS override owns Fake-IP: turning it off also turns off the
                // dependent preference instead of leaving a stale on state.
                let fake_ip =
                    dns_override && engine.settings.dns_override && engine.settings.fake_ip;
                let settings = Settings {
                    dns_override,
                    fake_ip,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(match (dns_override, fake_ip) {
                    (true, true) => language.text(
                        "DNS 覆盖已保存并应用；Fake-IP 保持开启",
                        "DNS override applied; Fake-IP remains on",
                    ),
                    (true, false) => language.text(
                        "DNS 覆盖已保存并应用；Fake-IP 保持关闭，可在网络设置中开启",
                        "DNS override applied; Fake-IP remains off and can be enabled in Network settings",
                    ),
                    (false, _) => language.text(
                        "DNS 覆盖与 Fake-IP 已关闭，其他 DNS 字段沿用订阅",
                        "DNS override and Fake-IP are off; other DNS settings use the subscription",
                    ),
                }
                .into())
            }
            Action::FakeIp(fake_ip) => {
                if fake_ip && !engine.settings.dns_override {
                    anyhow::bail!("请先开启 DNS 覆盖，再启用 Fake-IP");
                }
                let settings = Settings {
                    fake_ip,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(if fake_ip {
                    language.text("Fake-IP 已开启并应用", "Fake-IP enabled and applied")
                } else {
                    language.text("Fake-IP 已关闭，DNS 改用真实 IP", "Fake-IP disabled; DNS uses real IP addresses")
                }
                .into())
            }
            Action::FakeIpAdd(values) => {
                let mut settings = engine.settings.clone();
                settings.add_fake_ip_exclusions(&values)?;
                engine.save_settings(settings).await?;
                Ok(language
                    .text("Fake-IP 排除项已保存并应用", "Fake-IP exclusions saved and applied")
                    .into())
            }
            Action::FakeIpRemove(domain) => {
                let mut settings = engine.settings.clone();
                settings.remove_fake_ip_exclusion(&domain)?;
                engine.save_settings(settings).await?;
                Ok(language
                    .text("Fake-IP 排除项已保存并应用", "Fake-IP exclusions saved and applied")
                    .into())
            }
            Action::FakeIpRestoreDefaults => {
                let mut settings = engine.settings.clone();
                settings.restore_fake_ip_defaults()?;
                engine.save_settings(settings).await?;
                Ok(language
                    .text(
                        "DNS 覆盖排除项已还原，自定义排除项已保留",
                        "DNS override exclusions restored; custom exclusions retained",
                    )
                    .into())
            }
            Action::Ipv6(ipv6) => {
                let settings = Settings {
                    ipv6,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(language.text("IPv6 设置已保存并应用", "IPv6 settings saved and applied").into())
            }
            Action::Udp(udp) => {
                let settings = Settings {
                    udp,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(language.text("UDP 设置已保存并应用", "UDP settings saved and applied").into())
            }
            Action::ClaudeProtection(enabled) => {
                engine.claude_protection(enabled).await?;
                Ok(if enabled {
                    language.text(
                        "Claude 保护已启用，正在复测",
                        "Claude protection enabled; checking status",
                    )
                } else {
                    language.text(
                        "Claude 保护已关闭，原网络状态已恢复",
                        "Claude protection disabled; previous network state restored",
                    )
                }
                .into())
            }
            Action::Autostart(enabled) => {
                tokio::task::spawn_blocking(move || platform::set_autostart(enabled)).await??;
                Ok(if enabled {
                    language.text("已开启后台自启动", "Background autostart enabled")
                } else {
                    language.text("已关闭后台自启动", "Background autostart disabled")
                }
                .into())
            }
            Action::DelayInterval(minutes) => {
                let settings = Settings {
                    delay_interval_minutes: minutes,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(language.text("定时测速间隔已保存", "Speed test interval saved").into())
            }
            Action::SubscriptionInterval(minutes) => {
                let settings = Settings {
                    subscription_update_interval_minutes: minutes,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(language
                    .text("订阅自动更新间隔已保存", "Subscription update interval saved")
                    .into())
            }
            Action::UpdateInterval(minutes) => {
                let settings = Settings {
                    update_check_interval_minutes: minutes,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(language.text("自动检查更新间隔已保存", "Update check interval saved").into())
            }
            Action::NodeSort(node_sort) => {
                let settings = Settings {
                    node_sort,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(language.text("节点排序已保存", "Node sort order saved").into())
            }
            Action::GroupPagination(proxy_groups_paginated) => {
                let settings = Settings {
                    proxy_groups_paginated,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(language.text("代理组显示方式已保存", "Proxy group display saved").into())
            }
            Action::ProfilePagination(profiles_paginated) => {
                let settings = Settings {
                    profiles_paginated,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(language.text("订阅显示方式已保存", "Subscription display saved").into())
            }
            Action::DeleteProfile(id) => {
                engine.delete_profile(&id).await?;
                Ok(language
                    .text(
                        "订阅已删除；使用中的订阅会先切换到默认直连",
                        "Subscription removed; an active subscription is switched to the default direct connection",
                    )
                    .into())
            }
            Action::OpenProfiles => {
                platform::open_directory(&engine.store.root.join("profiles"))?;
                Ok(language.text("已打开订阅配置目录", "Subscription directory opened").into())
            }
            Action::Import(..) | Action::UpdateProfile(_) => {
                import_result.expect("import actions are handled before locking")
            }
            Action::EditProfile(id, name, source) => {
                engine.edit_profile(&id, &name, &source)?;
                Ok(language
                    .text(
                        "订阅信息已保存；更改来源后，点击更新可重新载入配置。",
                        "Subscription saved; select Update to reload a changed source.",
                    )
                    .into())
            }
            Action::Activate(id) => {
                engine.activate(id).await?;
                Ok(language.text("当前配置已切换", "Active configuration changed").into())
            }
            Action::Mode(mode) => {
                engine.select_mode(&mode).await?;
                Ok(language.text("运行模式已切换", "Routing mode changed").into())
            }
            Action::ProxyMode(mode) => {
                engine.select_proxy_mode(mode).await?;
                Ok(match mode {
                    ProxyMode::System => language.text("系统代理已开启", "System proxy enabled"),
                    ProxyMode::Tun => language.text("TUN 模式已开启", "TUN mode enabled"),
                    ProxyMode::Off => language.text("代理模式已关闭", "Proxy mode disabled"),
                }
                .into())
            }
            Action::Select(group, node) => {
                engine.select_node(&group, &node).await?;
                Ok(format!("{} {node}", language.text("已选择", "Selected")))
            }
            Action::Delay(node) => {
                let delay = engine.api.delay(&node).await?;
                Ok(if delay == 0 {
                    format!("{node} · {}", language.text("测速未成功", "Speed test failed"))
                } else {
                    format!("{node} · {delay} ms")
                })
            }
            Action::Close(id) => {
                engine.api.close_connection(&id).await?;
                Ok(language.text("连接已关闭", "Connection closed").into())
            }
            Action::CloseAll => {
                engine.api.close_connections().await?;
                Ok(language.text("连接已全部关闭", "All connections closed").into())
            }
            Action::ClearLogs => {
                engine.clear_logs()?;
                Ok(language.text("日志已清空", "Logs cleared").into())
            }
            Action::ExportLogs => {
                let path = engine.export_logs()?;
                Ok(format!(
                    "{} {}",
                    language.text("日志已导出到", "Logs exported to"),
                    path.display()
                ))
            }
        }
    }
    .await;
    let rules_applied = saving_rules && result.is_ok();
    let profile_updated = updating_profile && result.is_ok();
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
    let tray_nodes = if refresh_tray || external_change || profile_updated {
        Some(engine.tray_nodes().await.unwrap_or(None))
    } else {
        None
    };
    Reply {
        view_revision: engine.view_revision(),
        remote: engine.is_remote(),
        profile_edit,
        profile_import,
        profile_updated,
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
        geo_status: engine.geo_manifest.display_status(),
        autostart: platform::autostart_enabled().unwrap_or(false),
        protection_changed,
    }
}

#[cfg(test)]
mod synchronization_tests {
    use super::*;

    #[test]
    fn protection_allows_observation_and_disable_but_rejects_mutations() {
        for allowed in [
            Action::Refresh,
            Action::Start,
            Action::LoadRules("fixture".into()),
            Action::OpenProfiles,
            Action::Delay("node".into()),
            Action::ExportLogs,
            Action::ClearLogs,
            Action::ClaudeProtection(false),
            Action::Appearance(clash_client_core::config::ThemeMode::Dark),
            Action::Accent(clash_client_core::config::AccentColor::Blue),
            Action::TextScale(125),
            Action::TraySpeed(true),
            Action::SetLanguage(LanguagePreference::English),
            Action::NodeSort(proxy_order::NodeSort::Name),
            Action::GroupPagination(true),
        ] {
            assert!(allowed.allowed_during_protection());
        }
        for blocked in [
            Action::SavePorts(7897),
            Action::UpdatePrereleases(true),
            Action::DnsOverride(true),
            Action::FakeIp(true),
            Action::FakeIpAdd(vec!["example.test".into()]),
            Action::FakeIpRemove("example.test".into()),
            Action::FakeIpRestoreDefaults,
            Action::Ipv6(true),
            Action::Udp(true),
            Action::Import("fixture".into(), "https://example.test".into()),
            Action::EditProfile(
                "fixture".into(),
                "fixture".into(),
                "https://example.test".into(),
            ),
            Action::UpdateProfile("fixture".into()),
            Action::Autostart(true),
            Action::DelayInterval(30),
            Action::SubscriptionInterval(360),
            Action::UpdateInterval(360),
            Action::Mode("rule".into()),
            Action::ProxyMode(ProxyMode::Tun),
            Action::Select("GLOBAL".into(), "DIRECT".into()),
            Action::Close("fixture".into()),
            Action::CloseAll,
            Action::ClaudeProtection(true),
        ] {
            assert!(!blocked.allowed_during_protection());
        }
    }

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
