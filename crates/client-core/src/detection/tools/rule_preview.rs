use super::*;
use std::net::IpAddr;
use yaml_serde::Value;

pub(super) async fn run(
    url: &reqwest::Url,
    context: &Environment,
    report: &mut Report,
) -> Result<()> {
    let path = context
        .data_dir
        .as_ref()
        .ok_or_else(|| error::failure("config_unavailable", "当前实例数据目录不可用"))?
        .join("runtime/config.yaml");
    let host = url
        .host_str()
        .context("目标缺少主机名")?
        .trim_matches(['[', ']'])
        .to_owned();
    let port = url.port_or_known_default().context("缺少端口")?;
    let protected = context.protected;
    report.entries.extend(
        tokio::task::spawn_blocking(move || {
            let raw = super::config_diff::read_file(&path)?;
            preview(&raw, &host, port, protected)
        })
        .await??,
    );
    Ok(())
}

pub(super) fn preview(raw: &str, host: &str, port: u16, protected: bool) -> Result<Vec<Entry>> {
    let config: Value = yaml_serde::from_str(raw)
        .map_err(|_| error::failure("config_invalid", "已生成配置 YAML 无法解析"))?;
    let mut entries = vec![Entry::unknown(
        "预判范围",
        "读取最后生成的配置，不连接目标、不解析 DNS；实际线路请用“规则与出口”检查",
        "information",
    )];
    if protected {
        entries.push(Entry::unknown(
            "Claude 保护",
            "保护开启或状态未知，静态配置不足以确认当前受保护出口",
            "rule_unknown",
        ));
        return Ok(entries);
    }
    match config.get("mode").and_then(Value::as_str) {
        Some("direct") => {
            entries.push(Entry::ok("运行模式", "直连 · DIRECT"));
            return Ok(entries);
        }
        Some("global") => {
            entries.push(Entry::ok(
                "运行模式",
                "全局 · GLOBAL；此工具不解析组的实时选择",
            ));
            return Ok(entries);
        }
        _ => {}
    }
    let rules = config
        .get("rules")
        .and_then(Value::as_sequence)
        .ok_or_else(|| error::failure("config_unavailable", "配置没有可预判的规则列表"))?;
    for (index, raw) in rules.iter().enumerate() {
        let Some(raw) = raw.as_str() else {
            entries.push(Entry::unknown(
                "规则",
                format!("第 {} 条不是文本，无法继续预判", index + 1),
                "rule_unknown",
            ));
            return Ok(entries);
        };
        let Ok(rule) = crate::rule_manager::parts(raw) else {
            entries.push(Entry::unknown(
                "规则",
                format!("第 {} 条格式无法判断", index + 1),
                "rule_unknown",
            ));
            return Ok(entries);
        };
        match matches_rule(rule.kind, rule.payload, host, port) {
            Some(false) => {}
            Some(true) => {
                entries.push(Entry::ok(
                    "静态命中",
                    format!(
                        "第 {} 条 · {} → {}",
                        index + 1,
                        rule.kind,
                        bounded(rule.policy, 128)
                    ),
                ));
                return Ok(entries);
            }
            None => {
                entries.push(Entry::unknown(
                    "无法确认",
                    format!(
                        "第 {} 条 · {} 需要动态信息或未支持，后续命中不能确定",
                        index + 1,
                        rule.kind
                    ),
                    "rule_unknown",
                ));
                return Ok(entries);
            }
        }
    }
    entries.push(Entry::unknown(
        "静态命中",
        "没有可确认命中的规则",
        "rule_no_match",
    ));
    Ok(entries)
}

fn matches_rule(kind: &str, payload: &str, host: &str, port: u16) -> Option<bool> {
    let domain = host.trim_end_matches('.').to_ascii_lowercase();
    let pattern = payload.trim_end_matches('.').to_ascii_lowercase();
    let literal = host.parse::<IpAddr>().ok();
    match kind {
        "MATCH" => Some(true),
        "DOMAIN" | "DOMAIN-SUFFIX" | "DOMAIN-KEYWORD" if literal.is_some() => Some(false),
        "DOMAIN" => Some(domain == pattern),
        "DOMAIN-SUFFIX" => Some(
            domain == pattern
                || domain
                    .strip_suffix(&pattern)
                    .is_some_and(|prefix| prefix.ends_with('.')),
        ),
        "DOMAIN-KEYWORD" => Some(domain.contains(&pattern)),
        "IP-CIDR" | "IP-CIDR6" => cidr(payload, literal?),
        "DST-PORT" => payload.parse::<u16>().ok().map(|value| port == value),
        "NETWORK" => Some(payload.eq_ignore_ascii_case("tcp")),
        _ => None,
    }
}

fn cidr(payload: &str, target: IpAddr) -> Option<bool> {
    let (address, prefix) = payload.split_once('/')?;
    let address = address.parse::<IpAddr>().ok()?;
    let prefix = prefix.parse::<u32>().ok()?;
    match (address, target) {
        (IpAddr::V4(network), IpAddr::V4(target)) if prefix <= 32 => {
            let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
            Some(u32::from(network) & mask == u32::from(target) & mask)
        }
        (IpAddr::V6(network), IpAddr::V6(target)) if prefix <= 128 => {
            let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
            Some(u128::from(network) & mask == u128::from(target) & mask)
        }
        (IpAddr::V4(_), IpAddr::V6(_)) | (IpAddr::V6(_), IpAddr::V4(_)) => Some(false),
        _ => None,
    }
}
