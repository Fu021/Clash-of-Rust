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
        vec!["--json", "version"],
        vec!["completion", "bash"],
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
