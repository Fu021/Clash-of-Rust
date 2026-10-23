//! Shared runtime and native shell; the desktop host does not depend on Slint.
pub use clash_client_runtime::*;
pub use clash_desktop_shell::{file_dialog, icons, tray};
pub mod flags;
