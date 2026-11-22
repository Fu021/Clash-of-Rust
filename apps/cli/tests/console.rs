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

#[tokio::test]
async fn dns_uses_only_the_controller_and_preserves_machine_output() {
    use clash_client_core::config::atomic_write;
    use clash_client_runtime::control::PROTOCOL;
    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    let root = tempfile::tempdir().unwrap();
    let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let controller = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mixed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let token = "a".repeat(64);
    let endpoint = json!({
        "protocol": PROTOCOL, "port": backend.local_addr().unwrap().port(),
        "pid": std::process::id(), "owner": "daemon", "token": token,
    });
    atomic_write(
        &root.path().join("control.json"),
        &serde_json::to_vec(&endpoint).unwrap(),
    )
    .unwrap();
    let meta = json!({
        "profiles": [], "running": true, "version": "test", "owner": "daemon", "executable": "",
        "geo": {"version": "test", "updated": 0, "files": []},
        "settings": {
            "controller_port": controller.local_addr().unwrap().port(),
            "mixed_port": mixed.local_addr().unwrap().port(),
        },
    });
    let backend = tokio::spawn(async move {
        loop {
            let (mut stream, _) = backend.accept().await.unwrap();
            let size = stream.read_u32().await.unwrap();
            let mut request = vec![0; size as usize];
            stream.read_exact(&mut request).await.unwrap();
            let request: Value = serde_json::from_slice(&request).unwrap();
            assert!(request["token"] == token);
            let result = match request["operation"]["command"].as_str() {
                Some("meta") => &meta,
                Some("start") => &Value::Null,
                _ => panic!("unexpected backend command"),
            };
            let response = serde_json::to_vec(&json!({"result": result})).unwrap();
            stream.write_u32(response.len() as u32).await.unwrap();
            stream.write_all(&response).await.unwrap();
        }
    });
    let controller = tokio::spawn(async move {
        for index in 0..4 {
            let (mut stream, _) = controller.accept().await.unwrap();
            let mut request = [0; 4096];
            let size = stream.read(&mut request).await.unwrap();
            assert!(request[..size].starts_with(b"GET /dns/query?name=fixture.test&type="));
            let (status, body) = if index < 2 {
                ("503 Service Unavailable", "")
            } else {
                (
                    "200 OK",
                    r#"{"Status":0,"Answer":[{"type":1,"data":"203.0.113.5"}]}"#,
                )
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    for expected_success in [false, true] {
        let mut command = tokio::process::Command::from(cor());
        command
            .args(["--json", "dns", "fixture.test", "--timeout=2", "--data-dir"])
            .arg(root.path())
            .kill_on_drop(true);
        let output = tokio::select! {
            output = tokio::time::timeout(std::time::Duration::from_secs(5), command.output()) => output.unwrap().unwrap(),
            _ = mixed.accept() => panic!("cor dns contacted the mixed port"),
        };
        assert_eq!(output.status.success(), expected_success);
        if expected_success {
            let report: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(
                report,
                json!({
                    "schema": 1,
                    "command": "dns",
                    "data": {"source": "core/proxy", "answers": ["203.0.113.5"]},
                })
            );
        } else {
            assert!(output.stdout.is_empty());
        }
    }
    controller.await.unwrap();
    backend.abort();
}

#[tokio::test]
async fn profile_list_localizes_human_output_and_preserves_machine_output() {
    use clash_client_core::config::atomic_write;
    use clash_client_runtime::control::PROTOCOL;
    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let token = "a".repeat(64);
    let endpoint = json!({
        "protocol": PROTOCOL, "port": port, "pid": std::process::id(),
        "owner": "daemon", "token": token,
    });
    atomic_write(
        &root.path().join("control.json"),
        &serde_json::to_vec(&endpoint).unwrap(),
    )
    .unwrap();
    let rows = [
        ("builtin", "默认直连", "/resources/default.yaml", true),
        ("imported", "默认直连", "/imports/default.yaml", false),
        ("custom", "旅行节点", "/imports/travel.yaml", false),
        ("renamed", "自定义直连", "/resources/default.yaml", true),
    ];
    let expected_rows: Vec<_> = rows
        .iter()
        .map(|(id, name, source, _)| {
            json!({"id": id, "name": name, "source": source, "active": *id == "builtin",
                "updated": 0, "usage": null})
        })
        .collect();
    let profiles: Vec<_> = expected_rows
        .iter()
        .zip(rows)
        .map(|(row, (_, _, _, builtin))| {
            let mut profile = row.clone();
            profile["builtin"] = json!(builtin);
            profile
        })
        .collect();
    let reply = serde_json::to_vec(&json!({"result": {
        "profiles": profiles, "settings": {"active_profile": "builtin"}, "running": false,
        "geo": {"version": "test", "updated": 0, "files": []},
        "version": "test", "owner": "daemon", "executable": ""
    }}))
    .unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let size = stream.read_u32().await.unwrap();
            let mut request = vec![0; size as usize];
            stream.read_exact(&mut request).await.unwrap();
            let request: Value = serde_json::from_slice(&request).unwrap();
            assert_eq!(request["token"], token);
            assert_eq!(request["operation"]["command"], "meta");
            stream.write_u32(reply.len() as u32).await.unwrap();
            stream.write_all(&reply).await.unwrap();
        }
    });
    let list = |options: &[&str]| {
        let mut command = cor();
        command
            .args(["profile", "list", "--timeout=2", "--data-dir"])
            .arg(root.path())
            .args(options);
        async move {
            let result = tokio::process::Command::from(command)
                .output()
                .await
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            String::from_utf8(result.stdout).unwrap()
        }
    };
    let export = root.path().join("profiles-export.json");
    let export = export.to_str().unwrap();
    let english = list(&["--lang=en"]).await;
    assert!(
        english.starts_with("[Current] Default direct\n"),
        "{english}"
    );
    for name in ["默认直连", "旅行节点", "自定义直连"] {
        assert!(english.lines().any(|line| line == name), "{english}");
    }
    let chinese = list(&["--lang=zh-CN"]).await;
    assert!(chinese.starts_with("[当前] 默认直连\n"), "{chinese}");
    let expected = json!({"schema": 1, "command": "profile list", "data": expected_rows});
    for format in ["--json", "--jsonl"] {
        let text = list(&["--lang=en", format]).await;
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), expected);
    }
    assert!(list(&["--lang=en", "--output", export]).await.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(export).unwrap()).unwrap(),
        expected
    );

    let by_display_name = list(&["--lang=en", "--filter=dEfAuLt DiReCt"]).await;
    assert!(by_display_name.contains("ID: builtin"), "{by_display_name}");
    assert!(
        !by_display_name.contains("ID: imported"),
        "{by_display_name}"
    );
    let by_original_name = list(&["--lang=en", "--filter=默认直连"]).await;
    assert!(
        by_original_name.contains("ID: builtin"),
        "{by_original_name}"
    );
    assert!(
        by_original_name.contains("ID: imported"),
        "{by_original_name}"
    );
    let filtered = list(&["--lang=en", "--json", "--filter=默认直连"]).await;
    assert_eq!(
        serde_json::from_str::<Value>(&filtered).unwrap()["data"],
        json!(expected_rows[..2])
    );
    for format in ["--json", "--jsonl"] {
        let text = list(&["--lang=en", format, "--filter=Default direct"]).await;
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap()["data"],
            json!([])
        );
    }
    assert!(
        list(&["--lang=en", "--output", export, "--filter=Default direct"])
            .await
            .is_empty()
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(export).unwrap()).unwrap()["data"],
        json!([])
    );
    server.abort();
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
            .env_remove("LC_ALL")
            .env_remove("LC_MESSAGES")
            .env("LANG", "en_US.UTF-8")
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

