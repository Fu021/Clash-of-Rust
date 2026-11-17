use super::form::{self, Form};
use anyhow::{Context, Result, ensure};
use clash_client_runtime::control::{Command, Poll};
use serde_json::{Value, json};

pub(super) const KEYS: &[&str] = &[
    "run_mode",
    "proxy_mode",
    "claude_protection",
    "tui_node_sort",
    "mixed_port",
    "dns_override",
    "fake_ip",
    "fake_ip_exclusions",
    "fake_ip_disabled_defaults",
    "ipv6",
    "udp",
    "update_prereleases",
    "update_check_interval_minutes",
    "delay_interval_minutes",
    "subscription_update_interval_minutes",
];

pub(super) const RULE_KEYS: &[&str] = &[
    "shortcut_ads",
    "shortcut_private_domain",
    "shortcut_private_ip",
    "shortcut_cn_domain",
    "shortcut_cn_ip",
];

pub(super) fn category(key: &str) -> &'static str {
    match key {
        "run_mode" | "proxy_mode" | "claude_protection" | "tui_node_sort" => "常规与连接",
        "mixed_port"
        | "dns_override"
        | "fake_ip"
        | "fake_ip_exclusions"
        | "fake_ip_disabled_defaults"
        | "ipv6"
        | "udp" => "网络与 DNS",
        "update_prereleases" | "update_check_interval_minutes" => "软件更新",
        "active_profile" => "订阅",
        key if key.starts_with("shortcut_") => "快捷规则",
        _ => "计划任务",
    }
}

const SHORTCUTS: &[(&str, &str, &str)] = &[
    ("shortcut_ads", "ads", "广告拦截"),
    ("shortcut_private_domain", "private-domain", "私有域名直连"),
    ("shortcut_private_ip", "private-ip", "私有 IP 直连"),
    ("shortcut_cn_domain", "cn-domain", "中国域名直连"),
    ("shortcut_cn_ip", "cn-ip", "中国 IP 直连"),
];

pub(super) fn title(key: &str) -> &str {
    SHORTCUTS
        .iter()
        .find(|(setting, _, _)| *setting == key)
        .map_or_else(|| form::setting_title(key), |(_, _, title)| *title)
}

pub(super) fn value(poll: &Poll, key: &str) -> Value {
    if let Some(index) = SHORTCUTS.iter().position(|(setting, _, _)| *setting == key) {
        json!(
            poll.meta
                .profiles
                .iter()
                .find(|profile| Some(&profile.id) == poll.meta.settings.active_profile.as_ref())
                .is_some_and(|profile| {
                    clash_client_core::rule_manager::ProfileRules::for_profile(
                        &poll.meta.settings,
                        &profile.id,
                        profile.builtin,
                    )
                    .enabled[index]
                })
        )
    } else {
        match key {
            "proxy_mode" => json!(poll.snapshot.proxy_mode()),
            "run_mode" if !poll.snapshot.mode.is_empty() => json!(poll.snapshot.mode),
            "fake_ip" => json!(poll.meta.settings.fake_ip_enabled()),
            _ => form::setting_value(&poll.meta.settings, key),
        }
    }
}

pub(super) fn display(poll: &Poll, key: &str) -> String {
    if key.starts_with("shortcut_")
        && !poll.meta.profiles.iter().any(|profile| {
            Some(&profile.id) == poll.meta.settings.active_profile.as_ref() && !profile.builtin
        })
    {
        return "仅适用于非内置订阅".into();
    }
    let value = value(poll, key);
    match key {
        "active_profile" => poll
            .meta
            .profiles
            .iter()
            .find(|profile| Some(&profile.id) == poll.meta.settings.active_profile.as_ref())
            .map_or_else(|| "尚未选择".into(), |profile| profile.name.clone()),
        "tui_node_sort" => poll.meta.settings.tui_node_sort.to_string(),
        "fake_ip" if !poll.meta.settings.dns_override => "关闭（DNS 覆盖未开启）".into(),
        "fake_ip_exclusions" => format!(
            "{} 项自定义 · {} 项有效排除",
            poll.meta.settings.fake_ip_exclusions.len(),
            poll.meta.settings.fake_ip_exclusion_entries().len()
        ),
        "fake_ip_disabled_defaults" => format!(
            "{} 项已停用 · {} 项内置",
            poll.meta.settings.fake_ip_disabled_defaults.len(),
            clash_client_core::config::DEFAULT_FAKE_IP_EXCLUSIONS.len()
        ),
        "run_mode" => match value.as_str().unwrap_or("") {
            "rule" => "规则".into(),
            "global" => "全局".into(),
            "direct" => "直连".into(),
            other => other.into(),
        },
        "proxy_mode" => {
            let actual = poll.snapshot.proxy_mode();
            let label = |mode| match mode {
                clash_client_runtime::engine::ProxyMode::Off => "关闭",
                clash_client_runtime::engine::ProxyMode::System => "系统代理",
                clash_client_runtime::engine::ProxyMode::Tun => "TUN",
            };
            if actual == poll.meta.settings.proxy_mode {
                label(actual).into()
            } else {
                format!(
                    "{}（启动偏好：{}）",
                    label(actual),
                    label(poll.meta.settings.proxy_mode)
                )
            }
        }
        _ => match value {
            Value::Bool(enabled) => if enabled { "开启" } else { "关闭" }.into(),
            Value::String(value) => value,
            other => other.to_string(),
        },
    }
}

