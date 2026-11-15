use super::*;
use serde::Deserialize;
use std::collections::BTreeMap;

pub(super) async fn run(
    tool: Tool,
    target: &str,
    context: &Environment,
    report: &mut Report,
) -> Result<()> {
    if tool == Tool::ConfigDiff {
        let root = context
            .data_dir
            .clone()
            .ok_or_else(|| error::failure("config_unavailable", "当前实例数据目录不可用"))?;
        report
            .entries
            .extend(tokio::task::spawn_blocking(move || super::config_diff::read(&root)).await??);
        return Ok(());
    }
    let api = context
        .api
        .as_ref()
        .ok_or_else(|| error::failure("core_unavailable", "内核未运行；此查询不会启动内核"))?;
    match tool {
        Tool::Traffic => {
            let snapshot: Traffic = api.get("connections").await?;
            summarize(snapshot, report);
        }
        Tool::Performance => {
            #[derive(Deserialize)]
            struct Version {
                version: String,
            }
            match api.get::<Version>("version").await {
                Ok(version) => report
                    .entries
                    .push(Entry::ok("内核版本", bounded(&version.version, 128))),
                Err(error) => report.entries.push(error::entry("内核版本", &error)),
            }
            match api.get::<crate::api::ConnectionStats>("connections").await {
                Ok(stats) => report.entries.push(Entry::ok(
                    "当前连接",
                    format!(
                        "{} 个 · 累计上传 {} B · 累计下载 {} B",
                        stats.count, stats.upload_total, stats.download_total
                    ),
                )),
                Err(error) => report.entries.push(error::entry("当前连接", &error)),
            }
            match memory(api).await {
                Ok(bytes) => report.entries.push(Entry::ok(
                    "内核托管堆",
                    format!("{bytes} B · {:.2} MiB", bytes as f64 / 1048576.0),
                )),
                Err(error) => report.entries.push(error::entry("内核托管堆", &error)),
            }
            report.entries.push(Entry::unknown(
                "统计范围",
                "单次读取；托管堆不是进程 RSS，不含 GUI、后台或系统共享页；未提供的数据不推算",
                "information",
            ));
        }
        Tool::Node => {
            let proxies: crate::api::Proxies = api.get("proxies").await?;
            node(target, &proxies, report);
        }
        _ => bail!("工具入口无效"),
    }
    Ok(())
}

async fn memory(api: &Api) -> Result<u64> {
    let mut response = api
        .request(reqwest::Method::GET, api.url(&["memory"])?)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(error::failure("metric_unavailable", "内核未提供此指标"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        let end = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(chunk.len(), |index| index + 1);
        if bytes.len().saturating_add(end) > 8192 {
            return Err(error::failure(
                "controller_response_limit",
                "内存指标超过读取上限",
            ));
        }
        bytes.extend_from_slice(&chunk[..end]);
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            return value
                .get("inuse")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| error::failure("metric_unavailable", "内核未提供托管堆指标"));
        }
        if chunk[..end].ends_with(b"\n") {
            break;
        }
    }
    Err(error::failure(
        "controller_response_invalid",
        "内存指标响应无效或已截断",
    ))
}

pub(super) fn node(target: &str, proxies: &crate::api::Proxies, report: &mut Report) {
    let Some(proxy) = proxies.proxies.get(target) else {
        report.entries.push(Entry::error(
            "节点",
            "没有此名称；请使用节点页或 cor node list 查看完整名称",
            "node_not_found",
        ));
        return;
    };
    report
        .entries
        .push(Entry::ok("类型", bounded(&proxy.kind, 128)));
    report.entries.push(match proxy.delay {
        Some(0) => Entry::unknown(
            "上次测速",
            "上次测量失败，不代表现在必然不可达",
            "node_delay_stale",
        ),
        Some(delay) => Entry::ok("上次测速", format!("{delay} ms · 本次未主动测速")),
        None => Entry::unknown("上次测速", "没有历史测量", "information"),
    });
    report.entries.push(
        match crate::active_route::path_from_snapshot(target, proxies) {
            Some(path) => Entry::ok("当前选择", bounded(&path.join(" → "), 1024)),
            None => Entry::unknown(
                "当前选择",
                "策略链循环、过深或成员缺失",
                "node_path_unknown",
            ),
        },
    );
    let parents: Vec<_> = proxies
        .proxies
        .iter()
        .filter(|(_, group)| group.all.iter().any(|member| member.as_str() == target))
        .take(16)
        .map(|(name, _)| bounded(name, 64))
        .collect();
    report.entries.push(Entry::ok(
        "所属策略组",
        if parents.is_empty() {
            "没有已公布的父组".into()
        } else {
            parents.join("、")
        },
    ));
    report
        .entries
        .push(Entry::ok("成员数量", proxy.all.len().to_string()));
    report.entries.push(Entry::unknown(
        "检查范围",
        "只读内核快照；自动组可能变化，负载均衡的单次实际出口以真实连接为准",
        "information",
    ));
}

