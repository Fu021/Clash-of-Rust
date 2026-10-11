pub mod active_route;
pub mod api;
pub mod assets;
pub mod config;
pub mod connection;
pub mod engine;
pub mod flags;
pub mod icons;
pub mod ip_check;
pub mod ip_report;
mod network;
pub mod platform;
pub mod probe;
mod profile_transaction;
mod proxy;
pub mod proxy_order;
mod region_check;
pub mod rule_manager;
mod subscription;
pub mod tray;
pub mod update;

/// Uses the package version normally; installer test builds may override it.
pub const VERSION: &str = env!("CLASH_OF_RUST_APP_VERSION");
