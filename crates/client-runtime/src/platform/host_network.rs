use serde_json::{Value, json};

#[cfg(target_os = "linux")]
mod linux;

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
    #[cfg(target_os = "linux")]
    {
        let (ipv4, ipv6) =
            tokio::join!(route("-4", "1.1.1.1"), route("-6", "2606:4700:4700::1111"));
        let routes = json!({
            "scope":"current_network_namespace",
            "observer_uid":unsafe { libc::geteuid() },
            "ipv4":ipv4,
            "ipv6":ipv6
        });
        let fallback = routes.clone();
        tokio::task::spawn_blocking(move || linux::attach_holders(routes))
            .await
            .unwrap_or_else(|_| {
                let mut routes = fallback;
                routes["tun_holders"] =
                    json!({"state":"unavailable","reason":"owner_query_failed"});
                routes
            })
    }
    #[cfg(not(target_os = "linux"))]
    {
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
    let warning = if tun && proxy.is_none() {
        Some("tun_current_proxy_unknown")
    } else if tun && proxy != Some(ProxyMode::Tun) {
        Some("tun_owner_unconfirmed")
    } else if !tun && other && proxy == Some(ProxyMode::Tun) {
        Some("sample_routes_not_tun")
    } else {
        None
    };
    routes["warnings"] = json!(warning.into_iter().collect::<Vec<_>>());
    routes
}

pub fn notice(routes: &Value) -> Option<String> {
    if routes["scope"] != "current_network_namespace"
        || !routes["warnings"].as_array().is_some_and(|warnings| {
            warnings.iter().any(|warning| {
                matches!(
                    warning.as_str(),
                    Some("tun_owner_unconfirmed" | "tun_current_proxy_unknown")
                )
            })
        })
    {
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
        let mut owners = Vec::new();
        for holder in holders.iter().take(3) {
            let (Some(uid), Some(pid)) = (holder["uid"].as_u64(), holder["pid"].as_u64()) else {
                continue;
            };
            let user = if uid == 0 {
                "root".to_owned()
            } else {
                format!("UID {uid}")
            };
            owners.push(format!("{user} · PID {pid}"));
        }
        evidence.push(if owners.is_empty() {
            format!("{interface}；持有进程无法读取")
        } else {
            format!("{interface}；持有进程 {}", owners.join("、"))
        });
    }
    if evidence.is_empty() {
        return None;
    }
    let uncertain = routes["warnings"].as_array().is_some_and(|warnings| {
        warnings
            .iter()
            .any(|warning| warning == "tun_current_proxy_unknown")
    });
    let message = if uncertain {
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
}
