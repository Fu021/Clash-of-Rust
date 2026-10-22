//! Desktop presentation adapter. Runtime and native networking are shared with the CLI.
pub use clash_client_runtime::*;
#[path = "platform/file_dialog.rs"]
pub mod file_dialog;
#[path = "artwork/flags.rs"]
pub mod flags;
#[path = "artwork/icons.rs"]
pub mod icons;
#[path = "platform/tray.rs"]
pub mod tray;
