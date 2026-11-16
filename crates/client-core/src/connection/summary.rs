use super::{Connection, name};
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

fn bounded(value: &str) -> String {
    let value: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect();
    if value.is_empty() {
        "未提供".into()
    } else {
        value
    }
}

pub fn traffic_summary(connections: &[Connection]) -> Vec<String> {
    let mut groups: [Groups; 4] = std::array::from_fn(|_| Groups::default());
    for connection in connections {
        let keys = [
            bounded(name(connection)),
            bounded(&connection.metadata.process),
            bounded(
                &connection
                    .chains
                    .iter()
                    .rev()
                    .take(16)
                    .map(|part| part.as_str())
                    .collect::<Vec<_>>()
                    .join(" → "),
            ),
            bounded(&connection.rule),
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
    let mut lines = vec![format!(
        "全部当前连接 {} 条 · 累计字节快照，非全日用量；每类显示前三组，进程缺失显示“未提供”",
        connections.len()
    )];
    for (label, group) in ["主机", "进程", "策略链", "规则类型"]
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
                lines.push(format!(
                    "{label} · {name} · {} 条 · ↑ {} B · ↓ {} B",
                    counters.count, counters.upload, counters.download
                ));
            } else {
                other.add(counters.count, counters.upload, counters.download);
            }
        }
        if other.count > 0 {
            lines.push(format!(
                "{label} · 其余分组 · {} 条 · ↑ {} B · ↓ {} B",
                other.count, other.upload, other.download
            ));
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
}
