use super::*;
use serde_json::{Value, json};
use std::{io::Read, os::unix::fs::OpenOptionsExt};

const LIMIT: u64 = 16 * 1024;

pub(super) fn entry(executable: &Path) -> String {
    let executable = launcher(executable);
    format!(
        "[Desktop Entry]\nType=Application\nName=Clash of Rust\nExec={executable} --background\nIcon=clash-of-rust\nStartupWMClass=clash-of-rust\nTerminal=false\n"
    )
}

fn launcher(executable: &Path) -> String {
    let executable = executable
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    format!("\"{executable}\"")
}

fn matches_executable(command: &str, executable: &Path) -> bool {
    if command == launcher(executable) {
        return true;
    }
    let command = command
        .strip_prefix('"')
        .and_then(|command| command.strip_suffix('"'))
        .unwrap_or(command);
    if command
        .chars()
        .any(|character| character.is_whitespace() || "\"'\\`$%><~|&;*?#()".contains(character))
    {
        return false;
    }
    let path = Path::new(command);
    if !path.is_absolute() {
        return false;
    }
    path == executable
        || path
            .canonicalize()
            .ok()
            .zip(executable.canonicalize().ok())
            .is_some_and(|(actual, expected)| actual == expected && expected.is_file())
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
    let command = match commands.as_slice() {
        [command] => Some(*command),
        _ => None,
    };
    json!({
        "state":"collected","exists":true,"entry_count":commands.len(),
        "hidden":hidden,"desktop_disabled":disabled,
        "background_argument":commands.first().is_some_and(|command|
            command.split_whitespace().any(|argument| argument == "--background")),
        "matches_current_launcher":command.and_then(|command| command.strip_suffix(" --background"))
            .is_some_and(|command| matches_executable(command, executable)),
        "legacy_managed_launcher":command.is_some_and(|command| matches_executable(command, executable)),
    })
}

fn diagnostics_at(path: &Path, executable: &Path) -> Value {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return json!({"state":"collected","exists":false,"symlink":false});
        }
        Err(_) => return json!({"state":"unavailable","reason":"autostart_read_failed"}),
    };
    let symlink = metadata.file_type().is_symlink();
    let mut report = json!({
        "state":"unavailable","reason":"autostart_read_failed",
        "exists":true,"symlink":symlink,"link_resolved":Value::Null,
    });
    let target = if symlink {
        match path.canonicalize() {
            Ok(target) => {
                report["link_resolved"] = json!(true);
                target
            }
            Err(_) => {
                report["reason"] = json!("autostart_link_unresolved");
                report["link_resolved"] = json!(false);
                return report;
            }
        }
    } else {
        path.to_path_buf()
    };
    if let Ok((text, metadata)) = read(&target) {
        let summary = summarize(&text, executable);
        report.as_object_mut().unwrap().remove("reason");
        report
            .as_object_mut()
            .unwrap()
            .extend(summary.as_object().unwrap().clone());
        report["size"] = json!(metadata.len());
        report["modified_unix_ms"] = json!(
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|time| time.as_millis())
        );
    }
    report
}

