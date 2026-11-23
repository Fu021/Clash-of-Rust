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
async fn logs_localize_human_output_and_search_but_preserve_raw_records() {
    use clash_client_core::config::atomic_write;
    use clash_client_runtime::control::PROTOCOL;
    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let token = "a".repeat(64);
    let endpoint = json!({
        "protocol": PROTOCOL, "port": listener.local_addr().unwrap().port(),
        "pid": std::process::id(), "owner": "daemon", "token": token,
    });
    atomic_write(
        &root.path().join("control.json"),
        &serde_json::to_vec(&endpoint).unwrap(),
    )
    .unwrap();
    let client = "[客户端] 配置已导入/更新";
    let core = "[内核] 配置已导入/更新";
    let warning = "[warn] 连接失败 level=error";
    let info = "[info] 连接失败 level=error";
    let guarded = "[内核] [客户端] 操作失败：level=error";
    let translated = "[Client] Configuration imported or updated";
    let records = [
        (client, "info"),
        (core, "info"),
        (warning, "warning"),
        (info, "info"),
        (guarded, "info"),
    ];
    let raw = records.map(|(message, _)| message);
    let meta = serde_json::to_vec(&json!({"result": {
        "profiles": [], "running": false, "version": "test", "owner": "daemon", "executable": "",
        "geo": {"version": "test", "updated": 0, "files": []},
        "settings": {"secret": "fixture-export-secret"},
    }}))
    .unwrap();
    let logs = serde_json::to_vec(&json!({"result": raw})).unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let size = stream.read_u32().await.unwrap();
            let mut request = vec![0; size as usize];
            stream.read_exact(&mut request).await.unwrap();
            let request: Value = serde_json::from_slice(&request).unwrap();
            assert_eq!(request["protocol"], PROTOCOL);
            assert_eq!(request["token"], token);
            let response = match request["operation"]["command"].as_str() {
                Some("meta") => &meta,
                Some("logs") => {
                    assert_eq!(request["operation"]["clear"], false);
                    &logs
                }
                command => panic!("unexpected backend command {command:?}"),
            };
            stream.write_u32(response.len() as u32).await.unwrap();
            stream.write_all(response).await.unwrap();
        }
    });
    let run = |language: &str, options: &[&str]| {
        let mut command = tokio::process::Command::from(cor());
        command
            .arg("logs")
            .args(options)
            .args(["--lang", language, "--timeout=2", "--data-dir"])
            .arg(root.path())
            .kill_on_drop(true);
        async move {
            let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
                .await
                .unwrap()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap()
        }
    };

    let expected = records.map(|(message, level)| {
        json!({"schema": 1, "command": "logs", "data": {"level": level, "message": message}})
    });
    for (language, displayed_client) in [("en", translated), ("zh-CN", client)] {
        let text = run(language, &[]).await;
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [displayed_client, core, warning, info, guarded]
        );
        for format in ["--json", "--jsonl"] {
            let text = run(language, &[format]).await;
            let actual: Vec<Value> = text
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(actual, expected, "{language} {format}");
        }
        let export = root.path().join(format!("logs-{language}.txt"));
        let text = run(language, &["export", "--output", export.to_str().unwrap()]).await;
        assert!(text.is_empty());
        assert_eq!(std::fs::read(export).unwrap(), raw.join("\n").as_bytes());
    }
    let filtered = run("en", &["--level", "warning"]).await;
    assert_eq!(filtered.lines().collect::<Vec<_>>(), [warning]);
    let filtered = run("en", &["--level", "info"]).await;
    assert_eq!(
        filtered.lines().collect::<Vec<_>>(),
        [translated, core, info, guarded]
    );
    assert!(run("en", &["--level", "error"]).await.is_empty());
    let filtered = run("en", &["--filter", "cOnFiGuRaTiOn ImPoRtEd"]).await;
    assert_eq!(filtered.lines().collect::<Vec<_>>(), [translated]);
    let filtered = run("en", &["--filter", "配置已导入"]).await;
    assert_eq!(filtered.lines().collect::<Vec<_>>(), [translated, core]);
    assert!(!root.path().join("daemon.log").exists());
    server.abort();
}

