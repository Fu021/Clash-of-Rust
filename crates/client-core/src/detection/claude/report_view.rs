use super::{Group, Route};
use crate::config::Language;
use std::collections::BTreeSet;

const ROUTES_PER_GROUP: usize = 8;
const ROUTE_GROUP_BYTES: usize = 12 * 1024;

pub const REPORT_SECTIONS: [&str; 5] = [
    super::guard::REPORT_SECTION,
    "Claude 连接与选路",
    "出口与 IP 属性",
    "DNS 与 WebRTC",
    "浏览器与配置",
];

pub fn report_section(group: Group) -> &'static str {
    match group {
        Group::Exit | Group::Quality | Group::Cloudflare => REPORT_SECTIONS[2],
        Group::Connectivity => REPORT_SECTIONS[1],
        Group::WebRtc | Group::Dns => REPORT_SECTIONS[3],
        Group::Browser | Group::Desktop | Group::Code => REPORT_SECTIONS[4],
    }
}

pub struct BlockRow<T> {
    pub section: &'static str,
    pub title: String,
    pub value: String,
    pub priority: u8,
    pub tone: i32,
    pub summary_hint: bool,
    pub item: T,
}

pub struct ReportBlock<T> {
    pub section: &'static str,
    pub summary: String,
    pub overview: String,
    pub attention: String,
    pub tone: i32,
    pub rows: Vec<T>,
}

pub fn report_blocks<T>(rows: Vec<BlockRow<T>>) -> Vec<ReportBlock<T>> {
    report_blocks_in(rows, Language::SimplifiedChinese)
}

pub fn report_blocks_in<T>(rows: Vec<BlockRow<T>>, language: Language) -> Vec<ReportBlock<T>> {
    let mut blocks = group_blocks_in(rows, language);
    for section in REPORT_SECTIONS {
        if !blocks.iter().any(|block| block.section == section) {
            blocks.push(ReportBlock {
                section,
                summary: String::new(),
                overview: language.text("暂无结果", "No results yet").into(),
                attention: String::new(),
                tone: 0,
                rows: Vec::new(),
            });
        }
    }
    blocks.sort_by_key(|block| {
        REPORT_SECTIONS
            .iter()
            .position(|section| *section == block.section)
            .unwrap_or(REPORT_SECTIONS.len())
    });
    blocks
}

pub fn group_blocks<T>(rows: Vec<BlockRow<T>>) -> Vec<ReportBlock<T>> {
    group_blocks_in(rows, Language::SimplifiedChinese)
}

fn shorten(value: &str, limit: usize, language: Language) -> String {
    let mut characters = value.chars();
    let mut shortened = characters.by_ref().take(limit).collect::<String>();
    let Some(next) = characters.next() else {
        return shortened;
    };
    if language == Language::English
        && next.is_ascii_alphanumeric()
        && shortened
            .chars()
            .last()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && let Some(boundary) = shortened.rfind(char::is_whitespace)
        && shortened[..boundary].chars().count() >= limit / 2
    {
        shortened.truncate(boundary);
    }
    shortened.push('…');
    shortened
}

