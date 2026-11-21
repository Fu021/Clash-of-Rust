use super::{Group, Route};
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
    let mut blocks = group_blocks(rows);
    for section in REPORT_SECTIONS {
        if !blocks.iter().any(|block| block.section == section) {
            blocks.push(ReportBlock {
                section,
                summary: String::new(),
                overview: "暂无结果".into(),
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
                summary.push(format!("{} 组结果", section.len()));
            }
            if risks > 0 {
                summary.push(format!("{risks} 项风险"));
            }
            if disagreements > 0 {
                summary.push(format!("{disagreements} 项来源不一致"));
            }
            let title = primary.title.trim();
            let value = primary.value.trim();
            let description = if title.is_empty() || title == primary.section {
                value.to_owned()
            } else {
                format!("{title}：{value}")
            };
            let mut short = description.chars().take(100).collect::<String>();
            if description.chars().count() > 100 {
                short.push('…');
            }
            if !short.is_empty() {
                summary.push(short);
            }
            if let Some(grouped) = section
                .iter()
                .find(|row| row.summary_hint && !std::ptr::eq(*row, primary))
            {
                let value = grouped.value.trim();
                let mut short = value.chars().take(100).collect::<String>();
                if value.chars().count() > 100 {
                    short.push('…');
                }
                if !short.is_empty() {
                    summary.push(format!("其他：{short}"));
                }
            }
            let overview = section
                .iter()
                .filter(|row| row.priority < 4)
                .take(3)
                .map(|row| {
                    let title = row.title.trim();
                    let value = row.value.trim();
                    let text = if title.is_empty() || title == row.section {
                        value.to_owned()
                    } else {
                        format!("{title}：{value}")
                    };
                    let mut shortened = text.chars().take(64).collect::<String>();
                    if text.chars().count() > 64 {
                        shortened.push('…');
                    }
                    shortened
                })
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            let overview = if overview.is_empty() {
                format!("{} 项结果，点击查看详情", section.len())
            } else {
                overview
            };
            let mut attention = Vec::new();
            if risks > 0 {
                attention.push(format!("{risks} 项需关注"));
            }
            if disagreements > 0 {
                attention.push(format!("{disagreements} 项来源不一致"));
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
    let hosts = routes
        .iter()
        .filter_map(|route| {
            let host = route.host.trim();
            (!host.is_empty()).then(|| host.to_ascii_lowercase())
        })
        .collect::<BTreeSet<_>>();
    let count = if routes.iter().any(|route| route.host.trim().is_empty()) {
        format!("{} 条记录", routes.len())
    } else if hosts.len() == routes.len() {
        format!("{} 个域名", hosts.len())
    } else {
        format!("{} 个域名 · {} 条记录", hosts.len(), routes.len())
    };
    let value = routes.first().map_or("仅记录选路", |route| outcome(route));
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
        parts.push("线路不同".into());
    }
    if !chains.is_empty() && routes.iter().any(|route| route.chain.trim().is_empty()) {
        parts.push("部分线路未记录".into());
    }
    parts.join(" · ")
}

pub fn route_group_detail(routes: &[&Route]) -> String {
    routes
        .iter()
        .map(|route| {
            let mut lines = vec![
                route.display_title().to_owned(),
                format!("结果：{}", outcome(route)),
            ];
            let explanation = route.explanation();
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