#[cfg(target_os = "linux")]
#[test]
fn help_respects_linux_message_locale_precedence_and_c_locale() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("unused");
    for (list, all, messages, lang, override_lang, heading) in [
        (
            Some("zh_CN"),
            Some("C"),
            None,
            "zh_CN.UTF-8",
            None,
            "Usage: cor ",
        ),
        (
            Some("zh_CN"),
            Some("C.UTF-8"),
            None,
            "zh_CN.UTF-8",
            None,
            "Usage: cor ",
        ),
        (
            None,
            Some("POSIX"),
            None,
            "zh_CN.UTF-8",
            None,
            "Usage: cor ",
        ),
        (
            None,
            Some("fr_FR.UTF-8"),
            None,
            "zh_CN.UTF-8",
            None,
            "Usage: cor ",
        ),
        (
            None,
            None,
            Some("fr_FR.UTF-8"),
            "zh_CN.UTF-8",
            None,
            "Usage: cor ",
        ),
        (
            Some("fr:zh_CN"),
            Some("fr_FR.UTF-8"),
            None,
            "en_US.UTF-8",
            None,
            "用法：cor ",
        ),
        (
            Some("fr:C:zh_CN"),
            Some("fr_FR.UTF-8"),
            None,
            "zh_CN.UTF-8",
            None,
            "Usage: cor ",
        ),
        (
            Some("fr"),
            Some("fr_FR.UTF-8"),
            None,
            "zh_CN.UTF-8",
            None,
            "Usage: cor ",
        ),
        (
            None,
            Some("C"),
            None,
            "en_US.UTF-8",
            Some("zh-CN"),
            "用法：cor ",
        ),
    ] {
        let mut command = cor();
        command
            .args(["help", "--data-dir"])
            .arg(&data)
            .env_remove("LANGUAGE")
            .env_remove("LC_ALL")
            .env_remove("LC_MESSAGES")
            .env("LANG", lang);
        for (key, value) in [
            ("LANGUAGE", list),
            ("LC_ALL", all),
            ("LC_MESSAGES", messages),
        ] {
            if let Some(value) = value {
                command.env(key, value);
            }
        }
        if let Some(language) = override_lang {
            command.args(["--lang", language]);
        }
        let result = command.output().unwrap();
        assert!(result.status.success());
        let output = String::from_utf8(result.stdout).unwrap();
        assert!(
            output.contains(heading),
            "LANGUAGE={list:?}, LC_ALL={all:?}, LC_MESSAGES={messages:?}, LANG={lang}, override={override_lang:?}\n{output}"
        );
        assert!(!data.exists());
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
fn update_status_localizes_known_results_without_changing_saved_or_json_messages() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("update-install.json");
    for (state, message, english) in [
        (
            "succeeded",
            "已更新至 0.6.7；后台和内核已启动",
            "Updated to 0.6.7; background process and core started",
        ),
        (
            "succeeded",
            "已更新至 0.6.7；后台已恢复，内核保持关闭",
            "Updated to 0.6.7; background process restored; core remains stopped",
        ),
        (
            "failed",
            "更新失败：安装授权已取消；本次未停止原后台",
            "Update failed: Installation authorization cancelled; existing background process not stopped",
        ),
        (
            "failed",
            "更新失败：安装未完成：文件仍被占用（resources/GeoIP.dat），请处理后重试",
            "Update failed: Installation incomplete: File is still in use (resources/GeoIP.dat); resolve the issue and retry",
        ),
        (
            "failed",
            "更新失败：无法获得安装授权，请检查桌面授权服务",
            "Update failed: Installation authorization failed; check the desktop authorization service",
        ),
        (
            "failed",
            "更新失败：DEB 安装失败（exit status: 1）：example details；已恢复更新前的后台和内核状态",
            "Update failed: DEB installation failed (exit status: 1): example details; previous background and core state restored",
        ),
        (
            "failed",
            "更新失败：Arch 原生包 安装失败（exit status: 1）：example details；已恢复更新前的后台和内核状态",
            "Update failed: Arch package installation failed (exit status: 1): example details; previous background and core state restored",
        ),
        (
            "failed",
            "更新失败：DEB 安装失败（exit status: 1）：example details；恢复后台也失败：connection refused；请运行 cor start",
            "Update failed: DEB installation failed (exit status: 1): example details; background recovery also failed: connection refused; run cor start",
        ),
        (
            "failed",
            "更新失败：未获得安装授权，未执行停止后台；请在交互终端重试，或由管理员手动安装；本次未停止原后台",
            "Update failed: Installation authorization denied; retry in an interactive terminal or ask an administrator to install the package; existing background process not stopped",
        ),
        (
            "failed",
            "已安装 0.6.7，但恢复后台失败：example details；请运行 cor status 检查后重试",
            "Installed 0.6.7, but could not restore the background process: example details; check cor status and retry",
        ),
        (
            "failed",
            "外部错误：example details",
            "外部错误：example details",
        ),
    ] {
        let record = serde_json::json!({
            "version":"0.6.7", "state":state, "message":message,
            "error":state == "failed", "updated_at":123, "directory":root.path().join("expired"),
        });
        let saved = serde_json::to_vec(&record).unwrap();
        std::fs::write(&path, &saved).unwrap();
        let mut command = cor();
        command
            .args(["--lang=en", "--data-dir"])
            .arg(root.path())
            .args(["update", "status"]);
        let output = command.output().unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains(english), "{text}");
        let json_output = command.arg("--json").output().unwrap();
        assert!(json_output.status.success());
        let value: serde_json::Value = serde_json::from_slice(&json_output.stdout).unwrap();
        assert_eq!(value["data"]["message"], message);
        let chinese = cor()
            .args(["--lang=zh-CN", "--data-dir"])
            .arg(root.path())
            .args(["update", "status"])
            .output()
            .unwrap();
        assert!(chinese.status.success());
        assert!(String::from_utf8(chinese.stdout).unwrap().contains(message));
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        assert!(!root.path().join("control.json").exists());
    }
}

