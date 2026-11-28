//! Bounded projections from application state to the declarative UI contract.
use super::*;
use clash_desktop_protocol::List as ModelRc;
use clash_desktop_protocol::Text as SharedString;
use clash_desktop_protocol::{
    models::{ViewModels, reconcile},
    *,
};
use std::borrow::Cow;

pub(super) fn model<T: Clone + PartialEq>(old: ModelRc<T>, values: Vec<T>) -> ModelRc<T> {
    if old.with_rows(|rows| rows == values.as_slice()) {
        old
    } else {
        values.into()
    }
}

fn strings(
    old: ModelRc<SharedString>,
    values: impl IntoIterator<Item = impl AsRef<str>>,
) -> ModelRc<SharedString> {
    model(
        old,
        values
            .into_iter()
            .map(|value| SharedString::from(value.as_ref()))
            .collect(),
    )
}
fn tone(status: Status) -> i32 {
    match status {
        Status::Available | Status::Identified => 1,
        Status::Partial | Status::Reachable | Status::Unknown => 2,
        Status::Restricted | Status::Failed => 3,
        Status::Untested => 4,
    }
}

fn claude_report_item(entry: claude_view::Entry<'_>, tone: i32, language: Language) -> ReportItem {
    let section = entry.section();
    let (title, value) = entry.title_value(language);
    let country = match &entry {
        claude_view::Entry::Signal(_, signal)
            if signal.group == clash_of_rust::claude_check::Group::Dns && !signal.ip.is_empty() =>
        {
            match language {
                Language::SimplifiedChinese => {
                    clash_of_rust::probe::country_name(&signal.country).unwrap_or_default()
                }
                _ => clash_client_core::countries::english_name(&signal.country)
                    .unwrap_or_default()
                    .to_owned(),
            }
        }
        _ => String::new(),
    };
    let (_, flag) = clash_of_rust::flags::label(&country);
    let (detail, source) = match entry {
        claude_view::Entry::Config(finding) => (
            finding.explanation_in(language),
            finding.source_in(language).into(),
        ),
        claude_view::Entry::Clean(path) => (String::new(), path.into()),
        claude_view::Entry::Signal(_, signal) => {
            (signal.explanation_in(language), signal.source_in(language))
        }
        claude_view::Entry::Route(route) => (
            route.explanation_in(language),
            messages::claude_core_connection_records(language).into(),
        ),
        claude_view::Entry::GuardRoute(route) => (
            route.explanation_in(language),
            messages::claude_live_blocking_source(language).into(),
        ),
        claude_view::Entry::Routes(routes) => (
            clash_of_rust::claude_check::route_group_detail_in(&routes, language),
            messages::claude_core_connection_records(language).into(),
        ),
    };
    ReportItem {
        country: country.into(),
        flagged: flag.is_some(),
        flag: flag.unwrap_or_default().into(),
        id: section.into(),
        title: title.into(),
        value: value.into(),
        detail: detail.into(),
        source: source.into(),
        status_label: "".into(),
        tone,
        delay: "".into(),
        delay_tone: 4,
        block_items: 0,
    }
}

fn connection_details(
    connection: &clash_of_rust::api::Connection,
    resolved: &str,
    language: Language,
) -> Vec<DetailItem> {
    connection::detail_values_in(connection, resolved, language)
        .into_iter()
        .map(|(label, value)| DetailItem {
            label: label.into(),
            value: if value.is_empty() {
                "—".into()
            } else {
                value.into()
            },
        })
        .collect()
}

impl App {
    pub(super) fn proxy_rows(&self) -> (usize, Vec<(GroupItem, Vec<NodePair>)>) {
        let language = self.presentation_language;
        let query = self.query.to_lowercase();
        let mut groups: Vec<_> = self
            .snapshot
            .proxies
            .proxies
            .iter()
            .filter(|(group, proxy)| {
                !proxy.all.is_empty()
                    && (contains_query(group, &query)
                        || proxy.all.iter().any(|node| contains_query(node, &query)))
            })
            .collect();
        if let Some(index) = groups
            .iter()
            .position(|(name, _)| self.snapshot.active_group.as_deref() == Some(name.as_str()))
        {
            groups[..=index].rotate_right(1);
        }
        let total = groups.len();
        let offset = if self.settings.proxy_groups_paginated {
            self.list_offset
                .min(total.saturating_sub(1) / GROUP_PAGE_SIZE * GROUP_PAGE_SIZE)
        } else {
            0
        };
        let end = if self.settings.proxy_groups_paginated {
            (offset + GROUP_PAGE_SIZE).min(total)
        } else {
            total
        };
        let visible = &groups[offset..end];
        let mut orders = self.proxy_order.borrow_mut();
        orders.retain(|name, _| {
            visible.iter().enumerate().any(|(index, (group, _))| {
                *group == name
                    && (self.expanded.contains(name.as_str())
                        || (!query.is_empty() && index < GROUP_SEARCH_EXPANSION_LIMIT))
            })
        });
        let mut rows = Vec::with_capacity(visible.len());
        let mut delay_labels = BTreeMap::<Option<u32>, SharedString>::new();
        let unknown_kind = SharedString::from("");
        for (index, &(group, proxy)) in visible.iter().enumerate() {
            let group_name = SharedString::from(group.shared());
            let expanded = self.expanded.contains(group.as_str())
                || (!query.is_empty() && index < GROUP_SEARCH_EXPANSION_LIMIT);
            let mut pairs = Vec::new();
            let mut count = 0;
            let mut start = 0;
            if expanded {
                let group_match = contains_query(group, &query);
                let indices = orders.entry(group.clone()).or_insert_with(|| {
                    proxy_order::sorted_indices(
                        &proxy.all,
                        self.node_sort,
                        |node| self.node_delay(node),
                        |node| group_match || contains_query(node, &query),
                    )
                });
                count = indices.len();
                start = self
                    .group_offsets
                    .get(group.as_str())
                    .copied()
                    .unwrap_or(0)
                    .min(last_page_offset(count));
                let mut node = |index: usize| {
                    let source_name = &proxy.all[index];
                    let name = source_name.as_str();
                    let delay = self.node_delay(name);
                    let kind = self
                        .snapshot
                        .proxies
                        .proxies
                        .get(name)
                        .map_or_else(|| unknown_kind.clone(), |node| node.kind.shared().into());
                    let delay_label = delay_labels
                        .entry(delay)
                        .or_insert_with(|| proxy_order::delay_label_in(delay, language).into())
                        .clone();
                    NodeItem {
                        group: group_name.clone(),
                        name: source_name.shared().into(),
                        valid: true,
                        selected: proxy.now.as_str() == name,
                        kind,
                        delay: delay_label,
                        tone: proxy_order::delay_tone(delay),
                        details: proxy_order::node_details(name, group, &self.snapshot.proxies)
                            .into(),
                    }
                };
                for pair in indices[start..(start + PAGE_SIZE).min(count)].chunks(2) {
                    pairs.push(NodePair {
                        left: node(pair[0]),
                        right: pair
                            .get(1)
                            .map_or_else(NodeItem::default, |index| node(*index)),
                    });
                }
            }
            rows.push((
                GroupItem {
                    id: group_name.clone(),
                    title: group_name,
                    current: proxy.now.shared().into(),
                    detail: messages::proxy_group_detail(language, proxy.all.len(), &proxy.kind)
                        .into(),
                    expanded,
                    testing: self.testing_groups.contains(group.as_str()),
                    page: if expanded {
                        messages::proxy_group_page(
                            language,
                            count,
                            (start + PAGE_SIZE).min(count),
                            if count == 0 { 0 } else { start + 1 },
                        )
                    } else {
                        String::new()
                    }
                    .into(),
                    previous: start > 0,
                    next: start + PAGE_SIZE < count,
                    nodes: ModelRc::default(),
                },
                pairs,
            ));
        }
        (total, rows)
    }