pub(super) fn diagnostics() -> Value {
    match (linux::startup_path(), autostart_executable()) {
        (Ok(path), Ok(executable)) => diagnostics_at(&path, &executable),
        _ => json!({"state":"unavailable","reason":"autostart_read_failed"}),
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
    let metadata = std::fs::symlink_metadata(path)?;
    let target = if metadata.file_type().is_symlink() {
        path.canonicalize()?
    } else {
        path.to_path_buf()
    };
    let (text, _) = read(&target)?;
    let expected = entry(executable);
    if text == expected.replace(" --background\n", "\n") || packaged_menu_entry(&text, executable) {
        atomic_write(path, expected.as_bytes())?;
    }
    Ok(())
}

fn packaged_menu_entry(text: &str, executable: &Path) -> bool {
    let mut fields = std::collections::BTreeSet::new();
    let mut active = false;
    let mut command = None;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[Desktop Entry]" && !active {
            active = true;
            continue;
        }
        if !active {
            return false;
        }
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        if !fields.insert(key) {
            return false;
        }
        match (key, value) {
            ("Exec", value) => command = Some(value),
            ("Type", "Application")
            | ("Name", "Clash of Rust")
            | ("Comment", "Native desktop client for mihomo")
            | ("Icon", "clash-of-rust")
            | ("StartupWMClass", "clash-of-rust")
            | ("Terminal", "false")
            | ("Categories", "Network;") => {}
            _ => return false,
        }
    }
    fields.contains("Type")
        && fields.contains("Name")
        && command.is_some_and(|command| matches_executable(command, executable))
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
    fn diagnostics_report_flags_without_exporting_arbitrary_commands() {
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
        let report = diagnostics_at(&link, executable);
        assert_eq!(report["state"], "collected");
        assert_eq!(report["symlink"], true);
        assert_eq!(report["link_resolved"], true);
        assert_eq!(report["legacy_managed_launcher"], false);
        assert!(!report.to_string().contains("private"));
        std::fs::write(&file, vec![b'x'; LIMIT as usize + 1]).unwrap();
        assert!(read(&file).is_err());
        assert_eq!(diagnostics_at(&link, executable)["state"], "unavailable");
    }

    #[test]
    fn menu_link_migration_replaces_only_the_link_and_accepts_the_installed_executable_alias() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("clash-of-rust");
        std::fs::write(&executable, b"fixture").unwrap();
        let alias = root.path().join("launcher");
        symlink(&executable, &alias).unwrap();
        let menu = root.path().join("menu.desktop");
        let contents = format!(
            "[Desktop Entry]\nType=Application\nName=Clash of Rust\nComment=Native desktop client for mihomo\nExec={}\nIcon=clash-of-rust\nStartupWMClass=clash-of-rust\nTerminal=false\nCategories=Network;\n",
            alias.display()
        );
        std::fs::write(&menu, &contents).unwrap();
        let path = root.path().join("autostart.desktop");
        symlink("menu.desktop", &path).unwrap();
        let before = diagnostics_at(&path, &executable);
        assert_eq!(before["symlink"], true);
        assert_eq!(before["link_resolved"], true);
        assert_eq!(before["background_argument"], false);
        assert_eq!(before["legacy_managed_launcher"], true);
        migrate_at(&path, &executable).unwrap();
        assert!(
            !std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&menu).unwrap(), contents);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), entry(&executable));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            diagnostics_at(&path, &executable)["background_argument"],
            true
        );
        migrate_at(&path, &executable).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), entry(&executable));
        std::fs::write(&path, &contents).unwrap();
        migrate_at(&path, &executable).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), entry(&executable));
    }

    #[test]
    fn migration_preserves_custom_disabled_foreign_and_invalid_links() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("clash-of-rust");
        std::fs::write(&executable, b"fixture").unwrap();
        let expected = entry(&executable);
        let legacy = expected.replace(" --background\n", "\n");
        let menu = root.path().join("menu.desktop");
        let path = root.path().join("autostart.desktop");
        std::os::unix::fs::symlink(&menu, &path).unwrap();
        for contents in [
            expected.clone(),
            format!("{legacy}Hidden=true\n"),
            format!("{legacy}X-GNOME-Autostart-enabled=false\n"),
            legacy.replace("\nExec=", "\nExec=other\nExec="),
            expected.replace(" --background", " --custom"),
            legacy.replace("Name=Clash of Rust", "Name=Custom"),
            legacy.replace("Terminal=false", "Terminal=true"),
            legacy.replace("Exec=\"", "Exec=\"/other"),
            format!("{legacy}[Desktop Action custom]\nExec=other\n"),
        ] {
            std::fs::write(&menu, &contents).unwrap();
            migrate_at(&path, &executable).unwrap();
            assert!(
                std::fs::symlink_metadata(&path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(std::fs::read_to_string(&menu).unwrap(), contents);
        }
        std::fs::remove_file(&menu).unwrap();
        assert!(migrate_at(&path, &executable).is_err());
        let report = diagnostics_at(&path, &executable);
        assert_eq!(report["exists"], true);
        assert_eq!(report["symlink"], true);
        assert_eq!(report["link_resolved"], false);
        assert_eq!(report["reason"], "autostart_link_unresolved");
        assert!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn executable_matching_does_not_accept_extra_arguments_or_desktop_field_codes() {
        let executable = Path::new("/opt/Clash $Rust/clash-of-rust");
        let expected = entry(executable);
        assert!(packaged_menu_entry(
            &expected.replace(" --background\n", "\n"),
            executable
        ));
        assert!(!packaged_menu_entry(&expected, executable));
        let executable = Path::new("/opt/clash-of-rust/clash-of-rust");
        for command in [
            "clash-of-rust",
            "/opt/clash-of-rust/clash-of-rust %U",
            "/opt/clash-of-rust/clash-of-rust --custom",
            "/opt/clash-of-rust/clash-of-rust;other",
            "\"/opt/clash-of-rust/clash-of-rust\" --custom",
        ] {
            assert!(!matches_executable(command, executable), "{command}");
        }
    }
}
