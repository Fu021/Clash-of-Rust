use serde_json::{Value, json};

#[cfg(target_os = "linux")]
pub(crate) mod ipv6;
#[cfg(target_os = "linux")]
mod linux;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HolderScope {
    CurrentUser,
    AllUsers,
}

impl HolderScope {
    #[cfg(target_os = "linux")]
    fn label(self) -> &'static str {
        match self {
            Self::CurrentUser => "current_user",
            Self::AllUsers => "all_users",
        }
    }
}

#[cfg(target_os = "linux")]
fn parse_route(output: &[u8], sysfs: &std::path::Path) -> Value {
    let parsed: Value = match serde_json::from_slice(output) {
        Ok(value) => value,
        Err(_) => return json!({"state":"unknown","reason":"invalid_route_response"}),
    };
    let Some(interface) = parsed
        .as_array()
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.get("dev"))
        .and_then(Value::as_str)
        .filter(|name| {
            !name.is_empty()
                && name.len() <= 64
                && name.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
                })
                && *name != "."
                && *name != ".."
        })
    else {
        return json!({"state":"unknown","reason":"interface_unavailable"});
    };
    let interface_path = sysfs.join(interface);
    let kind = match std::fs::metadata(interface_path.join("tun_flags")) {
        Ok(_) => "tun",
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && interface_path.is_dir() => {
            "other"
        }
        Err(_) => "unknown",
    };
    json!({"state":"observed","interface":interface,"kind":kind})
}

#[cfg(target_os = "linux")]
async fn route(family: &str, destination: &str) -> Value {
    let mut command = tokio::process::Command::new("ip");
    command
        .args(["-j", family, "route", "get", destination])
        .kill_on_drop(true);
    match tokio::time::timeout(std::time::Duration::from_secs(2), command.output()).await {
        Ok(Ok(output)) if output.status.success() => {
            parse_route(&output.stdout, std::path::Path::new("/sys/class/net"))
        }
        Ok(Ok(_)) => json!({"state":"unknown","reason":"route_unavailable"}),
        Ok(Err(_)) => json!({"state":"unknown","reason":"ip_command_unavailable"}),
        Err(_) => json!({"state":"unknown","reason":"route_query_timeout"}),
    }
}

#[cfg(target_os = "linux")]
fn parse_policy(output: &[u8]) -> Value {
    if output.len() > 64 * 1024 {
        return json!({"state":"unknown","reason":"policy_response_too_large"});
    }
    let Ok(Value::Array(entries)) = serde_json::from_slice::<Value>(output) else {
        return json!({"state":"unknown","reason":"invalid_policy_response"});
    };
    if entries.iter().any(|entry| !entry.is_object()) {
        return json!({"state":"unknown","reason":"invalid_policy_response"});
    }
    let count = entries.len();
    let rules: Vec<Value> = entries
        .into_iter()
        .take(256)
        .map(|entry| {
            let fields = [
                "priority",
                "table",
                "action",
                "goto",
                "not",
                "uidrange",
                "fwmark",
                "fwmask",
                "ipproto",
                "sport",
                "dport",
                "iif",
                "oif",
                "suppress_prefixlen",
            ];
            Value::Object(
                fields
                    .into_iter()
                    .filter_map(|field| {
                        entry.get(field).cloned().map(|value| (field.into(), value))
                    })
                    .collect(),
            )
        })
        .collect();
    json!({"state":if count > 256 {"partial"} else {"observed"},"total":count,"rules":rules})
}

#[cfg(target_os = "linux")]
async fn policy(family: &str) -> Value {
    let mut command = tokio::process::Command::new("ip");
    command
        .args(["-j", family, "rule", "show"])
        .kill_on_drop(true);
    match tokio::time::timeout(std::time::Duration::from_secs(2), command.output()).await {
        Ok(Ok(output)) if output.status.success() => parse_policy(&output.stdout),
        Ok(Ok(_)) => json!({"state":"unknown","reason":"policy_unavailable"}),
        Ok(Err(_)) => json!({"state":"unknown","reason":"ip_command_unavailable"}),
        Err(_) => json!({"state":"unknown","reason":"policy_query_timeout"}),
    }
}

#[cfg(target_os = "linux")]
fn tcp_route_command(family: &str, destination: &str, uid: u32) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("ip");
    command
        .args([
            "-j",
            family,
            "route",
            "get",
            destination,
            "uid",
            &uid.to_string(),
            "ipproto",
            "tcp",
            "dport",
            "443",
        ])
        .kill_on_drop(true);
    command
}

#[cfg(target_os = "linux")]
async fn tcp_route(family: &str, destination: &str, uid: u32) -> Value {
    let mut command = tcp_route_command(family, destination, uid);
    match tokio::time::timeout(std::time::Duration::from_secs(2), command.output()).await {
        Ok(Ok(output)) if output.status.success() => {
            parse_route(&output.stdout, std::path::Path::new("/sys/class/net"))
        }
        Ok(Ok(_)) => json!({"state":"unknown","reason":"tcp_route_unavailable"}),
        Ok(Err(_)) => json!({"state":"unknown","reason":"ip_command_unavailable"}),
        Err(_) => json!({"state":"unknown","reason":"route_query_timeout"}),
    }
}

