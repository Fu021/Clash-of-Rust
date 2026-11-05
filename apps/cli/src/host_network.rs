use serde_json::{Value, json};

pub(crate) fn warn_tun() {
    eprintln!("TUN 可能接管当前网络命名空间中其他用户和程序的流量；实际范围取决于系统路由。");
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

pub(crate) async fn observe() -> Value {
    #[cfg(target_os = "linux")]
    {
        let (ipv4, ipv6) =
            tokio::join!(route("-4", "1.1.1.1"), route("-6", "2606:4700:4700::1111"));
        json!({
            "scope":"current_network_namespace",
            "ipv4":ipv4,
            "ipv6":ipv6
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        json!({"scope":"unsupported"})
    }
}

pub(crate) fn with_proxy_context(
    mut routes: Value,
    proxy: Option<clash_client_runtime::engine::ProxyMode>,
) -> Value {
    use clash_client_runtime::engine::ProxyMode;

    let samples = [&routes["ipv4"], &routes["ipv6"]];
    let tun = samples
        .iter()
        .any(|route| route["state"] == "observed" && route["kind"] == "tun");
    let other = samples
        .iter()
        .any(|route| route["state"] == "observed" && route["kind"] == "other");
    let warning = if tun && proxy != Some(ProxyMode::Tun) {
        Some("tun_owner_unconfirmed")
    } else if !tun && other && proxy == Some(ProxyMode::Tun) {
        Some("sample_routes_not_tun")
    } else {
        None
    };
    routes["warnings"] = json!(warning.into_iter().collect::<Vec<_>>());
    routes
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
    use clash_client_runtime::engine::ProxyMode;

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
            json!(["tun_owner_unconfirmed"])
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
