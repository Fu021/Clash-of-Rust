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

#[tokio::test]
#[ignore = "requires the native desktop terminal; run on each platform's GUI CI"]
async fn native_terminal_diagnostic_returns_automatic_report() {
    use clash_of_rust::claude_check::{browser::Session, terminal};
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project ' 中文");
    std::fs::create_dir_all(project.join(".claude")).unwrap();
    std::fs::write(
        project.join(".claude/settings.json"),
        r#"{"env":{"LANG":"en_US.UTF-8"}}"#,
    )
    .unwrap();
    let session = Session::start(clash_of_rust::claude_check::client(None).unwrap())
        .await
        .unwrap();
    let submit = session.submit_url();
    let _launch = terminal::launch(terminal::Request {
        executable: std::path::Path::new(env!("CARGO_BIN_EXE_clash-of-rust")),
        submit: &submit,
        directory: project.to_str().unwrap(),
        offline: true,
        keep_open: false,
    })
    .await
    .unwrap();
    let report = tokio::time::timeout(std::time::Duration::from_secs(45), async {
        loop {
            if let Some(report) = session.next_report() {
                break report;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("The native terminal did not return a diagnostic report");
    report.validate().unwrap();
    assert!(report.context.contains("自动新建终端"));
    assert!(
        report
            .signals
            .iter()
            .any(|s| s.title == "配置 LANG" && s.value == "en_US.UTF-8"),
        "Terminal must run in the requested project directory"
    );
    assert!(
        report
            .signals
            .iter()
            .any(|s| s.title == "采样入口" && s.value == "自动新建终端会话")
    );
}
