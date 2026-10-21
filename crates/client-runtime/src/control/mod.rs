//! Authenticated, local-only commands shared by GUI and CLI owners.
mod dispatch;
mod mutations;
mod transport;
use crate::{
    assets::GeoManifest,
    config::{Profile, Settings},
    engine::{ProxyMode, Scope},
};
pub use mutations::RuleChange;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
pub use transport::{Endpoint, Remote, Server, serve};

pub const PROTOCOL: u32 = 1;
pub const REQUEST_LIMIT: usize = 8 * 1024 * 1024;
pub const RESPONSE_LIMIT: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Owner {
    Gui,
    Daemon,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub settings: Settings,
    pub profiles: Vec<Profile>,
    pub geo: GeoManifest,
    pub running: bool,
    pub version: String,
    pub owner: Owner,
    pub executable: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Poll {
    pub meta: Meta,
    pub snapshot: crate::engine::Snapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Command {
    Meta,
    Snapshot {
        scope: Scope,
        header: bool,
    },
    Start,
    Stop,
    Restart,
    Shutdown,
    Settings {
        settings: Settings,
        expected: Option<Value>,
    },
    RuleOverrides {
        overrides: crate::config::RuleOverrides,
    },
    ConfigSet {
        key: String,
        value: Value,
    },
    RuleChange {
        id: String,
        change: RuleChange,
    },
    Ports {
        controller: u16,
        mixed: u16,
    },
    Import {
        name: String,
        source: String,
        id: Option<String>,
    },
    EditProfile {
        id: String,
        name: String,
        source: String,
    },
    Activate {
        id: String,
    },
    DeleteProfile {
        id: String,
    },
    RuleDocument {
        id: String,
    },
    SaveRules {
        id: String,
        rules: crate::rule_manager::ProfileRules,
    },
    Select {
        group: String,
        node: String,
    },
    Mode {
        mode: String,
    },
    Proxy {
        mode: ProxyMode,
    },
    Protect {
        enabled: bool,
    },
    TrayNodes,
    Delay {
        name: Option<String>,
        group: bool,
    },
    Close {
        id: Option<String>,
    },
    Logs {
        clear: bool,
    },
    ExportLogs,
    RecordFailure {
        message: String,
    },
    GeoUpdate,
    GeoInstall {
        path: PathBuf,
    },
    CheckConfig,
    EffectiveConfig,
    Notify {
        title: String,
        message: String,
    },
    Autostart {
        enabled: Option<bool>,
    },
    UpdateInstall,
}

#[derive(Serialize, Deserialize)]
struct Request {
    protocol: u32,
    token: String,
    operation: Command,
}
#[derive(Serialize, Deserialize)]
struct Reply {
    result: Value,
    error: Option<String>,
}
