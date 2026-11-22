use super::form::{self, Form};
use anyhow::{Context, Result, ensure};
use clash_client_core::{
    config::{Language, LanguagePreference},
    proxy_order::NodeSort,
};
use clash_client_runtime::control::{Command, Poll};
use clash_client_runtime::engine::ProxyMode;
use serde_json::{Value, json};

pub(super) const KEYS: &[&str] = &[
    "run_mode",
    "proxy_mode",
    "claude_protection",
    "tui_node_sort",
    "language",
    "mixed_port",
    "dns_override",
    "fake_ip",
    "fake_ip_exclusions",
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

const NETWORK_KEYS: &[&str] = &[
    "mixed_port",
    "dns_override",
    "fake_ip",
    "fake_ip_exclusions",
    "fake_ip_disabled_defaults",
    "ipv6",
    "udp",
];

pub(super) fn category(key: &str, language: Language) -> &'static str {
    match key {
        "run_mode" | "proxy_mode" | "claude_protection" | "tui_node_sort" | "language" => {
            language.text("常规", "General")
        }
        key if NETWORK_KEYS.contains(&key) => language.text("网络", "Network"),
        "update_prereleases" => language.text("更新", "Updates"),
        "active_profile" => language.text("订阅", "Subscriptions"),
        key if key.starts_with("shortcut_") => language.text("快捷规则", "Rule shortcuts"),
        _ => language.text("计划任务", "Scheduled tasks"),
    }
}

pub(super) fn matches_query(key: &str, query: &str, language: Language) -> bool {
    let category = category(key, language);
    [
        key,
        title(key, language),
        category,
        if NETWORK_KEYS.contains(&key) {
            "DNS"
        } else {
            ""
        },
    ]
    .iter()
    .any(|value| value.to_lowercase().contains(query))
}

const SHORTCUTS: &[(&str, &str, &str, &str)] = &[
    ("shortcut_ads", "ads", "广告拦截", "Block ads"),
    (
        "shortcut_private_domain",
        "private-domain",
        "私有域名直连",
        "Private domains direct",
    ),
    (
        "shortcut_private_ip",
        "private-ip",
        "私有 IP 直连",
        "Private IPs direct",
    ),
    (
        "shortcut_cn_domain",
        "cn-domain",
        "中国域名直连",
        "China domains direct",
    ),
    (
        "shortcut_cn_ip",
        "cn-ip",
        "中国 IP 直连",
        "China IPs direct",
    ),
];

pub(super) fn title(key: &str, language: Language) -> &str {
    SHORTCUTS
        .iter()
        .find(|(setting, _, _, _)| *setting == key)
        .map_or_else(
            || form::setting_title(key, language),
            |(_, _, chinese, english)| language.text(chinese, english),
        )
}

fn language_choices(language: Language) -> [(&'static str, &'static str); 3] {
    [
        ("system", language.text("跟随系统", "Follow system")),
        ("zh-CN", "简体中文"),
        ("en", "English"),
    ]
}

pub(super) fn language_display(preference: LanguagePreference, language: Language) -> &'static str {
    language_choices(language)
        .into_iter()
        .find(|(tag, _)| *tag == preference.tag())
        .map_or("", |(_, label)| label)
}

pub(super) fn language_form(preference: LanguagePreference, language: Language) -> Form {
    let mut form = Form::setting("language", &json!(preference.tag()), language);
    choices(&mut form, 1, &language_choices(language));
    form
}

fn enabled_label(enabled: bool, language: Language) -> &'static str {
    if enabled {
        language.text("开启", "On")
    } else {
        language.text("关闭", "Off")
    }
}

fn proxy_mode_label(mode: ProxyMode, language: Language) -> &'static str {
    match mode {
        ProxyMode::Off => enabled_label(false, language),
        ProxyMode::System => language.text("系统代理", "System proxy"),
        ProxyMode::Tun => "TUN",
    }
}

