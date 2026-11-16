use super::{Mapping, Value};
use anyhow::{Context, Result, ensure};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Component, Path};

const FORBIDDEN: &[&str] = &[
    "external-ui",
    "external-ui-url",
    "external-controller-tls",
    "external-controller-unix",
    "external-controller-pipe",
    "external-controller-cors",
    "external-doh-server",
    "tunnels",
    "authentication",
    "cor-application-rules",
    "iptables",
];

pub(super) fn subscription(map: &mut Mapping) {
    sanitize(map);
    map.remove(Value::from("listeners"));
}

fn sanitize(map: &mut Mapping) {
    for key in FORBIDDEN {
        map.remove(Value::from(*key));
    }
    if let Some(ntp) = map
        .get_mut(Value::from("ntp"))
        .and_then(Value::as_mapping_mut)
    {
        ntp.insert(Value::from("write-to-system"), Value::from(false));
    }
}

fn safe_listener(value: &Value) -> bool {
    let Some(listener) = value.as_mapping() else {
        return false;
    };
    let kind = listener.get(Value::from("type")).and_then(Value::as_str);
    let listen = listener.get(Value::from("listen")).and_then(Value::as_str);
    let port = listener.get(Value::from("port")).and_then(Value::as_u64);
    matches!(kind, Some("http" | "socks" | "mixed"))
        && listen == Some("127.0.0.1")
        && port.is_some_and(|port| (1024..=65535).contains(&port))
        && listener
            .get(Value::from("udp"))
            .is_none_or(|value| value.as_bool() == Some(false))
}

pub fn privileged(map: &mut Mapping) -> Result<()> {
    sanitize(map);
    for key in ["port", "socks-port", "redir-port", "tproxy-port"] {
        map.insert(Value::from(key), Value::from(0));
    }
    map.insert(Value::from("allow-lan"), Value::from(false));
    map.insert(Value::from("bind-address"), Value::from("127.0.0.1"));
    if let Some(value) = map.remove(Value::from("listeners"))
        && let Some(items) = value.as_sequence()
    {
        let listeners: Vec<_> = items
            .iter()
            .filter(|item| safe_listener(item))
            .take(4)
            .cloned()
            .collect();
        if !listeners.is_empty() {
            map.insert(Value::from("listeners"), Value::Sequence(listeners));
        }
    }
    let controller = map
        .get(Value::from("external-controller"))
        .and_then(Value::as_str)
        .context("控制接口缺失")?
        .parse::<SocketAddr>()
        .context("控制接口地址无效")?;
    ensure!(
        controller.ip() == Ipv4Addr::LOCALHOST && controller.port() != 0,
        "服务仅支持本机控制接口"
    );
    ensure!(
        map.get(Value::from("secret"))
            .and_then(Value::as_str)
            .is_some_and(|secret| !secret.is_empty()),
        "控制接口必须设置密钥"
    );
    if let Some(listen) = map
        .get(Value::from("dns"))
        .and_then(Value::as_mapping)
        .and_then(|dns| dns.get(Value::from("listen")))
    {
        let address = listen
            .as_str()
            .context("DNS 监听地址必须是字符串")?
            .parse::<SocketAddr>()
            .context("DNS 监听地址无效")?;
        ensure!(address.ip().is_loopback(), "服务仅支持本机 DNS 监听");
    }
    Ok(())
}