#[test]
fn network_check_localizes_offline_output_and_preserves_json() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("unused");
    let check = |options: &[&str]| {
        let result = cor()
            .args(["network", "check", "example.test", "--data-dir"])
            .arg(&data)
            .args(options)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    };

    let english = check(&["--lang=en"]);
    assert_eq!(
        english.lines().collect::<Vec<_>>(),
        [
            "Target: https://example.test",
            "System DNS: System resolver not queried while protection is active or unknown",
            "Core DNS: Core is not running or its control interface is unavailable",
            "Local proxy: Core is not running",
            "HTTPS: Core is not running; request not sent",
            "HTTP: Core is not running; request not sent",
            "Protocol: Not checked",
            "Route: No test connection established",
        ]
    );
    assert!(check(&["--lang=zh-CN"]).contains("实际线路：未建立检测连接"));

    let json = check(&["--lang=en", "--json"]);
    assert_eq!(json, check(&["--lang=zh-CN", "--json"]));
    let report: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(report["schema"], 1);
    assert_eq!(report["command"], "network check");
    for protocol in ["https", "http"] {
        assert_eq!(
            report["data"][protocol],
            serde_json::json!({"success": null, "detail": "内核未运行，未发起请求"})
        );
    }
    assert_eq!(
        report["data"]["route"],
        serde_json::json!({"success": null, "detail": "未建立检测连接"})
    );
    assert!(!data.exists());
}