#[tokio::test]
async fn profile_list_localizes_human_output_and_preserves_machine_output() {
    use chrono::{DateTime, Local};
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
    let expiry_timestamp = 1813147200;
    let no_expiry = "upload=1048576; download=2147483648; total=3221225472; expire=0";
    let dated = format!("upload=1048576; total=3221225472; expire={expiry_timestamp}");
    let rows = [
        ("builtin", "默认直连", "/resources/default.yaml", true, None),
        (
            "imported",
            "默认直连",
            "/imports/default.yaml",
            false,
            Some(no_expiry),
        ),
        (
            "custom",
            "旅行节点",
            "https://travel.example.test/",
            false,
            Some(no_expiry),
        ),
        (
            "renamed",
            "自定义直连",
            "https://builtin.example.test/",
            true,
            None,
        ),
        (
            "dated",
            "限期订阅",
            "https://dated.example.test/",
            false,
            Some(dated.as_str()),
        ),
        (
            "missing",
            "备用订阅",
            "https://backup.example.test/",
            false,
            None,
        ),
    ];
    let expected_rows: Vec<_> = rows
        .iter()
        .map(|(id, name, source, _, usage)| {
            json!({"id": id, "name": name, "active": *id == "builtin", "source": source,
                "updated": 0, "usage": usage})
        })
        .collect();
    let profiles: Vec<_> = expected_rows
        .iter()
        .zip(rows)
        .map(|(row, (_, _, _, builtin, _))| {
            let mut profile = row.clone();
            profile["builtin"] = json!(builtin);
            profile
        })
        .collect();
    let profiles_path = root.path().join("profiles.json");
    let stored_profiles = serde_json::to_vec_pretty(&profiles).unwrap();
    atomic_write(&profiles_path, &stored_profiles).unwrap();
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
    let expiry_date = DateTime::from_timestamp(expiry_timestamp, 0)
        .unwrap()
        .with_timezone(&Local)
        .format("%Y-%m-%d");
    for (text, id_label, summaries) in [
        (
            &english,
            "  ID: ",
            [
                "Used 2/3 GiB · No expiry".into(),
                format!("Upload 1 MiB · Total 3 GiB · Expires {expiry_date}"),
                "Usage and expiry unavailable".into(),
            ],
        ),
        (
            &chinese,
            "  ID：",
            [
                "已用 2/3 GiB · 不限期".into(),
                format!("上传 1 MiB · 总量 3 GiB · 到期 {expiry_date}"),
                "未提供用量与到期".into(),
            ],
        ),
    ] {
        let entry_for = |id| {
            let id_line = format!("{id_label}{id}");
            text.split("\n\n")
                .find(|entry| entry.lines().any(|line| line == id_line))
                .unwrap_or_else(|| panic!("missing {id}: {text}"))
        };
        for (id, summary) in ["custom", "dated", "missing"].into_iter().zip(summaries) {
            let summary_line = format!("  {summary}");
            let entry = entry_for(id);
            assert!(entry.lines().any(|line| line == summary_line), "{entry}");
        }
        for id in ["builtin", "imported", "renamed"] {
            let entry = entry_for(id);
            assert_eq!(entry.lines().count(), 4, "{entry}");
        }
        for raw_key in ["upload=", "download=", "total=", "expire="] {
            assert!(!text.contains(raw_key), "{text}");
        }
    }
    let expected = json!({"schema": 1, "command": "profile list", "data": expected_rows});
    let expected_stdout = format!("{expected}\n");
    for language in ["--lang=en", "--lang=zh-CN"] {
        for format in ["--json", "--jsonl"] {
            let text = list(&[language, format]).await;
            assert_eq!(text, expected_stdout, "{language} {format}");
        }
    }
    assert!(list(&["--lang=en", "--output", export]).await.is_empty());
    assert_eq!(
        std::fs::read(export).unwrap(),
        serde_json::to_vec_pretty(&expected).unwrap()
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
    assert_eq!(std::fs::read(profiles_path).unwrap(), stored_profiles);
    server.abort();
}

async fn mock_control_backend(
    root: &std::path::Path,
    mut respond: impl FnMut(&serde_json::Value) -> serde_json::Value + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    use clash_client_core::config::atomic_write;
    use clash_client_runtime::control::PROTOCOL;
    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let token = "a".repeat(64);
    let endpoint = json!({
        "protocol": PROTOCOL, "port": listener.local_addr().unwrap().port(),
        "pid": std::process::id(), "owner": "daemon", "token": token,
    });
    atomic_write(
        &root.join("control.json"),
        &serde_json::to_vec(&endpoint).unwrap(),
    )
    .unwrap();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let size = stream.read_u32().await.unwrap();
            let mut request = vec![0; size as usize];
            stream.read_exact(&mut request).await.unwrap();
            let request: Value = serde_json::from_slice(&request).unwrap();
            assert_eq!(request["token"], token);
            let response = serde_json::to_vec(&respond(&request["operation"])).unwrap();
            stream.write_u32(response.len() as u32).await.unwrap();
            stream.write_all(&response).await.unwrap();
        }
    })
}