pub fn group_blocks_in<T>(rows: Vec<BlockRow<T>>, language: Language) -> Vec<ReportBlock<T>> {
    let mut sections: Vec<Vec<BlockRow<T>>> = Vec::new();
    for row in rows {
        if let Some(section) = sections
            .iter_mut()
            .find(|section| section[0].section == row.section)
        {
            section.push(row);
        } else {
            sections.push(vec![row]);
        }
    }
    sections.sort_by_key(|section| {
        REPORT_SECTIONS
            .iter()
            .position(|name| *name == section[0].section)
            .unwrap_or(REPORT_SECTIONS.len())
    });
    sections
        .into_iter()
        .map(|mut section| {
            super::order_summary(&mut section, |row| row.priority);
            let primary = section.iter().min_by_key(|row| row.priority).unwrap();
            let risks = section.iter().filter(|row| row.tone == 3).count();
            let disagreements = section.iter().filter(|row| row.priority == 0).count();
            let tone = if risks > 0 {
                3
            } else if section.iter().any(|row| row.tone == 2) {
                2
            } else {
                1
            };
            let mut summary = Vec::new();
            if section.len() > 1 {
                summary.push(match language {
                    Language::SimplifiedChinese => format!("{} 组结果", section.len()),
                    Language::English => format!("{} result groups", section.len()),
                });
            }
            if risks > 0 {
                summary.push(match language {
                    Language::SimplifiedChinese => format!("{risks} 项风险"),
                    Language::English => format!("Risks: {risks}"),
                });
            }
            if disagreements > 0 {
                summary.push(match language {
                    Language::SimplifiedChinese => format!("{disagreements} 项来源不一致"),
                    Language::English => format!("Source disagreements: {disagreements}"),
                });
            }
            let title = primary.title.trim();
            let value = primary.value.trim();
            let description = if title.is_empty()
                || title == primary.section
                || title == super::report_text::title(primary.section, language)
            {
                value.to_owned()
            } else {
                format!("{title}{}{value}", language.text("：", ": "))
            };
            let short = shorten(&description, 100, language);
            if !short.is_empty() {
                summary.push(short);
            }
            if let Some(grouped) = section
                .iter()
                .find(|row| row.summary_hint && !std::ptr::eq(*row, primary))
            {
                let value = grouped.value.trim();
                let short = shorten(value, 100, language);
                if !short.is_empty() {
                    summary.push(format!("{}{short}", language.text("其他：", "Other: ")));
                }
            }
            let overview = section
                .iter()
                .filter(|row| row.priority < 4)
                .take(3)
                .map(|row| {
                    let title = row.title.trim();
                    let value = row.value.trim();
                    let text = if title.is_empty()
                        || title == row.section
                        || title == super::report_text::title(row.section, language)
                    {
                        value.to_owned()
                    } else {
                        format!("{title}{}{value}", language.text("：", ": "))
                    };
                    let limit = if language == Language::English {
                        80
                    } else {
                        64
                    };
                    shorten(&text, limit, language)
                })
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            let overview = if overview.is_empty() {
                match language {
                    Language::SimplifiedChinese => {
                        format!("{} 项结果，点击查看详情", section.len())
                    }
                    Language::English => format!("Results: {}; open for details", section.len()),
                }
            } else {
                overview
            };
            let mut attention = Vec::new();
            if risks > 0 {
                attention.push(match language {
                    Language::SimplifiedChinese => format!("{risks} 项需关注"),
                    Language::English => format!("Attention: {risks}"),
                });
            }
            if disagreements > 0 {
                attention.push(match language {
                    Language::SimplifiedChinese => format!("{disagreements} 项来源不一致"),
                    Language::English => format!("Source disagreements: {disagreements}"),
                });
            }
            ReportBlock {
                section: primary.section,
                summary: summary.join(" · "),
                overview,
                attention: attention.join(" · "),
                tone,
                rows: section.into_iter().map(|row| row.item).collect(),
            }
        })
        .collect()
}

fn outcome(route: &Route) -> &str {
    match route.outcome.trim() {
        "" => "仅记录选路",
        value => value,
    }
}

fn route_size(route: &Route) -> usize {
    route.host.len()
        + route.rule.len()
        + route.chain.len()
        + route.time.len()
        + route.outcome.len()
        + 64
}

pub fn group_routes<'a>(routes: impl IntoIterator<Item = &'a Route>) -> Vec<Vec<&'a Route>> {
    let mut groups: Vec<Vec<&Route>> = Vec::new();
    for route in routes {
        if !route.risky()
            && !route.chain.trim().is_empty()
            && let Some(group) = groups.iter_mut().find(|group| {
                !group[0].risky()
                    && outcome(group[0]) == outcome(route)
                    && group[0].chain.trim() == route.chain.trim()
                    && group.len() < ROUTES_PER_GROUP
                    && group.iter().map(|item| route_size(item)).sum::<usize>() + route_size(route)
                        <= ROUTE_GROUP_BYTES
            })
        {
            group.push(route);
        } else {
            groups.push(vec![route]);
        }
    }
    groups
}

pub fn route_group_value(routes: &[&Route]) -> String {
    route_group_value_in(routes, Language::SimplifiedChinese)
}

