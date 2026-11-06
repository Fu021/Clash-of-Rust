use crate::{args::Args, backend, output};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const JSON_LIMIT: u64 = 1024 * 1024;
const LOG_LIMIT: u64 = 64 * 1024;

pub(crate) fn failure_hint(args: Option<&Args>) {
    static SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if SHOWN.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if args.is_some_and(|args| args.path == "diagnose" && args.get("output").is_some()) {
        eprintln!("反馈问题时，请提供刚保存的诊断报告和具体操作；分享前请检查敏感信息。");
    } else {
        eprintln!(
            "反馈此问题时，请在相同用户和数据目录下运行 cor diagnose --output cor-diagnose.json，并提供报告与具体操作；分享前请检查敏感信息。"
        );
        eprintln!(
            "后台不可达或需先保留现场时：cor diagnose --offline --output cor-diagnose.json。指定过 --user / --data-dir / --resources 时请沿用相同参数。"
        );
    }
}

pub(crate) fn has_failure(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            (matches!(
                key.as_str(),
                "error" | "Err" | "backend_error" | "local_evidence_error"
            ) && !value.is_null()
                && value.as_str() != Some(""))
                || (matches!(key.as_str(), "state" | "status")
                    && value.as_str().is_some_and(|state| {
                        matches!(
                            state,
                            "failed" | "Failed" | "error" | "unavailable" | "withheld"
                        )
                    }))
                || (key == "status" && value.as_u64().is_some_and(|status| status >= 400))
                || has_failure(value)
        }),
        Value::Array(items) => items.iter().any(has_failure),
        _ => false,
    }
}

fn open_regular(path: &Path) -> Result<File> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(metadata.is_file(), "不是普通文件，不读取链接或特殊文件");
    let mut options = File::options();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    ensure!(file.metadata()?.is_file(), "文件类型发生变化");
    Ok(file)
}

fn read_json(path: &Path) -> Result<Value> {
    let file = open_regular(path)?;
    ensure!(file.metadata()?.len() <= JSON_LIMIT, "超过 1 MiB 读取上限");
    let mut bytes = Vec::new();
    file.take(JSON_LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= JSON_LIMIT, "超过 1 MiB 读取上限");
    Ok(serde_json::from_slice(&bytes)?)
}

fn failure(error: anyhow::Error) -> Value {
    let missing = error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|cause| cause.kind() == std::io::ErrorKind::NotFound)
    });
    json!({"state":if missing {"missing"} else {"unavailable"},"error":format!("{error:#}")})
}

fn collect_secrets(value: &Value, secrets: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if (output::sensitive_field(key) || key == "source")
                    && let Some(secret) = value.as_str().filter(|secret| !secret.is_empty())
                {
                    secrets.push(secret.into());
                } else {
                    collect_secrets(value, secrets);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_secrets(item, secrets);
            }
        }
        _ => {}
    }
}

