//! Shared client logic. Desktop windows, privileges and native services live in apps/desktop.
#[path = "rules/active_route.rs"]
pub mod active_route;
#[path = "controller/mod.rs"]
pub mod api;
#[path = "detection/claude/mod.rs"]
pub mod claude_check;
pub mod config;
pub mod connection;
#[path = "model/countries.rs"]
pub mod countries;
pub mod dns;
#[path = "storage/hash.rs"]
pub mod file_hash;
pub mod geodata;
#[path = "detection/ip/mod.rs"]
pub mod ip_check;
#[path = "detection/ip/report.rs"]
pub mod ip_report;
pub mod model;
#[path = "subscription/routes.rs"]
pub mod network;
#[path = "detection/probe.rs"]
pub mod probe;
#[path = "storage/transaction.rs"]
pub mod profile_transaction;
pub mod protection;
#[path = "connection/order.rs"]
pub mod proxy_order;
#[path = "detection/regions/mod.rs"]
mod region_check;
#[path = "rules/mod.rs"]
pub mod rule_manager;
pub mod runtime;
pub mod subscription;
pub mod update;
pub mod wsl;

pub const VERSION: &str = match option_env!("CLASH_OF_RUST_BUILD_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};
