//! Bounded Claude connection events from mihomo's HTTP log subscription.
//! No request bodies, process names or unrelated raw logs are retained.
use super::{Route, claude_host};
use crate::{api::Api, engine::LogFrames};
use futures_util::StreamExt;
use serde::Deserialize;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Default)]
pub struct Snapshot {
    pub revision: u64,
    pub total: u64,
    pub failures: u64,
    pub connected: bool,
    pub routes: VecDeque<Route>,
}
impl Snapshot {
    fn connection(&mut self, connected: bool) {
        if self.connected != connected {
            self.connected = connected;
            self.revision = self.revision.wrapping_add(1);
        }
    }
    fn record(&mut self, route: Route) {
        self.total = self.total.saturating_add(1);
        self.failures = self
            .failures
            .saturating_add(u64::from(route.outcome == "连接失败"));
        if self.routes.len() == 32 {
            self.routes.pop_front();
        }
        self.routes.push_back(route);
        self.revision = self.revision.wrapping_add(1);
    }
}
struct Inner {
    task: tokio::task::JoinHandle<()>,
    state: Arc<Mutex<Snapshot>>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[derive(Clone)]
pub struct Session(Arc<Inner>);
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClaudeConnectionMonitor")
    }
}
impl Session {
    pub fn start(api: Api) -> Self {
        let state = Arc::new(Mutex::new(Snapshot {
            revision: 1,
            ..Default::default()
        }));
        let captured = state.clone();
        let task = tokio::spawn(async move {
            loop {
                if let Ok(response) = api.logs().await {
                    if let Ok(mut state) = captured.lock() {
                        state.connection(true);
                    }
                    let mut stream = response.bytes_stream();
                    let mut frames = LogFrames::default();
                    while let Some(Ok(chunk)) = stream.next().await {
                        frames.push(&chunk, |frame| {
                            if let Some(route) = event(frame)
                                && let Ok(mut state) = captured.lock()
                            {
                                state.record(route);
                            }
                        });
                    }
                }
                if let Ok(mut state) = captured.lock() {
                    state.connection(false);
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });
        Self(Arc::new(Inner { task, state }))
    }
    pub fn after(&self, revision: u64) -> Option<Snapshot> {
        let state = self.0.state.lock().ok()?;
        (state.revision != revision).then(|| state.clone())
    }
}

fn clipped(value: &str) -> String {
    value.chars().take(512).collect()
}
pub fn event(bytes: &[u8]) -> Option<Route> {
    #[derive(Deserialize)]
    struct Entry<'a> {
        #[serde(borrow)]
        payload: std::borrow::Cow<'a, str>,
    }
    let entry: Entry<'_> = serde_json::from_slice(bytes).ok()?;
    let request = entry
        .payload
        .strip_prefix("[TCP] ")
        .or_else(|| entry.payload.strip_prefix("[UDP] "))?;
    let (source, destination) = request.split_once(" --> ")?;
    let (endpoint, decision) = destination.split_once(' ')?;
    let (host, port) = endpoint.rsplit_once(':')?;
    let host = host.trim_end_matches('.');
    if !claude_host(host)
        || host.len() > 253
        || port.parse::<u16>().ok()? == 0
        || !host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
    {
        return None;
    }
    let (rule, chain, outcome) = if decision.starts_with("error: ") {
        let dial = source.strip_prefix("dial ")?;
        let (proxy, rule) = if let Some((proxy, matched)) = dial.split_once(" (match ") {
            (proxy, matched.split_once(") ")?.0)
        } else {
            ("实际线路未确认", "未提供匹配规则")
        };
        (rule, proxy, "连接失败")
    } else {
        let (rule, chain) = if let Some((rule, chain)) = decision
            .strip_prefix("match ")
            .and_then(|s| s.split_once(" using "))
        {
            (rule, chain)
        } else if let Some(chain) = decision.strip_prefix("using ") {
            ("模式路由", chain)
        } else {
            (
                "未匹配规则",
                decision.strip_prefix("doesn't match any rule using ")?,
            )
        };
        (rule, chain, "连接已建立")
    };
    if rule.chars().chain(chain.chars()).any(char::is_control) {
        return None;
    }
    Some(Route {
        host: host.to_ascii_lowercase(),
        rule: clipped(rule),
        chain: clipped(chain),
        time: chrono::Utc::now().to_rfc3339(),
        outcome: outcome.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(payload: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"type":"info","payload":payload})).unwrap()
    }
    #[test]
    fn accepts_domain_boundaries_and_success_failure_without_retaining_errors() {
        let route = event(&frame("[TCP] 127.0.0.1:123 --> www.claude.ai:443 match DomainSuffix(claude.ai) using 代理选择[美国节点]")).unwrap();
        assert_eq!(route.chain, "代理选择[美国节点]");
        assert_eq!(route.outcome, "连接已建立");
        let route = event(&frame("[UDP] dial 代理选择 (match DomainSuffix/anthropic.com) 127.0.0.1:123 --> api.anthropic.com:443 error: SECRET_TOKEN")).unwrap();
        assert_eq!(route.rule, "DomainSuffix/anthropic.com");
        assert_eq!(route.chain, "代理选择");
        assert_eq!(route.outcome, "连接失败");
        assert!(
            !serde_json::to_string(&route)
                .unwrap()
                .contains("SECRET_TOKEN")
        );
        for host in [
            "evilclaude.ai",
            "claude.ai.evil.test",
            "claude.ai/path",
            "user@claude.ai",
        ] {
            assert!(event(&frame(&format!("[TCP] x --> {host}:443 using DIRECT"))).is_none());
        }
        assert!(event(&frame("[TCP] x --> CLAUDE.COM.:443 using DIRECT")).is_some());
    }
    #[test]
    fn counts_repeated_connections_and_keeps_only_the_latest_32() {
        let mut state = Snapshot::default();
        for i in 0..1000 {
            let mut route = event(&frame("[TCP] x --> claude.ai:443 using DIRECT")).unwrap();
            route.time = i.to_string();
            state.record(route);
        }
        assert_eq!(state.total, 1000);
        assert_eq!(state.routes.len(), 32);
        assert_eq!(state.routes.front().unwrap().time, "968");
        assert_eq!(state.routes.back().unwrap().time, "999");
    }
    #[tokio::test]
    async fn streaming_monitor_reconnects_keeps_short_events_and_stops_on_drop() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            for attempt in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![];
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
                    request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with("GET /logs?level=info "));
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("authorization: bearer fixture-monitor")
                );
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n").await.unwrap();
                let mut body = frame("[TCP] x --> unrelated.example:443 using DIRECT");
                body.push(b'\n');
                for _ in 0..if attempt == 0 { 100 } else { 1 } {
                    body.extend(frame("[TCP] x --> api.anthropic.com:443 match DomainSuffix(anthropic.com) using Proxy[node]"));
                    body.push(b'\n');
                }
                for chunk in body.chunks(31) {
                    socket.write_all(chunk).await.unwrap();
                }
                if attempt == 1 {
                    let mut byte = [0];
                    assert!(
                        tokio::time::timeout(Duration::from_secs(3), socket.read(&mut byte))
                            .await
                            .unwrap()
                            .map_or(true, |n| n == 0)
                    );
                }
            }
        });
        let settings = crate::config::Settings {
            controller_port: port,
            secret: "fixture-monitor".into(),
            ..Default::default()
        };
        let session = Session::start(Api::new(&settings).unwrap());
        let snapshot = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if let Some(snapshot) = session.after(0)
                    && snapshot.total == 101
                {
                    break snapshot;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(snapshot.routes.len(), 32);
        assert!(
            snapshot
                .routes
                .iter()
                .all(|r| r.host == "api.anthropic.com")
        );
        assert!(session.after(snapshot.revision).is_none());
        drop(session);
        server.await.unwrap();
    }
}