pub fn provider_paths(config: &mut Mapping, original: &Path) -> Result<()> {
    let canonical = original.canonicalize().ok();
    for key in ["proxy-providers", "rule-providers"] {
        let Some(providers) = config
            .get_mut(Value::from(key))
            .and_then(Value::as_mapping_mut)
        else {
            continue;
        };
        for provider in providers.values_mut() {
            let Some(path) = provider
                .as_mapping_mut()
                .and_then(|provider| provider.get_mut(Value::from("path")))
            else {
                continue;
            };
            let supplied = Path::new(path.as_str().context("Provider 路径必须是字符串")?);
            let relative = if supplied.is_absolute() {
                supplied
                    .strip_prefix(original)
                    .or_else(|_| supplied.strip_prefix(canonical.as_deref().unwrap_or(original)))
                    .context("Provider 路径必须位于运行目录")?
            } else {
                supplied
            };
            ensure!(
                !relative.as_os_str().is_empty()
                    && relative
                        .components()
                        .all(|part| matches!(part, Component::Normal(_))),
                "Provider 路径不能越出运行目录"
            );
            *path = Value::from(relative.to_str().context("Provider 路径编码无效")?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Mapping {
        yaml_serde::from_str(
            "external-controller: 127.0.0.1:9090\nsecret: test-controller-secret\n",
        )
        .unwrap()
    }

    #[test]
    fn privileged_ingress_preserves_only_managed_loopback_tcp_listeners() {
        let mut map = config();
        map.insert(
            Value::from("listeners"),
            yaml_serde::from_str(
                "- {name: direct, type: mixed, listen: 127.0.0.1, port: 42131, proxy: DIRECT, udp: false}\n\
                 - {name: unix, type: unix, listen: /tmp/core.sock, port: 1}\n\
                 - {name: public, type: mixed, listen: 0.0.0.0, port: 42132}\n\
                 - {name: udp, type: socks, listen: 127.0.0.1, port: 42133, udp: true}\n\
                 - {name: coerced-udp, type: socks, listen: 127.0.0.1, port: 42134, udp: 'true'}\n\
                 - {name: privileged, type: http, listen: 127.0.0.1, port: 80}\n",
            )
            .unwrap(),
        );
        map.insert(
            Value::from("tunnels"),
            yaml_serde::from_str("[tcp/udp, 0.0.0.0:5555, example.invalid:443]").unwrap(),
        );
        map.insert(Value::from("external-doh-server"), Value::from(":443"));
        map.insert(Value::from("allow-lan"), Value::from(true));
        map.insert(Value::from("bind-address"), Value::from("*"));
        privileged(&mut map).unwrap();
        let result = Value::Mapping(map);
        let listeners = result["listeners"].as_sequence().unwrap();
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0]["name"], "direct");
        assert_eq!(result["allow-lan"], false);
        assert_eq!(result["bind-address"], "127.0.0.1");
        assert!(result["tunnels"].is_null());
        assert!(result["external-doh-server"].is_null());
    }

    #[test]
    fn privileged_ingress_rejects_public_dns_and_unauthenticated_controllers() {
        for listen in ["0.0.0.0:5353", "[::]:5353", "localhost:5353", "invalid"] {
            let mut map = config();
            map.insert(
                Value::from("dns"),
                yaml_serde::to_value(serde_json::json!({"listen": listen})).unwrap(),
            );
            assert!(privileged(&mut map).is_err(), "{listen}");
        }
        for listen in ["127.0.0.1:5353", "[::1]:5353", "127.0.0.1:0"] {
            let mut map = config();
            map.insert(
                Value::from("dns"),
                yaml_serde::to_value(serde_json::json!({"listen": listen})).unwrap(),
            );
            privileged(&mut map).unwrap();
        }
        for controller in ["0.0.0.0:9090", "localhost:9090", "127.0.0.1:0"] {
            let mut map = config();
            map.insert(Value::from("external-controller"), Value::from(controller));
            assert!(privileged(&mut map).is_err(), "{controller}");
        }
        let mut map = config();
        map.remove(Value::from("secret"));
        assert!(privileged(&mut map).is_err());
    }

    #[test]
    fn provider_paths_remain_relative_across_privileged_reloads() {
        let root = tempfile::tempdir().unwrap();
        let absolute = root.path().join("providers/nodes.yaml");
        let mut config: Mapping = yaml_serde::from_str(&format!(
            "proxy-providers: {{example: {{path: '{}'}}}}\n",
            absolute.display()
        ))
        .unwrap();
        provider_paths(&mut config, root.path()).unwrap();
        assert_eq!(
            config["proxy-providers"]["example"]["path"],
            "providers/nodes.yaml"
        );
        provider_paths(&mut config, root.path()).unwrap();
        for path in ["../outside", "/outside/nodes.yaml"] {
            let mut config: Mapping = yaml_serde::from_str(&format!(
                "rule-providers: {{example: {{path: '{path}'}}}}\n"
            ))
            .unwrap();
            assert!(provider_paths(&mut config, root.path()).is_err());
        }
    }
}
