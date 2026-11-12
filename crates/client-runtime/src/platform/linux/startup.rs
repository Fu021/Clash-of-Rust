use super::*;
use serde_json::{Value, json};
use std::{io::Read, os::unix::fs::OpenOptionsExt};

const LIMIT: u64 = 16 * 1024;

pub(super) fn entry(executable: &Path) -> String {
    let executable = executable
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    format!(
        "[Desktop Entry]\nType=Application\nName=Clash of Rust\nExec=\"{executable}\" --background\nIcon=clash-of-rust\nStartupWMClass=clash-of-rust\nTerminal=false\n"
    )
}

fn read(path: &Path) -> Result<(String, std::fs::Metadata)> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= LIMIT,
        "自启动配置文件无效或过大"
    );
    let mut text = String::new();
    file.take(LIMIT + 1).read_to_string(&mut text)?;
    anyhow::ensure!(text.len() as u64 <= LIMIT, "自启动配置文件过大");
    Ok((text, metadata))
}

fn summarize(text: &str, executable: &Path) -> Value {
    let mut active = false;
    let mut commands = Vec::new();
    let mut hidden = false;
    let mut disabled = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            active = line == "[Desktop Entry]";
        } else if active {
            if let Some(command) = line.strip_prefix("Exec=") {
                commands.push(command);
            }
            hidden |= line == "Hidden=true";
            disabled |= line == "X-GNOME-Autostart-enabled=false";
        }
    }
    let expected = entry(executable);
    let expected = expected
        .lines()
        .find_map(|line| line.strip_prefix("Exec="))
        .unwrap();
    let managed = commands.as_slice() == [expected];
    json!({
        "state":"collected","exists":true,"entry_count":commands.len(),
        "hidden":hidden,"desktop_disabled":disabled,
        "background_argument":commands.first().is_some_and(|command|
            command.split_whitespace().any(|argument| argument == "--background")),
        "matches_current_launcher":managed,
        "legacy_managed_launcher":commands.as_slice() == [expected.trim_end_matches(" --background")],
    })
}

pub(super) fn diagnostics() -> Value {
    let result = (|| -> Result<Value> {
        let path = linux::startup_path()?;
        let (text, metadata) = read(&path)?;
        let mut report = summarize(&text, &autostart_executable()?);
        report["size"] = json!(metadata.len());
        report["modified_unix_ms"] = json!(
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|time| time.as_millis())
        );
        Ok(report)
    })();
    match result {
        Ok(report) => report,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            json!({"state":"collected","exists":false})
        }
        Err(_) => json!({"state":"unavailable","reason":"autostart_read_failed"}),
    }
}

pub(super) fn migrate() -> Result<()> {
    let path = linux::startup_path()?;
    if !path.try_exists()? {
        return Ok(());
    }
    migrate_at(&path, &autostart_executable()?)
}

fn migrate_at(path: &Path, executable: &Path) -> Result<()> {
    let (text, _) = read(path)?;
    let expected = entry(executable);
    if text == expected.replace(" --background\n", "\n") {
        atomic_write(path, expected.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_migration_only_changes_the_exact_managed_entry_and_preserves_custom_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("autostart.desktop");
        let executable = Path::new("/opt/clash-of-rust/clash-of-rust");
        let expected = entry(executable);
        std::fs::write(&path, expected.replace(" --background\n", "\n")).unwrap();
        migrate_at(&path, executable).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        for custom in [
            format!("{expected}Hidden=true\n"),
            expected.replace(" --background", " --custom"),
            expected.replace("/opt/", "/other/"),
            format!("{expected}Exec=other\n"),
        ] {
            std::fs::write(&path, &custom).unwrap();
            migrate_at(&path, executable).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), custom);
        }
    }

    #[test]
    fn diagnostics_report_flags_without_exporting_arbitrary_commands_and_reject_symlinks() {
        let executable = Path::new("/opt/clash-of-rust/clash-of-rust");
        let report = summarize(&entry(executable), executable);
        assert_eq!(report["background_argument"], true);
        assert_eq!(report["matches_current_launcher"], true);
        let text = "[Desktop Entry]\nExec=private-command --token private-token\nHidden=true\n";
        let report = summarize(text, executable);
        assert_eq!(report["background_argument"], false);
        assert_eq!(report["hidden"], true);
        assert!(!report.to_string().contains("private"));
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file");
        std::fs::write(&file, text).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(read(&link).is_err());
        std::fs::write(&file, vec![b'x'; LIMIT as usize + 1]).unwrap();
        assert!(read(&file).is_err());
    }
}
