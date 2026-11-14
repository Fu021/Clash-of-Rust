use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::UNIX_EPOCH,
};

const JSON_LIMIT: u64 = 1024 * 1024;
const LOG_LIMIT: u64 = 64 * 1024;

pub fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn incident(message: &str, critical: bool) -> Value {
    json!({"error":message,"time":timestamp(),"level":if critical {"critical"} else {"recoverable"}})
}

pub fn ip_results(results: &[crate::ip_report::ResultSlot]) -> Value {
    let services = crate::ip_check::services();
    Value::Array(
        results
            .iter()
            .enumerate()
            .map(|(index, result)| {
                let mut row = match result {
                    Some(Ok(result)) => json!({
                        "state":format!("{:?}",result.state),"summary":result.summary,
                        "country":result.country,"millis":result.millis,"detail":result.detail
                    }),
                    Some(Err(error)) => json!({"state":"failed","error":error}),
                    None => json!({"state":"pending"}),
                };
                row["service"] = json!(services.get(index).map(|service| &service.id));
                row
            })
            .collect(),
    )
}

pub fn has_failure(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            (matches!(
                key.as_str(),
                "error" | "Err" | "backend_error" | "local_evidence_error"
            ) && !value.is_null()
                && value.as_bool() != Some(false)
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
                if (sensitive_field(key) || key == "source")
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
                            crate::claude_check::preferences::redact_url(word[start..].trim_end())
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

pub fn core_logs(lines: Vec<String>, credentials: &Value) -> Value {
    let mut secrets = Vec::new();
    collect_secrets(credentials, &mut secrets);
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
                files["manifest"] = json_record(&directory.join("geodata.json"));
                if name.starts_with(".geo-recovery-") {
                    files["transaction"] = json_record(&directory.join("geo-transaction.json"));
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
        "update_installation":json_record(&root.join("update-install.json")),
        "desktop_startup":json_record(&root.join("desktop-startup.json")),
        "previous_desktop_startup":json_record(&root.join("desktop-startup.previous.json")),
        "runtime_files":inventory,"geo_records":geo,"geo_recovery_files":recovery,
        "limits":{"daemon_log_bytes":LOG_LIMIT,"json_file_bytes":JSON_LIMIT,"runtime_entries":128,"recovery_entries":32},
        "note":"只读快照，采集中状态可能变化；日志已尽力脱敏，分享前仍须检查域名、IP、路径和第三方返回内容。未读取订阅正文或输出设置/控制凭据。"
    })
}

fn json_record(path: &Path) -> Value {
    read_json(path)
        .map(|value| json!({"state":"collected","data":redact(value)}))
        .unwrap_or_else(failure)
}

fn tun_configuration(root: &Path) -> Value {
    const LIMIT: u64 = 4 * 1024 * 1024;
    let path = root.join("runtime/config.yaml");
    let file = match open_regular(&path) {
        Ok(file) => file,
        Err(error) => {
            let missing = error.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|cause| cause.kind() == std::io::ErrorKind::NotFound)
            });
            return if missing {
                json!({"state":"not_generated"})
            } else {
                json!({"state":"unavailable","reason":"read_failed"})
            };
        }
    };
    if !file
        .metadata()
        .is_ok_and(|metadata| metadata.len() <= LIMIT)
    {
        return json!({"state":"unavailable","reason":"file_invalid_or_too_large"});
    }
    let mut raw = String::new();
    if file.take(LIMIT + 1).read_to_string(&mut raw).is_err() {
        return json!({"state":"unavailable","reason":"read_failed"});
    }
    if raw.len() as u64 > LIMIT {
        return json!({"state":"unavailable","reason":"file_invalid_or_too_large"});
    }
    parse_tun_configuration(&raw)
}

fn parse_tun_configuration(raw: &str) -> Value {
    let Ok(config) = yaml_serde::from_str::<Value>(raw) else {
        return json!({"state":"unavailable","reason":"invalid_runtime_config"});
    };
    let tun = &config["tun"];
    if !tun.is_object() {
        return json!({"state":"unavailable","reason":"invalid_tun_config"});
    }
    let dns = &config["dns"];
    json!({
        "state":"observed",
        "source":"generated_runtime_config",
        "ipv6_enabled":config["ipv6"],
        "inet4_address":tun["inet4-address"],
        "inet6_address":tun["inet6-address"],
        "stack":tun["stack"],
        "auto_route":tun["auto-route"],
        "auto_redirect":tun["auto-redirect"],
        "auto_detect_interface":tun["auto-detect-interface"],
        "strict_route":tun["strict-route"],
        "mtu":tun["mtu"],
        "route_address":tun["route-address"],
        "inet4_route_address":tun["inet4-route-address"],
        "inet6_route_address":tun["inet6-route-address"],
        "route_exclude_address":tun["route-exclude-address"],
        "include_uid":tun["include-uid"],
        "include_uid_range":tun["include-uid-range"],
        "exclude_uid":tun["exclude-uid"],
        "exclude_uid_range":tun["exclude-uid-range"],
        "include_interface":tun["include-interface"],
        "exclude_interface":tun["exclude-interface"],
        "dns_hijack":tun["dns-hijack"],
        "dns_enhanced_mode":dns["enhanced-mode"],
        "fake_ip_range":dns["fake-ip-range"]
    })
}