async fn subscription_backend(root: &std::path::Path) -> tokio::task::JoinHandle<()> {
    use clash_client_core::{config, rule_manager::Document};
    use serde_json::json;

    let meta = json!({"result": {
        "profiles": [{"id": "fixture", "name": "本地订阅", "source": "/fixture.yaml",
            "updated": 0, "usage": null, "builtin": false}],
        "settings": {"active_profile": "fixture"}, "running": false,
        "geo": {"version": "test", "updated": 0, "files": []},
        "version": "test", "owner": "daemon", "executable": "",
    }});
    let document = Document::from_raw(
        "fixture",
        false,
        "proxies: []\nrules: ['MATCH,DIRECT', 'DOMAIN,example.test,DIRECT']",
    )
    .unwrap();
    let document = json!({"result": document});
    let error = config::parse_profile("name: sample").unwrap_err();
    let import_error = json!({"result": null, "error": format!("{error:#}")});
    mock_control_backend(root, move |operation| {
        match operation["command"].as_str() {
            Some("meta") => &meta,
            Some("rule-document") => {
                assert_eq!(operation["id"], "fixture");
                &document
            }
            Some("import") => {
                assert_eq!(operation["id"], "fixture");
                assert_eq!(operation["expect_current"], true);
                &import_error
            }
            command => panic!("unexpected backend command {command:?}"),
        }
        .clone()
    })
    .await
}

#[tokio::test]
async fn notify_localizes_the_default_and_preserves_explicit_messages() {
    use serde_json::{Value, json};

    let root = tempfile::tempdir().unwrap();
    let (send, mut received) = tokio::sync::mpsc::unbounded_channel();
    let server = mock_control_backend(root.path(), move |operation| {
        match operation["command"].as_str() {
            Some("meta") => json!({"result": {
                "profiles": [], "settings": {}, "running": false,
                "geo": {"version": "test", "updated": 0, "files": []},
                "version": "test", "owner": "daemon", "executable": "",
            }}),
            Some("notify") => {
                assert_eq!(operation["title"], "Clash of Rust");
                send.send(operation["message"].clone()).unwrap();
                json!({"result": null})
            }
            command => panic!("unexpected backend command {command:?}"),
        }
    })
    .await;
    for (language, message, expected) in [
        ("en", None, "Meow!"),
        ("zh-CN", None, "喵!"),
        ("en", Some("用户正文: custom"), "用户正文: custom"),
    ] {
        let mut command = cor();
        command
            .args([
                "notify",
                "--lang",
                language,
                "--timeout=2",
                "--json",
                "--data-dir",
            ])
            .arg(root.path())
            .args(message);
        let output = tokio::process::Command::from(command)
            .output()
            .await
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(received.try_recv().unwrap(), expected);
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["data"], json!({"sent": true}));
    }
    server.abort();
}

#[tokio::test]
async fn rule_list_localizes_warnings_and_preserves_quiet_and_machine_output() {
    let root = tempfile::tempdir().unwrap();
    let server = subscription_backend(root.path()).await;
    let list = |language: &str, options: &[&str]| {
        let mut command = cor();
        command
            .args(["rule", "list", "--timeout=2", "--data-dir"])
            .arg(root.path())
            .args(["--lang", language])
            .args(options);
        async move { tokio::process::Command::from(command).output().await }
    };
    let raw = "MATCH 之后有 1 条规则不会被匹配，请检查顺序。";
    let translated =
        "Rules after MATCH will never be evaluated (1 affected). Check the rule order.";
    let english = list("en", &[]).await.unwrap();
    assert!(english.status.success(), "{english:?}");
    let stderr = String::from_utf8(english.stderr).unwrap();
    assert!(stderr.lines().any(|line| line == translated), "{stderr}");
    assert!(!stderr.contains(raw), "{stderr}");

    let chinese = list("zh-CN", &[]).await.unwrap();
    assert!(chinese.status.success(), "{chinese:?}");
    assert!(
        String::from_utf8(chinese.stderr)
            .unwrap()
            .lines()
            .any(|line| line == raw)
    );

    for format in ["--json", "--jsonl"] {
        let raw_output = list("zh-CN", &[format]).await.unwrap();
        let quiet = list("en", &[format, "--quiet"]).await.unwrap();
        assert!(raw_output.status.success() && quiet.status.success());
        assert_eq!(quiet.stdout, raw_output.stdout);
        let stderr = String::from_utf8(quiet.stderr).unwrap();
        assert!(
            !stderr.contains(translated) && !stderr.contains(raw),
            "{stderr}"
        );
        let value: serde_json::Value = serde_json::from_slice(&quiet.stdout).unwrap();
        assert_eq!(
            value["data"]["items"][1]["rule"],
            "DOMAIN,example.test,DIRECT"
        );
        assert_eq!(value["data"]["items"][1]["source"], "Subscription");
    }
    server.abort();
}

