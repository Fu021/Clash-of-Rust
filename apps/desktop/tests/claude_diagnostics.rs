//! Run the production binary's CLI without GUI/core/store initialization.
use std::process::Command;

#[test]
fn terminal_diagnostics_are_headless_and_do_not_export_credentials() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("report.json");
    let data = root.path().join("must-not-initialize");
    let result = Command::new(env!("CARGO_BIN_EXE_clash-of-rust"))
        .args(["--claude-diagnose", "--offline", "--output"])
        .arg(&output)
        .env("CLASH_OF_RUST_DATA_DIR", &data)
        .env(
            "HTTPS_PROXY",
            "http://fixture-user:fixture-secret@127.0.0.1:7890/path?key=fixture-secret",
        )
        .env("ANTHROPIC_API_KEY", "fixture-api-key-never-export")
        .env(
            "ANTHROPIC_BASE_URL",
            "https://api.anthropic.com/private?key=fixture-api-key-never-export",
        )
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let bytes = std::fs::read(&output).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(!text.contains("fixture-secret"));
    assert!(!text.contains("fixture-api-key-never-export"));
    assert!(!text.contains("fixture-user"));
    let report: clash_of_rust::claude_check::Report = serde_json::from_slice(&bytes).unwrap();
    report.validate().unwrap();
    assert_eq!(
        report.environment,
        clash_of_rust::claude_check::Environment::Code
    );
    assert!(
        !data.exists(),
        "CLI must not start the engine, tray or data store"
    );
}
