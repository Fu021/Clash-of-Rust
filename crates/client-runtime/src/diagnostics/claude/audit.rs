//! Native discovery of Claude project locations. Scan policy lives in client-core.
pub use clash_client_core::claude_check::audit::*;
use std::path::PathBuf;

fn local_inputs(mixed_port: u16) -> Option<Inputs> {
    let home = directories::BaseDirs::new()?.home_dir().to_owned();
    let config = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".claude"));
    #[cfg(target_os = "macos")]
    let managed = vec![PathBuf::from(
        "/Library/Application Support/ClaudeCode/managed-settings.json",
    )];
    #[cfg(target_os = "linux")]
    let managed = vec![PathBuf::from("/etc/claude-code/managed-settings.json")];
    #[cfg(windows)]
    let managed = ["ProgramFiles"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|p| PathBuf::from(p).join("ClaudeCode/managed-settings.json"))
        .collect();
    Some(Inputs {
        home,
        config,
        current: std::env::current_dir().ok(),
        managed,
        mixed_port,
    })
}

pub fn local(mixed_port: u16, cancel: &Scan) -> Audit {
    scan_available(local_inputs(mixed_port), cancel)
}
