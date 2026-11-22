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
fn help_uses_the_saved_language_and_accepts_a_read_only_override() {
    let root = tempfile::tempdir().unwrap();
    let settings = root.path().join("settings.json");
    let contents = r#"{"language":"zh-CN"}"#;
    std::fs::write(&settings, contents).unwrap();

    for (options, heading) in [(&[][..], "用法：cor "), (&["--lang=en"][..], "Usage: cor ")] {
        let result = cor()
            .args(["help", "profile", "add", "--data-dir"])
            .arg(root.path())
            .args(options)
            .output()
            .unwrap();
        assert!(result.status.success());
        assert!(String::from_utf8(result.stdout).unwrap().contains(heading));
    }
    assert_eq!(std::fs::read_to_string(settings).unwrap(), contents);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn usage_errors_follow_the_selected_language_without_initializing_the_backend() {
    let root = tempfile::tempdir().unwrap();
    let settings = root.path().join("settings.json");
    let contents = r#"{"language":"zh-CN"}"#;
    std::fs::write(&settings, contents).unwrap();
    for (arguments, expected) in [
        (vec!["unknown", "--lang=en"], "Unknown command: unknown"),
        (
            vec!["status", "--typo", "--lang=en"],
            "status does not support --typo",
        ),
        (
            vec!["mode", "invalid", "--lang=en"],
            "mode requires one of: rule / global / direct",
        ),
        (
            vec!["status", "--timeout=3601", "--lang=en"],
            "--timeout cannot exceed 3600",
        ),
        (vec!["unknown"], "未知命令：unknown"),
    ] {
        let result = cor()
            .args(arguments)
            .arg("--data-dir")
            .arg(root.path())
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2));
        let error = String::from_utf8(result.stderr).unwrap();
        assert!(error.contains(expected), "{error}");
    }
    assert_eq!(std::fs::read_to_string(settings).unwrap(), contents);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn english_cli_explains_confirmation_and_terminal_requirements() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("unused");
    for (arguments, expected) in [
        (
            vec!["restart", "--network-only"],
            "Refreshing core routes and DNS closes existing connections",
        ),
        (
            vec!["node", "select"],
            "interactive selection requires a terminal",
        ),
        (
            vec!["version", "--json", "--jsonl"],
            "--json and --jsonl cannot be combined",
        ),
    ] {
        let result = cor()
            .args(arguments)
            .args(["--lang=en", "--data-dir"])
            .arg(&data)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(!result.status.success());
        let error = String::from_utf8(result.stderr).unwrap();
        assert!(error.contains(expected), "{error}");
    }
    assert!(!data.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn help_reads_ordered_system_languages_on_first_install_and_upgrade() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let settings = data.join("settings.json");
    let legacy = r#"{"dark":true}"#;
    for (system, heading) in [
        ("fr-FR:zh_CN:en", "用法：cor "),
        ("zh-Hant-HK:en:zh_CN", "Usage: cor "),
    ] {
        let result = cor()
            .args(["help", "--data-dir"])
            .arg(&data)
            .env("LANGUAGE", system)
            .output()
            .unwrap();
        assert!(result.status.success());
        assert!(String::from_utf8(result.stdout).unwrap().contains(heading));
        if settings.exists() {
            assert_eq!(std::fs::read_to_string(&settings).unwrap(), legacy);
        } else {
            assert!(!data.exists());
            std::fs::create_dir(&data).unwrap();
            std::fs::write(&settings, legacy).unwrap();
        }
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
        .args([
            "restart",
            "--network-only",
            "--yes",
            "--lang=zh-CN",
            "--data-dir",
        ])
        .arg(&data)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("后台未运行"));
    assert!(!data.join("control.json").exists());
    assert!(!data.join("daemon.log").exists());
}