pub fn route_group_value_in(routes: &[&Route], language: Language) -> String {
    let hosts = routes
        .iter()
        .filter_map(|route| {
            let host = route.host.trim();
            (!host.is_empty()).then(|| host.to_ascii_lowercase())
        })
        .collect::<BTreeSet<_>>();
    let count = if routes.iter().any(|route| route.host.trim().is_empty()) {
        match language {
            Language::SimplifiedChinese => format!("{} 条记录", routes.len()),
            Language::English => format!("{} records", routes.len()),
        }
    } else if hosts.len() == routes.len() {
        match language {
            Language::SimplifiedChinese => format!("{} 个域名", hosts.len()),
            Language::English => format!("{} domains", hosts.len()),
        }
    } else {
        match language {
            Language::SimplifiedChinese => {
                format!("{} 个域名 · {} 条记录", hosts.len(), routes.len())
            }
            Language::English => format!("{} domains · {} records", hosts.len(), routes.len()),
        }
    };
    let value = routes.first().map_or_else(
        || language.text("仅记录选路", "Route recorded"),
        |route| route.recorded_outcome_in(language),
    );
    let mut shortened = value.chars().take(80).collect::<String>();
    if value.chars().count() > 80 {
        shortened.push('…');
    }
    let chains = routes
        .iter()
        .map(|route| route.chain.trim())
        .filter(|chain| !chain.is_empty())
        .collect::<BTreeSet<_>>();
    let mut parts = vec![count, shortened];
    if chains.len() > 1 {
        parts.push(language.text("线路不同", "Different routes").into());
    }
    if !chains.is_empty() && routes.iter().any(|route| route.chain.trim().is_empty()) {
        parts.push(
            language
                .text("部分线路未记录", "Some routes not recorded")
                .into(),
        );
    }
    parts.join(" · ")
}

pub fn route_group_detail(routes: &[&Route]) -> String {
    route_group_detail_in(routes, Language::SimplifiedChinese)
}

