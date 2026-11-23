use clash_mobile_runtime::core::{
    api::Connection,
    config::{Language, Profile},
    connection::{Field, Sort, matches_query},
    error_message, rule_manager,
};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LogMessage {
    Raw(String),
    ActionFailed(String),
    NodesUnreachable,
    StatusRefreshFailed(String),
    SubscriptionUpdateFailed,
    SubscriptionDownloadFailed(String),
    SubscriptionPrepareFailed,
}

impl From<String> for LogMessage {
    fn from(value: String) -> Self {
        Self::Raw(value)
    }
}

impl From<&str> for LogMessage {
    fn from(value: &str) -> Self {
        Self::Raw(value.into())
    }
}

impl LogMessage {
    pub(crate) fn display(&self, language: Language) -> String {
        match self {
            Self::Raw(value) => value.clone(),
            Self::ActionFailed(detail) => error_message::localize(detail, language).into_owned(),
            Self::NodesUnreachable => language
                .text(
                    "全部节点经两个独立目标复测仍不可达，VPN 保持连接",
                    "All nodes remain unreachable after two independent checks; VPN stays connected",
                )
                .into(),
            Self::StatusRefreshFailed(detail) => format!(
                "{}{}",
                language.text("状态刷新失败：", "Status refresh failed: "),
                error_message::localize(detail, language)
            ),
            Self::SubscriptionUpdateFailed => language
                .text(
                    "订阅定时更新失败，已保留原配置与运行状态",
                    "Scheduled subscription update failed; configuration and connection preserved",
                )
                .into(),
            Self::SubscriptionDownloadFailed(detail) => format!(
                "{}{}",
                language.text(
                    "订阅定时下载失败，已保留原配置与运行状态：",
                    "Scheduled subscription download failed; configuration and connection preserved: "
                ),
                error_message::localize(detail, language)
            ),
            Self::SubscriptionPrepareFailed => language
                .text(
                    "订阅定时更新失败，已保留原配置",
                    "Scheduled subscription update failed; configuration preserved",
                )
                .into(),
        }
    }
}

pub(crate) struct Page<T> {
    pub items: Vec<T>,
    pub index: usize,
    pub total: usize,
}

pub(crate) const CONNECTION_PAGE_SIZE: usize = 50;

#[derive(Clone, Copy, Default)]
pub(crate) struct ConnectionPageCursor {
    index: usize,
    revision: u64,
}

#[derive(Default)]
pub(crate) struct ConnectionPager {
    cursor: ConnectionPageCursor,
}

impl ConnectionPager {
    pub(crate) fn reset(&mut self) {
        self.cursor.index = 0;
        self.cursor.revision = self.cursor.revision.wrapping_add(1);
    }

    pub(crate) fn turn(&mut self, displayed: ConnectionPageCursor, direction: isize) {
        if displayed.revision == self.cursor.revision {
            self.cursor.index = displayed.index.saturating_add_signed(direction);
        }
    }

    pub(crate) fn cursor(&self) -> ConnectionPageCursor {
        self.cursor
    }

    pub(crate) fn project<'a>(
        &mut self,
        connections: &'a [Connection],
        lower_query: &str,
        sort: Sort,
    ) -> Page<&'a Connection> {
        let mut matches: Vec<_> = connections
            .iter()
            .filter(|row| matches_query(row, lower_query))
            .collect();
        matches.sort_by(|left, right| sort.compare(left, right));
        let page = page(matches, self.cursor.index, CONNECTION_PAGE_SIZE, true);
        self.cursor.index = page.index;
        page
    }
}

pub(crate) fn contains_query(value: &str, lower_query: &str) -> bool {
    if lower_query.is_empty() {
        return true;
    }
    if value.is_ascii() && lower_query.is_ascii() {
        value
            .as_bytes()
            .windows(lower_query.len())
            .any(|part| part.eq_ignore_ascii_case(lower_query.as_bytes()))
    } else {
        value.to_lowercase().contains(lower_query)
    }
}

pub(crate) fn group_matches<'a>(
    name: &str,
    mut nodes: impl Iterator<Item = &'a str>,
    lower_query: &str,
) -> bool {
    contains_query(name, lower_query) || nodes.any(|node| contains_query(node, lower_query))
}

pub(crate) fn group_expanded(
    name: &str,
    active: Option<&str>,
    query: &str,
    index: usize,
    overrides: &BTreeMap<String, bool>,
) -> bool {
    overrides
        .get(name)
        .copied()
        .unwrap_or_else(|| active == Some(name) || (!query.is_empty() && index < 4))
}

