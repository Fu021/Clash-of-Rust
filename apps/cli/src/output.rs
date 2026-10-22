use crate::args::Args;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::io::{IsTerminal, Write};
mod human;

pub fn download_progress(progress: &clash_client_core::update::Progress) -> String {
    human::download_progress(progress)
}

pub fn emit(args: &Args, value: Value) -> Result<()> {
    let value = if args.flag("show-secrets") {
        value
    } else {
        redact(value)
    };
    if let Some(path) = args.get("output") {
        let bytes =
            serde_json::to_vec_pretty(&json!({"schema":1,"command":args.path,"data":value}))?;
        clash_client_core::config::atomic_write(&crate::backend::absolute(path)?, &bytes)?;
        if !args.flag("quiet") {
            eprintln!("已保存：{path}");
        }
        return Ok(());
    }
    write_output(&mut std::io::stdout().lock(), args, &value)
}

fn write_output(out: &mut dyn Write, args: &Args, value: &Value) -> Result<()> {
    if args.flag("json") || args.flag("jsonl") {
        serde_json::to_writer(
            &mut *out,
            &json!({"schema":1,"command":args.path,"data":value}),
        )?;
        writeln!(out)?;
    } else {
        human::write(out, args, value)?;
    }
    Ok(())
}

pub fn redact(mut value: Value) -> Value {
    match &mut value {
        Value::Object(map) => {
            for (key, field) in map {
                let normalized = key.to_ascii_lowercase().replace('-', "_");
                if [
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
                .any(|s| normalized == *s || normalized.ends_with(&format!("_{s}")))
                {
                    if !field.is_null() {
                        *field = Value::String("[已隐藏]".into());
                    }
                } else if matches!(normalized.as_str(), "source" | "url")
                    && field
                        .as_str()
                        .is_some_and(|s| s.starts_with("http://") || s.starts_with("https://"))
                {
                    *field = Value::String(clash_client_runtime::claude_check::system::redact_url(
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

pub async fn confirm(args: &Args, message: &str) -> Result<()> {
    if args.flag("yes") {
        return Ok(());
    }
    ensure!(
        std::io::stdin().is_terminal(),
        "{message}；非交互模式请明确传入 --yes"
    );
    eprint!("{message} [y/N] ");
    std::io::stderr().flush()?;
    // Tokio stdin uses a blocking-pool read that cannot be cancelled. A plain
    // reader thread keeps Ctrl+C responsive and never delays runtime shutdown.
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("cor-confirm".into())
        .spawn(move || {
            let mut answer = String::new();
            let result = std::io::stdin().read_line(&mut answer).map(|_| answer);
            let _ = sender.send(result);
        })?;
    let answer = receiver.await??;
    ensure!(matches!(answer.trim(), "y" | "Y" | "yes"), "操作已取消");
    Ok(())
}

pub fn on_off(value: &str) -> Result<bool> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => anyhow::bail!("参数应为 on 或 off"),
    }
}
pub fn page(args: &Args, rows: Vec<Value>) -> Result<Value> {
    let limit = args.number("limit", 100, 10_000)?;
    let offset = args.number("offset", 0, usize::MAX)?;
    let total = rows.len();
    Ok(
        json!({"total":total, "offset":offset, "items":rows.into_iter().skip(offset).take(limit).collect::<Vec<_>>() }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_configuration_credentials_are_removed_without_hiding_results() {
        let value = redact(
            json!({"secret":"controller","proxies":[{"name":"node","password":"pw","uuid":"key","private-key":"private"}],"source":"https://example.com/sub?token=key","latency":20}),
        );
        let text = value.to_string();
        for sensitive in ["controller", "\"pw\"", "\"key\"", "?token", "\"private\""] {
            assert!(!text.contains(sensitive), "{text}");
        }
        assert_eq!(value["latency"], 20);
    }

    fn render(command: &[&str], value: Value) -> String {
        let args = crate::args::parse(command.iter().map(|s| (*s).to_owned())).unwrap();
        let mut out = Vec::new();
        write_output(&mut out, &args, &redact(value)).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn node_output_explains_groups_unknown_delays_timeouts_and_selection() {
        let value = json!({"offset":0,"total":5,"items":[
            {"name":"Auto","type":"URLTest","now":"德国01","members":2,"latency_ms":null,"selected":true},
            {"name":"DIRECT","type":"Direct","now":"","members":0,"latency_ms":null,"selected":false},
            {"name":"德国01","type":"AnyTLS","members":0,"latency_ms":140,"selected":false},
            {"name":"德国01-ipv6","type":"AnyTLS","members":0,"latency_ms":0,"selected":false},
            {"name":"美国01","type":"AnyTLS","members":0,"latency_ms":null,"selected":false}
        ]});
        let text = render(&["node", "list", "Proxy"], value.clone());
        for expected in [
            "策略组：Proxy",
            "自动测速",
            "当前选择",
            "当前",
            "140 ms",
            "超时",
            "未测速",
            "内置出站",
            "直连",
            "共 5 项",
        ] {
            assert!(text.contains(expected), "{text}");
        }
        for raw in ["latency_ms", "selected=", "null", "false", "URLTest"] {
            assert!(!text.contains(raw), "{text}");
        }
        for format in ["--json", "--jsonl"] {
            let line = render(&["node", "list", "Proxy", format], value.clone());
            assert_eq!(line.lines().count(), 1);
            let document: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(
                document,
                json!({"schema":1,"command":"node list","data":value})
            );
        }
    }

    #[test]
    fn readable_status_has_units_and_stopped_status_does_not_claim_a_live_route() {
        let status = render(
            &["status"],
            json!({"running":true,"owner":"daemon","mode":"rule","proxy":"tun","upload_rate":1024,"download_rate":1536,"protected":false}),
        );
        for expected in [
            "运行中",
            "CLI 后台",
            "规则",
            "TUN",
            "1.0 KiB/s",
            "1.5 KiB/s",
            "已关闭",
        ] {
            assert!(status.contains(expected), "{status}");
        }
        let stopped = render(&["status"], json!({"running":false,"owner":null}));
        assert_eq!(stopped, "内核：未运行\n");
        let setting = render(
            &["config", "get", "theme_mode"],
            json!({"theme_mode":"dark"}),
        );
        assert!(setting.contains("深色（dark）"));
    }

    #[test]
    fn display_escapes_terminal_controls_and_never_truncates_actionable_ids() {
        let name = "节点\u{1b}[31m\nnode";
        let text = render(
            &["node", "list"],
            json!({"total":1,"items":[{"name":name,"type":"AnyTLS","members":0,"latency_ms":42}]}),
        );
        assert!(!text.contains('\u{1b}'));
        assert!(text.contains("\\nnode"));
        let id = "12345678-1234-1234-1234-123456789abc";
        let connections = render(
            &["connection", "show", id],
            json!({"id":id,"name":"长域名.example.com:443","network":"tcp","chain":["德国01","Auto","Proxy"],"upload_rate":null,"download_rate":0,"upload":2048,"download":1024}),
        );
        assert!(connections.contains(id));
        assert!(connections.contains("Proxy → Auto → 德国01"));
        assert!(connections.contains("尚未采样"));
        assert!(connections.contains("0 B/s"));
    }

    #[test]
    fn start_summarizes_settings_and_failed_checks_keep_the_actual_reason() {
        let started = render(
            &["start"],
            json!({
                "settings":{"active_profile":"profile-id","run_mode":"rule","proxy_mode":"tun","mixed_port":7897,"controller_port":9090,"secret":"sensitive"},
                "profiles":[{"id":"profile-id","name":"我的订阅","source":"https://example.com/?token=private"}],
                "geo":{"files":[{"sha256":"irrelevant-manifest"}]}
            }),
        );
        assert!(started.contains("内核已启动") && started.contains("我的订阅"));
        assert!(started.contains("TUN") && started.contains("7897"));
        for hidden in [
            "sensitive",
            "token",
            "irrelevant-manifest",
            "active_profile",
        ] {
            assert!(!started.contains(hidden), "{started}");
        }
        let report = render(
            &["ip-check"],
            json!({"total":1,"confirmed_available":0,"non_red":0,"results":[
                {"name":"测试服务","state":"Failed","detail":"DNS 查询超时"}
            ]}),
        );
        assert!(report.contains("检测失败") && report.contains("DNS 查询超时"));
    }
}
