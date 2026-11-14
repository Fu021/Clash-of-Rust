use crate::args::Args;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::io::{IsTerminal, Write};
mod human;
pub(crate) use clash_client_core::support::redact;

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
        if needs_failure_hint(args, &value) {
            crate::diagnostics::failure_hint(Some(args));
        }
        return Ok(());
    }
    write_output(&mut std::io::stdout().lock(), args, &value)?;
    if needs_failure_hint(args, &value) {
        crate::diagnostics::failure_hint(Some(args));
    }
    Ok(())
}

fn needs_failure_hint(args: &Args, value: &Value) -> bool {
    if matches!(args.path.as_str(), "status" | "stop" | "proxy")
        && let Some(fields) = value.as_object()
    {
        return fields.iter().any(|(key, item)| {
            key != "host_routes" && crate::diagnostics::has_failure(&json!({(key):item}))
        });
    }
    crate::diagnostics::has_failure(value)
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
    fn service_output_separates_enabled_from_running_and_owner() {
        let value = json!({
            "installed":true,"enabled":true,"active":false,"main_pid":0,
            "owner_pid":123,"ownership":"detached","matches_current":true
        });
        let text = render(&["service", "status"], value.clone());
        assert!(text.contains("登录自启：已开启"), "{text}");
        assert!(text.contains("服务状态：未运行"), "{text}");
        assert!(text.contains("服务之外的 CLI 后台"), "{text}");
        let mut unknown = value;
        unknown["enabled"] = Value::Null;
        unknown["active"] = Value::Null;
        unknown["ownership"] = json!("unknown");
        unknown["service_error"] = json!("用户总线不可达");
        let text = render(&["service", "status"], unknown);
        assert!(text.contains("登录自启：未确认"), "{text}");
        assert!(text.contains("服务状态：未确认"), "{text}");
        assert!(text.contains("用户总线不可达"), "{text}");
    }

    #[test]
    fn offline_list_output_discloses_its_source() {
        let value =
            json!({"items":[],"offset":0,"total":0,"offline":true,"source":"saved_profile"});
        let nodes = render(&["node", "list"], value.clone());
        assert!(nodes.contains("已保存订阅"), "{nodes}");
        assert!(nodes.contains("未测速"), "{nodes}");
        let connections = render(&["connection", "list"], value);
        assert!(connections.contains("没有活动连接"), "{connections}");
        let json = render(
            &["node", "list", "--json"],
            json!({"offline":true,"items":[]}),
        );
        assert_eq!(
            serde_json::from_str::<Value>(&json).unwrap()["data"]["offline"],
            true
        );
    }

    #[test]
    fn update_handoff_and_result_have_distinct_output() {
        let handed_off = render(&["update", "install"], json!({"shutdown":true}));
        assert!(handed_off.contains("尚未确认安装完成"));
        assert!(handed_off.contains("cor update status"));
        let completed_install = render(
            &["update", "install"],
            json!({"state":"succeeded","version":"0.6.7","message":"已更新至 0.6.7；可用 cor start 启动后台","error":false}),
        );
        assert!(completed_install.contains("已更新至 0.6.7"));
        assert!(!completed_install.contains("尚未确认安装完成"));
        let finished = render(
            &["update", "status"],
            json!({"state":"succeeded","version":"0.6.7","message":"已更新至 0.6.7","error":false}),
        );
        assert!(finished.contains("已更新至 0.6.7"));
        assert!(!finished.contains("尚未确认"));
        let failed = render(
            &["update", "status"],
            json!({"state":"failed","version":"0.6.7","message":"安装授权已取消","error":true}),
        );
        assert!(failed.contains("安装授权已取消"));
    }

    #[test]
    fn update_interval_output_uses_minutes_or_disabled_while_json_keeps_the_number() {
        for (minutes, expected) in [(0, "已关闭"), (360, "360 分钟")] {
            let value = json!({"update_check_interval_minutes": minutes});
            let text = render(
                &["config", "get", "update_check_interval_minutes"],
                value.clone(),
            );
            assert!(text.contains("自动检查更新间隔"));
            assert!(text.contains(expected));
            let json = render(
                &["config", "get", "update_check_interval_minutes", "--json"],
                value.clone(),
            );
            let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed["data"], value);
        }
    }

    #[test]
    fn failed_group_tests_offer_network_recovery_without_changing_json() {
        let all_failed = json!({"节点 A": 0, "节点 B": 0});
        let human = render(&["node", "test", "--all"], all_failed.clone());
        assert!(human.contains("cor proxy off"));
        assert!(human.contains("cor diagnose --output cor-diagnose.json"));
        assert!(
            !render(&["node", "test", "--all"], json!({"A": 1, "B": 0})).contains("cor proxy off")
        );
        assert!(!render(&["node", "test", "A"], json!({"A": 0})).contains("cor proxy off"));
        let json = render(&["node", "test", "--all", "--json"], all_failed);
        assert!(!json.contains("cor proxy off"));
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
    fn status_distinguishes_reference_routes_from_real_connections() {
        let base = json!({
            "running":true,"owner":"daemon","mode":"rule","proxy":"tun",
            "route":["PROXY","AUTO","🇺🇸美国02-cn2"],"node":"🇺🇸美国02-cn2",
            "upload_rate":0,"download_rate":0,"connections":1,"protected":false
        });
        let rule = render(&["status"], base.clone());
        assert!(
            rule.contains("规则参考线路：PROXY → AUTO → 🇺🇸美国02-cn2"),
            "{rule}"
        );
        assert!(rule.contains("cor connection list"), "{rule}");
        let mut global = base.clone();
        global["mode"] = json!("global");
        global["route"] = json!(["GLOBAL", "🇺🇸美国02-cn2"]);
        let global = render(&["status"], global);
        assert!(
            global.contains("当前线路：GLOBAL → 🇺🇸美国02-cn2"),
            "{global}"
        );
        assert!(!global.contains("规则参考"), "{global}");
        let mut protected = base.clone();
        protected["protected"] = json!(true);
        protected["route"] = json!(["🇺🇸美国02-cn2"]);
        let protected = render(&["status"], protected);
        assert!(protected.contains("保护节点：🇺🇸美国02-cn2"), "{protected}");
        assert!(!protected.contains("规则参考线路"), "{protected}");
        let mut old_backend = base;
        old_backend["route"] = Value::Null;
        let old_backend = render(&["status"], old_backend);
        assert!(
            old_backend.contains("当前节点：🇺🇸美国02-cn2"),
            "{old_backend}"
        );
    }

    #[test]
    fn unavailable_route_evidence_does_not_trigger_a_general_failure_hint() {
        let args = crate::args::parse(vec!["status".into()]).unwrap();
        let value = json!({
            "running":true,
            "host_routes":{"ipv6_uplink":{"state":"unavailable"},"warnings":[]}
        });
        assert!(!needs_failure_hint(&args, &value));
        let mut failed = value.clone();
        failed["backend_error"] = json!("控制接口超时");
        assert!(needs_failure_hint(&args, &failed));
        let diagnose = crate::args::parse(vec!["diagnose".into()]).unwrap();
        assert!(needs_failure_hint(&diagnose, &value));
    }

    #[test]
    fn status_reports_external_tun_and_an_action_instead_of_route_sampling_noise() {
        let routes = json!({
            "scope":"current_network_namespace",
            "holder_scope":"current_user",
            "ipv4":{"state":"observed","interface":"Meta","kind":"tun"},
            "ipv6":{"state":"unknown"},
            "tun_holders":{
                "state":"complete","scope":"current_user",
                "interfaces":{"Meta":[]},"skipped_other_users":1
            },
            "warnings":["tun_owner_unconfirmed"]
        });
        let status = render(
            &["status"],
            json!({"running":false,"owner":null,"host_routes":routes}),
        );
        assert!(status.contains("外部 TUN"));
        assert!(status.contains("Meta"));
        assert!(status.contains("当前用户范围未发现持有进程"), "{status}");
        assert!(status.contains("sudo cor status --all-users"), "{status}");
        assert!(!status.contains("无法读取"), "{status}");
        assert!(status.contains("cor diagnose --output cor-diagnose.json"));
        assert!(!status.contains("ipv6"));
        assert!(!status.contains("未检测连通性"));
        let report = render(
            &["diagnose"],
            json!({
                "dns_host":"example.test",
                "system_dns":{"Ok":["192.0.2.1"]},
                "core_dns":null,
                "system_route_http":{"status":200,"millis":12,"url":"https://example.test","detail":"HTTP 200"},
                "http":null,
                "ipv4":null,
                "ipv6":null,
                "backend_error":"当前用户后台未运行",
                "host_routes":{"scope":"unsupported"},
                "proxy_environment":{"https":true}
            }),
        );
        for expected in [
            "example.test",
            "192.0.2.1",
            "不使用代理环境变量",
            "HTTP 200",
            "当前用户后台未运行",
            "HTTPS_PROXY",
        ] {
            assert!(report.contains(expected), "{report}");
        }
    }

    #[test]
    fn status_reports_a_root_tun_holder_without_claiming_who_enabled_it() {
        let status = render(
            &["status"],
            json!({
                "running":false,
                "host_routes":{
                    "scope":"current_network_namespace","observer_uid":1000,
                    "ipv4":{"state":"observed","interface":"Meta","kind":"tun"},
                    "ipv6":{"state":"unknown","reason":"route_unavailable"},
                    "tun_holders":{"state":"partial","interfaces":{
                        "Meta":[{"uid":0,"pid":42,"process":"mihomo"}]
                    }},
                    "warnings":["tun_owner_unconfirmed"]
                }
            }),
        );
        assert!(status.contains("root · PID 42"), "{status}");
        assert!(status.contains("持有进程"));
        assert!(!status.contains("开启者"));
        assert!(!status.contains("无可用示例路由"));
        assert!(!status.contains("IPv6 不可用"));
    }

    #[test]
    fn explicit_all_user_scan_shows_holders_even_when_this_instance_owns_tun() {
        let status = render(
            &["status", "--all-users"],
            json!({
                "running":true,"owner":"daemon","version":"0.6.7","proxy":"tun","mode":"rule",
                "profile":"sub","node":"Node","protected":false,"connections":1,
                "upload_rate":1,"download_rate":2,
                "host_routes":{
                    "scope":"current_network_namespace","holder_scope":"all_users",
                    "ipv4":{"state":"observed","interface":"Meta","kind":"tun"},
                    "ipv6":{"state":"unknown"},
                    "tun_holders":{"state":"complete","scope":"all_users","interfaces":{
                        "Meta":[{"uid":1000,"pid":42,"process":"mihomo"}]
                    }},
                    "warnings":[]
                }
            }),
        );
        assert!(status.contains("TUN 持有进程"), "{status}");
        assert!(status.contains("UID 1000 · PID 42"), "{status}");
    }

    #[test]
    fn routine_status_omits_unavailable_samples_and_stop_does_not_claim_global_recovery() {
        let host_routes = json!({
            "scope":"current_network_namespace",
            "ipv4":{"state":"observed","interface":"eth0","kind":"other"},
            "ipv6":{"state":"unknown","reason":"route_unavailable"},
            "warnings":[]
        });
        for command in ["status", "proxy", "stop"] {
            let text = render(
                &[command],
                json!({"running":false,"proxy":"off","host_routes":host_routes}),
            );
            assert!(!text.contains("IPv6"), "{text}");
            assert!(!text.contains("ipv6"), "{text}");
            assert!(!text.contains("其他用户"), "{text}");
            assert!(!text.contains("网络设置已恢复"), "{text}");
        }
        let text = render(
            &["stop"],
            json!({"host_routes":{
                "scope":"current_network_namespace",
                "ipv4":{"state":"observed","interface":"Meta","kind":"tun"},
                "warnings":["tun_owner_unconfirmed"]
            }}),
        );
        assert!(text.starts_with("内核已停止\n"));
        assert!(text.contains("外部 TUN"));
        assert!(text.contains("cor diagnose --output cor-diagnose.json"));
    }

    #[test]
    fn network_only_restart_does_not_claim_the_core_was_restarted() {
        let text = render(
            &["restart", "--network-only"],
            json!({"network_refreshed":true}),
        );
        assert!(text.contains("关闭旧连接"));
        assert!(!text.contains("内核已重启"));
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
