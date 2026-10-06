pub mod api;
pub mod assets;
pub mod config;
pub mod engine;
pub mod flags;
pub mod icons;
pub mod ip_check;
pub mod platform;
pub mod probe;
mod proxy;
mod region_check;
pub mod tray;
pub mod update;

/// Uses the package version normally; installer test builds may override it.
pub const VERSION: &str = env!("CLASH_OF_RUST_APP_VERSION");