pub fn collect(root: &Path) -> Value {
    json!({
        "version":crate::VERSION,
        "os":std::env::consts::OS,"arch":std::env::consts::ARCH,
        "captured_at":timestamp(),
        "executable":std::env::current_exe().ok(),
        "data_directory":root,
        "local_evidence":evidence(root),
        "tun_configuration":tun_configuration(root)
    })
}

pub fn issue_report(root: &Path, context: Value, lines: Vec<String>) -> Value {
    let mut report = collect(root);
    let credentials =
        ["settings.json", "control.json", "profiles.json"].map(|name| read_json(&root.join(name)));
    if credentials.iter().any(|result| {
        result.as_ref().is_err_and(|error| {
            !error.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|cause| cause.kind() == std::io::ErrorKind::NotFound)
            })
        })
    }) {
        report["context"] = json!({"state":"withheld","reason":"credential_files_unreadable"});
        report["core_logs"] = json!({"state":"withheld","reason":"credential_files_unreadable"});
    } else {
        let credentials = Value::Array(credentials.into_iter().filter_map(Result::ok).collect());
        let mut secrets = Vec::new();
        collect_secrets(&credentials, &mut secrets);
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        secrets.dedup();
        report["context"] = sanitize_value(redact(context), &secrets);
        report["core_logs"] = core_logs(lines, &credentials);
    }
    report["sharing"] = json!({
        "issue_url":"https://github.com/Fu021/Clash-of-Rust/issues",
        "instructions":"附上报告、操作步骤和发生时间；分享前检查域名、IP、路径及第三方内容。",
        "scope":"本地有界快照；不含完整历史日志，未自动上传或尝试修复故障。"
    });
    json!({"schema":1,"command":"diagnose","data":report})
}

pub fn issue_draft_url(report: &Value) -> String {
    let data = &report["data"];
    let context = &data["context"];
    let original_error = context["error"]
        .as_str()
        .unwrap_or("See the exported diagnostic report.");
    let mut original_error = original_error.to_owned();
    for (field, replacement) in [
        ("data_directory", "<data-dir>"),
        ("executable", "<executable>"),
    ] {
        if let Some(path) = data[field].as_str().filter(|path| !path.is_empty()) {
            original_error = original_error.replace(path, replacement);
        }
    }
    let original_length = original_error.chars().count();
    let mut error: String = original_error.chars().take(500).collect();
    loop {
        let summary = json!({
            "version":data["version"].as_str().map(|text|text.chars().take(80).collect::<String>()),
            "os":data["os"].as_str().map(|text|text.chars().take(40).collect::<String>()),
            "arch":data["arch"].as_str().map(|text|text.chars().take(40).collect::<String>()),
            "captured_at":data["captured_at"].as_str().map(|text|text.chars().take(80).collect::<String>()),
            "error":error,
            "error_truncated":error.chars().count()<original_length,
            "running":context["running"].as_bool().or(context["core_running"].as_bool()),
            "context_state":context["state"].as_str().map(|text|text.chars().take(80).collect::<String>()),
        });
        let body = format!(
            "## What happened?\n\n\n## Steps to reproduce\n\n1. \n\n\
             ## Automatic diagnostic summary\n\n```json\n{summary:#}\n```\n\n\
             Review this summary before submitting. Attach the full JSON from Export diagnostics \
             if needed; check it for sensitive data first.\n"
        );
        let mut url =
            reqwest::Url::parse("https://github.com/Fu021/Clash-of-Rust/issues/new").unwrap();
        url.query_pairs_mut().append_pair("body", &body);
        if url.as_str().len() <= 6500 || error.is_empty() {
            return url.to_string();
        }
        error.pop();
    }
}

fn sanitize_value(value: Value, secrets: &[String]) -> Value {
    match value {
        Value::String(text) => Value::String(sanitize_log(&text, secrets)),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| sanitize_value(value, secrets))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, sanitize_value(value, secrets)))
                .collect(),
        ),
        other => other,
    }
}

