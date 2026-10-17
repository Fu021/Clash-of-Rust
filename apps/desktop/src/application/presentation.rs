//! Bounded projections from application state to the declarative UI contract.
use super::*;
use clash_client_ui::{
    models::{ViewModels, reconcile},
    *,
};
use slint::{Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;

fn model<T: Clone + PartialEq + 'static>(old: ModelRc<T>, values: Vec<T>) -> ModelRc<T> {
    if old.row_count() == values.len()
        && values
            .iter()
            .enumerate()
            .all(|(index, value)| old.row_data(index).as_ref() == Some(value))
    {
        old
    } else {
        Rc::new(VecModel::from(values)).into()
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

fn connection_details(
    connection: &clash_of_rust::api::Connection,
    resolved: &str,
) -> Vec<DetailItem> {
    connection::detail_values(connection, resolved)
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
        let query = self.query.to_lowercase();
        let groups: Vec<_> = self
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
        let total = groups.len();
        let offset = self
            .list_offset
            .min(total.saturating_sub(1) / GROUP_PAGE_SIZE * GROUP_PAGE_SIZE);
        let visible = &groups[offset..(offset + GROUP_PAGE_SIZE).min(total)];
        let mut orders = self.proxy_order.borrow_mut();
        orders.retain(|name, _| {
            visible.iter().any(|(group, _)| {
                *group == name && (self.expanded.contains(name.as_str()) || !query.is_empty())
            })
        });
        let mut rows = Vec::with_capacity(visible.len());
        for &(group, proxy) in visible {
            let expanded = self.expanded.contains(group.as_str()) || !query.is_empty();
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
                let node = |index: usize| {
                    let name = proxy.all[index].as_str();
                    NodeItem {
                        group: group.as_str().into(),
                        name: name.into(),
                        valid: true,
                        selected: proxy.now.as_str() == name,
                        kind: self
                            .snapshot
                            .proxies
                            .proxies
                            .get(name)
                            .map_or("", |node| node.kind.as_str())
                            .into(),
                        delay: proxy_order::delay_label(self.node_delay(name)).into(),
                        tone: proxy_order::delay_tone(self.node_delay(name)),
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
                    id: group.as_str().into(),
                    title: group.as_str().into(),
                    current: proxy.now.as_str().into(),
                    detail: format!("{} · {} 个节点", proxy.kind, proxy.all.len()).into(),
                    expanded,
                    testing: self.testing_groups.contains(group.as_str()),
                    page: if expanded {
                        format!(
                            "{count} 个节点 · {}–{}",
                            if count == 0 { 0 } else { start + 1 },
                            (start + PAGE_SIZE).min(count)
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
                    (query.is_empty()
                        || query.split_whitespace().all(|word| {
                            [
                                connection.metadata.host.as_ref(),
                                connection.metadata.sniff_host.as_str(),
                                connection.metadata.destination_ip.as_ref(),
                                connection.rule.as_str(),
                                connection.rule_payload.as_str(),
                            ]
                            .into_iter()
                            .chain(connection.chains.iter().map(|name| name.as_str()))
                            .any(|field| contains_query(field, word))
                        }))
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

    fn pager(&self, data: &UiData<'_>, total: usize, step: usize) {
        let offset = self.list_offset.min(total.saturating_sub(1) / step * step);
        data.set_page_offset(offset as i32);
        data.set_previous(offset > 0);
        data.set_next(offset + step < total);
        data.set_page_status(
            format!(
                "{total} 条 · {}–{}",
                if total == 0 { 0 } else { offset + 1 },
                (offset + step).min(total)
            )
            .into(),
        );
    }

    pub(super) fn present(&self, data: &UiData<'_>, models: &mut ViewModels) {
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
                set!($get, $put, SharedString::from(value));
            }};
        }
        text!(get_version, set_version, clash_of_rust::VERSION);
        set!(get_page, set_page, self.page as i32);
        set!(get_ready, set_ready, self.engine.is_some());
        set!(get_running, set_running, self.snapshot.running);
        set!(get_working, set_working, self.working);
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
        set!(
            get_dns_override,
            set_dns_override,
            self.settings.dns_override
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
            get_notice_revision,
            set_notice_revision,
            self.notice_revision as i32
        );
        set!(
            get_notification_busy,
            set_notification_busy,
            self.notification_busy
        );
        set!(
            get_notice_elapsed,
            set_notice_elapsed,
            self.notice_started
                .map_or(0.0, |started| started.elapsed().as_secs_f32())
        );
        text!(
            get_notice,
            set_notice,
            if self.notice_visible {
                self.notice.as_str()
            } else {
                ""
            }
        );
        text!(get_query, set_query, &self.query);
        text!(
            get_profile,
            set_profile,
            self.profiles
                .iter()
                .find(|profile| self.settings.active_profile.as_deref() == Some(&profile.id))
                .map_or("未选择订阅", |profile| profile.name.as_str())
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
            if self.snapshot.tun {
                1
            } else if self.snapshot.system_proxy {
                0
            } else {
                2
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
        text!(get_dns_host, set_dns_host, &self.dns_host);
        set!(get_probing, set_probing, self.probing);
        text!(
            get_controller_port,
            set_controller_port,
            &self.controller_port
        );
        text!(get_mixed_port, set_mixed_port, &self.mixed_port);
        text!(get_window_width, set_window_width, &self.window_width);
        text!(get_window_height, set_window_height, &self.window_height);
        text!(get_interval, set_interval, &self.interval_input);
        text!(
            get_geo_status,
            set_geo_status,
            format::geo_version(&self.geo_status)
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
                .map_or(0.0, |progress| if progress.total == 0 {
                    0.0
                } else {
                    (progress.received as f64 / progress.total as f64).min(1.0) as f32
                })
        );
        text!(
            get_update_status,
            set_update_status,
            if !update::SELF_UPDATE_ENABLED {
                update::FORK_UPDATE_NOTICE
            } else {
                self.updates
                    .failure
                    .as_deref()
                    .or(self.updates.outcome.as_deref())
                    .or(self.updates.install_stage.as_deref())
                    .unwrap_or(&self.updates.status)
            }
        );
        data.set_network_results(strings(data.get_network_results(), &self.test_results));
        self.present_wsl(data, models);
        if !self.visible {
            models.clear();
            return;
        }
        let query = self.query.to_lowercase();
        if self.page == Page::Proxies {
            let (total, groups) = self.proxy_rows();
            models.set_groups(groups);
            self.pager(data, total, GROUP_PAGE_SIZE);
        } else {
            models.set_groups(Vec::new());
        }
        if self.page == Page::Profiles {
            let profiles: Vec<_> = self.profiles.iter().collect();
            self.pager(data, profiles.len(), PAGE_SIZE);
            reconcile(
                &models.profiles,
                profiles
                    .into_iter()
                    .skip(data.get_page_offset() as usize)
                    .take(PAGE_SIZE)
                    .map(|profile| ProfileItem {
                        id: profile.id.as_str().into(),
                        name: profile.name.as_str().into(),
                        source: profile.source.as_str().into(),
                        active: self.settings.active_profile.as_deref() == Some(&profile.id),
                        local: profile.is_default(),
                        detail: format!(
                            "{} · 更新于 {}",
                            format::subscription_usage(profile.usage.as_deref()),
                            format::timestamp(profile.updated)
                        )
                        .into(),
                    })
                    .collect(),
            );
        } else {
            reconcile(&models.profiles, Vec::new());
        }
        if self.page == Page::Connections {
            let (total, rows) = self.connection_rows();
            self.pager(data, total, PAGE_SIZE);
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
                    (self.log_level.is_none_or(|selected| selected == level)
                        && contains_query(line, &query))
                    .then_some((line, level))
                })
                .collect();
            self.pager(data, logs.len(), PAGE_SIZE);
            reconcile(
                &models.logs,
                logs.into_iter()
                    .skip(data.get_page_offset() as usize)
                    .take(PAGE_SIZE)
                    .map(|(line, level)| LogItem {
                        time: "".into(),
                        level: level.label().into(),
                        message: line.as_ref().into(),
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

    fn present_rules(&self, data: &UiData<'_>, models: &ViewModels, query: &str) {
        let state = &self.rule_state;
        data.set_shortcuts_open(state.expanded);
        data.set_rule_dirty(state.dirty());
        data.set_rule_selected(state.selected.is_some());
        data.set_rule_custom(state.selected_custom().is_some());
        data.set_rule_source(state.filter.to_string().into());
        data.set_rule_warning(state.warnings.join("\n").into());
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
                    label: profile.name.as_str().into(),
                })
                .collect(),
        ));
        data.set_rule_profile_options(strings(
            data.get_rule_profile_options(),
            self.profiles.iter().map(|profile| &profile.name),
        ));
        data.set_rule_profile(
            self.profiles
                .iter()
                .find(|profile| {
                    state
                        .document
                        .as_ref()
                        .is_some_and(|doc| doc.profile_id == profile.id)
                })
                .map_or("选择订阅", |profile| profile.name.as_str())
                .into(),
        );
        data.set_rule_types(strings(
            data.get_rule_types(),
            if state.document.as_ref().is_some_and(|doc| !doc.builtin) {
                rules_view::TYPES.as_slice()
            } else {
                &[]
            },
        ));
        if let Some(doc) = &state.document {
            data.set_rule_policies(strings(data.get_rule_policies(), &doc.policies));
            data.set_shortcuts(strings(
                data.get_shortcuts(),
                rules_view::SHORTCUT_LABELS
                    .iter()
                    .enumerate()
                    .map(|(index, label)| {
                        if doc.included[index] {
                            format!("{label} · 订阅已包含")
                        } else {
                            (*label).into()
                        }
                    }),
            ));
            data.set_shortcut_available(model(
                data.get_shortcut_available(),
                (0..5)
                    .map(|index| !doc.builtin && !doc.included[index])
                    .collect(),
            ));
            data.set_shortcut_values(model(
                data.get_shortcut_values(),
                (0..5)
                    .map(|index| doc.included[index] || state.draft.enabled[index])
                    .collect(),
            ));
        }
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
                        kind: parts.as_ref().map_or("复杂规则", |parts| parts.kind).into(),
                        value: parts
                            .as_ref()
                            .map_or(row.raw.as_ref(), |parts| parts.payload)
                            .into(),
                        policy: parts.as_ref().map_or("—", |parts| parts.policy).into(),
                        source: row.source.to_string().into(),
                        selected: state.selected.as_deref() == Some(&row.id),
                        enabled: row.enabled,
                    }
                })
                .collect(),
        );
    }

    fn present_sites(&self, data: &UiData<'_>, models: &ViewModels, query: &str) {
        let report = ip_report::Report::from_results(&self.site_results);
        let total = report.totals.total();
        let completed = report.totals.completed();
        data.set_site_progress(format!("完成 {completed} / {total}").into());
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
                    label: format!("{} {}", status, report.totals.count(status)).into(),
                })
                .collect(),
        ));
        let exit = self.site_result(0).and_then(|result| result.as_ref().ok());
        data.set_site_summary(
            exit.map_or_else(
                || "完成检测后显示出口与各类平台结果".into(),
                |exit| {
                    format!(
                        "出口 IP：{} · 各平台识别到 {} 个地区",
                        exit.summary, region_count
                    )
                },
            )
            .into(),
        );
        let (country, flag) =
            clash_of_rust::flags::label(exit.map_or("", |exit| exit.country.as_str()));
        data.set_site_exit_country(country.into());
        data.set_site_exit_flagged(flag.is_some());
        data.set_site_exit_flag(flag.unwrap_or_default());
        let categories: Vec<_> = report
            .categories
            .into_iter()
            .map(|category| CategoryItem {
                id: category.name.into(),
                title: category.name.into(),
                count: format!(
                    "{} / {}",
                    category.counts.non_red(),
                    category.counts.total()
                )
                .into(),
                detail: format!(
                    "确认可用 {} · 网页可达 {} · 受限 {}",
                    category.counts.count(Status::Available),
                    category.counts.count(Status::Reachable),
                    category.counts.count(Status::Restricted)
                )
                .into(),
                tone: if category.counts.non_red() == category.counts.total() {
                    1
                } else {
                    2
                },
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
                    let (country, flag) = result
                        .and_then(|result| result.as_ref().ok())
                        .map_or(("", None), |result| {
                            clash_of_rust::flags::label(&result.country)
                        });
                    ReportItem {
                        flag: flag.clone().unwrap_or_default(),
                        flagged: flag.is_some(),
                        id: service.id.as_str().into(),
                        title: service.name.as_str().into(),
                        value: match result {
                            Some(Ok(result)) => format!("{} · {}", status, result.summary),
                            Some(Err(error)) => error.clone(),
                            None => status.to_string(),
                        }
                        .into(),
                        detail: result
                            .and_then(|result| result.as_ref().ok())
                            .map_or("", |result| result.detail.as_str())
                            .into(),
                        source: result
                            .and_then(|result| result.as_ref().ok())
                            .map_or("".into(), |result| {
                                format!("{} · {} ms", country, result.millis)
                            })
                            .into(),
                        tone: tone(status),
                    }
                })
                .collect(),
        );
    }

    fn present_claude(&self, data: &UiData<'_>, models: &ViewModels) {
        let entries = self.claude.entries();
        let counts = self.claude.counts(&entries);
        data.set_claude_normal(counts[0].to_string().into());
        data.set_claude_risk(counts[1].to_string().into());
        data.set_claude_clues(counts[2].to_string().into());
        data.set_browser_session(self.claude.session.is_some());
        data.set_claude_busy(self.claude.busy());
        data.set_claude_status(self.claude.status.as_str().into());
        data.set_claude_error(self.claude.error);
        data.set_claude_guard(self.claude.guard_status.as_str().into());
        data.set_protection_pending(self.claude.protection_pending.is_some());
        data.set_protection_status(
            match self.claude.protection_pending {
                Some(true) => "正在开启保护，切换 TUN、规则与 DNS…",
                Some(false) => "正在关闭保护，恢复原网络状态…",
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
                    format!(
                        "已检查 {} 个已记录项目 · {} 份配置{}{}",
                        audit.projects,
                        audit.files,
                        if audit.missing_projects > 0 {
                            format!(" · {} 个目录已不存在", audit.missing_projects)
                        } else {
                            String::new()
                        },
                        if audit.incomplete {
                            " · 扫描未完成，详见风险与其他线索"
                        } else {
                            ""
                        }
                    )
                })
                .into(),
        );
        data.set_claude_filter(self.claude.results_filter.map_or(-1, |kind| kind as i32));
        let entries: Vec<_> = entries
            .into_iter()
            .filter(|entry| {
                self.claude
                    .results_filter
                    .is_none_or(|kind| self.claude.kind(entry) == kind)
            })
            .collect();
        let page = self
            .claude
            .results_page
            .min(entries.len().saturating_sub(1) / 20);
        data.set_claude_result_page(page as i32);
        data.set_claude_previous(page > 0);
        data.set_claude_next((page + 1) * 20 < entries.len());
        data.set_claude_result_status(
            format!(
                "列出 {} 项 · 第 {} / {} 页",
                entries.len(),
                page + 1,
                entries.len().div_ceil(20).max(1)
            )
            .into(),
        );
        reconcile(
            &models.claude_results,
            entries
                .into_iter()
                .skip(page * 20)
                .take(20)
                .map(|entry| {
                    let kind = self.claude.kind(&entry);
                    let tone = match kind {
                        claude_view::ResultKind::Normal => 1,
                        claude_view::ResultKind::Risk => 3,
                        claude_view::ResultKind::Unknown => 2,
                    };
                    let (title, value, detail, source) = match entry {
                        claude_view::Entry::Config(finding) => (
                            finding.field.clone(),
                            finding.value.clone(),
                            format!("{}\n{}", finding.reason, finding.advice),
                            finding.path.clone(),
                        ),
                        claude_view::Entry::Clean(path) => (
                            "项目配置".into(),
                            "未发现额外代理或地区覆盖".into(),
                            String::new(),
                            path.into(),
                        ),
                        claude_view::Entry::Signal(_, signal) => (
                            signal.title.clone(),
                            signal.value.clone(),
                            format!("{}\n{}", signal.detail, signal.advice),
                            signal.source.clone(),
                        ),
                        claude_view::Entry::Route(route) => (
                            route.host.clone(),
                            format!("{} · {}", route.outcome, route.chain),
                            format!("规则：{}", route.rule),
                            route.time.clone(),
                        ),
                    };
                    ReportItem {
                        flag: slint::Image::default(),
                        flagged: false,
                        id: "".into(),
                        title: title.into(),
                        value: value.into(),
                        detail: detail.into(),
                        source: source.into(),
                        tone,
                    }
                })
                .collect(),
        );
    }

    fn present_dialog(&self, data: &UiData<'_>) {
        data.set_modal_error("".into());
        data.set_modal_copy("".into());
        data.set_modal_country("".into());
        data.set_modal_flagged(false);
        data.set_modal_flag(slint::Image::default());
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
            data.set_modal_accept(confirmation.accept.into());
            data.set_modal_danger(true);
        } else if let Some(editor) = &self.profile_editor {
            kind = 2;
            data.set_modal_title("编辑订阅".into());
            data.set_edit_name(editor.name.as_str().into());
            data.set_edit_source(editor.source.as_str().into());
            data.set_modal_error(editor.error.as_str().into());
            data.set_modal_accept("保存".into());
        } else if self.import_open {
            kind = 3;
            data.set_modal_title("添加订阅".into());
            data.set_modal_accept("导入".into());
            data.set_modal_can_accept(!self.working && !self.profile_source.trim().is_empty());
        } else if let Some(profile) = &self.rule_state.pending_profile {
            kind = 6;
            data.set_modal_title("切换规则配置？".into());
            data.set_modal_description(
                format!("当前未应用的草稿将丢弃，切换到“{}”。", profile.name).into(),
            );
            data.set_modal_accept("放弃并切换".into());
        } else if let Some(editor) = &self.rule_state.editor {
            kind = 4;
            data.set_modal_title("编辑规则".into());
            data.set_modal_accept("保存到草稿".into());
            data.set_rule_text_mode(editor.text_mode);
            data.set_rule_raw(editor.raw.as_str().into());
            data.set_rule_kind(editor.kind.as_str().into());
            data.set_rule_payload(editor.payload.as_str().into());
            data.set_rule_policy(editor.policy.as_str().into());
            data.set_rule_position(editor.position.as_str().into());
            data.set_rule_enabled(editor.enabled);
            data.set_rule_no_resolve(editor.no_resolve);
            data.set_modal_error(editor.error.as_str().into());
        } else if self.rule_state.move_open {
            kind = 5;
            data.set_modal_title("移动到指定位置".into());
            data.set_modal_accept("移动".into());
            data.set_rule_move_position(self.rule_state.move_position.as_str().into());
        } else if let Some(guide) = self.claude.guide {
            match guide {
                claude_view::Guide::Protection => {
                    kind = 7;
                    let wait = self.claude.protection_wait();
                    data.set_modal_title("开启 Claude 保护？".into());
                    data.set_modal_description("将使用当前具体节点，启用 TUN、严格路由、IPv6 接管、UDP、广泛规则、加密 DNS 与实时异常连接拦截。\n\n这些设置可能影响其他应用、国内网站与内网服务；节点故障时相关解析也可能失败。节点域名沿用原节点 DNS 在本地解析；公网域名通过保护节点使用加密 DNS，局域网名称保留本地解析。\n\n原代理模式、DNS、UDP、规则和节点选择会保存；重启后仍保留，关闭保护后恢复。\n\n不能保证消除所有地区线索或满足服务的账号资格要求。".into());
                    data.set_modal_accept(if wait == 0 {
                        "开启并重新检测".into()
                    } else {
                        format!("请阅读（{wait} 秒）").into()
                    });
                    data.set_modal_can_accept(wait == 0 && !self.working && !self.claude.busy());
                }
                claude_view::Guide::Supplement => {
                    kind = 8;
                    data.set_modal_title("补充采样".into());
                    data.set_modal_description("自动浏览器检测来自本机检测页面。需要核对 Claude 网页本身的设置时，可在该网页的开发者工具 Console 运行采样脚本，再导入结果。".into());
                }
                claude_view::Guide::Diagnostics => {
                    kind = 9;
                    data.set_modal_title("检测选项".into());
                    data.set_reputation_key(self.claude.reputation_key.as_str().into());
                    data.set_modal_description("完整的机房、VPN、代理、Tor 与滥用标签需要 ipapi.is 的个人 API 密钥。留空时相关项目显示未确认。\n\n密钥只保存在本机，以 HTTPS 提交检测服务，不交给浏览器或 Claude、不写入报告。\n\n补充检测手动触发：全球探针最多使用 20 个位置；小流量测速下载 10 MiB、上传 2 MiB。".into());
                }
            }
        } else if let Some(id) = &self.connection_detail {
            kind = 10;
            data.set_modal_title("连接详情".into());
            let connection = self
                .snapshot
                .connections
                .connections
                .iter()
                .find(|connection| connection.id.as_ref() == id);
            let rows = connection.map_or_else(Vec::new, |connection| {
                let resolved = self.connection_lookup.addresses(connection);
                connection_details(connection, resolved)
            });
            data.set_modal_copy(
                rows.iter()
                    .map(|row| format!("{}：{}", row.label, row.value))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into(),
            );
            data.set_connection_details(model(data.get_connection_details(), rows));
            if connection.is_none() {
                data.set_modal_description("此连接已结束，详情不再保留。".into());
            }
        } else if let Some(line) = &self.log_detail {
            kind = 13;
            data.set_modal_title("日志详情".into());
            data.set_modal_description(line.as_str().into());
            data.set_modal_copy(line.as_str().into());
        } else if let Some(index) = self.site_detail {
            kind = 11;
            let service = &ip_check::services()[index];
            data.set_modal_title(service.name.as_str().into());
            data.set_modal_description(
                match self.site_result(index) {
                    Some(Ok(result)) => {
                        let (country, flag) = clash_of_rust::flags::label(&result.country);
                        data.set_modal_country(country.into());
                        data.set_modal_flagged(flag.is_some());
                        data.set_modal_flag(flag.unwrap_or_default());
                        format!(
                            "{}\n{} ms\n\n{}",
                            result.summary, result.millis, result.detail
                        )
                    }
                    Some(Err(error)) => error.clone(),
                    None => "尚未完成检测".into(),
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
            data.set_modal_title("强调色".into());
            data.set_modal_can_accept(!self.exiting);
            data.set_modal_accept("完成".into());
        } else {
            kind = 0;
        }
        data.set_modal(kind);
    }
}