#[test]
fn invalid_network_target_localizes_human_output_and_preserves_machine_output() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("unused");
    let check = |tool: &str, options: &[&str]| {
        let result = cor()
            .args(["network", tool, "ftp://example.test", "--data-dir"])
            .arg(&data)
            .args(options)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1));
        result
    };
    let english = check("http", &["--lang=en"]);
    assert!(
        String::from_utf8(english.stdout)
            .unwrap()
            .starts_with("Target: Not checked\n")
    );
    let chinese = check("http", &["--lang=zh-CN"]);
    assert!(
        String::from_utf8(chinese.stdout)
            .unwrap()
            .starts_with("目标：未检查\n")
    );
    let raw = |format: &str| {
        let result = check("http", &["--lang=en", format]);
        let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(value["data"]["target"], "未检查");
        assert_eq!(value["data"]["entries"][0]["code"], "invalid_target");
    };
    raw("--json");
    raw("--jsonl");
    let export = root.path().join("report.json");
    let result = check("http", &["--lang=en", "--output", export.to_str().unwrap()]);
    assert!(result.stdout.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(export).unwrap()).unwrap();
    assert_eq!(value["data"]["target"], "未检查");
    assert_eq!(value["data"]["entries"][0]["code"], "invalid_target");

    let invalid = check("check", &["--lang=en"]);
    assert!(invalid.stdout.is_empty());
    let error = String::from_utf8(invalid.stderr).unwrap();
    assert!(
        error.starts_with("cor: Enter a domain or HTTP/HTTPS URL: Enter an HTTP/HTTPS URL\n"),
        "{error}"
    );
    assert!(!data.exists());
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