pub fn route_group_detail_in(routes: &[&Route], language: Language) -> String {
    routes
        .iter()
        .map(|route| {
            let mut lines = vec![
                route.display_title_in(language).to_owned(),
                format!(
                    "{}{}",
                    language.text("结果：", "Result: "),
                    route.recorded_outcome_in(language)
                ),
            ];
            let explanation = route.explanation_in(language);
            if !explanation.is_empty() {
                lines.push(explanation);
            }
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_fixed_sections_and_distinguishes_absent_results() {
        let blocks = report_blocks(vec![BlockRow {
            section: REPORT_SECTIONS[2],
            title: "出口 IP 质量".into(),
            value: "机房来源不一致".into(),
            priority: 0,
            tone: 2,
            summary_hint: false,
            item: 7,
        }]);
        assert_eq!(
            blocks.iter().map(|block| block.section).collect::<Vec<_>>(),
            REPORT_SECTIONS
        );
        assert_eq!(blocks[0].overview, "暂无结果");
        assert!(blocks[0].rows.is_empty());
        assert_eq!(blocks[2].attention, "1 项来源不一致");
        assert_eq!(blocks[2].rows, [7]);
    }

    #[test]
    fn english_blocks_keep_section_keys_and_risk_counts() {
        let blocks = report_blocks_in(
            vec![BlockRow {
                section: REPORT_SECTIONS[1],
                title: "Claude connections and routing".into(),
                value: "3 domains · Route recorded".into(),
                priority: 1,
                tone: 3,
                summary_hint: false,
                item: 7,
            }],
            Language::English,
        );
        assert_eq!(blocks[0].overview, "No results yet");
        assert_eq!(blocks[1].section, "Claude 连接与选路");
        assert_eq!(blocks[1].overview, "3 domains · Route recorded");
        assert_eq!(blocks[1].attention, "Attention: 1");
        assert_eq!(blocks[1].rows, [7]);
    }

    #[test]
    fn overview_preserves_short_english_results_and_truncates_at_word_boundaries() {
        let blocks = report_blocks_in(
            vec![BlockRow {
                section: REPORT_SECTIONS[2],
                title: "Exit IP quality · 203.0.113.12".into(),
                value: "Datacenter or hosting: Yes · 2 unconfirmed".into(),
                priority: 1,
                tone: 2,
                summary_hint: false,
                item: 7,
            }],
            Language::English,
        );
        assert_eq!(
            blocks[2].overview,
            "Exit IP quality · 203.0.113.12: Datacenter or hosting: Yes · 2 unconfirmed"
        );
        assert_eq!(
            shorten(
                "The quick brown fox jumps over the lazy dog",
                18,
                Language::English
            ),
            "The quick brown…"
        );
        assert_eq!(
            shorten("a-very-long-unbroken-value", 10, Language::English),
            "a-very-lon…"
        );
        assert_eq!(
            shorten("中文结果不会依赖空格截断", 5, Language::SimplifiedChinese),
            "中文结果不…"
        );
    }

    #[test]
    fn english_routes_preserve_host_evidence_and_failures() {
        let routes = [
            Route {
                host: "api.anthropic.com".into(),
                chain: "Node A".into(),
                rule: "DOMAIN".into(),
                ..Default::default()
            },
            Route {
                host: "claude.ai".into(),
                chain: "Node A".into(),
                rule: "DOMAIN-SUFFIX".into(),
                ..Default::default()
            },
            Route {
                host: "failed.example".into(),
                outcome: "连接失败".into(),
                ..Default::default()
            },
        ];
        let groups = group_routes(&routes);
        assert_eq!(
            route_group_value_in(&groups[0], Language::English),
            "2 domains · Route recorded"
        );
        let detail = route_group_detail_in(&groups[0], Language::English);
        assert!(
            detail
                .contains("api.anthropic.com\nResult: Route recorded\nRule: DOMAIN\nRoute: Node A")
        );
        assert!(detail.contains("claude.ai"));
        assert_eq!(
            groups[1][0].display_outcome_in(Language::English),
            "Connection failed"
        );
        assert!(groups[1][0].risky());
    }

    #[test]
    fn blocks_keep_risks_and_source_disagreements_visible() {
        let rows = vec![
            BlockRow {
                section: REPORT_SECTIONS[3],
                title: "IPv4".into(),
                value: "正常".into(),
                priority: 3,
                tone: 1,
                summary_hint: false,
                item: 1,
            },
            BlockRow {
                section: REPORT_SECTIONS[2],
                title: "出口 IP".into(),
                value: "机房来源不一致".into(),
                priority: 0,
                tone: 2,
                summary_hint: false,
                item: 2,
            },
            BlockRow {
                section: REPORT_SECTIONS[3],
                title: "WebRTC".into(),
                value: "出口不同".into(),
                priority: 1,
                tone: 3,
                summary_hint: false,
                item: 3,
            },
        ];
        let blocks = group_blocks(rows);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].section, REPORT_SECTIONS[2]);
        assert!(blocks[0].summary.contains("1 项来源不一致"));
        assert!(blocks[0].summary.contains("机房来源不一致"));
        assert_eq!(blocks[1].section, REPORT_SECTIONS[3]);
        assert!(blocks[1].summary.contains("1 项风险"));
        assert!(blocks[1].summary.contains("WebRTC：出口不同"));
        assert_eq!(blocks[1].rows, [3, 1]);
    }

    #[test]
    fn low_priority_fields_stay_in_details_but_not_in_the_overview() {
        let blocks = report_blocks(vec![BlockRow {
            section: REPORT_SECTIONS[4],
            title: "Canvas 指纹摘要".into(),
            value: "演示结果".into(),
            priority: 4,
            tone: 2,
            summary_hint: false,
            item: 1,
        }]);
        assert_eq!(blocks[4].overview, "1 项结果，点击查看详情");
        assert_eq!(blocks[4].rows, [1]);
    }

    #[test]
    fn grouped_routes_remain_visible_beside_risks_in_a_collapsed_block() {
        let blocks = group_blocks(vec![
            BlockRow {
                section: "Claude 域名选路",
                title: "Claude 域名选路".into(),
                value: "3 个域名 · 仅记录选路".into(),
                priority: 3,
                tone: 2,
                summary_hint: true,
                item: 1,
            },
            BlockRow {
                section: "Claude 域名选路",
                title: "failed.example".into(),
                value: "连接失败".into(),
                priority: 1,
                tone: 3,
                summary_hint: false,
                item: 2,
            },
        ]);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].summary.contains("2 组结果"));
        assert!(blocks[0].summary.contains("failed.example：连接失败"));
        assert!(blocks[0].summary.contains("其他：3 个域名 · 仅记录选路"));
        assert_eq!(blocks[0].rows, [2, 1]);
    }

    #[test]
    fn groups_matching_routes_without_hiding_different_lines_or_failures() {
        let routes = [
            Route {
                host: "api.anthropic.com".into(),
                rule: "DOMAIN".into(),
                chain: "节点 A".into(),
                time: "08:00".into(),
                ..Default::default()
            },
            Route {
                host: "claude.ai".into(),
                rule: "DOMAIN-SUFFIX".into(),
                chain: "节点 B".into(),
                time: "08:01".into(),
                ..Default::default()
            },
            Route {
                host: "claude.com".into(),
                ..Default::default()
            },
            Route {
                host: "same.example".into(),
                rule: "DOMAIN-SUFFIX".into(),
                chain: "节点 A".into(),
                time: "08:02".into(),
                ..Default::default()
            },
            Route {
                host: "failed.example".into(),
                outcome: "连接失败".into(),
                ..Default::default()
            },
            Route {
                host: "direct.example".into(),
                chain: "DIRECT".into(),
                ..Default::default()
            },
        ];
        let mut report = super::super::Report::new(super::super::Environment::Core, "fixture");
        report.routes = routes.to_vec();
        assert_eq!(report.counts(), (0, 2, 4));
        let groups = group_routes(&routes);
        assert_eq!(groups.len(), 5);
        assert_eq!(route_group_value(&groups[0]), "2 个域名 · 仅记录选路");
        let detail = route_group_detail(&groups[0]);
        for value in [
            "api.anthropic.com",
            "same.example",
            "规则：DOMAIN",
            "线路：节点 A",
            "时间：08:02",
        ] {
            assert!(detail.contains(value), "{value}");
        }
        assert!(!detail.contains("节点 B"));
        assert_eq!(groups[1][0].host, "claude.ai");
        assert_eq!(groups[2][0].host, "claude.com");
        assert_eq!(groups[3][0].host, "failed.example");
        assert_eq!(groups[4][0].host, "direct.example");
        assert!(groups[3][0].risky() && groups[4][0].risky());
    }

    #[test]
    fn repeated_hosts_and_large_imported_fields_have_bounded_groups() {
        let routes = (0..10)
            .map(|index| Route {
                host: if index == 1 {
                    "CLAUDE.AI".into()
                } else {
                    "claude.ai".into()
                },
                rule: "x".repeat(2048),
                chain: "y".repeat(2048),
                time: "z".repeat(2048),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let groups = group_routes(&routes);
        assert!(groups.iter().all(|group| group.len() == 1));
        assert_eq!(groups.iter().map(Vec::len).sum::<usize>(), routes.len());

        let compact = routes
            .iter()
            .take(2)
            .map(|route| Route {
                host: route.host.clone(),
                chain: "节点 A".into(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let groups = group_routes(&compact);
        assert_eq!(
            route_group_value(&groups[0]),
            "1 个域名 · 2 条记录 · 仅记录选路"
        );
        assert_eq!(route_group_detail(&groups[0]).matches("结果：").count(), 2);
    }

    #[test]
    fn unrecorded_routes_are_not_assumed_to_use_the_same_line() {
        let routes = [
            Route {
                host: "claude.ai".into(),
                ..Default::default()
            },
            Route {
                host: "api.anthropic.com".into(),
                chain: " ".into(),
                ..Default::default()
            },
        ];
        let groups = group_routes(&routes);
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().all(|group| group.len() == 1));
    }

    #[test]
    fn imported_route_whitespace_does_not_hide_failures_or_direct_connections() {
        let routes = [
            Route {
                host: "failed.example".into(),
                outcome: " 连接失败 ".into(),
                ..Default::default()
            },
            Route {
                host: "direct.example".into(),
                chain: "节点[DIRECT] ".into(),
                ..Default::default()
            },
            Route {
                host: "rejected.example".into(),
                chain: "组[REJECT-DROP] → 节点".into(),
                ..Default::default()
            },
        ];
        assert!(routes.iter().all(Route::risky));
        assert_eq!(routes[0].display_outcome(), "连接失败");
        assert_eq!(routes[1].display_outcome(), "异常线路");
        let observed = Route {
            chain: "DIRECT".into(),
            outcome: "内核已选路".into(),
            ..Default::default()
        };
        assert_eq!(observed.display_outcome(), "异常线路 · 内核已选路");
        assert_eq!(group_routes(&routes).len(), 3);
        let mut report = super::super::Report::new(super::super::Environment::Core, "fixture");
        report.routes = routes.to_vec();
        assert_eq!(report.counts(), (0, 3, 0));
    }
}
