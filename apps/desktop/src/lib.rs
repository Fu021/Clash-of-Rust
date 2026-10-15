//! Desktop runtime and native integrations, using the shared client business library.
pub use clash_client_core::{
    active_route, api, config, connection, ip_check, ip_report, probe, proxy_order, rule_manager,
};
pub(crate) use clash_client_core::profile_transaction;
#[path = "engine/assets.rs"]
pub mod assets;
#[path = "diagnostics/claude/mod.rs"]
pub mod claude_check;
pub mod engine;
#[path = "platform/file_dialog.rs"]
pub mod file_dialog;
#[path = "views/flags.rs"]
pub mod flags;
#[path = "views/icons.rs"]
pub mod icons;
#[path = "engine/network.rs"]
mod network;
pub mod platform;
#[path = "engine/proxy.rs"]
mod proxy;
#[path = "engine/subscription.rs"]
mod subscription;
#[path = "platform/tray.rs"]
pub mod tray;
pub mod update;

pub const VERSION: &str = env!("CLASH_OF_RUST_APP_VERSION");
