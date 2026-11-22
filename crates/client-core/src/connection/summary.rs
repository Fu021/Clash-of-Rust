use super::{Connection, name};
use crate::config::Language;
use std::collections::BTreeMap;

#[derive(Default)]
struct Counters {
    count: usize,
    upload: u64,
    download: u64,
}

impl Counters {
    fn add(&mut self, count: usize, upload: u64, download: u64) {
        self.count = self.count.saturating_add(count);
        self.upload = self.upload.saturating_add(upload);
        self.download = self.download.saturating_add(download);
    }
}

#[derive(Default)]
struct Groups {
    values: BTreeMap<String, Counters>,
    other: Counters,
}

fn bounded(value: &str, language: Language) -> String {
    let value: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect();
    if value.is_empty() {
        language.text("未提供", "Not provided").into()
    } else {
        value
    }
}

pub fn traffic_summary(connections: &[Connection]) -> Vec<String> {
    traffic_summary_in(connections, Language::SimplifiedChinese)
}

pub fn traffic_summary_in(connections: &[Connection], language: Language) -> Vec<String> {
    let mut groups: [Groups; 4] = std::array::from_fn(|_| Groups::default());
    for connection in connections {
        let keys = [
            bounded(name(connection), language),
            bounded(&connection.metadata.process, language),
            bounded(
                &connection
                    .chains
                    .iter()
                    .rev()
                    .take(16)
                    .map(|part| part.as_str())
                    .collect::<Vec<_>>()
                    .join(" → "),
                language,
            ),
            bounded(&connection.rule, language),
        ];
        for (group, key) in groups.iter_mut().zip(keys) {
            let counters = if group.values.contains_key(&key) || group.values.len() < 128 {
                group.values.entry(key).or_default()
            } else {
                &mut group.other
            };
            counters.add(1, connection.upload, connection.download);
        }
    }
    let mut lines = vec![match language {
        Language::SimplifiedChinese => format!(
            "全部当前连接 {} 条 · 累计字节快照，非全日用量；每类显示前三组，进程缺失显示“未提供”",
            connections.len()
        ),
        Language::English => format!(
            "{} current connection{} · cumulative byte snapshot; top three groups per category",
            connections.len(),
            if connections.len() == 1 { "" } else { "s" }
        ),
    }];
    for (label, group) in [
        language.text("主机", "Host"),
        language.text("进程", "Process"),
        language.text("策略链", "Route"),
        language.text("规则类型", "Rule type"),
    ]
    .into_iter()
    .zip(groups)
    {
        let mut values: Vec<_> = group.values.into_iter().collect();
        values.sort_unstable_by(|(left_name, left), (right_name, right)| {
            (u128::from(right.upload) + u128::from(right.download))
                .cmp(&(u128::from(left.upload) + u128::from(left.download)))
                .then_with(|| left_name.cmp(right_name))
        });
        let mut other = group.other;
        for (index, (name, counters)) in values.into_iter().enumerate() {
            if index < 3 {
                lines.push(match language {
                    Language::SimplifiedChinese => format!(
                        "{label} · {name} · {} 条 · ↑ {} B · ↓ {} B",
                        counters.count, counters.upload, counters.download
                    ),
                    Language::English => format!(
                        "{label} · {name} · {} connection{} · ↑ {} B · ↓ {} B",
                        counters.count,
                        if counters.count == 1 { "" } else { "s" },
                        counters.upload,
                        counters.download
                    ),
                });
            } else {
                other.add(counters.count, counters.upload, counters.download);
            }
        }
        if other.count > 0 {
            lines.push(match language {
                Language::SimplifiedChinese => format!(
                    "{label} · 其余分组 · {} 条 · ↑ {} B · ↓ {} B",
                    other.count, other.upload, other.download
                ),
                Language::English => format!(
                    "{label} · Other groups · {} connection{} · ↑ {} B · ↓ {} B",
                    other.count,
                    if other.count == 1 { "" } else { "s" },
                    other.upload,
                    other.download
                ),
            });
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_bounds_groups_saturates_and_keeps_omitted_connections() {
        let mut connections = Vec::new();
        for index in 0..200 {
            let mut connection = Connection {
                upload: u64::MAX,
                download: 2,
                ..Default::default()
            };
            connection.metadata.host = format!("host-{index}.test").into();
            connections.push(connection);
        }
        let summary = traffic_summary(&connections);
        assert!(summary.len() <= 17);
        assert!(
            summary
                .iter()
                .any(|line| line.starts_with("主机 · 其余分组 · 197 条"))
        );
        assert!(
            summary
                .iter()
                .any(|line| line.starts_with("进程 · 未提供 · 200 条"))
        );
        assert!(
            summary
                .iter()
                .any(|line| line.contains(&u64::MAX.to_string()))
        );
        assert_eq!(traffic_summary(&[]).len(), 1);
    }

    #[test]
    fn english_summary_keeps_connection_values_and_localizes_missing_process() {
        let mut connection = Connection {
            upload: 23,
            download: 42,
            ..Default::default()
        };
        connection.metadata.host = "example.test".into();
        let summary = traffic_summary_in(&[connection], Language::English);
        assert!(summary[0].starts_with("1 current connection ·"));
        assert!(
            summary
                .iter()
                .any(|line| line.contains("Host · example.test · 1 connection · ↑ 23 B · ↓ 42 B"))
        );
        assert!(
            summary
                .iter()
                .any(|line| line.starts_with("Process · Not provided"))
        );
    }
}
