//! Authenticated, local-only commands shared by GUI and CLI owners.
#[cfg(windows)]
mod background;
mod dispatch;
#[cfg(windows)]
pub use background::helper_main as background_helper_main;
mod gui;
mod mutations;
pub(crate) mod network;
mod subscriptions;
mod transport;
use crate::{
    assets::GeoManifest,
    config::{Profile, Settings},
    engine::{ProxyMode, Scope},
};
#[cfg(all(test, any(windows, target_os = "macos")))]
pub(crate) use dispatch::prepare_update_handoff;
pub use gui::{GuiSession, gui_exit_requested, gui_pid, request_gui_exit};
pub use mutations::RuleChange;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
pub use transport::{Endpoint, Remote, Server, serve};

pub const PROTOCOL: u32 = 3;
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
    #[serde(default)]
    pub view_revision: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measurements: Vec<clash_client_core::controller::MeasurementDiagnostic>,
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
    RecoverNetwork,
    RecoverUnreachable,
    Shutdown,
    Quit,
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
        mixed: u16,
    },
    Import {
        name: String,
        source: String,
        id: Option<String>,
        // Older clients omit this field and may intentionally replace the source.
        #[serde(default)]
        expect_current: bool,
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
        expected: Option<String>,
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
    UpdatePrepareTerminal,
    UpdateSelected {
        version: String,
        preview: bool,
        terminal: bool,
    },
    UpdateCommitTerminal {
        directory: PathBuf,
    },
}

impl Command {
    pub(crate) fn check_protection(&self, settings: &Settings) -> anyhow::Result<()> {
        if !matches!(
            self,
            Self::Meta
                | Self::Snapshot { .. }
                | Self::RuleDocument { .. }
                | Self::TrayNodes
                | Self::Delay { .. }
                | Self::RecoverUnreachable
                | Self::Logs { clear: false }
                | Self::ExportLogs
                | Self::RecordFailure { .. }
                | Self::CheckConfig
                | Self::EffectiveConfig
                | Self::Notify { .. }
                | Self::Autostart { enabled: None }
                | Self::Protect { enabled: false }
        ) {
            settings.require_mutable()?;
        }
        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn protected_settings() -> Settings {
        use crate::claude_check::protection::{Previous, Protection};
        Settings {
            claude_protection: Some(std::sync::Arc::new(Protection::new(
                "fixture".into(),
                "node".into(),
                Previous {
                    mode: "rule".into(),
                    proxy_mode: ProxyMode::Off,
                    selectors: Default::default(),
                },
            ))),
            ..Settings::default()
        }
    }

    #[test]
    fn protection_rejects_mutations_but_keeps_observation_and_disable_available() {
        let settings = protected_settings();
        for allowed in [
            Command::Meta,
            Command::Snapshot {
                scope: Scope::Other,
                header: false,
            },
            Command::RuleDocument {
                id: "fixture".into(),
            },
            Command::TrayNodes,
            Command::Delay {
                name: None,
                group: false,
            },
            Command::RecoverUnreachable,
            Command::Logs { clear: false },
            Command::ExportLogs,
            Command::RecordFailure {
                message: "fixture".into(),
            },
            Command::CheckConfig,
            Command::EffectiveConfig,
            Command::Notify {
                title: "fixture".into(),
                message: "fixture".into(),
            },
            Command::Autostart { enabled: None },
            Command::Protect { enabled: false },
        ] {
            assert!(allowed.check_protection(&settings).is_ok());
        }
        for blocked in [
            Command::Start,
            Command::Stop,
            Command::Restart,
            Command::RecoverNetwork,
            Command::Shutdown,
            Command::Quit,
            Command::GeoUpdate,
            Command::GeoInstall {
                path: "/tmp/geo".into(),
            },
            Command::UpdateInstall,
            Command::UpdatePrepareTerminal,
            Command::UpdateSelected {
                version: "0.7.1".into(),
                preview: false,
                terminal: true,
            },
            Command::UpdateCommitTerminal {
                directory: "/tmp/update".into(),
            },
            Command::RuleOverrides {
                overrides: crate::config::RuleOverrides::default(),
            },
            Command::ConfigSet {
                key: "ipv6".into(),
                value: serde_json::json!(true),
            },
            Command::RuleChange {
                id: "fixture".into(),
                change: RuleChange::Add {
                    raw: "MATCH,DIRECT".into(),
                    position: 1,
                    enabled: true,
                },
            },
            Command::Ports { mixed: 7897 },
            Command::Import {
                name: "fixture".into(),
                source: "https://example.test".into(),
                id: None,
                expect_current: false,
            },
            Command::EditProfile {
                id: "fixture".into(),
                name: "fixture".into(),
                source: "https://example.test".into(),
            },
            Command::Activate {
                id: "fixture".into(),
            },
            Command::DeleteProfile {
                id: "fixture".into(),
            },
            Command::SaveRules {
                id: "fixture".into(),
                rules: crate::rule_manager::ProfileRules::default(),
                expected: None,
            },
            Command::Settings {
                settings: Settings::default(),
                expected: None,
            },
            Command::Select {
                group: "GLOBAL".into(),
                node: "DIRECT".into(),
            },
            Command::Mode {
                mode: "rule".into(),
            },
            Command::Proxy {
                mode: ProxyMode::Off,
            },
            Command::Protect { enabled: true },
            Command::Close { id: None },
            Command::Logs { clear: true },
            Command::Autostart {
                enabled: Some(false),
            },
        ] {
            assert!(
                blocked.check_protection(&settings).is_err(),
                "{blocked:?} must be rejected during protection"
            );
        }
    }
}