pub(super) fn hint(key: &str) -> &'static str {
    match key {
        "proxy_mode" | "claude_protection" => {
            "选择状态后确认；系统授权在普通终端完成，返回后读取实际状态。"
        }
        "active_profile" => "选择订阅名称；新增、编辑来源、更新和删除在操作页。",
        "fake_ip_exclusions" | "fake_ip_disabled_defaults" => {
            "域名以逗号分隔或输入 JSON 数组；清空请填 []。"
        }
        "fake_ip" => "先开启 DNS 覆盖；关闭 DNS 覆盖会同时关闭 Fake-IP。",
        "tui_node_sort" => "只改变终端节点显示；节点页也可按 s 循环切换。",
        key if key.starts_with("shortcut_") => {
            "作用于当前订阅；自定义规则、顺序和订阅原规则在操作页管理。"
        }
        _ => "Enter/Space 或 ←→ 切换选项；文字与数值按 Enter 编辑，↓ 选择保存。",
    }
}

fn choices(form: &mut Form, index: usize, values: &[(&str, &str)]) {
    form.fields[index].choices = values
        .iter()
        .map(|(value, title)| ((*value).into(), (*title).into()))
        .collect();
}

pub(super) fn open(poll: &Poll, key: &str) -> Result<Form> {
    ensure!(
        KEYS.contains(&key) || RULE_KEYS.contains(&key) || key == "active_profile",
        "该设置请使用操作页的专用入口"
    );
    let value = value(poll, key);
    let mut form = Form::setting(key, &value);
    match key {
        "run_mode" | "proxy_mode" | "claude_protection" | "active_profile" => {
            let path = match key {
                "run_mode" => "mode",
                "proxy_mode" => "proxy",
                "claude_protection" => "claude protect",
                _ => "profile use",
            };
            form = Form::new(form::commands().find(|spec| spec.path == path).unwrap());
            form.fields.truncate(1);
            form.fields[0].label = "值".into();
            form.setting = Some(key.into());
            match key {
                "run_mode" => {
                    form.fields[0].value = value.as_str().context("运行模式无效")?.into();
                    choices(
                        &mut form,
                        0,
                        &[("rule", "规则"), ("global", "全局"), ("direct", "直连")],
                    );
                }
                "proxy_mode" => {
                    form.fields[0].value = value.as_str().context("代理方式无效")?.into();
                    choices(
                        &mut form,
                        0,
                        &[("off", "关闭"), ("system", "系统代理"), ("tun", "TUN")],
                    );
                }
                "claude_protection" => {
                    form.fields[0].value = if poll.meta.settings.claude_protection.is_some() {
                        "on"
                    } else {
                        "off"
                    }
                    .into();
                    choices(&mut form, 0, &[("off", "关闭"), ("on", "开启")]);
                }
                _ => {
                    ensure!(
                        !poll.meta.profiles.is_empty(),
                        "没有订阅，请先在操作页添加订阅"
                    );
                    form.fields[0].value = poll
                        .meta
                        .settings
                        .active_profile
                        .clone()
                        .unwrap_or_default();
                    form.fields[0].choices = poll
                        .meta
                        .profiles
                        .iter()
                        .map(|profile| (profile.id.clone(), profile.name.clone()))
                        .collect();
                }
            }
        }
        "tui_node_sort" => choices(
            &mut form,
            1,
            &[
                ("name", "节点名称"),
                ("latency_ascending", "延迟从低到高"),
                ("latency_descending", "延迟从高到低"),
            ],
        ),
        key if key.starts_with("shortcut_") => {
            let (_, name, _) = SHORTCUTS
                .iter()
                .find(|(setting, _, _)| *setting == key)
                .unwrap();
            let id = poll
                .meta
                .settings
                .active_profile
                .as_ref()
                .context("尚未选择订阅")?;
            ensure!(
                poll.meta
                    .profiles
                    .iter()
                    .any(|profile| profile.id == *id && !profile.builtin),
                "快捷规则只适用于非内置订阅"
            );
            form = Form::new(
                form::commands()
                    .find(|spec| spec.path == "rule shortcut")
                    .unwrap(),
            );
            form.setting = Some(key.into());
            form.fields[0].value = (*name).into();
            form.fields[0].locked = true;
            form.fields[1].label = "值".into();
            form.fields[1].value = if value == true { "on" } else { "off" }.into();
            form.fields[2].value = id.clone();
            form.fields[2].locked = true;
            choices(&mut form, 1, &[("off", "关闭"), ("on", "开启")]);
            form.selected = 1;
        }
        _ if value.is_boolean() => choices(&mut form, 1, &[("false", "关闭"), ("true", "开启")]),
        _ => {}
    }
    Ok(form)
}

pub(super) fn inline(args: &crate::args::Args) -> bool {
    matches!(
        args.path.as_str(),
        "config set" | "mode" | "profile use" | "rule shortcut"
    )
}

pub(super) fn command(args: &crate::args::Args) -> Result<Command> {
    let first = args.positional.first().context("设置值缺失")?;
    Ok(match args.path.as_str() {
        "config set" => {
            ensure!(
                KEYS.contains(&first.as_str())
                    && clash_client_core::config::MUTABLE_SETTINGS.contains(&first.as_str()),
                "该设置请使用专用入口"
            );
            let raw = args.positional.get(1).context("设置值缺失")?;
            Command::ConfigSet {
                key: first.clone(),
                value: serde_json::from_str(raw).unwrap_or_else(|_| json!(raw)),
            }
        }
        "mode" => Command::Mode {
            mode: first.clone(),
        },
        "profile use" => Command::Activate { id: first.clone() },
        "rule shortcut" => Command::ActiveShortcut {
            id: args
                .get("profile")
                .context("订阅身份缺失，请重新打开设置")?
                .into(),
            index: SHORTCUTS
                .iter()
                .position(|(_, name, _)| *name == first.as_str())
                .context("未知快捷规则")?,
            enabled: args.positional.get(1).context("开关状态缺失")? == "on",
        },
        _ => anyhow::bail!("该操作需要普通终端授权"),
    })
}
