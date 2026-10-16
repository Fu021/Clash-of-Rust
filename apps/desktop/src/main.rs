#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod application;

fn main() -> anyhow::Result<()> {
    application::run()
}
