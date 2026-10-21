//! Page data shared by clients, with bounded lifetime and explicit release of inactive data.
use crate::api::{Connections, Proxies, Rules};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    Home,
    Proxies,
    Connections,
    Rules,
    Logs,
    Claude,
    Other,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: String,
    pub mode: String,
    pub active_node: Option<String>,
    pub tun: bool,
    pub system_proxy: bool,
    pub proxies: Proxies,
    pub connections: Connections,
    #[serde(skip)]
    pub connections_sampled_at: Option<Instant>,
    pub connection_count: usize,
    pub rules: Rules,
    pub upload_rate: u64,
    pub download_rate: u64,
    pub logs: Vec<Arc<str>>,
    pub running: bool,
    pub claude_guard: Option<crate::claude_check::guard::Snapshot>,
}

impl Snapshot {
    /// Release heap-backed page data, including spare vector capacity.
    pub fn retain_scope(&mut self, scope: Scope) {
        if scope != Scope::Claude {
            self.claude_guard = None;
        }
        if scope != Scope::Proxies {
            self.proxies = Proxies::default();
        }
        if scope != Scope::Rules {
            self.rules = Rules::default();
        }
        if scope != Scope::Connections {
            self.connections.connections = Vec::new();
            self.connections_sampled_at = None;
        }
        if scope != Scope::Logs {
            self.logs = Vec::new();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn leaving_a_page_releases_data_and_vector_capacity() {
        let mut snapshot = Snapshot::default();
        snapshot.connections.connections.reserve(10_000);
        snapshot.rules.rules.reserve(10_000);
        snapshot
            .proxies
            .proxies
            .insert("group".into(), Default::default());
        snapshot.logs.push(Arc::from("log"));
        snapshot.retain_scope(Scope::Rules);
        assert_eq!(snapshot.connections.connections.capacity(), 0);
        assert!(snapshot.proxies.proxies.is_empty());
        assert_eq!(snapshot.logs.capacity(), 0);
        assert!(snapshot.rules.rules.capacity() >= 10_000);
        snapshot.retain_scope(Scope::Other);
        assert_eq!(snapshot.rules.rules.capacity(), 0);
    }
}