fn sanitize_log(contents: &str, secrets: &[String]) -> String {
    let mut sanitized = contents.to_owned();
    for secret in secrets {
        sanitized = sanitized.replace(secret, "[已隐藏]");
    }
    sanitized
        .lines()
        .map(|line| {
            let normalized = line.to_ascii_lowercase();
            if [
                "authorization",
                "bearer ",
                "cookie:",
                "password",
                "secret",
                "token=",
                "token:",
                "api_key",
                "private_key",
            ]
            .iter()
            .any(|marker| normalized.contains(marker))
            {
                return "[已隐藏含凭据的日志行]".to_owned();
            }
            line.split_inclusive(char::is_whitespace)
                .map(|word| {
                    let start = word.find("https://").or_else(|| word.find("http://"));
                    if let Some(start) = start {
                        format!(
                            "{}{}",
                            &word[..start],
                            clash_client_runtime::claude_check::system::redact_url(
                                word[start..].trim_end()
                            )
                        )
                    } else {
                        word.to_owned()
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn core_logs(lines: Vec<String>, meta: &clash_client_runtime::control::Meta) -> Value {
    let mut secrets = Vec::new();
    let Ok(meta) = serde_json::to_value(meta) else {
        return json!({"state":"withheld","reason":"credential_redaction_unavailable"});
    };
    collect_secrets(&meta, &mut secrets);
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    secrets.dedup();
    let contents = lines.join("\n");
    let mut offset = contents.len().saturating_sub(LOG_LIMIT as usize);
    while !contents.is_char_boundary(offset) {
        offset += 1;
    }
    json!({
        "state":"collected","truncated":offset > 0,
        "contents":sanitize_log(&contents[offset..], &secrets),
        "note":"仅为当前最多 500 条、单条最多 1000 字符的缓冲，不含已淘汰的历史日志"
    })
}

fn daemon_log(root: &Path, secrets: &[String], redaction_ready: bool) -> Value {
    if !redaction_ready {
        return json!({"state":"withheld","reason":"credential_files_unreadable"});
    }
    let result = (|| -> Result<Value> {
        let mut file = open_regular(&root.join("daemon.log"))?;
        let size = file.metadata()?.len();
        let offset = size.saturating_sub(LOG_LIMIT);
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.take(LOG_LIMIT).read_to_end(&mut bytes)?;
        if offset > 0 {
            bytes = bytes
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|newline| bytes[newline + 1..].to_vec())
                .unwrap_or_default();
        }
        Ok(json!({
            "state":"collected","size":size,"truncated":offset > 0,
            "contents":sanitize_log(&String::from_utf8_lossy(&bytes), secrets)
        }))
    })();
    result.unwrap_or_else(failure)
}

fn file_inventory(directory: &Path, limit: usize) -> Value {
    let result = (|| -> Result<Value> {
        let mut entries = Vec::new();
        let mut truncated = false;
        for entry in std::fs::read_dir(directory)? {
            if entries.len() == limit {
                truncated = true;
                break;
            }
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            entries.push(json!({
                "name":entry.file_name().to_string_lossy(),
                "size":metadata.len(),
                "kind":if metadata.is_file() {"file"} else if metadata.is_dir() {"directory"} else {"other"},
                "modified_unix":metadata.modified().ok().and_then(|time|time.duration_since(UNIX_EPOCH).ok()).map(|time|time.as_secs())
            }));
        }
        entries.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        Ok(json!({"state":"collected","truncated":truncated,"files":entries}))
    })();
    result.unwrap_or_else(failure)
}

fn evidence(root: &Path) -> Value {
    let mut secrets = Vec::new();
    let mut redaction_ready = true;
    let mut credential_files = serde_json::Map::new();
    for name in ["settings.json", "control.json", "profiles.json"] {
        match read_json(&root.join(name)) {
            Ok(value) => {
                collect_secrets(&value, &mut secrets);
                credential_files.insert(name.into(), json!({"state":"read_for_redaction"}));
            }
            Err(error) => {
                let state = failure(error);
                redaction_ready &= state["state"] == "missing";
                credential_files.insert(name.into(), state);
            }
        }
    }
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    secrets.dedup();
    let runtime = root.join("runtime");
    let inventory = file_inventory(&runtime, 128);
    let mut recovery = serde_json::Map::new();
    let mut geo = serde_json::Map::new();
    for entry in inventory["files"].as_array().into_iter().flatten() {
        if entry["kind"] == "file"
            && let Some(name) = entry["name"].as_str()
            && name.starts_with(".geo-backup-")
            && name.ends_with(".committed")
        {
            geo.insert(name.into(), json_record(&runtime.join(name)));
        }
        if entry["kind"] == "directory"
            && let Some(name) = entry["name"].as_str()
            && (name.starts_with(".geo-backup-") || name.starts_with(".geo-recovery-"))
        {
            let directory = runtime.join(name);
            if std::fs::symlink_metadata(&directory).is_ok_and(|metadata| metadata.is_dir()) {
                let mut files = file_inventory(&directory, 32);
                if name.starts_with(".geo-recovery-") {
                    files["transaction"] = json_record(&directory.join("geo-transaction.json"));
                    files["manifest"] = json_record(&directory.join("geodata.json"));
                }
                recovery.insert(name.into(), files);
            }
        }
    }
    for name in ["geo-transaction.json", "geodata.json"] {
        geo.insert(name.into(), json_record(&runtime.join(name)));
    }
    json!({
        "credential_files":credential_files,
        "daemon_log":daemon_log(root, &secrets, redaction_ready),
        "runtime_files":inventory,"geo_records":geo,"geo_recovery_files":recovery,
        "limits":{"daemon_log_bytes":LOG_LIMIT,"json_file_bytes":JSON_LIMIT,"runtime_entries":128,"recovery_entries":32},
        "note":"只读快照，采集中状态可能变化；日志已尽力脱敏，分享前仍须检查域名、IP、路径和第三方返回内容。未读取订阅正文或输出设置/控制凭据。"
    })
}

fn json_record(path: &Path) -> Value {
    read_json(path)
        .map(|value| json!({"state":"collected","data":output::redact(value)}))
        .unwrap_or_else(failure)
}

pub(crate) fn validate_output(args: &Args) -> Result<()> {
    if let Some(path) = args.get("output") {
        let output_path = backend::absolute(path)?;
        let parent = output_path
            .parent()
            .context("无效报告路径")?
            .canonicalize()
            .context("诊断报告的父目录必须已经存在")?;
        let root = backend::data_root(args)?;
        if let Ok(root) = root.canonicalize() {
            ensure!(
                !parent.starts_with(root),
                "诊断报告须保存到应用数据目录之外，避免覆盖排障现场"
            );
        }
    }
    Ok(())
}

pub(crate) async fn collect(args: &Args) -> Result<Value> {
    let root = backend::data_root(args)?;
    let explicit_resources = args.get("resources").map(backend::absolute).transpose()?;
    tokio::task::spawn_blocking(move || {
        let resources = explicit_resources
            .map(Ok)
            .unwrap_or_else(clash_client_runtime::assets::discover);
        json!({
            "version":clash_client_core::VERSION,
            "os":std::env::consts::OS,"arch":std::env::consts::ARCH,
            "captured_at":chrono::Utc::now().to_rfc3339(),
            "executable":std::env::current_exe().ok(),
            "data_directory":root,
            "resources":resources.map(|path: PathBuf|path.to_string_lossy().into_owned()).map_err(|error|format!("{error:#}")),
            "local_evidence":evidence(&root)
        })
    })
    .await
    .context("本地诊断采集任务失败")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_failures_are_actionable_without_treating_unknown_routes_as_errors() {
        assert!(has_failure(
            &json!([{"name":"example","updated":false,"error":"HTTP 503"}])
        ));
        assert!(has_failure(&json!({"system_dns":{"Err":"lookup failed"}})));
        assert!(has_failure(&json!({"http":{"status":403}})));
        assert!(!has_failure(
            &json!({"error":null,"ipv6":{"state":"unknown"}})
        ));
        assert!(!has_failure(
            &json!({"state":"collected","http":{"status":200}})
        ));
    }

    #[test]
    fn offline_evidence_does_not_initialize_or_recover_a_data_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("missing-data");
        let value = evidence(&root);
        assert_eq!(value["runtime_files"]["state"], "missing");
        assert!(!root.exists());
    }

    #[test]
    fn exports_cannot_replace_files_in_the_diagnosed_data_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let runtime = temporary.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let target = runtime.join("geo-transaction.json");
        std::fs::write(&target, b"preserve-this-evidence").unwrap();
        let args = crate::args::parse(vec![
            "diagnose".into(),
            "--offline".into(),
            "--data-dir".into(),
            temporary.path().to_string_lossy().into_owned(),
            "--output".into(),
            target.to_string_lossy().into_owned(),
        ])
        .unwrap();
        assert!(validate_output(&args).is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"preserve-this-evidence");
    }

    #[test]
    fn report_preserves_geo_state_and_masks_known_secrets_and_subscription_urls() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        std::fs::create_dir(root.join("runtime")).unwrap();
        std::fs::write(
            root.join("settings.json"),
            br#"{"secret":"local-controller-credential"}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("profiles.json"),
            br#"[{"source":"https://subscription.test/private-path"}]"#,
        )
        .unwrap();
        std::fs::write(
            root.join("control.json"),
            br#"{"token":"control-credential"}"#,
        )
        .unwrap();
        let journal = br#"{"backup":".geo-backup-absent","files":[]}"#;
        std::fs::write(root.join("runtime/geo-transaction.json"), journal).unwrap();
        std::fs::write(root.join("daemon.log"), "failed: local-controller-credential control-credential\nfetch https://subscription.test/private-path\nfailed https://user:example-password@example.test/get?key=abc\nAuthorization: Bearer other-value\nGeo backup missing\n").unwrap();
        let value = evidence(root);
        let serialized = value.to_string();
        for secret in [
            "local-controller-credential",
            "control-credential",
            "private-path",
            "example-password",
            "key=abc",
            "other-value",
        ] {
            assert!(!serialized.contains(secret), "{serialized}");
        }
        assert!(serialized.contains("Geo backup missing"));
        assert_eq!(
            std::fs::read(root.join("runtime/geo-transaction.json")).unwrap(),
            journal
        );
        assert_eq!(
            value["geo_records"]["geo-transaction.json"]["state"],
            "collected"
        );
    }

    #[test]
    fn log_capture_is_bounded_and_unreadable_credentials_withhold_the_log() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        std::fs::write(
            root.join("daemon.log"),
            format!("{}\nlast line\n", "x".repeat(LOG_LIMIT as usize)),
        )
        .unwrap();
        let value = evidence(root);
        assert_eq!(value["daemon_log"]["truncated"], true);
        assert_eq!(value["daemon_log"]["contents"], "last line");
        std::fs::write(root.join("settings.json"), b"broken settings").unwrap();
        let value = evidence(root);
        assert_eq!(value["daemon_log"]["state"], "withheld");
        assert!(value["daemon_log"]["contents"].is_null());
    }

    #[cfg(unix)]
    #[test]
    fn evidence_does_not_follow_a_log_symlink() {
        let temporary = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/etc/passwd", temporary.path().join("daemon.log")).unwrap();
        assert_eq!(
            evidence(temporary.path())["daemon_log"]["state"],
            "unavailable"
        );
    }
}
