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
                .map(|()| messages::action_config_imported(language).into()),
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
                        .map(|()| messages::action_subscription_updated(language).into())
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
                Ok(messages::action_core_started(language).into())
            }
            Action::SavePorts(mixed) => {
                engine.save_ports(mixed).await?;
                Ok(messages::action_port_saved(language).into())
            }
            Action::LoadRules(_) => Ok(String::new()),
            Action::SaveRules(id, rules, saved) => {
                engine
                    .save_profile_rules_checked(&id, rules, Some(rule_manager::revision(&saved)?))
                    .await?;
                Ok(if engine.settings.active_profile.as_deref() == Some(&id) {
                    messages::action_active_rules_saved(language)
                } else {
                    messages::action_inactive_rules_saved(language)
                }
                .into())
            }
            Action::Appearance(theme_mode) => {
                let settings = Settings {
                    theme_mode: Some(theme_mode),
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(messages::action_appearance_saved(language).into())
            }
            Action::Accent(accent_color) => {
                let settings = Settings {
                    accent_color,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(messages::action_accent_saved(language).into())
            }
            Action::TextScale(text_scale) => {
                let settings = Settings {
                    text_scale,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(messages::action_text_size_saved(language).into())
            }
            Action::TraySpeed(tray_speed) => {
                let settings = Settings {
                    tray_speed,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(messages::action_tray_display_saved(language).into())
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
                Ok(messages::action_update_channel_saved(language).into())
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
                    (true, true) => messages::action_dns_override_fake_ip_on(language),
                    (true, false) => messages::action_dns_override_fake_ip_off(language),
                    (false, _) => messages::action_dns_override_disabled(language),
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
                    messages::action_fake_ip_enabled(language)
                } else {
                    messages::action_fake_ip_disabled(language)
                }
                .into())
            }
            Action::FakeIpAdd(values) => {
                let mut settings = engine.settings.clone();
                settings.add_fake_ip_exclusions(&values)?;
                engine.save_settings(settings).await?;
                Ok(messages::action_fake_ip_exclusions_saved(language).into())
            }
            Action::FakeIpRemove(domain) => {
                let mut settings = engine.settings.clone();
                settings.remove_fake_ip_exclusion(&domain)?;
                engine.save_settings(settings).await?;
                Ok(messages::action_fake_ip_exclusions_saved(language).into())
            }
            Action::FakeIpRestoreDefaults => {
                let mut settings = engine.settings.clone();
                settings.restore_fake_ip_defaults()?;
                engine.save_settings(settings).await?;
                Ok(messages::action_dns_exclusions_restored(language).into())
            }
            Action::Ipv6(ipv6) => {
                let settings = Settings {
                    ipv6,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(messages::action_ipv6_saved(language).into())
            }
            Action::Udp(udp) => {
                let settings = Settings {
                    udp,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(messages::action_udp_saved(language).into())
            }
            Action::ClaudeProtection(enabled) => {
                engine.claude_protection(enabled).await?;
                Ok(if enabled {
                    messages::action_claude_protection_enabled(language)
                } else {
                    messages::action_claude_protection_disabled(language)
                }
                .into())
            }
            Action::Autostart(enabled) => {
                tokio::task::spawn_blocking(move || platform::set_autostart(enabled)).await??;
                Ok(if enabled {
                    messages::action_autostart_enabled(language)
                } else {
                    messages::action_autostart_disabled(language)
                }
                .into())
            }
            Action::DelayInterval(minutes) => {
                let settings = Settings {
                    delay_interval_minutes: minutes,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(messages::action_delay_interval_saved(language).into())
            }
            Action::SubscriptionInterval(minutes) => {
                let settings = Settings {
                    subscription_update_interval_minutes: minutes,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(messages::action_subscription_interval_saved(language).into())
            }
            Action::UpdateInterval(minutes) => {
                let settings = Settings {
                    update_check_interval_minutes: minutes,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(messages::action_update_interval_saved(language).into())
            }
            Action::NodeSort(node_sort) => {
                let settings = Settings {
                    node_sort,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(messages::action_node_sort_saved(language).into())
            }
            Action::GroupPagination(proxy_groups_paginated) => {
                let settings = Settings {
                    proxy_groups_paginated,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(messages::action_group_view_saved(language).into())
            }
            Action::ProfilePagination(profiles_paginated) => {
                let settings = Settings {
                    profiles_paginated,
                    ..engine.settings.clone()
                };
                engine.save_settings(settings).await?;
                Ok(messages::action_subscription_view_saved(language).into())
            }
            Action::DeleteProfile(id) => {
                engine.delete_profile(&id).await?;
                Ok(messages::action_subscription_deleted(language).into())
            }
            Action::OpenProfiles => {
                platform::open_directory(&engine.store.root.join("profiles"))?;
                Ok(messages::action_subscription_directory_opened(language).into())
            }
            Action::Import(..) | Action::UpdateProfile(_) => {
                import_result.expect("import actions are handled before locking")
            }
            Action::EditProfile(id, name, source) => {
                engine.edit_profile(&id, &name, &source)?;
                Ok(messages::action_subscription_edited(language).into())
            }
            Action::Activate(id) => {
                engine.activate(id).await?;
                Ok(messages::action_active_profile_changed(language).into())
            }
            Action::Mode(mode) => {
                engine.select_mode(&mode).await?;
                Ok(messages::action_routing_mode_changed(language).into())
            }
            Action::ProxyMode(mode) => {
                engine.select_proxy_mode(mode).await?;
                Ok(match mode {
                    ProxyMode::System => messages::action_system_proxy_enabled(language),
                    ProxyMode::Tun => messages::action_tun_enabled(language),
                    ProxyMode::Off => messages::action_proxy_mode_disabled(language),
                }
                .into())
            }
            Action::Select(group, node) => {
                engine.select_node(&group, &node).await?;
                Ok(messages::action_node_selected(language, node))
            }
            Action::Delay(node) => {
                let delay = engine.api.delay(&node).await?;
                Ok(if delay == 0 {
                    messages::action_node_delay_failed(language, node)
                } else {
                    messages::action_node_delay(language, delay, node)
                })
            }
            Action::Close(id) => {
                engine.api.close_connection(&id).await?;
                Ok(messages::action_connection_closed(language).into())
            }
            Action::CloseAll => {
                engine.api.close_connections().await?;
                Ok(messages::action_connections_closed(language).into())
            }
            Action::ClearLogs => {
                engine.clear_logs()?;
                Ok(messages::action_logs_cleared(language).into())
            }
            Action::ExportLogs => {
                let path = engine.export_logs()?;
                Ok(messages::action_logs_exported(language, path.display()))
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