#[derive(Deserialize)]
pub(super) struct Traffic {
    #[serde(default, deserialize_with = "aggregate_connections")]
    connections: TrafficGroups,
}

type Counters = (usize, u64, u64);

#[derive(Default)]
struct TrafficGroups {
    count: usize,
    groups: [BTreeMap<String, Counters>; 4],
}

fn aggregate_connections<'de, Decoder: serde::Deserializer<'de>>(
    decoder: Decoder,
) -> std::result::Result<TrafficGroups, Decoder::Error> {
    struct Aggregator;
    impl<'de> serde::de::Visitor<'de> for Aggregator {
        type Value = TrafficGroups;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a connection array")
        }
        fn visit_unit<Failure: serde::de::Error>(
            self,
        ) -> std::result::Result<Self::Value, Failure> {
            Ok(TrafficGroups::default())
        }
        fn visit_seq<Sequence: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: Sequence,
        ) -> std::result::Result<Self::Value, Sequence::Error> {
            let mut result = TrafficGroups::default();
            while let Some(connection) = sequence.next_element::<TrafficConnection>()? {
                result.count += 1;
                for (group, totals) in result.groups.iter_mut().enumerate() {
                    let key = match group {
                        0 => bounded(&connection.metadata.host, 128),
                        1 => bounded(&connection.metadata.process, 128),
                        2 => bounded(
                            &connection
                                .chains
                                .iter()
                                .rev()
                                .take(16)
                                .map(String::as_str)
                                .collect::<Vec<_>>()
                                .join(" → "),
                            256,
                        ),
                        _ => bounded(&connection.rule, 64),
                    };
                    let key = if key.is_empty() {
                        "未提供".into()
                    } else {
                        key
                    };
                    let key = if !totals.contains_key(&key) && totals.len() >= 128 {
                        "其他（分组上限）".into()
                    } else {
                        key
                    };
                    let value = totals.entry(key).or_default();
                    value.0 += 1;
                    value.1 = value.1.saturating_add(connection.upload);
                    value.2 = value.2.saturating_add(connection.download);
                }
            }
            Ok(result)
        }
    }
    decoder.deserialize_any(Aggregator)
}

#[derive(Deserialize)]
pub(super) struct TrafficConnection {
    #[serde(default)]
    metadata: TrafficMetadata,
    #[serde(default)]
    chains: Vec<String>,
    #[serde(default)]
    rule: String,
    #[serde(default)]
    upload: u64,
    #[serde(default)]
    download: u64,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct TrafficMetadata {
    host: String,
    process: String,
}

pub(super) fn summarize(snapshot: Traffic, report: &mut Report) {
    report.entries.push(Entry::ok(
        "当前连接",
        format!(
            "{} 个；只统计仍在内核中的连接，不是历史总量",
            snapshot.connections.count
        ),
    ));
    for (label, totals) in ["主机", "进程", "策略链", "规则类型"]
        .into_iter()
        .zip(snapshot.connections.groups)
    {
        let mut rows: Vec<_> = totals.into_iter().collect();
        rows.sort_by(|left, right| {
            right
                .1
                .1
                .saturating_add(right.1.2)
                .cmp(&left.1.1.saturating_add(left.1.2))
                .then_with(|| left.0.cmp(&right.0))
        });
        for (name, (count, upload, download)) in rows.into_iter().take(8) {
            report.entries.push(Entry::ok(
                label,
                format!("{name} · {count} 个 · ↑ {upload} B · ↓ {download} B"),
            ));
        }
    }
    report.entries.push(Entry::unknown(
        "显示上限",
        "每类最多 8 组；进程信息取决于内核和系统权限，未提供时不猜测",
        "information",
    ));
}
