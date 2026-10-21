//! Desktop runtime and native integrations, using the shared client business library.
#[cfg(windows)]
pub(crate) use clash_client_core::file_hash;
pub(crate) use clash_client_core::profile_transaction;
pub use clash_client_core::{
    active_route, api, config, connection, ip_check, ip_report, probe, proxy_order, rule_manager,
};
#[path = "engine/assets.rs"]
pub mod assets;
#[path = "diagnostics/claude/mod.rs"]
pub mod claude_check;
pub mod engine;
#[path = "engine/network.rs"]
mod network;
pub mod platform;
#[path = "engine/proxy.rs"]
mod proxy;
#[path = "engine/subscription.rs"]
mod subscription;
pub mod update;

pub const VERSION: &str = clash_client_core::VERSION;
pub mod control;
pub mod tray_model;
