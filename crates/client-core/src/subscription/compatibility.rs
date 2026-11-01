//! Subscription format negotiation; these aliases do not change node protocols.
use anyhow::{Result, bail};
use std::fmt;
use yaml_serde::Value;

pub(super) fn user_agent(yaml_only: bool) -> String {
    let aliases = "clash.meta mihomo ClashMeta clash-verge ClashVerge ClashMetaForAndroid";
    let extra = if yaml_only { "" } else { " v2rayN v2rayNG" };
    format!("{aliases}{extra} clash-of-rust/{}", crate::VERSION)
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Incompatible {
    Format,
    Hint,
    Http(u16),
}

impl Incompatible {
    pub(super) fn is_format(self) -> bool {
        !matches!(self, Self::Http(_))
    }
}

impl fmt::Display for Incompatible {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format => {
                formatter.write_str("订阅格式不兼容，请选择 Mihomo/Clash Meta YAML 订阅")
            }
            Self::Hint => formatter
                .write_str("订阅服务器返回客户端兼容提示节点，请选择 Mihomo/Clash Meta YAML 订阅"),
            Self::Http(status) => write!(formatter, "订阅服务器返回 HTTP {status}"),
        }
    }
}

impl std::error::Error for Incompatible {}

pub(super) fn validate(raw: &str) -> Result<()> {
    // Parser diagnostics may contain subscription credentials or node links.
    let map = crate::config::parse_profile(raw).map_err(|_| Incompatible::Format)?;
    if map
        .get(Value::from("proxies"))
        .is_some_and(|value| !value.is_sequence())
        || map
            .get(Value::from("proxy-providers"))
            .is_some_and(|value| !value.is_mapping())
    {
        bail!(Incompatible::Format);
    }
    if map
        .get(Value::from("proxy-providers"))
        .and_then(Value::as_mapping)
        .is_some_and(|providers| !providers.is_empty())
    {
        return Ok(());
    }
    let Some(proxies) = map.get(Value::from("proxies")).and_then(Value::as_sequence) else {
        return Ok(());
    };
    let has_hint = proxies.iter().any(|proxy| {
        proxy
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| {
                name.contains("不支持本机场协议") || name.contains("请更换以下支持协议的代理软件")
            })
    });
    let Some(first) = proxies.first().and_then(Value::as_mapping) else {
        return Ok(());
    };
    // Some panels clone one dummy SS endpoint into several help-message nodes.
    // A real node or provider alongside the hint must remain usable.
    if has_hint
        && first.get(Value::from("type")).and_then(Value::as_str) == Some("ss")
        && first.contains_key(Value::from("server"))
        && first.contains_key(Value::from("port"))
        && proxies.iter().all(|proxy| {
            proxy.as_mapping().is_some_and(|proxy| {
                proxy.len() == first.len()
                    && proxy.iter().all(|(key, value)| {
                        key.as_str() == Some("name") || first.get(key) == Some(value)
                    })
            })
        })
    {
        bail!(Incompatible::Hint);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) const HINTS: &str = "proxies:\n\
        - {name: 当前Clash客户端不支持本机场协议, type: ss, server: 127.0.0.1, port: 1, cipher: aes-128-gcm, password: placeholder}\n\
        - {name: 'Win客户端：V2rayN,ClashVerge', type: ss, server: 127.0.0.1, port: 1, cipher: aes-128-gcm, password: placeholder}\n\
        - {name: 请更换以下支持协议的代理软件, type: ss, server: 127.0.0.1, port: 1, cipher: aes-128-gcm, password: placeholder}\n";

    #[test]
    fn dummy_help_nodes_are_rejected_without_rejecting_real_nodes_or_providers() {
        assert!(validate(HINTS).unwrap_err().is::<Incompatible>());
        let real = format!(
            "{HINTS}- {{name: real, type: vless, server: example.test, port: 443, uuid: fixture}}\n"
        );
        validate(&real).unwrap();
        let real_ss = format!(
            "{HINTS}- {{name: real, type: ss, server: example.test, port: 443, cipher: aes-128-gcm, password: fixture}}\n"
        );
        validate(&real_ss).unwrap();
        let provider = format!(
            "{HINTS}proxy-providers:\n  remote: {{type: http, url: 'https://example.test/proxies'}}\n"
        );
        validate(&provider).unwrap();
        let named_like_a_hint = HINTS.replace("type: ss", "type: vless");
        validate(&named_like_a_hint).unwrap();
    }
}