pub fn sensitive_field(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace('-', "_");
    [
        "password",
        "secret",
        "token",
        "uuid",
        "private_key",
        "ip_reputation_key",
        "authorization",
        "auth",
        "psk",
        "preshared_key",
        "api_key",
    ]
    .iter()
    .any(|suffix| normalized == *suffix || normalized.ends_with(&format!("_{suffix}")))
}

pub fn redact(mut value: Value) -> Value {
    match &mut value {
        Value::Object(map) => {
            for (key, field) in map {
                let normalized = key.to_ascii_lowercase().replace('-', "_");
                if sensitive_field(key) {
                    if !field.is_null() {
                        *field = Value::String("[已隐藏]".into());
                    }
                } else if matches!(normalized.as_str(), "source" | "url")
                    && field.as_str().is_some_and(|text| {
                        text.starts_with("http://") || text.starts_with("https://")
                    })
                {
                    *field = Value::String(crate::claude_check::preferences::redact_url(
                        field.as_str().unwrap(),
                    ));
                } else {
                    *field = redact(std::mem::take(field));
                }
            }
        }
        Value::Array(items) => {
            for field in items {
                *field = redact(std::mem::take(field));
            }
        }
        _ => {}
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incident_level_is_explicit_and_never_inferred_from_its_text() {
        assert_eq!(
            incident("内核启动失败的历史报告已保留", false)["level"],
            "recoverable"
        );
        assert_eq!(incident("连接已终止", true)["level"], "critical");
    }

    #[test]
    fn report_keeps_partial_results_failures_and_pending_distinct() {
        let results = vec![
            Some(Ok(crate::ip_check::CheckResult {
                state: crate::ip_check::State::Partial,
                summary: "partial".into(),
                country: String::new(),
                millis: 12,
                detail: "sampling incomplete".into(),
            })),
            Some(Err("connection timed out".into())),
            None,
        ];
        let report = ip_results(&results);
        assert_eq!(report[0]["state"], "Partial");
        assert_eq!(report[1]["error"], "connection timed out");
        assert_eq!(report[2]["state"], "pending");
        assert!(!report[0]["service"].is_null());
    }

    #[test]
    fn update_success_with_false_error_flag_does_not_request_a_diagnostic_report() {
        assert!(!has_failure(&json!({
            "state":"succeeded","version":"0.7.1","error":false
        })));
        assert!(!has_failure(&json!([{"result":{"error":false}}])));
        assert!(has_failure(&json!({"error":true})));
        assert!(has_failure(&json!({"state":"failed","error":false})));
        assert!(has_failure(&json!({"error":"installation failed"})));
    }

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
    fn tun_diagnostic_keeps_address_families_and_drops_resolvers_and_credentials() {
        let temporary = tempfile::tempdir().unwrap();
        std::fs::create_dir(temporary.path().join("runtime")).unwrap();
        std::fs::write(
            temporary.path().join("runtime/config.yaml"),
            "ipv6: true\nsecret: private-controller-key\nproxies: [{name: private-node, password: private-password}]\ntun:\n  stack: mips\n  auto-route: true\n  strict-route: false\n  inet6-address: []\n  route-address: [0.0.0.0/1, 128.0.0.0/1]\n  dns-hijack: [any:53]\ndns:\n  enhanced-mode: fake-ip\n  fake-ip-range: 198.18.0.1/16\n  nameserver: ['https://private.example/dns-query']\n",
        )
        .unwrap();
        let report = collect(temporary.path());
        let tun = &report["tun_configuration"];
        assert_eq!(tun["state"], "observed");
        assert_eq!(tun["source"], "generated_runtime_config");
        assert_eq!(tun["ipv6_enabled"], true);
        assert_eq!(tun["inet6_address"], json!([]));
        assert_eq!(tun["auto_route"], true);
        assert_eq!(tun["strict_route"], false);
        assert_eq!(tun["dns_hijack"][0], "any:53");
        assert_eq!(tun["dns_enhanced_mode"], "fake-ip");
        for secret in [
            "private-controller-key",
            "private-node",
            "private-password",
            "private.example",
        ] {
            assert!(!report.to_string().contains(secret));
        }
    }

    #[test]
    fn tun_diagnostic_keeps_user_and_interface_routing_restrictions() {
        let value = parse_tun_configuration(
            "tun:\n  auto-redirect: true\n  include-uid: [0]\n  exclude-uid: [1000]\n  include-uid-range: ['0:999']\n  exclude-uid-range: ['1000:65535']\n  include-interface: [eth0]\n  exclude-interface: [tailscale0]\n  route-exclude-address: [10.0.0.0/8]\n",
        );
        assert_eq!(value["auto_redirect"], true);
        assert_eq!(value["include_uid"], json!([0]));
        assert_eq!(value["exclude_uid"], json!([1000]));
        assert_eq!(value["include_uid_range"], json!(["0:999"]));
        assert_eq!(value["exclude_uid_range"], json!(["1000:65535"]));
        assert_eq!(value["include_interface"], json!(["eth0"]));
        assert_eq!(value["exclude_interface"], json!(["tailscale0"]));
        assert_eq!(value["route_exclude_address"], json!(["10.0.0.0/8"]));
    }

    #[test]
    fn tun_diagnostics_distinguish_missing_invalid_and_oversized_configuration() {
        let temporary = tempfile::tempdir().unwrap();
        assert_eq!(
            tun_configuration(temporary.path())["state"],
            "not_generated"
        );
        std::fs::create_dir(temporary.path().join("runtime")).unwrap();
        let path = temporary.path().join("runtime/config.yaml");
        std::fs::write(&path, "tun: [").unwrap();
        assert_eq!(
            tun_configuration(temporary.path())["reason"],
            "invalid_runtime_config"
        );
        std::fs::write(&path, "tun: []").unwrap();
        assert_eq!(
            tun_configuration(temporary.path())["reason"],
            "invalid_tun_config"
        );
        File::create(&path)
            .unwrap()
            .set_len(4 * 1024 * 1024 + 1)
            .unwrap();
        assert_eq!(
            tun_configuration(temporary.path())["reason"],
            "file_invalid_or_too_large"
        );
    }

    #[test]
    fn issue_reports_preserve_failures_and_redact_context_and_buffer() {
        let temporary = tempfile::tempdir().unwrap();
        std::fs::write(
            temporary.path().join("settings.json"),
            br#"{"secret":"controller-credential"}"#,
        )
        .unwrap();
        let report = issue_report(
            temporary.path(),
            json!({"error":"connect controller-credential https://subscription.test/private?key=abc"}),
            vec!["[ERROR] controller-credential connect failed".into()],
        );
        let text = report.to_string();
        assert!(!text.contains("controller-credential"));
        assert!(!text.contains("key=abc"));
        assert!(text.contains("connect failed"));
        assert_eq!(report["data"]["core_logs"]["state"], "collected");
        std::fs::write(temporary.path().join("settings.json"), b"broken").unwrap();
        let report = issue_report(
            temporary.path(),
            json!({"error":"controller-credential"}),
            vec![],
        );
        assert_eq!(report["data"]["context"]["state"], "withheld");
        assert!(!report.to_string().contains("controller-credential"));
    }

    #[test]
    fn issue_drafts_carry_a_redacted_summary_without_uploading_the_full_report() {
        let temporary = tempfile::tempdir().unwrap();
        std::fs::write(
            temporary.path().join("settings.json"),
            br#"{"secret":"controller-credential"}"#,
        )
        .unwrap();
        let report = issue_report(
            temporary.path(),
            json!({"error":format!("connect controller-credential {}\nhttps://example.test/sub?token=private",temporary.path().display()),"running":false}),
            vec!["private full log entry".into()],
        );
        let url = reqwest::Url::parse(&issue_draft_url(&report)).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("github.com"));
        assert_eq!(url.path(), "/Fu021/Clash-of-Rust/issues/new");
        let body = url.query_pairs().find(|(key, _)| key == "body").unwrap().1;
        assert!(body.contains(crate::VERSION));
        assert!(body.contains("connect"));
        assert!(body.contains("已隐藏含凭据的日志行"));
        assert!(!body.contains("controller-credential"));
        assert!(!body.contains("token=private"));
        assert!(!body.contains("private full log entry"));
        assert!(!body.contains(temporary.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn issue_drafts_bound_the_encoded_url_and_keep_unicode_valid() {
        let report = json!({"data":{"version":crate::VERSION,"os":"android","arch":"x86_64",
            "context":{"error":"网络连接超时😀".repeat(5000)}}});
        let text = issue_draft_url(&report);
        assert!(text.len() <= 6500);
        let url = reqwest::Url::parse(&text).unwrap();
        let body = url.query_pairs().find(|(key, _)| key == "body").unwrap().1;
        assert!(!body.contains('\u{fffd}'));
        assert!(body.contains("网络连接超时"));
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
        std::fs::create_dir(root.join("runtime/.geo-backup-original")).unwrap();
        std::fs::write(
            root.join("runtime/.geo-backup-original/geodata.json"),
            br#"{"version":"previous","files":[]}"#,
        )
        .unwrap();
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
        assert_eq!(
            value["geo_recovery_files"][".geo-backup-original"]["manifest"]["data"]["version"],
            "previous"
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