#[cfg(target_os = "linux")]
async fn tcp_sample(uid: u32) -> Value {
    let (ipv4, ipv6) = tokio::join!(
        tcp_route("-4", "1.1.1.1", uid),
        tcp_route("-6", "2606:4700:4700::1111", uid)
    );
    json!({"uid":uid,"protocol":"tcp","destination_port":443,"ipv4":ipv4,"ipv6":ipv6})
}

pub async fn diagnose(holder_scope: HolderScope) -> Value {
    let mut routes = observe_scope(holder_scope).await;
    #[cfg(target_os = "linux")]
    {
        let observer = unsafe { libc::geteuid() };
        let other = if observer == 0 {
            std::env::var("SUDO_UID")
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|uid| *uid != observer)
        } else {
            Some(0)
        };
        let samples = async {
            if let Some(uid) = other {
                Some(tcp_sample(uid).await)
            } else {
                None
            }
        };
        let (ipv4, ipv6, sample) = tokio::join!(policy("-4"), policy("-6"), samples);
        routes["policy_rules"] = json!({"ipv4":ipv4,"ipv6":ipv6});
        if let Some(sample) = sample
            && let Some(samples) = routes["tcp_uid_samples"].as_array_mut()
        {
            samples.push(sample);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = &mut routes;
    routes
}

pub async fn observe() -> Value {
    observe_scope(HolderScope::CurrentUser).await
}

pub async fn observe_all_users() -> Value {
    observe_scope(HolderScope::AllUsers).await
}

async fn observe_scope(holder_scope: HolderScope) -> Value {
    #[cfg(target_os = "linux")]
    {
        let observer = unsafe { libc::geteuid() };
        let (ipv4, ipv6, tcp) = tokio::join!(
            route("-4", "1.1.1.1"),
            route("-6", "2606:4700:4700::1111"),
            tcp_sample(observer)
        );
        let routes = json!({
            "scope":"current_network_namespace",
            "observer_uid":observer,
            "holder_scope":holder_scope.label(),
            "ipv4":ipv4,
            "ipv6":ipv6,
            "tcp_uid_samples":[tcp]
        });
        let fallback = routes.clone();
        tokio::task::spawn_blocking(move || {
            let mut routes = linux::attach_holders(routes, holder_scope);
            routes["ipv6_uplink"] = ipv6::Observation::report();
            routes
        })
        .await
        .unwrap_or_else(|_| {
            let mut routes = fallback;
            routes["tun_holders"] = json!({
                "state":"unavailable","reason":"owner_query_failed",
                "scope":holder_scope.label()
            });
            routes
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = holder_scope;
        json!({"scope":"unsupported"})
    }
}

fn current_route<'a>(routes: &'a Value, family: &str) -> Option<&'a Value> {
    let Some(samples) = routes.get("tcp_uid_samples") else {
        return routes.get(family);
    };
    let uid = routes["observer_uid"].as_u64()?;
    samples
        .as_array()?
        .iter()
        .find(|sample| {
            sample["uid"].as_u64() == Some(uid)
                && sample["protocol"] == "tcp"
                && sample["destination_port"] == 443
        })?
        .get(family)
}

pub fn with_proxy_context(routes: Value, proxy: Option<crate::engine::ProxyMode>) -> Value {
    with_tun_expectations(routes, proxy, None)
}

pub fn with_settings_context(
    routes: Value,
    proxy: Option<crate::engine::ProxyMode>,
    settings: Option<&crate::config::Settings>,
) -> Value {
    let expected = settings.map(|settings| {
        ipv6_expected(
            &routes,
            settings.claude_protection.is_some() || settings.ipv6,
        )
    });
    let source = match settings {
        Some(_) if routes["ipv6_uplink"]["state"] == "unavailable" => "no_uplink_ipv6",
        Some(settings) if settings.claude_protection.is_some() => "claude_protection",
        Some(settings) if !settings.ipv6 => "ipv6_disabled",
        Some(_) => "current_settings",
        None => "unknown",
    };
    let mut routes = with_tun_expectations(routes, proxy, expected);
    routes["coverage_expectations"]["source"] = json!(source);
    routes
}

fn with_tun_expectations(
    mut routes: Value,
    proxy: Option<crate::engine::ProxyMode>,
    ipv6_expected: Option<bool>,
) -> Value {
    use crate::engine::ProxyMode;

    let expected = |family: &str| {
        proxy != Some(ProxyMode::Tun) || family != "ipv6" || ipv6_expected != Some(false)
    };
    let samples = ["ipv4", "ipv6"].map(|family| {
        expected(family)
            .then(|| current_route(&routes, family))
            .flatten()
    });
    let tun = samples
        .iter()
        .flatten()
        .any(|route| route["state"] == "observed" && route["kind"] == "tun");
    let other = samples
        .iter()
        .flatten()
        .any(|route| route["state"] == "observed" && route["kind"] == "other");
    let observed_tun = tun
        || ["ipv4", "ipv6"]
            .into_iter()
            .any(|family| routes[family]["state"] == "observed" && routes[family]["kind"] == "tun");
    let mut warnings = Vec::new();
    if observed_tun && proxy.is_none() {
        warnings.push("tun_current_proxy_unknown");
    } else if observed_tun && proxy != Some(ProxyMode::Tun) {
        warnings.push("tun_owner_unconfirmed");
    } else if !tun && other && proxy == Some(ProxyMode::Tun) {
        warnings.push("sample_routes_not_tun");
    }
    if proxy == Some(ProxyMode::Tun) && tun {
        for (family, warning) in [
            ("ipv4", "tun_ipv4_route_bypass"),
            ("ipv6", "tun_ipv6_route_bypass"),
        ] {
            if expected(family)
                && current_route(&routes, family)
                    .is_some_and(|route| route["state"] == "observed" && route["kind"] == "other")
            {
                warnings.push(warning);
            }
        }
    }
    if routes.get("tcp_uid_samples").is_some()
        && ["ipv4", "ipv6"].into_iter().any(|family| {
            expected(family)
                && routes[family]["state"] == "observed"
                && routes[family]["kind"] == "tun"
                && current_route(&routes, family)
                    .is_some_and(|route| route["state"] == "observed" && route["kind"] == "other")
        })
    {
        warnings.push("tun_current_uid_route_bypass");
    }
    routes["coverage_expectations"] = json!({"ipv4":true,"ipv6":ipv6_expected,"source":if ipv6_expected.is_some() {
        "current_settings"
    } else {
        "unknown"
    }});
    routes["warnings"] = json!(warnings);
    routes
}

fn ipv6_expected(routes: &Value, enabled: bool) -> bool {
    enabled && routes["ipv6_uplink"]["state"] != "unavailable"
}

/// A positive sample of a non-TUN route while this process owns TUN means the
/// host may still send some traffic outside the tunnel. Unknown or unavailable
/// samples are deliberately not treated as proof either way.
pub fn coverage_issue(routes: &Value, config: &Value, require_ipv6: bool) -> Option<String> {
    if routes["scope"] != "current_network_namespace" {
        return None;
    }
    let ipv6_expected = ipv6_expected(routes, require_ipv6 || config["ipv6"] != false);
    let mut issues = Vec::new();
    for (family, label, expected) in [("ipv4", "IPv4", true), ("ipv6", "IPv6", ipv6_expected)] {
        let uid_sample = config["tun"]["auto-redirect"].as_bool() != Some(true)
            && routes.get("tcp_uid_samples").is_some();
        let route = if uid_sample {
            current_route(routes, family)
        } else {
            routes.get(family)
        };
        let Some(route) = route else {
            continue;
        };
        if !expected || route["state"] != "observed" || route["kind"] != "other" {
            continue;
        }
        let interface = route["interface"].as_str().unwrap_or("物理接口");
        let sample = if uid_sample {
            format!(
                "当前 UID {} 的 TCP/443 {label} 样本",
                routes["observer_uid"]
            )
        } else {
            format!("示例 {label} 路由")
        };
        issues.push(format!("{sample}仍经过 {interface}"));
    }
    (!issues.is_empty()).then(|| {
        format!(
            "TUN 已开启，但{}；部分程序可能绕过 TUN，请导出诊断检查 UID、路由与 DNS 覆盖",
            issues.join("，")
        )
    })
}

fn holder_labels(holders: &[Value]) -> Vec<String> {
    holders
        .iter()
        .take(3)
        .filter_map(|holder| {
            let uid = holder["uid"].as_u64()?;
            let pid = holder["pid"].as_u64()?;
            let user = if uid == 0 {
                "root".to_owned()
            } else {
                format!("UID {uid}")
            };
            Some(format!("{user} · PID {pid}"))
        })
        .collect()
}

pub fn holder_summary(routes: &Value) -> Option<String> {
    if routes["scope"] != "current_network_namespace" || routes["holder_scope"] != "all_users" {
        return None;
    }
    let mut interfaces = std::collections::BTreeSet::new();
    let mut evidence = Vec::new();
    for route in ["ipv4", "ipv6"]
        .into_iter()
        .flat_map(|family| [routes.get(family), current_route(routes, family)])
        .flatten()
    {
        if route["state"] != "observed" || route["kind"] != "tun" {
            continue;
        }
        let Some(interface) = route["interface"].as_str() else {
            continue;
        };
        if !interfaces.insert(interface) {
            continue;
        }
        let holders = routes["tun_holders"]["interfaces"][interface]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let owners = holder_labels(holders);
        evidence.push(if owners.is_empty() {
            format!("{interface}：未找到持有进程")
        } else {
            format!("{interface}：{}", owners.join("、"))
        });
    }
    if evidence.is_empty() {
        return None;
    }
    let state = match routes["tun_holders"]["state"].as_str() {
        Some("complete") => "",
        Some("partial") => "；扫描不完整",
        Some("unavailable") => "；扫描不可用",
        _ => "",
    };
    Some(format!("{}{state}", evidence.join("；")))
}

pub fn notice(routes: &Value) -> Option<String> {
    if routes["scope"] != "current_network_namespace" {
        return None;
    }
    let warnings = routes["warnings"].as_array()?;
    let external = warnings.iter().any(|warning| {
        matches!(
            warning.as_str(),
            Some("tun_owner_unconfirmed" | "tun_current_proxy_unknown")
        )
    });
    let bypass = warnings.iter().any(|warning| {
        matches!(
            warning.as_str(),
            Some(
                "sample_routes_not_tun"
                    | "tun_ipv4_route_bypass"
                    | "tun_ipv6_route_bypass"
                    | "tun_current_uid_route_bypass"
            )
        )
    });
    if !external && !bypass {
        return None;
    }
    let mut interfaces = std::collections::BTreeSet::new();
    let mut evidence = Vec::new();
    for family in ["ipv4", "ipv6"] {
        let Some(route) = current_route(routes, family) else {
            continue;
        };
        if route["state"] != "observed" {
            continue;
        }
        let Some(interface) = route["interface"].as_str() else {
            continue;
        };
        if !interfaces.insert(interface) {
            continue;
        }
        if route["kind"] != "tun" {
            evidence.push(interface.to_owned());
            continue;
        }
        let holders = routes["tun_holders"]["interfaces"][interface]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let owners = holder_labels(holders);
        evidence.push(if !owners.is_empty() {
            format!("{interface}；持有进程 {}", owners.join("、"))
        } else if routes["holder_scope"] == "all_users" {
            match routes["tun_holders"]["state"].as_str() {
                Some("complete") => format!("{interface}；跨用户扫描未找到持有进程"),
                _ => format!("{interface}；跨用户持有进程读取不完整"),
            }
        } else if routes["tun_holders"]["state"] == "complete" {
            format!(
                "{interface}；当前用户范围未发现持有进程，跨用户检查请运行 sudo cor status --all-users"
            )
        } else {
            format!(
                "{interface}；当前用户持有进程读取不完整，跨用户检查请运行 sudo cor status --all-users"
            )
        });
    }
    if evidence.is_empty() {
        if !bypass {
            return None;
        }
        evidence.push("未能读取具体接口".into());
    }
    let uncertain = warnings
        .iter()
        .any(|warning| warning == "tun_current_proxy_unknown");
    let uid_bypass = warnings
        .iter()
        .any(|warning| warning == "tun_current_uid_route_bypass");
    let message = if uid_bypass {
        "检测到 TUN 路由，但当前用户的 TCP/443 样本仍绕过 TUN"
    } else if bypass && external {
        "检测到 TUN 路由异常，同时存在外部 TUN 接管"
    } else if bypass {
        "TUN 已开启，但示例路由显示仍有流量绕过 TUN"
    } else if uncertain {
        "检测到流量经过 TUN，本程序代理状态暂时无法读取"
    } else {
        "检测到流量经过外部 TUN，本程序的“关闭代理”无法关闭它"
    };
    Some(format!("{message}（{}）。", evidence.join("；")))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn route_type_requires_a_visible_tun_interface() {
        let temporary = tempfile::tempdir().unwrap();
        let interface = temporary.path().join("Meta");
        std::fs::create_dir(&interface).unwrap();
        std::fs::write(interface.join("tun_flags"), "0x1").unwrap();
        let route = parse_route(br#"[{"dev":"Meta"}]"#, temporary.path());
        assert_eq!(route["interface"], "Meta");
        assert_eq!(route["kind"], "tun");
        std::fs::create_dir(temporary.path().join("eth0")).unwrap();
        let route = parse_route(br#"[{"dev":"eth0"}]"#, temporary.path());
        assert_eq!(route["kind"], "other");
        assert_eq!(
            parse_route(br#"[{"dev":"missing"}]"#, temporary.path())["kind"],
            "unknown"
        );
        assert_eq!(
            parse_route(br#"[{"dev":"../private"}]"#, temporary.path())["state"],
            "unknown"
        );
        assert_eq!(
            parse_route(b"invalid", temporary.path())["state"],
            "unknown"
        );
    }

    #[test]
    fn policy_evidence_keeps_user_selectors_without_exporting_arbitrary_fields() {
        let value = parse_policy(
            br#"[{"priority":10,"uidrange":{"start":0,"end":0},"table":2022,"private":"hidden"},{"priority":20,"fwmark":"0x80000","fwmask":"0xff0000","table":"main"}]"#,
        );
        assert_eq!(value["state"], "observed");
        assert_eq!(value["rules"][0]["uidrange"]["end"], 0);
        assert_eq!(value["rules"][1]["fwmark"], "0x80000");
        assert!(!value.to_string().contains("hidden"));
        assert_eq!(parse_policy(b"[]")["state"], "observed");
        for invalid in [b"{}".as_slice(), b"null", b"[null]", b"not json"] {
            assert_eq!(parse_policy(invalid)["state"], "unknown");
        }
        assert_eq!(parse_policy(&vec![b' '; 65537])["state"], "unknown");
        assert_eq!(
            parse_policy(&serde_json::to_vec(&vec![json!({"priority":10}); 257]).unwrap())["state"],
            "partial"
        );
    }

    #[test]
    fn tcp_route_queries_select_the_user_protocol_and_destination_port() {
        for (family, destination) in [("-4", "1.1.1.1"), ("-6", "2606:4700:4700::1111")] {
            for uid in [0, 1000] {
                let command = tcp_route_command(family, destination, uid);
                let args: Vec<_> = command
                    .as_std()
                    .get_args()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect();
                assert_eq!(
                    args,
                    [
                        "-j".to_owned(),
                        family.into(),
                        "route".into(),
                        "get".into(),
                        destination.into(),
                        "uid".into(),
                        uid.to_string(),
                        "ipproto".into(),
                        "tcp".into(),
                        "dport".into(),
                        "443".into(),
                    ]
                );
            }
        }
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;
    use crate::engine::ProxyMode;

    fn uid_routes(current: Value, other: Value) -> Value {
        json!({
            "scope":"current_network_namespace",
            "observer_uid":1000,
            "holder_scope":"current_user",
            "ipv4":{"state":"observed","kind":"tun","interface":"Meta"},
            "ipv6":{"state":"unknown"},
            "tcp_uid_samples":[
                {"uid":0,"protocol":"tcp","destination_port":443,"ipv4":other,"ipv6":{"state":"unknown"}},
                {"uid":1000,"protocol":"tcp","destination_port":443,"ipv4":current,"ipv6":{"state":"unknown"}}
            ]
        })
    }

    #[test]
    fn a_generic_tun_route_does_not_hide_current_user_tcp_bypass() {
        let routes = uid_routes(
            json!({"state":"observed","kind":"other","interface":"eth0"}),
            json!({"state":"observed","kind":"tun","interface":"Meta"}),
        );
        let config = json!({"tun":{"inet6-address":[]}});
        let issue = coverage_issue(&routes, &config, false).unwrap();
        assert!(issue.contains("UID 1000"));
        assert!(issue.contains("TCP/443 IPv4"));
        assert!(issue.contains("eth0"));
        for proxy in [None, Some(ProxyMode::Off), Some(ProxyMode::Tun)] {
            let context = with_proxy_context(routes.clone(), proxy);
            assert!(
                context["warnings"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("tun_current_uid_route_bypass"))
            );
            let message = notice(&context).unwrap();
            assert!(message.contains("当前用户"));
            assert!(message.contains("eth0"));
            assert!(message.contains("绕过 TUN"));
        }
    }

    #[test]
    fn current_user_tcp_tun_takes_precedence_over_a_generic_physical_route() {
        let mut routes = uid_routes(
            json!({"state":"observed","kind":"tun","interface":"Meta"}),
            json!({"state":"observed","kind":"other","interface":"eth0"}),
        );
        routes["ipv4"] = json!({"state":"observed","kind":"other","interface":"eth0"});
        assert!(coverage_issue(&routes, &json!({}), false).is_none());
        let context = with_proxy_context(routes.clone(), Some(ProxyMode::Tun));
        assert_eq!(context["warnings"], json!([]));
        assert!(notice(&context).is_none());
        routes["holder_scope"] = "all_users".into();
        routes["tun_holders"] = json!({
            "state":"complete","interfaces":{"Meta":[{"pid":42,"uid":0}]}
        });
        let owners = holder_summary(&routes).unwrap();
        assert!(owners.contains("Meta"));
        assert!(owners.contains("root · PID 42"));
        let external = with_proxy_context(routes, Some(ProxyMode::Off));
        assert_eq!(external["warnings"], json!(["tun_owner_unconfirmed"]));
        assert!(notice(&external).unwrap().contains("Meta"));
    }

    #[test]
    fn unavailable_current_user_samples_cannot_borrow_root_or_generic_evidence() {
        let mut routes = uid_routes(
            json!({"state":"unknown","reason":"tcp_route_unavailable"}),
            json!({"state":"observed","kind":"tun","interface":"Meta"}),
        );
        routes["ipv4"] = json!({"state":"observed","kind":"other","interface":"eth0"});
        let original = routes.clone();
        for replacement in [
            json!({"state":"unknown","reason":"route_query_timeout"}),
            json!({"state":"unknown","reason":"ip_command_unavailable"}),
        ] {
            routes["tcp_uid_samples"][1]["ipv4"] = replacement;
            assert!(coverage_issue(&routes, &json!({}), true).is_none());
            assert_eq!(
                with_proxy_context(routes.clone(), Some(ProxyMode::Tun))["warnings"],
                json!([])
            );
        }
        for (field, invalid) in [
            ("uid", json!(2000)),
            ("protocol", json!("udp")),
            ("destination_port", json!(53)),
        ] {
            let mut routes = original.clone();
            routes["tcp_uid_samples"][1][field] = invalid;
            assert!(coverage_issue(&routes, &json!({}), true).is_none());
            assert_eq!(
                with_proxy_context(routes, Some(ProxyMode::Tun))["warnings"],
                json!([])
            );
        }
        for invalid in [json!(null), json!([]), json!({})] {
            let mut routes = original.clone();
            routes["tcp_uid_samples"] = invalid;
            assert!(coverage_issue(&routes, &json!({}), true).is_none());
        }
    }

    #[test]
    fn uid_specific_ipv6_bypass_respects_the_configured_address_family() {
        let mut routes = uid_routes(
            json!({"state":"observed","kind":"tun","interface":"Meta"}),
            json!({"state":"observed","kind":"tun","interface":"Meta"}),
        );
        routes["ipv6"] = json!({"state":"observed","kind":"tun","interface":"Meta"});
        routes["tcp_uid_samples"][1]["ipv6"] =
            json!({"state":"observed","kind":"other","interface":"eth0"});
        let ipv4_only = json!({"ipv6":false,"tun":{"inet6-address":[]}});
        assert!(coverage_issue(&routes, &ipv4_only, false).is_none());
        assert!(
            coverage_issue(&routes, &ipv4_only, true)
                .unwrap()
                .contains("TCP/443 IPv6")
        );
        let context = with_proxy_context(routes, Some(ProxyMode::Tun));
        assert_eq!(
            context["warnings"],
            json!(["tun_ipv6_route_bypass", "tun_current_uid_route_bypass"])
        );
    }

    #[test]
    fn tcp_redirect_does_not_treat_a_lookup_without_packet_marks_as_a_bypass_verdict() {
        let mut routes = uid_routes(
            json!({"state":"observed","kind":"other","interface":"eth0"}),
            json!({"state":"observed","kind":"tun","interface":"Meta"}),
        );
        let config = json!({"tun":{"auto-redirect":true,"inet6-address":[]}});
        assert!(coverage_issue(&routes, &config, false).is_none());
        routes["ipv6"] = json!({"state":"observed","kind":"other","interface":"eth0"});
        assert!(
            coverage_issue(&routes, &config, true)
                .unwrap()
                .contains("IPv6")
        );
    }

    #[test]
    fn notices_require_observed_tun_and_do_not_mislabel_our_privileged_helper() {
        let routes = json!({
            "scope":"current_network_namespace",
            "observer_uid":1000,
            "ipv4":{"state":"observed","kind":"tun","interface":"Meta"},
            "ipv6":{"state":"observed","kind":"tun","interface":"Meta"},
            "tun_holders":{"state":"partial","interfaces":{
                "Meta":[{"pid":42,"uid":0,"process":"mihomo"}]
            }}
        });
        assert!(notice(&with_proxy_context(routes.clone(), Some(ProxyMode::Tun))).is_none());
        let external = notice(&with_proxy_context(routes.clone(), Some(ProxyMode::Off))).unwrap();
        assert!(external.contains("外部 TUN"));
        assert!(external.contains("root · PID 42"));
        assert_eq!(external.matches("Meta").count(), 1);
        let unknown = notice(&with_proxy_context(routes, None)).unwrap();
        assert!(unknown.contains("状态暂时无法读取"));
        assert!(!unknown.contains("外部"));
        assert!(notice(&with_proxy_context(json!({"scope":"unsupported"}), None)).is_none());
    }

    #[test]
    fn warnings_distinguish_observed_routes_from_owner_and_unavailable_samples() {
        let routes = json!({
            "scope":"current_network_namespace",
            "ipv4":{"state":"observed","kind":"tun"},
            "ipv6":{"state":"unknown"}
        });
        assert_eq!(
            with_proxy_context(routes.clone(), Some(ProxyMode::Off))["warnings"],
            json!(["tun_owner_unconfirmed"])
        );
        assert_eq!(
            with_proxy_context(routes.clone(), None)["warnings"],
            json!(["tun_current_proxy_unknown"])
        );
        assert_eq!(
            with_proxy_context(routes, Some(ProxyMode::Tun))["warnings"],
            json!([])
        );
        let other = json!({"ipv4":{"state":"observed","kind":"other"},"ipv6":{"state":"unknown"}});
        assert_eq!(
            with_proxy_context(other, Some(ProxyMode::Tun))["warnings"],
            json!(["sample_routes_not_tun"])
        );
        let unknown = json!({"ipv4":{"state":"unknown"},"ipv6":{"state":"unknown"}});
        assert_eq!(
            with_proxy_context(unknown, Some(ProxyMode::Tun))["warnings"],
            json!([])
        );
    }

    #[test]
    fn partial_tun_coverage_is_reported_instead_of_treated_as_healthy() {
        let routes = json!({
            "scope":"current_network_namespace",
            "ipv4":{"state":"observed","kind":"other","interface":"eth0"},
            "ipv6":{"state":"observed","kind":"tun","interface":"Meta"}
        });
        let context = with_proxy_context(routes.clone(), Some(ProxyMode::Tun));
        assert_eq!(context["warnings"], json!(["tun_ipv4_route_bypass"]));
        let notice = notice(&context).unwrap();
        assert!(notice.contains("绕过 TUN"));
        assert!(notice.contains("eth0"));
        let dual_stack = json!({
            "ipv6":true,
            "tun":{"inet6-address":["fdfe:dcba:9876::1/126"]}
        });
        assert!(
            coverage_issue(&routes, &dual_stack, false)
                .unwrap()
                .contains("IPv4")
        );
        let ipv6_only = json!({
            "scope":"current_network_namespace",
            "ipv4":{"state":"observed","kind":"tun","interface":"Meta"},
            "ipv6":{"state":"observed","kind":"other","interface":"eth0"}
        });
        assert!(
            coverage_issue(&ipv6_only, &dual_stack, false)
                .unwrap()
                .contains("IPv6")
        );
    }

    #[test]
    fn absent_uplink_allows_ipv4_tun_and_protection_without_hiding_ipv4_bypass() {
        let routes = json!({
            "scope":"current_network_namespace",
            "ipv4":{"state":"observed","kind":"tun","interface":"Meta"},
            "ipv6":{"state":"observed","kind":"other","interface":"wlan0"},
            "ipv6_uplink":{"state":"unavailable"}
        });
        for tun in [
            json!({"enable":true,"inet4-address":["198.18.0.1/30"]}),
            json!({"enable":true,"inet6-address":[]}),
        ] {
            let config = json!({"ipv6":true,"tun":tun});
            for required in [false, true] {
                assert!(coverage_issue(&routes, &config, required).is_none());
            }
            let context = with_settings_context(
                routes.clone(),
                Some(ProxyMode::Tun),
                Some(&crate::config::Settings::default()),
            );
            assert_eq!(context["warnings"], json!([]));
            assert!(notice(&context).is_none());
            let mut bypass = routes.clone();
            bypass["ipv4"]["kind"] = "other".into();
            assert!(
                coverage_issue(&bypass, &config, false)
                    .unwrap()
                    .contains("IPv4")
            );
        }
    }

    #[test]
    fn configured_ipv4_only_tun_keeps_raw_ipv6_evidence_without_a_false_alarm() {
        use crate::engine::ProxyMode;
        let routes = json!({
            "scope":"current_network_namespace",
            "observer_uid":1000,
            "ipv4":{"state":"observed","kind":"tun","interface":"Meta"},
            "ipv6":{"state":"observed","kind":"other","interface":"wlp3s0"},
            "tcp_uid_samples":[{
                "uid":1000,"protocol":"tcp","destination_port":443,
                "ipv4":{"state":"observed","kind":"tun","interface":"Meta"},
                "ipv6":{"state":"observed","kind":"other","interface":"wlp3s0"}
            }]
        });
        let mut settings = crate::config::Settings {
            ipv6: false,
            ..Default::default()
        };
        let context = with_settings_context(routes.clone(), Some(ProxyMode::Tun), Some(&settings));
        assert!(notice(&context).is_none());
        assert_eq!(context["ipv6"]["interface"], "wlp3s0");
        assert_eq!(context["coverage_expectations"]["ipv6"], false);
        settings.ipv6 = true;
        assert!(
            notice(&with_settings_context(
                routes.clone(),
                Some(ProxyMode::Tun),
                Some(&settings)
            ))
            .is_some()
        );
        assert!(
            notice(&with_settings_context(
                routes.clone(),
                Some(ProxyMode::Tun),
                None
            ))
            .is_some()
        );
        settings.ipv6 = false;
        settings.claude_protection = Some(std::sync::Arc::new(
            crate::claude_check::protection::Protection::new(
                "fixture".into(),
                "fixture".into(),
                crate::claude_check::protection::Previous {
                    mode: "rule".into(),
                    proxy_mode: ProxyMode::Off,
                    selectors: Default::default(),
                },
            ),
        ));
        assert!(
            notice(&with_settings_context(
                routes.clone(),
                Some(ProxyMode::Tun),
                Some(&settings)
            ))
            .is_some()
        );
        let mut absent = routes.clone();
        absent["ipv6_uplink"] = json!({"state":"unavailable"});
        let context = with_settings_context(absent.clone(), Some(ProxyMode::Tun), Some(&settings));
        assert!(notice(&context).is_none());
        assert_eq!(context["coverage_expectations"]["source"], "no_uplink_ipv6");
        assert!(coverage_issue(&absent, &json!({"tun":{"enable":true}}), true).is_none());
        absent["ipv6_uplink"]["state"] = json!("available");
        assert!(
            notice(&with_settings_context(
                absent,
                Some(ProxyMode::Tun),
                Some(&settings)
            ))
            .is_some()
        );
        let mut bypass = routes;
        bypass["tcp_uid_samples"][0]["ipv4"]["kind"] = json!("other");
        settings.claude_protection = None;
        assert!(
            notice(&with_settings_context(
                bypass,
                Some(ProxyMode::Tun),
                Some(&settings)
            ))
            .is_some()
        );
    }

    #[test]
    fn default_ipv6_handles_absence_arrival_loss_and_unknown_uplink_without_hiding_bypass() {
        let settings = crate::config::Settings::default();
        assert!(settings.ipv6);
        let mut routes = json!({
            "scope":"current_network_namespace",
            "ipv4":{"state":"observed","kind":"tun","interface":"Meta"},
            "ipv6":{"state":"observed","kind":"other","interface":"eth0"}
        });
        let config = json!({"ipv6":true,"tun":{"enable":true,"inet6-address":[]}});
        for state in ["unavailable", "available", "unavailable", "unknown"] {
            routes["ipv6_uplink"] = json!({"state":state});
            let context =
                with_settings_context(routes.clone(), Some(ProxyMode::Tun), Some(&settings));
            assert_eq!(notice(&context).is_some(), state != "unavailable");
            assert_eq!(context["ipv6"], routes["ipv6"]);
            assert_eq!(
                coverage_issue(&routes, &config, false).is_some(),
                state != "unavailable"
            );
            assert_eq!(
                coverage_issue(&routes, &config, true).is_some(),
                state != "unavailable"
            );
        }
        routes["ipv6_uplink"]["state"] = json!("available");
        routes["ipv6"]["kind"] = json!("tun");
        routes["ipv6"]["interface"] = json!("Meta");
        assert!(coverage_issue(&routes, &config, false).is_none());
        assert!(
            notice(&with_settings_context(
                routes.clone(),
                Some(ProxyMode::Tun),
                Some(&settings)
            ))
            .is_none()
        );
        routes["ipv4"]["kind"] = json!("other");
        routes["ipv6_uplink"]["state"] = json!("unavailable");
        assert!(
            coverage_issue(&routes, &config, false)
                .unwrap()
                .contains("IPv4")
        );
        assert!(
            notice(&with_settings_context(
                routes,
                Some(ProxyMode::Tun),
                Some(&settings)
            ))
            .is_some()
        );
    }

    #[test]
    fn disabled_ipv6_is_not_required_even_on_a_dual_stack_uplink_but_protection_is_strict() {
        let routes = json!({
            "scope":"current_network_namespace",
            "ipv4":{"state":"observed","kind":"tun","interface":"Meta"},
            "ipv6":{"state":"observed","kind":"other","interface":"eth0"},
            "ipv6_uplink":{"state":"available"}
        });
        let config = json!({"ipv6":false,"tun":{"inet6-address":[]}});
        assert!(coverage_issue(&routes, &config, false).is_none());
        assert!(
            coverage_issue(&routes, &config, true)
                .unwrap()
                .contains("IPv6")
        );
        let settings = crate::config::Settings {
            ipv6: false,
            ..Default::default()
        };
        assert!(
            notice(&with_settings_context(
                routes,
                Some(ProxyMode::Tun),
                Some(&settings)
            ))
            .is_none()
        );
    }

    #[test]
    fn missing_route_evidence_is_unknown_but_invalid_address_metadata_cannot_skip_checks() {
        let routes = json!({
            "scope":"current_network_namespace",
            "ipv4":{"state":"observed","kind":"tun","interface":"Meta"},
            "ipv6":{"state":"observed","kind":"other","interface":"eth0"}
        });
        let mut config = json!({
            "ipv6":true,
            "tun":{"enable":true,"inet6-address":["fdfe:dcba:9876::1/126"]}
        });
        assert!(
            coverage_issue(&routes, &config, false)
                .unwrap()
                .contains("IPv6")
        );
        config["tun"]["inet6-address"] = "invalid".into();
        assert!(
            coverage_issue(&routes, &config, false)
                .unwrap()
                .contains("IPv6")
        );
        let mut unknown = routes.clone();
        unknown["ipv6"] = json!({"state":"unknown","reason":"route_unavailable"});
        for required in [false, true] {
            assert!(coverage_issue(&unknown, &config, required).is_none());
            assert!(coverage_issue(&json!({"scope":"unsupported"}), &config, required).is_none());
        }
    }

    #[test]
    fn cross_user_tun_requires_an_explicit_privileged_scan() {
        let routes = json!({
            "scope":"current_network_namespace",
            "holder_scope":"current_user",
            "ipv4":{"state":"observed","kind":"tun","interface":"Meta"},
            "ipv6":{"state":"unknown"},
            "tun_holders":{"state":"complete","interfaces":{"Meta":[]}}
        });
        let current = notice(&with_proxy_context(routes.clone(), Some(ProxyMode::Off))).unwrap();
        assert!(current.contains("sudo cor status --all-users"), "{current}");
        let mut all_users = routes;
        all_users["holder_scope"] = json!("all_users");
        all_users["tun_holders"]["scope"] = json!("all_users");
        assert!(
            holder_summary(&all_users)
                .unwrap()
                .contains("未找到持有进程")
        );
        let all_users = notice(&with_proxy_context(all_users, Some(ProxyMode::Off))).unwrap();
        assert!(
            all_users.contains("跨用户扫描未找到持有进程"),
            "{all_users}"
        );
        assert!(!all_users.contains("sudo cor status"), "{all_users}");
    }
}
