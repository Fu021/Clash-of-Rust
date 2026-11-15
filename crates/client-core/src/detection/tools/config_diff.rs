use super::*;
use crate::config::{Settings, Store};
use std::{io::Read, path::Path};
use yaml_serde::Value;

pub(super) fn read(root: &Path) -> Result<Vec<Entry>> {
    let store = Store { root: root.into() };
    let settings: Settings =
        serde_json::from_reader(std::fs::File::open(root.join("settings.json"))?.take(1024 * 1024))
            .map_err(|_| error::failure("config_unavailable", "已保存设置不可读或格式无效"))?;
    let id = settings
        .active_profile
        .as_deref()
        .ok_or_else(|| error::failure("config_unavailable", "未选择订阅"))?;
    let original = read_file(&store.profile_path(id)?)?;
    let effective = read_file(&store.runtime().join("config.yaml"))?;
    compare(&original, &effective)
}

pub(super) fn read_file(path: &Path) -> Result<String> {
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(8 * 1024 * 1024 + 1)
        .read_to_string(&mut text)?;
    if text.len() > 8 * 1024 * 1024 {
        return Err(error::failure(
            "config_snapshot_limit",
            "配置超过本次 8 MiB 分析上限，未继续解析",
        ));
    }
    Ok(text)
}

pub(super) fn compare(original: &str, effective: &str) -> Result<Vec<Entry>> {
    let original: Value = yaml_serde::from_str(original)
        .map_err(|_| error::failure("config_invalid", "原订阅 YAML 无法解析"))?;
    let effective: Value = yaml_serde::from_str(effective)
        .map_err(|_| error::failure("config_invalid", "已生成配置 YAML 无法解析"))?;
    ensure!(
        original.is_mapping() && effective.is_mapping(),
        "配置不是映射"
    );
    let mut entries = Vec::new();
    for field in [
        "mode",
        "ipv6",
        "allow-lan",
        "mixed-port",
        "dns",
        "tun",
        "sniffer",
        "rules",
        "proxies",
        "proxy-groups",
        "proxy-providers",
        "rule-providers",
    ] {
        let before = original.get(field);
        let after = effective.get(field);
        let changed = before != after;
        entries.push(Entry::ok(
            field,
            format!(
                "{} → {} · {}",
                shape(before),
                shape(after),
                if changed {
                    "已覆盖或变化"
                } else {
                    "一致"
                }
            ),
        ));
    }
    for (section, field) in [
        ("dns", "enable"),
        ("dns", "enhanced-mode"),
        ("tun", "enable"),
        ("tun", "auto-route"),
        ("tun", "strict-route"),
    ] {
        let value = effective
            .get(section)
            .and_then(|section| section.get(field));
        let text = match value {
            Some(Value::Bool(value)) => {
                if *value {
                    "开启"
                } else {
                    "关闭"
                }
            }
            Some(Value::String(value))
                if field == "enhanced-mode"
                    && matches!(value.as_str(), "fake-ip" | "redir-host") =>
            {
                value
            }
            _ => "未提供或格式未知",
        };
        entries.push(Entry::ok(&format!("{section}.{field}"), text));
    }
    entries.push(Entry::unknown(
        "对照范围",
        "原订阅与磁盘上最后生成的配置；不等同内核实时配置。不显示值正文、节点地址、DNS 上游或凭据",
        "information",
    ));
    Ok(entries)
}

fn shape(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "未设置".into(),
        Some(Value::Sequence(items)) => format!("列表 {} 项", items.len()),
        Some(Value::Mapping(items)) => format!("对象 {} 项", items.len()),
        Some(Value::Bool(_)) => "布尔值".into(),
        Some(Value::Number(_)) => "数字".into(),
        _ => "文本".into(),
    }
}