    pub(super) fn connection_rows(&self) -> (usize, Vec<ConnectionItem>) {
        let mut cached = self.connection_order.borrow_mut();
        let indices = cached.get_or_insert_with(|| {
            let query = self.query.to_lowercase();
            let connections = &self.snapshot.connections.connections;
            let mut indices: Vec<_> = connections
                .iter()
                .enumerate()
                .filter_map(|(index, connection)| {
                    clash_client_core::connection::matches_query(connection, &query)
                        .then_some(index)
                })
                .collect();
            if self.connection_sort.field.is_some() {
                indices.sort_unstable_by(|a, b| {
                    self.connection_sort
                        .compare(&connections[*a], &connections[*b])
                });
            }
            indices
        });
        let offset = self.list_offset.min(last_page_offset(indices.len()));
        let rows = indices
            .iter()
            .skip(offset)
            .take(PAGE_SIZE)
            .map(|index| {
                let connection = &self.snapshot.connections.connections[*index];
                let rate = |value: Option<u64>| {
                    value.map_or_else(|| "—".into(), |value| format!("{}/s", bytes(value)))
                };
                ConnectionItem {
                    id: connection.id.as_ref().into(),
                    host: format!(
                        "{}:{}",
                        connection::name(connection),
                        connection.metadata.destination_port
                    )
                    .into(),
                    detail: format!(
                        "{} · {}",
                        connection.metadata.network,
                        format::date_text(&connection.start)
                    )
                    .into(),
                    rule: format!("{} {}", connection.rule, connection.rule_payload)
                        .trim()
                        .into(),
                    route: connection
                        .chains
                        .iter()
                        .rev()
                        .map(|name| name.as_str())
                        .collect::<Vec<_>>()
                        .join(" → ")
                        .into(),
                    up: rate(connection.rates.map(|rate| rate.upload)).into(),
                    down: rate(connection.rates.map(|rate| rate.download)).into(),
                    upload: bytes(connection.upload).into(),
                    download: bytes(connection.download).into(),
                }
            })
            .collect();
        (indices.len(), rows)
    }

    fn pager(&self, data: &UiData, total: usize, step: usize) {
        let offset = self.list_offset.min(total.saturating_sub(1) / step * step);
        data.set_page_offset(offset as i32);
        data.set_previous(offset > 0);
        data.set_next(offset + step < total);
        data.set_page_status(
            messages::page_range(
                self.presentation_language,
                (offset + step).min(total),
                if total == 0 { 0 } else { offset + 1 },
                total,
            )
            .into(),
        );
    }

