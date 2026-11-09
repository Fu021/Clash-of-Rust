//! The console entry point must stay usable without a display or installed core.
use std::process::Command;
fn cor() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cor"));
    command.env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
    command
}
#[test]
fn help_version_completion_and_usage_do_not_initialize_a_backend() {
    let root = tempfile::tempdir().unwrap();
    for arguments in [
        vec!["help"],
        vec!["help", "rule", "add"],
        vec!["node", "help"],
        vec!["--json", "version"],
        vec!["completion", "bash"],
        vec!["__complete", "config", "set", "theme_mode", ""],
    ] {
        let result = cor()
            .args(arguments)
            .env("CLASH_OF_RUST_DATA_DIR", root.path().join("unused"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!result.stdout.is_empty());
        assert!(!root.path().join("unused").exists());
    }
    for arguments in [
        vec!["unknown"],
        vec!["mode", "rule", "extra"],
        vec!["node", "list", "--typo"],
    ] {
        let result = cor().args(arguments).output().unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
    }
}

#[test]
fn update_status_reads_the_result_without_spawning_a_backend() {
    let root = tempfile::tempdir().unwrap();
    for recorded in [false, true] {
        if recorded {
            std::fs::write(
                root.path().join("update-install.json"),
                serde_json::to_vec(&serde_json::json!({
                    "version":"0.6.7", "state":"succeeded", "message":"已更新至 0.6.7",
                    "error":false, "updated_at":123, "directory":root.path().join("expired"),
                }))
                .unwrap(),
            )
            .unwrap();
        }
        let output = cor()
            .args(["--json", "--data-dir"])
            .arg(root.path())
            .args(["update", "status"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            value["data"]["state"],
            if recorded { "succeeded" } else { "none" }
        );
        assert!(!root.path().join("control.json").exists());
        assert!(!root.path().join("daemon.log").exists());
    }
}

#[test]
fn network_only_restart_does_not_spawn_a_missing_backend() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let result = cor()
        .args(["restart", "--network-only", "--yes", "--data-dir"])
        .arg(&data)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("后台未运行"));
    assert!(!data.join("control.json").exists());
    assert!(!data.join("daemon.log").exists());
}