pub(crate) fn connection_sort(value: &str) -> Sort {
    let (label, direction) = value.rsplit_once(' ').unwrap_or((value, ""));
    let field = match label {
        "默认顺序" => None,
        "名称" => Some(Field::Name),
        "规则" => Some(Field::Rule),
        "代理链" => Some(Field::Chain),
        "上传速度" => Some(Field::UploadRate),
        "上传量" => Some(Field::UploadTotal),
        "下载量" => Some(Field::DownloadTotal),
        "开始时间" => Some(Field::Start),
        _ => Some(Field::DownloadRate),
    };
    Sort {
        field,
        descending: matches!(direction, "↓" | "Z–A"),
    }
}

pub(crate) fn page<T>(items: Vec<T>, requested: usize, size: usize, paginated: bool) -> Page<T> {
    let total = items.len();
    let size = size.max(1);
    let index = if paginated {
        requested.min(total.saturating_sub(1) / size)
    } else {
        0
    };
    let items = if paginated {
        items.into_iter().skip(index * size).take(size).collect()
    } else {
        items
    };
    Page {
        items,
        index,
        total,
    }
}

pub(crate) fn profiles<'a>(
    profiles: &'a [Profile],
    lower_query: &str,
    requested: usize,
    paginated: bool,
) -> Page<&'a Profile> {
    let matches = profiles
        .iter()
        .filter(|profile| {
            contains_query(&profile.name, lower_query)
                || contains_query(&profile.source, lower_query)
        })
        .collect();
    page(matches, requested, 60, paginated)
}

pub(crate) fn logs(
    entries: &VecDeque<(String, LogMessage)>,
    level: &str,
    lower_query: &str,
    language: Language,
) -> Vec<(String, String)> {
    let other_language = match language {
        Language::SimplifiedChinese => Language::English,
        Language::English => Language::SimplifiedChinese,
    };
    entries
        .iter()
        .rev()
        .filter_map(|(entry_level, message)| {
            if level != "全部等级" && entry_level != level {
                return None;
            }
            let line = message.display(language);
            (contains_query(&line, lower_query)
                || (!matches!(message, LogMessage::Raw(_))
                    && contains_query(&message.display(other_language), lower_query)))
            .then(|| (entry_level.clone(), line))
        })
        .collect()
}