    pub(super) fn present(&self, data: &UiData, models: &mut ViewModels) {
        let language = self.presentation_language;
        macro_rules! set {
            ($get:ident, $put:ident, $value:expr) => {{
                let value = $value;
                if data.$get() != value {
                    data.$put(value);
                }
            }};
        }
        macro_rules! text {
            ($get:ident, $put:ident, $value:expr) => {{
                let source = $value;
                let value: &str = source.as_ref();
                if data.$get().as_str() != value {
                    data.$put(value.into());
                }
            }};
        }
        text!(get_version, set_version, clash_of_rust::VERSION);
        let (copy_revision, written) = self.copy_result.unwrap_or((0, true));
        set!(get_copy_failed, set_copy_failed, !written);
        set!(get_copy_revision, set_copy_revision, copy_revision);
        set!(get_page, set_page, self.page as i32);
        set!(get_ready, set_ready, self.engine.is_some());
        set!(get_running, set_running, self.snapshot.running);
        text!(
            get_network_alert,
            set_network_alert,
            self.network_watch.notice_in(Some(&self.settings), language)
        );
        self.present_protection(data);
        set!(get_working, set_working, self.working);
        set!(get_geo_updating, set_geo_updating, self.geo_updating);
        set!(get_exiting, set_exiting, self.exiting);
        set!(get_dark, set_dark, self.dark);
        set!(get_accent, set_accent, self.settings.accent_color as i32);
        set!(
            get_appearance,
            set_appearance,
            match self.appearance_mode() {
                ThemeMode::Dark => 0,
                ThemeMode::Light => 1,
                ThemeMode::System => 2,
            }
        );
        set!(
            get_text_scale,
            set_text_scale,
            i32::from(self.settings.text_scale)
        );
        set!(get_autostart, set_autostart, self.autostart);
        set!(get_tray_speed, set_tray_speed, self.settings.tray_speed);
        text!(get_language, set_language, self.settings.language.tag());
        set!(
            get_update_prereleases,
            set_update_prereleases,
            self.settings.update_prereleases
        );
        set!(
            get_dns_override,
            set_dns_override,
            self.settings.dns_override
        );
        set!(get_fake_ip, set_fake_ip, self.settings.fake_ip_enabled());
        set!(get_fake_ip_editor, set_fake_ip_editor, self.fake_ip_editor);
        set!(
            get_fake_ip_exclusions,
            set_fake_ip_exclusions,
            model(
                data.get_fake_ip_exclusions(),
                if self.visible && self.page == Page::Settings && self.fake_ip_editor {
                    self.settings
                        .fake_ip_exclusion_entries()
                        .into_iter()
                        .map(|entry| FakeIpExclusionItem {
                            domain: entry.domain.into(),
                            source: match (entry.custom, entry.dns_override) {
                                (true, true) => messages::fake_ip_source_custom_and_dns(language),
                                (true, false) => messages::fake_ip_source_custom(language),
                                _ => messages::fake_ip_source_dns(language),
                            }
                            .into(),
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            )
        );
        set!(
            get_fake_ip_removed_defaults,
            set_fake_ip_removed_defaults,
            self.settings.fake_ip_disabled_defaults.len() as i32
        );
        set!(get_ipv6, set_ipv6, self.settings.ipv6);
        set!(get_udp, set_udp, self.settings.udp);
        set!(
            get_protected,
            set_protected,
            self.settings.claude_protection.is_some()
        );
        set!(get_ports_dirty, set_ports_dirty, self.ports_dirty());
        set!(get_retry_core, set_retry_core, self.ready_to_retry());
        set!(get_notice_error, set_notice_error, self.error);
        set!(
            get_notice_recover_network,
            set_notice_recover_network,
            self.notice_network_recovery
        );
        set!(
            get_notice_critical,
            set_notice_critical,
            self.notice_critical
        );
        set!(get_diagnosing, set_diagnosing, self.diagnosing);
        set!(
            get_notice_revision,
            set_notice_revision,
            self.notice_revision as i32
        );
        set!(
            get_notification_busy,
            set_notification_busy,
            self.notification_busy || self.notification_checking || !self.notification_prepared
        );
        set!(
            get_notice_elapsed,
            set_notice_elapsed,
            self.notice_started
                .map_or(0.0, |started| started.elapsed().as_secs_f32())
        );
        text!(get_notice, set_notice, self.display_notice());
        text!(get_query, set_query, &self.query);
        text!(
            get_profile,
            set_profile,
            self.profiles
                .iter()
                .find(|profile| self.settings.active_profile.as_deref() == Some(&profile.id))
                .map_or(messages::subscription_not_selected(language), |profile| {
                    profile.display_name(language)
                })
        );
        text!(
            get_node,
            set_node,
            self.snapshot.active_node.as_deref().unwrap_or("")
        );
        set!(
            get_mode,
            set_mode,
            match self.snapshot.mode.as_str() {
                "global" => 1,
                "direct" => 2,
                _ => 0,
            }
        );
        set!(
            get_proxy_mode,
            set_proxy_mode,
            match self.snapshot.proxy_mode() {
                ProxyMode::System => 0,
                ProxyMode::Tun => 1,
                ProxyMode::Off => 2,
            }
        );
        text!(
            get_download_rate,
            set_download_rate,
            format!("{}/s", bytes(self.snapshot.download_rate))
        );
        text!(
            get_upload_rate,
            set_upload_rate,
            format!("{}/s", bytes(self.snapshot.upload_rate))
        );
        text!(
            get_connection_count,
            set_connection_count,
            self.snapshot.connection_count.to_string()
        );
        text!(
            get_upload_total,
            set_upload_total,
            bytes(self.snapshot.connections.upload_total)
        );
        text!(
            get_download_total,
            set_download_total,
            bytes(self.snapshot.connections.download_total)
        );
        text!(get_profile_name, set_profile_name, &self.profile_name);
        text!(get_profile_source, set_profile_source, &self.profile_source);
        text!(get_test_url, set_test_url, &self.test_url);
        set!(get_probing, set_probing, self.probing);
        set!(get_network_tool, set_network_tool, self.network_tool);
        text!(get_mixed_port, set_mixed_port, &self.mixed_port);
        text!(get_window_width, set_window_width, &self.window_width);
        text!(get_window_height, set_window_height, &self.window_height);
        text!(get_interval, set_interval, &self.interval_input);
        text!(
            get_subscription_interval,
            set_subscription_interval,
            &self.subscription_interval_input
        );
        text!(
            get_update_interval,
            set_update_interval,
            &self.update_interval_input
        );
        text!(get_fake_ip_input, set_fake_ip_input, &self.fake_ip_input);
        text!(
            get_fake_ip_error,
            set_fake_ip_error,
            notice::localize_notice(&self.fake_ip_error, true, language)
        );
        text!(
            get_geo_status,
            set_geo_status,
            clash_client_core::geodata::localize_status(&self.geo_status, language)
        );
        set!(
            get_node_sort,
            set_node_sort,
            match self.node_sort {
                NodeSort::LatencyAscending => 0,
                NodeSort::LatencyDescending => 1,
                NodeSort::Name => 2,
            }
        );
        text!(
            get_connection_sort,
            set_connection_sort,
            match self.connection_sort.field {
                Some(ConnectionField::Name) => "name",
                Some(ConnectionField::Rule) => "rule",
                Some(ConnectionField::Chain) => "chain",
                Some(ConnectionField::UploadRate) => "upload-rate",
                Some(ConnectionField::DownloadRate) => "download-rate",
                Some(ConnectionField::UploadTotal) => "upload-total",
                Some(ConnectionField::DownloadTotal) => "download-total",
                Some(ConnectionField::Start) => "start",
                None => "",
            }
        );
        set!(
            get_descending,
            set_descending,
            self.connection_sort.descending
        );
        set!(
            get_update_checking,
            set_update_checking,
            self.updates.checking
        );
        set!(
            get_update_available,
            set_update_available,
            self.updates.available.is_some()
        );
        set!(
            get_update_installable,
            set_update_installable,
            self.updates
                .available
                .as_ref()
                .is_some_and(|update| update.package.is_some())
        );
        set!(get_update_busy, set_update_busy, self.updates.busy());
        set!(
            get_update_downloading,
            set_update_downloading,
            self.updates.progress.is_some() && self.updates.installing.is_none()
        );
        set!(
            get_update_fraction,
            set_update_fraction,
            self.updates
                .progress
                .as_ref()
                .map_or(0.0, |progress| if progress.total == 0 {
                    0.0
                } else {
                    (progress.received as f64 / progress.total as f64).min(1.0) as f32
                })
        );
        let download_status = self.updates.progress.as_ref().map(|progress| {
            let language = self.presentation_language;
            if progress.route.is_empty() {
                messages::download_preparing(language).to_owned()
            } else if progress.received == 0 {
                messages::download_connecting(language, progress.routing_in(language))
            } else {
                messages::download_progress(
                    language,
                    bytes(progress.received),
                    progress.routing_in(language),
                    bytes(progress.bytes_per_second),
                    bytes(progress.total),
                )
            }
        });
        let update_status = self
            .updates
            .failure
            .as_deref()
            .or(self.updates.outcome.as_deref())
            .or(self.updates.install_stage.as_deref())
            .map(|message| notice::localize_update_message(message, language))
            .or_else(|| download_status.as_deref().map(Cow::Borrowed))
            .unwrap_or_else(|| self.updates.status_in(language));
        text!(get_update_status, set_update_status, update_status);
        data.set_network_results(strings(data.get_network_results(), &self.test_results));
        self.present_overview(data);
        if !self.visible {
            models.clear();
            return;
        }
        let query = self.query.to_lowercase();
        if self.page == Page::Proxies {
            let (total, groups) = self.proxy_rows();
            models.set_groups(groups);
            set!(
                get_groups_paginated,
                set_groups_paginated,
                self.settings.proxy_groups_paginated
            );
            self.pager(
                data,
                total,
                if self.settings.proxy_groups_paginated {
                    GROUP_PAGE_SIZE
                } else {
                    total.max(1)
                },
            );
        } else {
            models.set_groups(Vec::new());
        }
        if self.page == Page::Profiles {
            let profiles: Vec<_> = self
                .profiles
                .iter()
                .filter(|profile| {
                    profile.matches_name(profile.display_name(language), &query)
                        || contains_query(&profile.source, &query)
                })
                .collect();
            let step = if self.settings.profiles_paginated {
                PAGE_SIZE
            } else {
                profiles.len().max(1)
            };
            set!(
                get_profiles_paginated,
                set_profiles_paginated,
                self.settings.profiles_paginated
            );
            self.pager(data, profiles.len(), step);
            reconcile(
                &models.profiles,
                profiles
                    .into_iter()
                    .skip(data.get_page_offset() as usize)
                    .take(step)
                    .map(|profile| ProfileItem {
                        id: profile.id.as_str().into(),
                        name: profile.display_name(language).into(),
                        source: profile.source.as_str().into(),
                        active: self.settings.active_profile.as_deref() == Some(&profile.id),
                        local: profile.is_local(),
                        builtin: profile.is_default(),
                        detail: if profile.is_local() {
                            shared_messages::subscription_local_configuration(language).into()
                        } else {
                            clash_client_core::subscription::summary(
                                profile.usage.as_deref(),
                                profile.updated,
                                language,
                            )
                            .into()
                        },
                    })
                    .collect(),
            );
        } else {
            reconcile(&models.profiles, Vec::new());
        }
        if matches!(self.page, Page::Home | Page::Connections) {
            let (total, rows) = self.connection_rows();
            if self.page == Page::Connections {
                self.pager(data, total, PAGE_SIZE);
            }
            reconcile(&models.connections, rows);
        } else {
            reconcile(&models.connections, Vec::new());
        }
        if self.page == Page::Rules {
            self.present_rules(data, models, &query);
        } else {
            reconcile(&models.rules, Vec::new());
        }
        data.set_log_level(self.log_level.map_or("全部等级", logs::Level::label).into());
        if self.page == Page::Logs {
            let logs: Vec<_> = self
                .snapshot
                .logs
                .iter()
                .rev()
                .filter_map(|line| {
                    let level = logs::Level::of(line);
                    let message = clash_client_core::runtime::logging::display(line, language);
                    (self.log_level.is_none_or(|selected| selected == level)
                        && (contains_query(line, &query) || contains_query(&message, &query)))
                    .then(|| (line, message.into_owned(), level))
                })
                .collect();
            self.pager(data, logs.len(), PAGE_SIZE);
            reconcile(
                &models.logs,
                logs.into_iter()
                    .skip(data.get_page_offset() as usize)
                    .take(PAGE_SIZE)
                    .map(|(line, message, level)| LogItem {
                        time: "".into(),
                        level: level.label().into(),
                        original: if line.as_ref() == message {
                            "".into()
                        } else {
                            line.as_ref().into()
                        },
                        message: message.into(),
                        tone: level.tone(),
                    })
                    .collect(),
            );
        } else {
            reconcile(&models.logs, Vec::new());
        }
        if self.page == Page::Websites {
            self.present_sites(data, models, &query);
        } else {
            reconcile(&models.categories, Vec::new());
            reconcile(&models.site_results, Vec::new());
        }
        if self.page == Page::Claude {
            self.present_claude(data, models);
        } else {
            reconcile(&models.claude_results, Vec::new());
        }
        self.present_dialog(data);
    }

    fn present_rules(&self, data: &UiData, models: &ViewModels, query: &str) {
        let language = self.presentation_language;
        let state = &self.rule_state;
        data.set_rule_dirty(state.dirty());
        data.set_rule_selected(state.selected.is_some());
        data.set_rule_custom(state.selected_custom().is_some());
        data.set_rule_source(state.filter.to_string().into());
        data.set_rule_warning(state.warning_text(language).into());
        data.set_rule_detail(
            state
                .selected_index()
                .map_or("".into(), |index| state.rows[index].raw.as_ref().into()),
        );
        data.set_rule_profiles(model(
            data.get_rule_profiles(),
            self.profiles
                .iter()
                .map(|profile| Choice {
                    id: profile.id.as_str().into(),
                    label: profile.display_name(language).into(),
                })
                .collect(),
        ));
        data.set_rule_profile_options(strings(
            data.get_rule_profile_options(),
            self.profiles
                .iter()
                .map(|profile| profile.display_name(language)),
        ));
        let selected_profile = self.profiles.iter().find(|profile| {
            state
                .document
                .as_ref()
                .is_some_and(|doc| doc.profile_id == profile.id)
        });
        data.set_rule_profile(
            selected_profile
                .map_or("", |profile| profile.display_name(language))
                .into(),
        );
        data.set_rule_profile_id(selected_profile.map_or("", |profile| &profile.id).into());
        data.set_rule_types(strings(
            data.get_rule_types(),
            if state.document.as_ref().is_some_and(|doc| !doc.builtin) {
                rules_view::TYPES.as_slice()
            } else {
                &[]
            },
        ));
        let document = state.document.as_deref();
        data.set_rule_policies(strings(
            data.get_rule_policies(),
            document.map_or(&[][..], |doc| doc.policies.as_slice()),
        ));
        let shortcuts: [_; clash_client_core::config::SUBSCRIPTION_RULES.len()] = [
            messages::shortcut_block_ads(language),
            messages::shortcut_private_domains(language),
            messages::shortcut_private_ips(language),
            messages::shortcut_china_domains(language),
            messages::shortcut_china_ips(language),
        ];
        data.set_shortcuts(strings(
            data.get_shortcuts(),
            shortcuts.into_iter().enumerate().map(|(index, label)| {
                if document.is_some_and(|doc| doc.included[index]) {
                    messages::shortcut_already_in_subscription(language, label)
                } else {
                    label.into()
                }
            }),
        ));
        data.set_shortcut_available(model(
            data.get_shortcut_available(),
            (0..shortcuts.len())
                .map(|index| document.is_some_and(|doc| !doc.builtin && !doc.included[index]))
                .collect(),
        ));
        data.set_shortcut_values(model(
            data.get_shortcut_values(),
            (0..shortcuts.len())
                .map(|index| {
                    document.is_some_and(|doc| doc.included[index] || state.draft.enabled[index])
                })
                .collect(),
        ));
        let rows: Vec<_> = state
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| state.filter.matches(row.source) && contains_query(&row.raw, query))
            .collect();
        self.pager(data, rows.len(), PAGE_SIZE);
        reconcile(
            &models.rules,
            rows.into_iter()
                .skip(data.get_page_offset() as usize)
                .take(PAGE_SIZE)
                .map(|(index, row)| {
                    let parts = rule_manager::parts(&row.raw).ok();
                    RuleItem {
                        id: row.id.as_str().into(),
                        position: (index + 1).to_string().into(),
                        kind: parts
                            .as_ref()
                            .map_or(messages::rule_complex(language), |parts| parts.kind)
                            .into(),
                        value: parts
                            .as_ref()
                            .map_or(row.raw.as_ref(), |parts| parts.payload)
                            .into(),
                        policy: parts.as_ref().map_or("—", |parts| parts.policy).into(),
                        source: row.source.to_string().into(),
                        selected: state.selected.as_deref() == Some(&row.id),
                        enabled: row.enabled,
                        raw: row.raw.as_ref().into(),
                    }
                })
                .collect(),
        );
    }

    fn present_sites(&self, data: &UiData, models: &ViewModels, query: &str) {
        let language = self.presentation_language;
        let report = ip_report::Report::from_results(&self.site_results);
        let total = report.totals.total();
        let completed = report.totals.completed();
        data.set_site_progress(messages::site_check_progress(language, completed, total).into());
        data.set_site_fraction(if total == 0 {
            0.0
        } else {
            completed as f32 / total as f32
        });
        data.set_sites_busy(self.sites_running());
        data.set_sites_started(self.site_has_run);
        data.set_sites_report(!self.site_show_results);
        data.set_site_category(self.site_category.to_string().into());
        data.set_site_status(self.site_status.to_string().into());
        data.set_site_region(self.site_region.to_string().into());
        data.set_site_category_options(strings(
            data.get_site_category_options(),
            std::iter::once("全部分类")
                .chain(report.categories.iter().map(|category| category.name)),
        ));
        data.set_site_status_options(strings(
            data.get_site_status_options(),
            std::iter::once("全部结果".to_owned())
                .chain(Status::ALL.into_iter().map(|status| status.to_string())),
        ));
        let regions: BTreeSet<_> = self
            .site_results
            .iter()
            .filter_map(|result| ip_report::region(result.as_ref()))
            .collect();
        let region_count = regions.len();
        data.set_site_region_options(strings(
            data.get_site_region_options(),
            ["全部识别地区".to_owned(), "未识别地区".to_owned()]
                .into_iter()
                .chain(
                    regions
                        .into_iter()
                        .map(|code| RegionFilter::Country(code).to_string()),
                ),
        ));
        data.set_site_counts(model(
            data.get_site_counts(),
            Status::ALL
                .into_iter()
                .map(|status| Choice {
                    id: status.to_string().into(),
                    label: format!(
                        "{} {}",
                        status.label_in(language),
                        report.totals.count(status)
                    )
                    .into(),
                })
                .collect(),
        ));
        let exit = self.site_result(0).and_then(|result| result.as_ref().ok());
        let summary = exit.map_or_else(
            || messages::site_check_empty(language).into(),
            |exit| messages::site_exit_summary(language, &exit.summary, region_count),
        );
        data.set_site_summary(summary.into());
        let (country, flag) =
            clash_of_rust::flags::label(exit.map_or("", |exit| exit.country.as_str()));
        data.set_site_exit_country(
            match language {
                Language::SimplifiedChinese => country,
                _ => clash_client_core::countries::country_code(country)
                    .and_then(clash_client_core::countries::english_name)
                    .unwrap_or(country),
            }
            .into(),
        );
        data.set_site_exit_flagged(flag.is_some());
        data.set_site_exit_flag(flag.unwrap_or_default().into());
        let categories: Vec<_> = report
            .categories
            .into_iter()
            .map(|category| {
                let counts = &category.counts;
                let has_failure =
                    counts.count(Status::Restricted) + counts.count(Status::Failed) > 0;
                CategoryItem {
                    id: category.name.into(),
                    title: category.name.into(),
                    count: counts.total().to_string().into(),
                    detail: counts.summary_in(language).into(),
                    tone: if has_failure { 2 } else { 1 },
                }
            })
            .collect();
        reconcile(
            &models.categories,
            categories
                .chunks(2)
                .map(|pair| CategoryPair {
                    left: pair[0].clone(),
                    right: pair.get(1).cloned().unwrap_or_default(),
                })
                .collect(),
        );
        let indices: Vec<_> = (0..ip_check::services().len())
            .filter(|index| self.site_matches(*index, query))
            .collect();
        self.pager(data, indices.len(), PAGE_SIZE);
        reconcile(
            &models.site_results,
            indices
                .into_iter()
                .skip(data.get_page_offset() as usize)
                .take(PAGE_SIZE)
                .map(|index| {
                    let service = &ip_check::services()[index];
                    let result = self.site_result(index);
                    let status = Status::of(result);
                    let measured = result.and_then(|result| result.as_ref().ok());
                    let country =
                        measured.map_or("", |result| site_result_country(result, language));
                    let flag = measured
                        .and_then(|result| clash_of_rust::flags::flag_emoji(&result.country));
                    ReportItem {
                        country: String::new().into(),
                        flag: flag.clone().unwrap_or_default().into(),
                        flagged: flag.is_some(),
                        id: service.id.as_str().into(),
                        title: service.name.as_str().into(),
                        value: site_result_value(result, status, language).as_ref().into(),
                        detail: ip_report::list_detail_in(service, result, language).into(),
                        source: country.into(),
                        status_label: status.to_string().into(),
                        tone: tone(status),
                        delay: result
                            .and_then(|result| result.as_ref().ok())
                            .and_then(ip_check::CheckResult::response_millis)
                            .map_or(String::new(), |millis| format!("{millis} ms"))
                            .into(),
                        delay_tone: result
                            .and_then(|result| result.as_ref().ok())
                            .and_then(ip_check::CheckResult::response_millis)
                            .map_or(4, proxy_order::response_tone),
                        block_items: 0,
                    }
                })
                .collect(),
        );
    }

    fn present_protection(&self, data: &UiData) {
        data.set_global_override_enabled(self.settings.global_override_enabled);
        data.set_protection_pending(self.claude.protection_pending.is_some());
        let health = if self.claude.protection_pending.is_some() {
            clash_client_core::protection::status::Status::observe(true, true, false, false, None)
        } else if self.settings.claude_protection.is_none() {
            Default::default()
        } else if !self.snapshot.running {
            clash_client_core::protection::status::Status::observe(true, false, false, false, None)
        } else {
            let mut status = self.snapshot.protection_status.clone();
            if status.state == clash_client_core::protection::status::State::Off {
                status = clash_client_core::protection::status::Status::observe(
                    true, false, true, true, None,
                );
            }
            status
        };
        data.set_protection_health(health.state as i32);
        data.set_protection_message(
            clash_client_core::error_message::localize(&health.message, self.presentation_language)
                .as_ref()
                .into(),
        );
    }

    fn present_claude(&self, data: &UiData, models: &ViewModels) {
        let language = self.presentation_language;
        let entries = self.claude.entries();
        let counts = self.claude.counts(&entries);
        data.set_claude_normal(counts[0].to_string().into());
        data.set_claude_risk(counts[1].to_string().into());
        data.set_claude_clues(counts[2].to_string().into());
        data.set_browser_session(self.claude.session.is_some());
        data.set_claude_busy(self.claude.busy());
        data.set_claude_status(self.claude.status.display(language).as_ref().into());
        data.set_claude_error(self.claude.error);
        data.set_claude_guard(self.claude.guard_status_in(language).into());
        data.set_protection_status(
            match self.claude.protection_pending {
                Some(true) => messages::claude_protection_enabling(language),
                Some(false) => messages::claude_protection_disabling(language),
                None => "",
            }
            .into(),
        );
        data.set_protection_node(
            self.settings
                .claude_protection
                .as_ref()
                .map_or("", |protection| protection.node.as_str())
                .into(),
        );
        data.set_claude_progress(
            (usize::from(!self.claude.local_busy)
                + usize::from(!self.claude.network_busy)
                + usize::from(!self.claude.audit_busy)
                + usize::from(!self.claude.browser_busy && !self.claude.session_starting))
                as f32
                / 4.0,
        );
        data.set_claude_audit(
            self.claude
                .audit
                .as_ref()
                .map_or(String::new(), |audit| {
                    let missing = if audit.missing_projects > 0 {
                        messages::claude_audit_missing(language, audit.missing_projects)
                    } else {
                        String::new()
                    };
                    let incomplete = if audit.incomplete {
                        messages::claude_audit_incomplete(language)
                    } else {
                        ""
                    };
                    messages::claude_audit_summary(
                        language,
                        audit.files,
                        incomplete,
                        missing,
                        audit.projects,
                    )
                })
                .into(),
        );
        data.set_claude_filter(self.claude.results_filter.map_or(-1, |index| index as i32));
        let mut entries: Vec<_> = entries
            .into_iter()
            .filter(|entry| {
                self.claude.results_filter.is_none_or(|index| {
                    entry.section() == clash_of_rust::claude_check::REPORT_SECTIONS[index]
                })
            })
            .collect();
        let summary = self.claude.results_filter.is_none();
        if summary {
            clash_of_rust::claude_check::order_summary(&mut entries, |entry| {
                let signal = match entry {
                    claude_view::Entry::Signal(_, signal) => Some(*signal),
                    _ => None,
                };
                clash_of_rust::claude_check::summary_order(
                    signal,
                    self.claude.kind(entry) == claude_view::ResultKind::Risk,
                )
            });
        }
        let page_size = if summary { 6 } else { 20 };
        let page = if summary {
            0
        } else {
            self.claude
                .results_page
                .min(entries.len().saturating_sub(1) / page_size)
        };
        data.set_claude_result_page(page as i32);
        data.set_claude_previous(page > 0);
        data.set_claude_next(!summary && (page + 1) * page_size < entries.len());
        let pages = entries.len().div_ceil(page_size).max(1);
        data.set_claude_result_status(
            messages::claude_result_page(language, entries.len(), page + 1, pages).into(),
        );
        reconcile(&models.claude_results, {
            let rows = entries
                .into_iter()
                .skip(if summary { 0 } else { page * page_size })
                .take(if summary { usize::MAX } else { page_size })
                .map(|entry| {
                    let kind = self.claude.kind(&entry);
                    let tone = match kind {
                        claude_view::ResultKind::Normal => 1,
                        claude_view::ResultKind::Risk => 3,
                        claude_view::ResultKind::Unknown => 2,
                    };
                    let section = entry.section();
                    let signal = match &entry {
                        claude_view::Entry::Signal(_, signal) => Some(*signal),
                        _ => None,
                    };
                    let priority = clash_of_rust::claude_check::summary_order(
                        signal,
                        kind == claude_view::ResultKind::Risk,
                    );
                    let (title, value) = entry.title_value(language);
                    clash_of_rust::claude_check::BlockRow {
                        section,
                        title,
                        value,
                        priority,
                        tone,
                        summary_hint: matches!(&entry, claude_view::Entry::Routes(_)),
                        item: (entry, tone),
                    }
                })
                .collect::<Vec<_>>();
            if summary {
                let mut headers = Vec::new();
                let mut items = Vec::new();
                for block in clash_of_rust::claude_check::report_blocks_in(rows, language) {
                    let total = block.rows.len();
                    headers.push(ReportItem {
                        id: block.section.into(),
                        title: block.section.into(),
                        value: block.overview.into(),
                        detail: block.attention.into(),
                        tone: 0,
                        delay_tone: block.tone,
                        block_items: total as i32,
                        ..Default::default()
                    });
                    items.extend(
                        block
                            .rows
                            .into_iter()
                            .take(12)
                            .map(|(entry, tone)| claude_report_item(entry, tone, language)),
                    );
                }
                headers.extend(items);
                headers
            } else {
                rows.into_iter()
                    .map(|row| claude_report_item(row.item.0, row.item.1, language))
                    .collect()
            }
        });
    }

    fn present_dialog(&self, data: &UiData) {
        let language = self.presentation_language;
        let rules_locked = self.page != Page::Rules
            || self.settings.claude_protection.is_some()
            || self.claude.protection_pending.is_some();
        data.set_modal_error("".into());
        data.set_modal_copy("".into());
        data.set_connection_selected_id("".into());
        data.set_modal_country("".into());
        data.set_modal_flagged(false);
        data.set_modal_flag(String::new().into());
        data.set_modal_description("".into());
        data.set_modal_accept("".into());
        data.set_modal_danger(false);
        data.set_modal_can_accept(!self.working && !self.exiting);
        if self.connection_detail.is_none() {
            data.set_connection_details(model(data.get_connection_details(), Vec::new()));
        }
        let kind;
        if let Some(confirmation) = &self.confirmation {
            kind = 1;
            data.set_modal_title(confirmation.title.as_str().into());
            data.set_modal_description(confirmation.description.as_str().into());
            let wait = confirmation.wait();
            data.set_modal_accept(if wait == 0 {
                confirmation.accept.into()
            } else {
                messages::confirmation_countdown(language, confirmation.accept, wait).into()
            });
            data.set_modal_can_accept(wait == 0 && !self.working && !self.exiting);
            data.set_modal_danger(!matches!(
                &confirmation.pending,
                desktop_view::Pending::NotificationSettings
                    | desktop_view::Pending::InstallUpdate(_)
            ));
        } else if let Some(editor) = &self.profile_editor {
            kind = 2;
            data.set_modal_title(messages::subscription_edit_title(language).into());
            data.set_edit_name(editor.name.as_str().into());
            data.set_edit_source(editor.source.as_str().into());
            data.set_modal_error(
                clash_client_core::error_message::localize(&editor.error, language)
                    .as_ref()
                    .into(),
            );
            data.set_modal_accept(messages::button_save(language).into());
        } else if self.import_open {
            kind = 3;
            data.set_modal_title(messages::subscription_add_title(language).into());
            data.set_modal_accept(messages::button_import(language).into());
            data.set_modal_can_accept(!self.working && !self.profile_source.trim().is_empty());
        } else if let Some(profile) = self
            .rule_state
            .pending_profile
            .as_ref()
            .filter(|_| !rules_locked)
        {
            kind = 6;
            let name = self
                .profiles
                .iter()
                .find(|candidate| candidate.id == profile.id)
                .map_or(profile.name.as_str(), |candidate| {
                    candidate.display_name(language)
                });
            data.set_modal_title(messages::rule_switch_configuration_title(language).into());
            data.set_modal_description(
                messages::rule_switch_configuration_detail(language, name).into(),
            );
            data.set_modal_accept(messages::rule_switch_configuration_confirm(language).into());
        } else if let Some(editor) = self.rule_state.editor.as_ref().filter(|_| !rules_locked) {
            kind = 4;
            data.set_modal_title(messages::rule_edit_title(language).into());
            data.set_modal_accept(messages::rule_save_draft(language).into());
            data.set_rule_text_mode(editor.text_mode);
            data.set_rule_raw(editor.raw.as_str().into());
            data.set_rule_kind(editor.kind.as_str().into());
            data.set_rule_payload(editor.payload.as_str().into());
            data.set_rule_policy(editor.policy.as_str().into());
            data.set_rule_position(editor.position.as_str().into());
            data.set_rule_enabled(editor.enabled);
            data.set_rule_no_resolve(editor.no_resolve);
            data.set_modal_error(
                clash_client_core::error_message::localize(&editor.error, language)
                    .as_ref()
                    .into(),
            );
        } else if self.rule_state.move_open && !rules_locked {
            kind = 5;
            data.set_modal_title(messages::rule_move_title(language).into());
            data.set_modal_accept(messages::rule_move_confirm(language).into());
            data.set_rule_move_position(self.rule_state.move_position.as_str().into());
        } else if self.page == Page::Rules
            && self.rule_state.detail_open
            && let Some(index) = self.rule_state.selected_index()
        {
            kind = 15;
            let rule = &self.rule_state.rows[index];
            let source = match rule.source {
                rule_manager::Source::Custom => messages::rule_source_custom(language),
                rule_manager::Source::Shortcut => messages::rule_source_shortcut(language),
                rule_manager::Source::Subscription => messages::rule_source_subscription(language),
            };
            let status = if rule.enabled {
                messages::rule_status_enabled(language)
            } else {
                messages::rule_status_disabled(language)
            };
            data.set_modal_title(messages::rule_detail_title(language).into());
            data.set_modal_description(
                messages::rule_detail_summary(language, source, status).into(),
            );
            data.set_modal_copy(rule.raw.as_ref().into());
            data.set_rule_enabled(rule.enabled);
        } else if let Some(guide) = self.claude.guide {
            match guide {
                claude_view::Guide::Protection => {
                    kind = 7;
                    let wait = self.claude.protection_wait();
                    data.set_modal_title(messages::claude_protection_title(language).into());
                    data.set_modal_description(messages::claude_protection_detail(language).into());
                    data.set_modal_accept(if wait == 0 {
                        messages::claude_protection_confirm(language).into()
                    } else {
                        messages::claude_protection_countdown(language, wait).into()
                    });
                    data.set_modal_can_accept(wait == 0 && !self.working && !self.claude.busy());
                }
                claude_view::Guide::Supplement => {
                    kind = 8;
                    data.set_modal_title(messages::claude_supplement_title(language).into());
                    data.set_modal_description(messages::claude_supplement_detail(language).into());
                }
            }
        } else if let Some(id) = &self.connection_detail {
            kind = 10;
            data.set_connection_selected_id(id.as_str().into());
            data.set_modal_title(messages::connection_details_title(language).into());
            let connection = self
                .snapshot
                .connections
                .connections
                .iter()
                .find(|connection| connection.id.as_ref() == id);
            let rows = connection.map_or_else(Vec::new, |connection| {
                let resolved = self.connection_lookup.addresses(connection);
                connection_details(connection, resolved, language)
            });
            data.set_modal_copy(
                rows.iter()
                    .map(|row| messages::connection_details_line(language, &row.label, &row.value))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into(),
            );
            data.set_connection_details(model(data.get_connection_details(), rows));
            if connection.is_none() {
                data.set_modal_description(messages::connection_details_ended(language).into());
            }
        } else if let Some(line) = &self.log_detail {
            kind = 13;
            data.set_modal_title(messages::log_details_title(language).into());
            let message = clash_client_core::runtime::logging::display(line, language);
            data.set_modal_description(message.as_ref().into());
            data.set_modal_copy(message.as_ref().into());
        } else if let Some(index) = self.site_detail {
            kind = 11;
            let service = &ip_check::services()[index];
            data.set_modal_title(service.name.as_str().into());
            data.set_modal_description(
                match self.site_result(index) {
                    Some(outcome @ Ok(result)) => {
                        let country = site_result_country(result, language);
                        let flag = clash_of_rust::flags::flag_emoji(&result.country);
                        data.set_modal_country(country.into());
                        data.set_modal_flagged(flag.is_some());
                        data.set_modal_flag(flag.unwrap_or_default().into());
                        let summary = if result.authentication_check_skipped {
                            Status::Unknown.label_in(language)
                        } else {
                            Status::of(Some(outcome)).summary_in(&result.summary, language)
                        };
                        let duration = ip_report::response_duration_in(result, language);
                        let detail = ip_report::detail_in(service, outcome, language);
                        if duration.is_empty() {
                            format!("{summary}\n\n{detail}")
                        } else {
                            format!("{summary}\n{duration}\n\n{detail}")
                        }
                    }
                    Some(outcome @ Err(_)) => {
                        ip_report::detail_in(service, outcome, language).into_owned()
                    }
                    None => messages::site_check_not_completed(language).into(),
                }
                .into(),
            );
            data.set_modal_copy(if data.get_modal_country().is_empty() {
                data.get_modal_description()
            } else {
                format!(
                    "{}\n{}",
                    data.get_modal_country(),
                    data.get_modal_description()
                )
                .into()
            });
        } else if self.accent_open {
            kind = 12;
            data.set_modal_title(messages::accent_color_title(language).into());
            data.set_modal_can_accept(!self.exiting);
            data.set_modal_accept(messages::button_done(language).into());
        } else {
            kind = 0;
        }
        data.set_modal(kind);
    }
}
