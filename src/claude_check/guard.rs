//! Background enforcement of the pinned Claude route. Bounded state, no processes.
use super::{Route, observe, protection};
use crate::api::Api;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub total: u64,
    pub blocked: u64,
    pub alerts: u64,
    pub connected: bool,
    pub error: String,
    pub routes: VecDeque<Route>,
}
struct Inner {
    task: tokio::task::JoinHandle<()>,
    state: Arc<Mutex<Snapshot>>,
}
struct AlertTask(Option<tokio::task::JoinHandle<anyhow::Result<()>>>);
impl Drop for AlertTask {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub struct Guard(Arc<Inner>);
impl Guard {
    pub fn snapshot(&self) -> Snapshot {
        self.0.state.lock().map(|s| s.clone()).unwrap_or_default()
    }
    pub fn start(api: Api, node: String) -> Self {
        let state = Arc::new(Mutex::new(Snapshot::default()));
        let captured = state.clone();
        let task = tokio::spawn(async move {
            let stream = observe::Session::start(api.clone());
            let mut revision = 0;
            let mut seen_total = 0;
            let mut recent = VecDeque::<String>::new();
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut next_scan = Instant::now();
            let mut last_alert = Instant::now().checked_sub(Duration::from_secs(60)).unwrap();
            let mut notification = AlertTask(None);
            loop {
                tick.tick().await;
                if notification.0.as_ref().is_some_and(|n| n.is_finished()) {
                    if let Some(task) = notification.0.take() {
                        if let Ok(Err(error)) = task.await {
                            if let Ok(mut state) = captured.lock() {
                                state.error = format!("连接已处理，但系统通知失败：{error}");
                            }
                        }
                    }
                }
                if let Some(events) = stream.after(revision) {
                    revision = events.revision;
                    let new_count =
                        (events.total.saturating_sub(seen_total) as usize).min(events.routes.len());
                    seen_total = events.total;
                    if let Ok(mut state) = captured.lock() {
                        state.connected = events.connected;
                        state.total = events.total;
                        for route in events.routes.iter().skip(events.routes.len() - new_count) {
                            if suspicious_route(route, &node) {
                                next_scan = Instant::now();
                            }
                            retain_route(&mut state, route.clone());
                        }
                    }
                }
                if Instant::now() < next_scan {
                    continue;
                }
                next_scan = Instant::now() + Duration::from_secs(1);
                let Ok(connections) = api.unsafe_claude_connections(node.clone()).await else {
                    if let Ok(mut state) = captured.lock() {
                        state.connected = false;
                        state.error = "控制接口暂不可用，实时拦截未确认；保护规则仍保留。".into();
                    }
                    continue;
                };
                if let Ok(mut state) = captured.lock() {
                    state.connected = true;
                }
                let mut alert_hosts = BTreeSet::new();
                for connection in connections {
                    let result = api.close_connection(&connection.id).await;
                    if let Ok(mut state) = captured.lock() {
                        if result.is_ok() {
                            if !recent.contains(&connection.id) {
                                if recent.len() == 128 {
                                    recent.pop_front();
                                }
                                recent.push_back(connection.id.clone());
                                state.blocked = state.blocked.saturating_add(1);
                                alert_hosts.insert(connection.host.clone());
                                retain_route(
                                    &mut state,
                                    Route {
                                        host: connection.host,
                                        rule: connection.rule,
                                        chain: connection.chains.join(" → "),
                                        time: chrono::Utc::now().to_rfc3339(),
                                        outcome: "异常连接已关闭".into(),
                                    },
                                );
                            }
                        } else {
                            state.error =
                                "异常连接关闭失败，请检查内核状态；不能确认拦截完成。".into();
                            alert_hosts.insert(connection.host);
                        }
                    }
                }
                if !alert_hosts.is_empty()
                    && last_alert.elapsed() >= Duration::from_secs(30)
                    && notification.0.is_none()
                {
                    last_alert = Instant::now();
                    if let Ok(mut state) = captured.lock() {
                        state.alerts = state.alerts.saturating_add(1);
                    }
                    let message = format!(
                        "检测到 Claude 或关联服务偏离保护节点，已尝试关闭对应连接。\n域名：{}\n保护节点：{}\n共享服务可能属于其他应用，请在 A÷检测中查看记录。",
                        alert_hosts
                            .into_iter()
                            .take(4)
                            .collect::<Vec<_>>()
                            .join("、"),
                        node
                    );
                    notification.0 = Some(tokio::spawn(async move {
                        crate::platform::notification::show("Clash of Rust · Claude 保护", &message)
                            .await
                    }));
                }
            }
        });
        Self(Arc::new(Inner { task, state }))
    }
}
fn retain_route(state: &mut Snapshot, route: Route) {
    if state.routes.len() == 32 {
        state.routes.pop_front();
    }
    state.routes.push_back(route);
}
pub fn suspicious_chain(chains: &[String], node: &str) -> bool {
    // Both documented/native orders are accepted. A named group that happens
    // to contain the node text does not pass this exact-name comparison.
    chains.is_empty()
        || chains.iter().any(|s| crate::active_route::terminal(s))
        || !chains.iter().any(|s| s == node)
}
fn suspicious_route(route: &Route, node: &str) -> bool {
    route.outcome != "连接失败"
        && (route.chain == "DIRECT"
            || route.chain.ends_with("[DIRECT]")
            || !route.chain.ends_with(&format!("[{node}]")) && route.chain != node)
}
#[derive(Debug, Deserialize)]
struct Connection {
    id: String,
    #[serde(default)]
    chains: Vec<String>,
    #[serde(default)]
    rule: String,
    metadata: Metadata,
}
#[derive(Debug, Deserialize)]
struct Metadata {
    #[serde(default)]
    host: String,
    #[serde(rename = "sniffHost", default)]
    sniff_host: String,
    #[serde(rename = "destinationIP", default)]
    destination_ip: String,
}
pub(crate) struct Row {
    pub id: String,
    pub host: String,
    pub chains: Vec<String>,
    pub rule: String,
}
pub(crate) fn decode(reader: impl std::io::Read, node: &str) -> serde_json::Result<Vec<Row>> {
    use serde::de::DeserializeSeed;
    struct Document<'a>(&'a str);
    impl<'de> serde::de::Visitor<'de> for Document<'_> {
        type Value = Vec<Row>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("connections object")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut rows = Vec::new();
            while let Some(key) = map.next_key::<String>()? {
                if key == "connections" {
                    rows = map.next_value_seed(Filtered(self.0))?;
                } else {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
            Ok(rows)
        }
    }
    impl<'de> DeserializeSeed<'de> for Document<'_> {
        type Value = Vec<Row>;
        fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
            d.deserialize_map(self)
        }
    }
    struct Filtered<'a>(&'a str);
    impl<'de> DeserializeSeed<'de> for Filtered<'_> {
        type Value = Vec<Row>;
        fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
            d.deserialize_any(self)
        }
    }
    impl<'de> serde::de::Visitor<'de> for Filtered<'_> {
        type Value = Vec<Row>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("core connections")
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(vec![])
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut rows = Vec::new();
            while let Some(connection) = seq.next_element::<Connection>()? {
                let host = if !connection.metadata.host.is_empty() {
                    connection.metadata.host
                } else if !connection.metadata.sniff_host.is_empty() {
                    connection.metadata.sniff_host
                } else {
                    connection.metadata.destination_ip
                };
                if rows.len() < 128
                    && host.len() <= 253
                    && connection.id.len() <= 128
                    && connection.chains.len() <= 16
                    && connection.chains.iter().all(|s| s.len() <= 512)
                    && (protection::owned_host(&host)
                        || protection::associated_host(&host)
                        || protection::anthropic_ip(&host))
                    && suspicious_chain(&connection.chains, self.0)
                {
                    rows.push(Row {
                        id: connection.id,
                        host,
                        chains: connection.chains,
                        rule: connection.rule.chars().take(256).collect(),
                    });
                }
            }
            Ok(rows)
        }
    }
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let rows = Document(node).deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_direct_and_wrong_nodes_without_prefix_false_positives() {
        assert!(!suspicious_chain(&["node".into(), "group".into()], "node"));
        assert!(!suspicious_chain(&["group".into(), "node".into()], "node"));
        for chains in [
            vec![],
            vec!["DIRECT".into()],
            vec!["node2".into()],
            vec!["node".into(), "DIRECT".into()],
        ] {
            assert!(suspicious_chain(&chains, "node"));
        }
    }
    #[test]
    fn retains_only_relevant_bounded_connections_without_credentials_or_processes() {
        let entries:Vec<_>=(0..5000).map(|i|serde_json::json!({"id":i.to_string(),"chains":["DIRECT"],"metadata":{"host":if i%2==0 {"api.anthropic.com"} else {"example.org"},"process":"secret-process"}})).collect();
        let bytes = serde_json::to_vec(&serde_json::json!({"connections":entries})).unwrap();
        let connections = decode(bytes.as_slice(), "node").unwrap();
        assert_eq!(connections.len(), 128);
        assert!(connections.iter().all(|r| r.host == "api.anthropic.com"));
    }
    #[test]
    fn normal_connections_cannot_hide_a_later_unsafe_connection() {
        let mut entries:Vec<_>=(0..5000).map(|i|serde_json::json!({"id":i.to_string(),"chains":["node"],"metadata":{"host":"claude.ai"}})).collect();
        entries.push(
            serde_json::json!({"id":"bad","chains":["DIRECT"],"metadata":{"host":"claude.ai"}}),
        );
        let bytes = serde_json::to_vec(&serde_json::json!({"connections":entries})).unwrap();
        let connections = decode(bytes.as_slice(), "node").unwrap();
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].id, "bad");
    }
}