fn routing_mode_label(mode: &str, language: Language) -> &str {
    match mode {
        "rule" => language.text("规则", "Rule"),
        "global" => language.text("全局", "Global"),
        "direct" => language.text("直连", "Direct"),
        _ => mode,
    }
}

pub(super) fn value(poll: &Poll, key: &str) -> Value {
    if let Some(index) = SHORTCUTS
        .iter()
        .position(|(setting, _, _, _)| *setting == key)
    {
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

pub(super) fn display(poll: &Poll, key: &str, language: Language) -> String {
    if key.starts_with("shortcut_")
        && !poll.meta.profiles.iter().any(|profile| {
            Some(&profile.id) == poll.meta.settings.active_profile.as_ref() && !profile.builtin
        })
    {
        return language
            .text("仅适用于非内置订阅", "Available for added subscriptions")
            .into();
    }
    let value = value(poll, key);
    match key {
        "active_profile" => poll
            .meta
            .profiles
            .iter()
            .find(|profile| Some(&profile.id) == poll.meta.settings.active_profile.as_ref())
            .map_or_else(
                || language.text("尚未选择", "None selected").into(),
                |profile| profile.name.clone(),
            ),
        "tui_node_sort" => match language {
            Language::SimplifiedChinese => poll.meta.settings.tui_node_sort.to_string(),
            Language::English => match poll.meta.settings.tui_node_sort {
                NodeSort::Name => "Node name".into(),
                NodeSort::LatencyAscending => "Lowest latency first".into(),
                NodeSort::LatencyDescending => "Highest latency first".into(),
            },
        },
        "language" => language_display(poll.meta.settings.language, language).into(),
        "fake_ip" if !poll.meta.settings.dns_override => language
            .text("关闭（DNS 覆盖未开启）", "Off (DNS override is off)")
            .into(),
        "fake_ip_exclusions" => {
            let custom = poll.meta.settings.fake_ip_exclusions.len();
            let active = poll.meta.settings.fake_ip_exclusion_entries().len();
            match language {
                Language::SimplifiedChinese => format!("{custom} 项自定义 · {active} 项有效排除"),
                Language::English => format!("{custom} custom · {active} active"),
            }
        }
        "fake_ip_disabled_defaults" => {
            let disabled = poll.meta.settings.fake_ip_disabled_defaults.len();
            let defaults = clash_client_core::config::DEFAULT_FAKE_IP_EXCLUSIONS.len();
            match language {
                Language::SimplifiedChinese => format!("{disabled} 项已停用 · {defaults} 项内置"),
                Language::English => format!("{disabled} disabled · {defaults} defaults"),
            }
        }
        "run_mode" => routing_mode_label(value.as_str().unwrap_or(""), language).into(),
        "proxy_mode" => {
            let actual = poll.snapshot.proxy_mode();
            let label = |mode| proxy_mode_label(mode, language);
            if actual == poll.meta.settings.proxy_mode {
                label(actual).into()
            } else {
                let current = label(actual);
                let preferred = label(poll.meta.settings.proxy_mode);
                match language {
                    Language::SimplifiedChinese => format!("{current}（启动偏好：{preferred}）"),
                    Language::English => format!("{current} (startup preference: {preferred})"),
                }
            }
        }
        _ => match value {
            Value::Bool(enabled) => enabled_label(enabled, language).into(),
            Value::String(value) => value,
            other => other.to_string(),
        },
    }
}

pub(super) fn hint(key: &str, language: Language) -> &'static str {
    match key {
        "proxy_mode" | "claude_protection" => language.text(
            "选择状态后确认；系统授权在普通终端完成，返回后读取实际状态。",
            "Choose a mode and confirm. Complete any system prompt in the terminal.",
        ),
        "active_profile" => language.text(
            "选择订阅名称；新增、编辑来源、更新和删除在操作页。",
            "Choose a subscription. Manage subscriptions under Commands.",
        ),
        "fake_ip_exclusions" => language.text(
            "Enter 管理已有排除项、新增项目或恢复默认。",
            "Press Enter to manage exclusions or restore defaults.",
        ),
        "fake_ip" => language.text(
            "先开启 DNS 覆盖；关闭 DNS 覆盖会同时关闭 Fake-IP。",
            "Fake-IP requires DNS override.",
        ),
        "tui_node_sort" => language.text(
            "只改变终端节点显示；节点页也可按 s 循环切换。",
            "Press s on the Nodes page to cycle through sort orders.",
        ),
        "language" => "",
        key if key.starts_with("shortcut_") => language.text(
            "作用于当前订阅；自定义规则、顺序和订阅原规则在规则页管理。",
            "Applies to the active subscription. Manage other rules under Rules.",
        ),
        _ => language.text(
            "←→ 或 Space 调整选项，Enter 确认；文字与数值按 Enter 编辑，↓ 选择保存。",
            "Use ← → or Space to change a choice, Enter to edit, and ↓ to select Save.",
        ),
    }
}

pub(super) fn apply_saved_language(
    state: &mut super::model::State,
    poll: &Poll,
    key: Option<&str>,
    args: &mut crate::args::Args,
) {
    if key != Some("language") {
        return;
    }
    apply_language(state, poll.meta.settings.language, args);
}

pub(super) fn apply_language(
    state: &mut super::model::State,
    preference: LanguagePreference,
    args: &mut crate::args::Args,
) {
    state.language_preference = preference;
    state.language = preference.resolve_desktop();
    args.options
        .insert("lang".into(), state.language.tag().into());
    if let Some(report) = &state.network_report {
        state.network_results = report.lines_in(state.language);
    }
}

pub(super) fn sync_language(
    state: &mut super::model::State,
    args: &mut crate::args::Args,
    follow_preference: bool,
) -> bool {
    if !follow_preference
        || state.form.is_some()
        || state.language == state.language_preference.resolve_desktop()
    {
        return false;
    }
    apply_language(state, state.language_preference, args);
    true
}

fn choices(form: &mut Form, index: usize, values: &[(&str, &str)]) {
    form.fields[index].choices = values
        .iter()
        .map(|(value, title)| ((*value).into(), (*title).into()))
        .collect();
}

pub(super) fn open(poll: &Poll, key: &str, language: Language) -> Result<Form> {
    ensure!(
        (KEYS.contains(&key) && key != "fake_ip_exclusions")
            || RULE_KEYS.contains(&key)
            || key == "active_profile",
        "{}",
        language.text(
            "该设置请使用操作页的专用入口",
            "Manage this setting under Commands."
        )
    );
    if key == "language" {
        return Ok(language_form(poll.meta.settings.language, language));
    }
    let value = value(poll, key);
    let mut form = Form::setting(key, &value, language);
    let off = enabled_label(false, language);
    let on = enabled_label(true, language);
    match key {
        "run_mode" | "proxy_mode" | "claude_protection" | "active_profile" => {
            let path = match key {
                "run_mode" => "mode",
                "proxy_mode" => "proxy",
                "claude_protection" => "claude protect",
                _ => "profile use",
            };
            form = Form::new_in(
                form::commands().find(|spec| spec.path == path).unwrap(),
                language,
            );
            form.fields.truncate(1);
            form.fields[0].label = language.text("值", "Value").into();
            form.setting = Some(key.into());
            match key {
                "run_mode" => {
                    form.fields[0].value = value
                        .as_str()
                        .context(language.text("运行模式无效", "Invalid routing mode"))?
                        .into();
                    choices(
                        &mut form,
                        0,
                        &["rule", "global", "direct"]
                            .map(|mode| (mode, routing_mode_label(mode, language))),
                    );
                }
                "proxy_mode" => {
                    form.fields[0].value = value
                        .as_str()
                        .context(language.text("代理方式无效", "Invalid proxy mode"))?
                        .into();
                    choices(
                        &mut form,
                        0,
                        &[
                            ("off", off),
                            ("system", proxy_mode_label(ProxyMode::System, language)),
                            ("tun", "TUN"),
                        ],
                    );
                }
                "claude_protection" => {
                    form.fields[0].value = if poll.meta.settings.claude_protection.is_some() {
                        "on"
                    } else {
                        "off"
                    }
                    .into();
                    choices(&mut form, 0, &[("off", off), ("on", on)]);
                }
                _ => {
                    ensure!(
                        !poll.meta.profiles.is_empty(),
                        "{}",
                        language.text(
                            "没有订阅，请先在操作页添加订阅",
                            "Add a subscription under Commands."
                        )
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
                ("name", language.text("节点名称", "Node name")),
                (
                    "latency_ascending",
                    language.text("延迟从低到高", "Lowest latency first"),
                ),
                (
                    "latency_descending",
                    language.text("延迟从高到低", "Highest latency first"),
                ),
            ],
        ),
        key if key.starts_with("shortcut_") => {
            let (_, name, _, _) = SHORTCUTS
                .iter()
                .find(|(setting, _, _, _)| *setting == key)
                .unwrap();
            let id = poll
                .meta
                .settings
                .active_profile
                .as_ref()
                .context(language.text("尚未选择订阅", "No subscription selected"))?;
            ensure!(
                poll.meta
                    .profiles
                    .iter()
                    .any(|profile| profile.id == *id && !profile.builtin),
                "{}",
                language.text(
                    "快捷规则只适用于非内置订阅",
                    "Rule shortcuts require an added subscription."
                )
            );
            form = Form::new_in(
                form::commands()
                    .find(|spec| spec.path == "rule shortcut")
                    .unwrap(),
                language,
            );
            form.setting = Some(key.into());
            form.fields[0].value = (*name).into();
            form.fields[0].locked = true;
            form.fields[1].label = language.text("值", "Value").into();
            form.fields[1].value = if value == true { "on" } else { "off" }.into();
            form.fields[2].value = id.clone();
            form.fields[2].locked = true;
            choices(&mut form, 1, &[("off", off), ("on", on)]);
            form.selected = 1;
        }
        _ if value.is_boolean() => choices(&mut form, 1, &[("false", off), ("true", on)]),
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

#[cfg(test)]
pub(super) fn command(args: &crate::args::Args) -> Result<Command> {
    command_in(args, Language::SimplifiedChinese)
}

pub(super) fn command_in(args: &crate::args::Args, language: Language) -> Result<Command> {
    let first = args
        .positional
        .first()
        .context(language.text("设置值缺失", "Setting value is missing"))?;
    Ok(match args.path.as_str() {
        "config set" => {
            ensure!(
                KEYS.contains(&first.as_str())
                    && clash_client_core::config::MUTABLE_SETTINGS.contains(&first.as_str()),
                "{}",
                language.text(
                    "该设置请使用专用入口",
                    "Use the dedicated action for this setting"
                )
            );
            let raw = args
                .positional
                .get(1)
                .context(language.text("设置值缺失", "Setting value is missing"))?;
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
                .context(language.text(
                    "订阅身份缺失，请重新打开设置",
                    "Subscription ID is missing; reopen settings",
                ))?
                .into(),
            index: SHORTCUTS
                .iter()
                .position(|(_, name, _, _)| *name == first.as_str())
                .context(language.text("未知快捷规则", "Unknown rule shortcut"))?,
            enabled: args
                .positional
                .get(1)
                .context(language.text("开关状态缺失", "Switch state is missing"))?
                == "on",
        },
        _ => anyhow::bail!(
            "{}",
            language.text(
                "该操作需要普通终端授权",
                "This action requires authorization in the terminal"
            )
        ),
    })
}
