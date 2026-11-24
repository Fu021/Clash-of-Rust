#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod application;
#[allow(
    dead_code,
    reason = "The generated host catalog includes platform-specific messages"
)]
mod messages {
    use clash_of_rust::config::Language;
    include!(concat!(env!("OUT_DIR"), "/messages.rs"));
}

fn main() -> anyhow::Result<()> {
    application::run()
}
