//! Native desktop integration shared by the host and its renderer. No Slint.
pub use clash_client_runtime::*;
pub mod file_dialog;
pub mod icons;
pub mod tray;

#[allow(
    dead_code,
    reason = "Native titles and errors vary by operating system"
)]
mod messages {
    use crate::config::Language;
    include!(concat!(env!("OUT_DIR"), "/messages.rs"));
}
