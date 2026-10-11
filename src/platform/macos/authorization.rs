//! Fixed macOS administrator prompt and argument quoting. No GUI dependencies.
use anyhow::{Result, ensure};
use std::process::Command;

/// Quote separately for the shell and the AppleScript string literal.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
pub fn administrator(script: &str) -> Result<()> {
    let literal = serde_json::to_string(script)?;
    let output = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            &format!("do shell script {literal} with administrator privileges"),
        ])
        .output()?;
    ensure!(
        output.status.success(),
        "管理员授权失败或已取消：{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}