#[tokio::test]
async fn profile_update_localizes_human_error_and_preserves_machine_output() {
    use serde_json::{Value, json};

    let root = tempfile::tempdir().unwrap();
    let server = subscription_backend(root.path()).await;
    let update = |language: &str, options: &[&str]| {
        let mut command = cor();
        command
            .args(["profile", "update", "fixture", "--timeout=2", "--data-dir"])
            .arg(root.path())
            .args(["--lang", language])
            .args(options)
            .env("COLUMNS", "120");
        async move { tokio::process::Command::from(command).output().await }
    };
    let raw = "配置中没有 proxies 或 proxy-providers";
    let translated = "The configuration has no proxies or proxy-providers";
    for (language, expected, unexpected) in [("en", translated, raw), ("zh-CN", raw, translated)] {
        let output = update(language, &[]).await.unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("本地订阅"), "{text}");
        assert!(
            text.contains(expected) && !text.contains(unexpected),
            "{text}"
        );
    }
    let expected = json!({"schema": 1, "command": "profile update", "data": [{
        "id": "fixture", "name": "本地订阅", "updated": false, "error": raw,
    }]});
    for format in ["--json", "--jsonl"] {
        let output = update("en", &[format]).await.unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            expected
        );
    }
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
fn service_status_localizes_a_real_service_query_timeout() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let systemctl = root.path().join("systemctl");
    std::fs::write(&systemctl, "#!/bin/sh\nexec /bin/sleep 30\n").unwrap();
    std::fs::set_permissions(systemctl, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = cor()
        .args(["service", "status", "--lang=en", "--quiet", "--data-dir"])
        .arg(root.path().join("data"))
        .env("PATH", root.path())
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env_remove("SUDO_COMMAND")
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(
        output.contains("Query failed: Login service query timed out after 5 seconds"),
        "{output}"
    );
    assert!(!root.path().join("data").exists());
    assert!(!root.path().join("config").exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn service_status_localizes_native_failures_and_preserves_machine_errors() {
    use clash_client_runtime::{config::Store, platform::cli_service};
    use serde_json::{Value, json};
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let _owner = Store { root: data.clone() }.instance_lock().unwrap();
    let backend_cause = "无法检查后台的占用锁: fixture backend failure";
    let server = mock_control_backend(&data, move |operation| {
        assert_eq!(operation["command"], "meta");
        json!({"result": null, "error": backend_cause})
    })
    .await;

    let config = root.path().join("config");
    std::fs::create_dir_all(config.join("systemd/user").join(cli_service::NAME)).unwrap();
    let systemctl = root.path().join("systemctl");
    std::fs::write(
        &systemctl,
        "#!/bin/sh\nprintf '%s\\n' 'fixture: 系统原文' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(systemctl, std::fs::Permissions::from_mode(0o700)).unwrap();
    let status = |language: &str, format: Option<&str>| {
        let mut command = cor();
        command
            .args([
                "service",
                "status",
                "--lang",
                language,
                "--quiet",
                "--data-dir",
            ])
            .arg(&data)
            .args(format)
            .env("PATH", root.path())
            .env("XDG_CONFIG_HOME", &config)
            .env_remove("SUDO_COMMAND");
        async move {
            let output = tokio::process::Command::from(command)
                .output()
                .await
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            String::from_utf8(output.stdout).unwrap()
        }
    };

    let english = status("en", None).await;
    for expected in [
        "Query failed: Could not read the CLI login startup configuration: ",
        "Query failed: Login service query failed: fixture: 系统原文",
        &format!(
            "Query failed: The running backend is temporarily unreachable; preserve the diagnostic state: {backend_cause}"
        ),
    ] {
        assert!(english.contains(expected), "{english}");
    }
    let chinese = status("zh-CN", None).await;
    assert!(
        chinese.contains("登录服务查询失败：fixture: 系统原文"),
        "{chinese}"
    );
    assert!(chinese.contains("无法读取 CLI 登录自启配置："), "{chinese}");
    assert!(chinese.contains(backend_cause), "{chinese}");

    for format in ["--json", "--jsonl"] {
        let english = status("en", Some(format)).await;
        assert_eq!(english, status("zh-CN", Some(format)).await);
        let response: Value = serde_json::from_str(&english).unwrap();
        let value = &response["data"];
        assert_eq!(value["ownership"], "unknown");
        assert!(value["owner_pid"].is_null());
        assert_eq!(
            value["service_error"],
            "登录服务查询失败：fixture: 系统原文"
        );
        assert!(
            value["config_error"]
                .as_str()
                .unwrap()
                .starts_with("无法读取 CLI 登录自启配置：")
        );
        assert_eq!(
            value["backend_error"],
            format!("存活的后台暂时不可达，请先保留排障现场: {backend_cause}")
        );
    }
    server.abort();
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