pub(crate) fn rule_warning(warnings: &[String], language: Language) -> String {
    warnings
        .iter()
        .map(|warning| rule_manager::warning_in(warning, language))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_mobile_runtime::core::api::{Connection, ConnectionRates, Metadata};

    #[test]
    fn rule_warning_projection_uses_the_selected_language_and_preserves_raw_warnings() {
        let warnings = [
            "MATCH 之后有 2 条规则不会被匹配，请检查顺序。".to_owned(),
            "provider warning: 東京".to_owned(),
        ];
        let original = warnings.clone();
        assert_eq!(
            rule_warning(&warnings, Language::SimplifiedChinese),
            "MATCH 之后有 2 条规则不会被匹配，请检查顺序。\nprovider warning: 東京",
        );
        assert_eq!(
            rule_warning(&warnings, Language::English),
            "Rules after MATCH will never be evaluated (2 affected). Check the rule order.\nprovider warning: 東京",
        );
        assert_eq!(warnings, original);
    }

    #[test]
    fn searches_ascii_and_unicode_without_changing_the_source() {
        assert!(contains_query("HTTPS://Example.COM/订阅", "example"));
        assert!(contains_query("École 香港", "école"));
        assert!(contains_query("École 香港", "香港"));
        assert!(contains_query("", ""));
        assert!(!contains_query("short", "longer query"));
    }

    #[test]
    fn group_search_finds_group_names_and_nodes_without_empty_matches() {
        assert!(group_matches(
            "Automatic",
            ["Alpha", "Tokyo"].into_iter(),
            "automatic"
        ));
        assert!(group_matches(
            "Automatic",
            ["Alpha", "Tokyo"].into_iter(),
            "tokyo"
        ));
        assert!(!group_matches(
            "Automatic",
            ["Alpha", "Tokyo"].into_iter(),
            "paris"
        ));
    }

    #[test]
    fn group_expansion_follows_group_identity_after_filtering_and_pagination() {
        let mut overrides = BTreeMap::new();
        assert!(group_expanded("active", Some("active"), "", 0, &overrides));
        assert!(group_expanded("match", None, "query", 0, &overrides));
        assert!(!group_expanded("later", None, "query", 4, &overrides));
        overrides.insert("active".into(), false);
        overrides.insert("later".into(), true);
        assert!(!group_expanded(
            "active",
            Some("active"),
            "query",
            0,
            &overrides
        ));
        assert!(group_expanded("later", None, "", 7, &overrides));
        assert!(!group_expanded("other", None, "", 0, &overrides));
    }

    #[test]
    fn pages_filtered_items_and_clamps_after_deletion() {
        let items: Vec<_> = (0..125).filter(|item| item % 2 == 0).collect();
        let result = page(items.clone(), 1, 60, true);
        assert_eq!((result.index, result.total), (1, 63));
        assert_eq!(result.items, [120, 122, 124]);

        let result = page(items[..12].to_vec(), usize::MAX, 60, true);
        assert_eq!((result.index, result.total), (0, 12));
        assert_eq!(result.items, items[..12]);

        let result = page(items.clone(), 1, 60, false);
        assert_eq!((result.index, result.total), (0, 63));
        assert_eq!(result.items, items);

        let result: Page<usize> = page(Vec::new(), 1, 60, true);
        assert_eq!((result.index, result.total), (0, 0));
        assert!(result.items.is_empty());
    }

    #[test]
    fn subscription_search_matches_names_and_sources_before_paging() {
        let profiles = (0..122)
            .map(|index| Profile {
                id: format!("{index}"),
                name: if index == 121 {
                    "École".into()
                } else {
                    format!("Subscription {index}")
                },
                source: if index % 2 == 0 {
                    "https://example.invalid/one".into()
                } else {
                    "https://other.invalid/two".into()
                },
                updated: 0,
                usage: None,
                builtin: false,
            })
            .collect::<Vec<_>>();
        let by_source = super::profiles(&profiles, "example", 1, true);
        assert_eq!((by_source.index, by_source.total), (1, 61));
        assert_eq!(by_source.items[0].id, "120");
        let by_name = super::profiles(&profiles, "école", 0, false);
        assert_eq!((by_name.total, by_name.items[0].id.as_str()), (1, "121"));
        let missing = super::profiles(&profiles, "missing", 8, true);
        assert_eq!((missing.index, missing.total), (0, 0));
        assert!(missing.items.is_empty());
    }

    #[test]
    fn logs_show_newest_matches_first_without_changing_the_saved_entries() {
        let entries = VecDeque::from([
            ("INFO".into(), "first example".into()),
            ("WARN".into(), "unrelated".into()),
            ("WARN".into(), "Latest EXAMPLE".into()),
        ]);
        assert_eq!(
            logs(&entries, "全部等级", "example", Language::English),
            [
                ("WARN".into(), "Latest EXAMPLE".into()),
                ("INFO".into(), "first example".into()),
            ]
        );
        assert_eq!(
            logs(&entries, "WARN", "", Language::English),
            [
                ("WARN".into(), "Latest EXAMPLE".into()),
                ("WARN".into(), "unrelated".into()),
            ]
        );
        assert_eq!(
            entries.front().unwrap().1,
            LogMessage::Raw("first example".into())
        );
    }

    #[test]
    fn app_logs_follow_language_and_leave_external_messages_unchanged() {
        let entries = VecDeque::from([
            ("WARN".into(), LogMessage::NodesUnreachable),
            (
                "WARN".into(),
                LogMessage::SubscriptionDownloadFailed("provider 原始错误".into()),
            ),
            (
                "WARN".into(),
                LogMessage::Raw("订阅定时更新失败，已保留原配置".into()),
            ),
        ]);
        let original = entries.clone();
        let english = logs(&entries, "全部等级", "", Language::English);
        assert_eq!(english[0].1, "订阅定时更新失败，已保留原配置");
        assert_eq!(
            english[1].1,
            "Scheduled subscription download failed; configuration and connection preserved: provider 原始错误"
        );
        assert!(english[2].1.starts_with("All nodes remain unreachable"));
        let chinese = logs(&entries, "WARN", "订阅", Language::SimplifiedChinese);
        assert_eq!(chinese.len(), 2);
        assert!(chinese[0].1.starts_with("订阅定时更新失败"));
        assert!(chinese[1].1.ends_with("provider 原始错误"));
        assert_eq!(entries, original);
        for (message, chinese, english) in [
            (
                LogMessage::ActionFailed("YAML 配置格式错误: provider 原始错误".into()),
                "YAML 配置格式错误: provider 原始错误",
                "Invalid YAML configuration: provider 原始错误",
            ),
            (
                LogMessage::StatusRefreshFailed("timeout".into()),
                "状态刷新失败：timeout",
                "Status refresh failed: timeout",
            ),
            (
                LogMessage::StatusRefreshFailed("控制接口刷新超时".into()),
                "状态刷新失败：控制接口刷新超时",
                "Status refresh failed: Control interface refresh timed out",
            ),
            (
                LogMessage::ActionFailed("内核启动失败：控制接口刷新超时".into()),
                "内核启动失败：控制接口刷新超时",
                "Core startup failed: Control interface refresh timed out",
            ),
            (
                LogMessage::ActionFailed("保护状态异常：保护状态不可用: native error 原文".into()),
                "保护状态异常：保护状态不可用: native error 原文",
                "Protection status error: Protection status unavailable: native error 原文",
            ),
            (
                LogMessage::ActionFailed("保护开启失败：确认保护节点超时".into()),
                "保护开启失败：确认保护节点超时",
                "Failed to enable protection: Timed out checking the protected node",
            ),
            (
                LogMessage::ActionFailed("保护回滚失败：VPN 授权等待超时\nprovider: 東京".into()),
                "保护回滚失败：VPN 授权等待超时\nprovider: 東京",
                "Protection rollback failed: VPN authorization timed out\nprovider: 東京",
            ),
            (
                LogMessage::SubscriptionDownloadFailed(
                    "订阅格式不兼容，请选择 Mihomo/Clash Meta YAML 订阅".into(),
                ),
                "订阅定时下载失败，已保留原配置与运行状态：订阅格式不兼容，请选择 Mihomo/Clash Meta YAML 订阅",
                "Scheduled subscription download failed; configuration and connection preserved: Incompatible subscription; use a Mihomo/Clash Meta YAML subscription",
            ),
            (
                LogMessage::SubscriptionUpdateFailed,
                "订阅定时更新失败，已保留原配置与运行状态",
                "Scheduled subscription update failed; configuration and connection preserved",
            ),
            (
                LogMessage::SubscriptionPrepareFailed,
                "订阅定时更新失败，已保留原配置",
                "Scheduled subscription update failed; configuration preserved",
            ),
        ] {
            assert_eq!(message.display(Language::SimplifiedChinese), chinese);
            assert_eq!(message.display(Language::English), english);
        }
    }

    #[test]
    fn log_search_matches_original_and_translated_text_after_language_switches() {
        let detail = "订阅格式不兼容，请选择 Mihomo/Clash Meta YAML 订阅";
        let raw = "[observed_at=12:00] [客户端] 配置已导入/更新";
        let chinese = format!("订阅定时下载失败，已保留原配置与运行状态：{detail}");
        let english = "Scheduled subscription download failed; configuration and connection preserved: Incompatible subscription; use a Mihomo/Clash Meta YAML subscription";
        let entries = VecDeque::from([
            (
                "WARN".into(),
                LogMessage::SubscriptionDownloadFailed(detail.into()),
            ),
            ("INFO".into(), LogMessage::Raw(raw.into())),
        ]);
        let original = entries.clone();
        for (language, displayed) in [
            (Language::SimplifiedChinese, chinese.as_str()),
            (Language::English, english),
            (Language::SimplifiedChinese, chinese.as_str()),
        ] {
            for query in [
                "订阅定时下载失败",
                "scheduled subscription download failed",
                "订阅格式不兼容",
                "incompatible subscription",
            ] {
                assert_eq!(
                    logs(&entries, "全部等级", query, language),
                    [("WARN".into(), displayed.into())]
                );
            }
            assert!(logs(&entries, "INFO", "incompatible subscription", language).is_empty());
            assert_eq!(
                logs(&entries, "全部等级", "配置已导入/更新", language),
                [("INFO".into(), raw.into())]
            );
            assert!(logs(&entries, "全部等级", "configuration imported", language).is_empty());
        }
        assert_eq!(entries, original);
    }

    #[test]
    fn connection_sort_options_order_actual_fields_in_both_directions() {
        let earlier = Connection {
            id: "earlier".into(),
            metadata: Metadata {
                host: "z.example".into(),
                ..Default::default()
            },
            upload: 50,
            download: 100,
            rates: Some(ConnectionRates {
                upload: 1,
                download: 10,
            }),
            chains: vec!["DIRECT".into()],
            rule: "DOMAIN".into(),
            start: "2026-11-20T10:00:00Z".into(),
            ..Default::default()
        };
        let later = Connection {
            id: "later".into(),
            metadata: Metadata {
                host: "a.example".into(),
                ..Default::default()
            },
            upload: 100,
            download: 200,
            rates: Some(ConnectionRates {
                upload: 2,
                download: 20,
            }),
            chains: vec!["Proxy".into()],
            rule: "MATCH".into(),
            start: "2026-11-21T10:00:00Z".into(),
            ..Default::default()
        };
        for (ascending, descending, first) in [
            ("名称 A–Z", "名称 Z–A", "later"),
            ("规则 A–Z", "规则 Z–A", "earlier"),
            ("代理链 A–Z", "代理链 Z–A", "earlier"),
            ("上传速度 ↑", "上传速度 ↓", "earlier"),
            ("下载速度 ↑", "下载速度 ↓", "earlier"),
            ("上传量 ↑", "上传量 ↓", "earlier"),
            ("下载量 ↑", "下载量 ↓", "earlier"),
            ("开始时间 ↑", "开始时间 ↓", "earlier"),
        ] {
            let mut rows = [later.clone(), earlier.clone()];
            rows.sort_by(|left, right| connection_sort(ascending).compare(left, right));
            assert_eq!(rows[0].id.as_ref(), first, "{ascending}");
            rows.sort_by(|left, right| connection_sort(descending).compare(left, right));
            assert_ne!(rows[0].id.as_ref(), first, "{descending}");
        }
        let mut rows = [later, earlier];
        rows.sort_by(|left, right| connection_sort("默认顺序").compare(left, right));
        assert_eq!(rows[0].id.as_ref(), "later");
        assert_eq!(rows[1].id.as_ref(), "earlier");
    }

    #[test]
    fn connection_pages_follow_shrinking_results_and_turn_from_the_displayed_index() {
        let rows: Vec<_> = (0..125)
            .map(|index| Connection {
                id: index.to_string().into(),
                ..Default::default()
            })
            .collect();
        let sort = connection_sort("默认顺序");
        let mut pager = ConnectionPager::default();
        let first = pager.project(&rows, "", sort);
        assert_eq!((first.index, first.total, first.items.len()), (0, 125, 50));
        let first_cursor = pager.cursor();
        pager.turn(first_cursor, 1);
        pager.turn(first_cursor, 1);
        assert_eq!(pager.project(&rows, "", sort).index, 1);
        pager.turn(pager.cursor(), 1);
        assert_eq!(pager.project(&rows, "", sort).items[0].id.as_ref(), "100");

        let old_display = pager.cursor();
        let shrunk = pager.project(&rows[..65], "", sort);
        assert_eq!(
            (shrunk.index, shrunk.total, shrunk.items.len()),
            (1, 65, 15)
        );
        assert_eq!(pager.cursor().index, shrunk.index);
        pager.turn(old_display, -1);
        assert_eq!(pager.project(&rows[..65], "", sort).index, 1);

        let shrunk_display = pager.cursor();
        pager.turn(shrunk_display, -1);
        let previous = pager.project(&rows[..65], "", sort);
        assert_eq!(previous.index, 0);
        assert_eq!(previous.items[0].id.as_ref(), "0");

        pager.turn(pager.cursor(), 1);
        assert_eq!(pager.project(&rows[..65], "", sort).index, 1);
        assert_eq!(pager.project(&[], "", sort).index, 0);
        assert_eq!(pager.project(&rows, "", sort).index, 0);
    }

    #[test]
    fn connection_sort_and_filter_resets_reject_turns_from_an_old_view() {
        let mut rows: Vec<_> = (0..125)
            .map(|index| Connection {
                id: index.to_string().into(),
                metadata: Metadata {
                    host: if index == 124 {
                        "other.test"
                    } else {
                        "match.test"
                    }
                    .into(),
                    ..Default::default()
                },
                download: index,
                ..Default::default()
            })
            .collect();
        rows.reverse();
        let mut pager = ConnectionPager::default();
        let original = connection_sort("默认顺序");
        assert_eq!(pager.project(&rows, "", original).index, 0);
        pager.turn(pager.cursor(), 1);
        assert_eq!(pager.project(&rows, "", original).index, 1);
        let before_query = pager.cursor();

        pager.reset();
        pager.turn(before_query, 1);
        let filtered = pager.project(&rows, "match.test", original);
        assert_eq!((filtered.index, filtered.total), (0, 124));
        assert_eq!(filtered.items[0].id.as_ref(), "123");
        pager.turn(pager.cursor(), 1);
        assert_eq!(pager.project(&rows, "match.test", original).index, 1);
        let before_sort = pager.cursor();

        pager.reset();
        pager.turn(before_sort, 1);
        let ascending = connection_sort("下载量 ↑");
        let sorted = pager.project(&rows, "match.test", ascending);
        assert_eq!((sorted.index, sorted.total), (0, 124));
        assert_eq!(sorted.items[0].id.as_ref(), "0");
        pager.turn(pager.cursor(), 1);
        assert_eq!(
            pager.project(&rows, "match.test", ascending).items[0]
                .id
                .as_ref(),
            "50"
        );
    }
}
