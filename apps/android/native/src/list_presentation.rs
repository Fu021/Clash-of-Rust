use clash_mobile_runtime::core::{
    config::Profile,
    connection::{Field, Sort},
};
use std::collections::{BTreeMap, VecDeque};

pub(crate) struct Page<T> {
    pub items: Vec<T>,
    pub index: usize,
    pub total: usize,
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
    entries: &VecDeque<(String, String)>,
    level: &str,
    lower_query: &str,
) -> Vec<(String, String)> {
    entries
        .iter()
        .rev()
        .filter(|(entry_level, line)| {
            (level == "全部等级" || entry_level == level) && contains_query(line, lower_query)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_mobile_runtime::core::api::{Connection, ConnectionRates, Metadata};

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
            logs(&entries, "全部等级", "example"),
            [
                ("WARN".into(), "Latest EXAMPLE".into()),
                ("INFO".into(), "first example".into()),
            ]
        );
        assert_eq!(
            logs(&entries, "WARN", ""),
            [
                ("WARN".into(), "Latest EXAMPLE".into()),
                ("WARN".into(), "unrelated".into()),
            ]
        );
        assert_eq!(entries.front().unwrap().1, "first example");
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
}
