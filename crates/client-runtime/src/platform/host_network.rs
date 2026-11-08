use serde_json::{Value, json};

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

pub async fn observe() -> Value {
    observe_scope(HolderScope::CurrentUser).await
}

pub async fn observe_all_users() -> Value {
    observe_scope(HolderScope::AllUsers).await
}

async fn observe_scope(holder_scope: HolderScope) -> Value {
    #[cfg(target_os = "linux")]
    {
        let (ipv4, ipv6) =
            tokio::join!(route("-4", "1.1.1.1"), route("-6", "2606:4700:4700::1111"));
        let routes = json!({
            "scope":"current_network_namespace",
            "observer_uid":unsafe { libc::geteuid() },
            "holder_scope":holder_scope.label(),
            "ipv4":ipv4,
            "ipv6":ipv6
        });
        let fallback = routes.clone();
        tokio::task::spawn_blocking(move || linux::attach_holders(routes, holder_scope))
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

pub fn with_proxy_context(mut routes: Value, proxy: Option<crate::engine::ProxyMode>) -> Value {
    use crate::engine::ProxyMode;

    let samples = [&routes["ipv4"], &routes["ipv6"]];
    let tun = samples
        .iter()
        .any(|route| route["state"] == "observed" && route["kind"] == "tun");
    let other = samples
        .iter()
        .any(|route| route["state"] == "observed" && route["kind"] == "other");
    let mut warnings = Vec::new();
    if tun && proxy.is_none() {
        warnings.push("tun_current_proxy_unknown");
    } else if tun && proxy != Some(ProxyMode::Tun) {
        warnings.push("tun_owner_unconfirmed");
    } else if !tun && other && proxy == Some(ProxyMode::Tun) {
        warnings.push("sample_routes_not_tun");
    }
    if proxy == Some(ProxyMode::Tun) && tun {
        if routes["ipv4"]["state"] == "observed" && routes["ipv4"]["kind"] == "other" {
            warnings.push("tun_ipv4_route_bypass");
        }
        if routes["ipv6"]["state"] == "observed" && routes["ipv6"]["kind"] == "other" {
            warnings.push("tun_ipv6_route_bypass");
        }
    }
    routes["warnings"] = json!(warnings);
    routes
}

/// A positive sample of a non-TUN route while this process owns TUN means the
/// host may still send some traffic outside the tunnel. Unknown or unavailable
/// samples are deliberately not treated as proof either way.
pub fn coverage_issue(routes: &Value, ipv6_expected: bool) -> Option<String> {
    if routes["scope"] != "current_network_namespace" {
        return None;
    }
    let mut issues = Vec::new();
    for (family, label, expected) in [("ipv4", "IPv4", true), ("ipv6", "IPv6", ipv6_expected)] {
        let route = &routes[family];
        if !expected || route["state"] != "observed" || route["kind"] != "other" {
            continue;
        }
        let interface = route["interface"].as_str().unwrap_or("物理接口");
        issues.push(format!("示例 {label} 路由仍经过 {interface}"));
    }
    (!issues.is_empty()).then(|| {
        format!(
            "TUN 已开启，但{}；部分程序可能绕过 TUN，请导出诊断检查 route-address、strict-route 与 DNS 覆盖",
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
    for family in ["ipv4", "ipv6"] {
        let route = &routes[family];
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
            Some("sample_routes_not_tun" | "tun_ipv4_route_bypass" | "tun_ipv6_route_bypass")
        )
    });
    if !external && !bypass {
        return None;
    }
    let mut interfaces = std::collections::BTreeSet::new();
    let mut evidence = Vec::new();
    for family in ["ipv4", "ipv6"] {
        let route = &routes[family];
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
    let message = if bypass && external {
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
}

#[cfg(test)]
mod context_tests {
    use super::*;
    use crate::engine::ProxyMode;

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
        assert!(coverage_issue(&routes, true).unwrap().contains("IPv4"));
        let ipv6_only = json!({
            "scope":"current_network_namespace",
            "ipv4":{"state":"observed","kind":"tun","interface":"Meta"},
            "ipv6":{"state":"observed","kind":"other","interface":"eth0"}
        });
        assert!(coverage_issue(&ipv6_only, false).is_none());
        assert!(coverage_issue(&ipv6_only, true).unwrap().contains("IPv6"));
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
