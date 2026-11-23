#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod application;
#[cfg_attr(
    not(target_os = "macos"),
    allow(dead_code, reason = "The host catalog includes macOS-only messages")
)]
mod messages {
    use clash_of_rust::config::Language;
    include!(concat!(env!("OUT_DIR"), "/messages.rs"));
}

fn main() -> anyhow::Result<()> {
    application::run()
}
