use super::form::{self, Form};
use super::model::Page;
use crate::messages;
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
            messages::tui_settings_category_general(language)
        }
        key if NETWORK_KEYS.contains(&key) => messages::tui_settings_category_network(language),
        "update_prereleases" => messages::tui_settings_category_updates(language),
        "active_profile" => messages::tui_settings_category_subscriptions(language),
        key if key.starts_with("shortcut_") => {
            messages::tui_settings_category_rule_shortcuts(language)
        }
        _ => messages::tui_settings_category_scheduled_tasks(language),
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

struct Shortcut {
    setting: &'static str,
    command: &'static str,
    title: fn(Language) -> &'static str,
}

const SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        setting: "shortcut_ads",
        command: "ads",
        title: messages::tui_settings_shortcut_ads,
    },
    Shortcut {
        setting: "shortcut_private_domain",
        command: "private-domain",
        title: messages::tui_settings_shortcut_private_domain,
    },
    Shortcut {
        setting: "shortcut_private_ip",
        command: "private-ip",
        title: messages::tui_settings_shortcut_private_ip,
    },
    Shortcut {
        setting: "shortcut_cn_domain",
        command: "cn-domain",
        title: messages::tui_settings_shortcut_cn_domain,
    },
    Shortcut {
        setting: "shortcut_cn_ip",
        command: "cn-ip",
        title: messages::tui_settings_shortcut_cn_ip,
    },
];

pub(super) fn title(key: &str, language: Language) -> &str {
    SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.setting == key)
        .map_or_else(
            || form::setting_title(key, language),
            |shortcut| (shortcut.title)(language),
        )
}

pub(super) fn language_display(preference: LanguagePreference, language: Language) -> &'static str {
    preference.explicit().map_or_else(
        || clash_client_core::messages::settings_language_follow_system(language),
        Language::native_name,
    )
}

pub(super) fn language_form(preference: LanguagePreference, language: Language) -> Form {
    let mut form = Form::setting("language", &json!(preference.tag()), language);
    form.fields[1].choices = LanguagePreference::TAGS
        .iter()
        .filter_map(|tag| LanguagePreference::from_tag(tag))
        .map(|choice| {
            (
                choice.tag().into(),
                language_display(choice, language).into(),
            )
        })
        .collect();
    form
}

fn enabled_label(enabled: bool, language: Language) -> &'static str {
    if enabled {
        messages::tui_settings_enabled(language)
    } else {
        messages::tui_settings_disabled(language)
    }
}

fn proxy_mode_label(mode: ProxyMode, language: Language) -> &'static str {
    match mode {
        ProxyMode::Off => enabled_label(false, language),
        ProxyMode::System => messages::tui_settings_system_proxy(language),
        ProxyMode::Tun => "TUN",
    }
}

fn routing_mode_label(mode: &str, language: Language) -> &str {
    match mode {
        "rule" => messages::tui_settings_mode_rule(language),
        "global" => messages::tui_settings_mode_global(language),
        "direct" => messages::tui_settings_mode_direct(language),
        _ => mode,
    }
}

pub(super) fn value(poll: &Poll, key: &str, language: Language) -> Value {
    if let Some(index) = SHORTCUTS
        .iter()
        .position(|shortcut| shortcut.setting == key)
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
            _ => form::setting_value(&poll.meta.settings, key, language),
        }
    }
}

pub(super) fn display(poll: &Poll, key: &str, language: Language) -> String {
    if key.starts_with("shortcut_")
        && !poll.meta.profiles.iter().any(|profile| {
            Some(&profile.id) == poll.meta.settings.active_profile.as_ref() && !profile.builtin
        })
    {
        return messages::tui_settings_shortcut_unavailable(language).into();
    }
    let value = value(poll, key, language);
    match key {
        "active_profile" => poll
            .meta
            .profiles
            .iter()
            .find(|profile| Some(&profile.id) == poll.meta.settings.active_profile.as_ref())
            .map_or_else(
                || messages::tui_settings_profile_none(language).into(),
                |profile| profile.display_name(language).into(),
            ),
        "tui_node_sort" => node_sort_label(poll.meta.settings.tui_node_sort, language).into(),
        "language" => language_display(poll.meta.settings.language, language).into(),
        "fake_ip" if !poll.meta.settings.dns_override => {
            messages::tui_settings_fake_ip_disabled(language).into()
        }
        "fake_ip_exclusions" => {
            let custom = poll.meta.settings.fake_ip_exclusions.len();
            let active = poll.meta.settings.fake_ip_exclusion_entries().len();
            messages::tui_fake_ip_custom_count(language, active, custom)
        }
        "fake_ip_disabled_defaults" => {
            let disabled = poll.meta.settings.fake_ip_disabled_defaults.len();
            let defaults = clash_client_core::config::DEFAULT_FAKE_IP_EXCLUSIONS.len();
            messages::tui_fake_ip_disabled_count(language, defaults, disabled)
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
                messages::tui_proxy_startup_preference(language, current, preferred)
            }
        }
        _ => match value {
            Value::Bool(enabled) => enabled_label(enabled, language).into(),
            Value::String(value) => value,
            other => other.to_string(),
        },
    }
}

fn node_sort_label(sort: NodeSort, language: Language) -> &'static str {
    match sort {
        NodeSort::Name => messages::tui_node_sort_name(language),
        NodeSort::LatencyAscending => messages::tui_node_sort_latency_ascending(language),
        NodeSort::LatencyDescending => messages::tui_node_sort_latency_descending(language),
    }
}

pub(super) fn hint(key: &str, language: Language) -> &'static str {
    match key {
        "proxy_mode" | "claude_protection" => messages::tui_settings_hint_authorize(language),
        "active_profile" => messages::tui_settings_hint_profile(language),
        "fake_ip_exclusions" => messages::tui_settings_hint_exclusions(language),
        "fake_ip" => messages::tui_settings_hint_fake_ip(language),
        "tui_node_sort" => messages::tui_settings_hint_node_sort(language),
        "language" => "",
        key if key.starts_with("shortcut_") => messages::tui_settings_hint_shortcut(language),
        _ => messages::tui_settings_hint_edit(language),
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
    state.set_language(preference.resolve_desktop());
    set_session_language(args, state.language);
}

pub(super) fn set_session_language(args: &mut crate::args::Args, language: Language) {
    args.options.insert("lang".into(), language.tag().into());
}

pub(super) fn sync_language(
    state: &mut super::model::State,
    args: &mut crate::args::Args,
    follow_preference: bool,
    width: usize,
) -> bool {
    if !follow_preference
        || state.form.is_some()
        || state.language == state.language_preference.resolve_desktop()
    {
        return false;
    }
    apply_language(state, state.language_preference, args);
    if state.page == Page::Network {
        super::network::relocalize_result(state, width);
    }
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
        messages::tui_settings_use_command(language)
    );
    if key == "language" {
        return Ok(language_form(poll.meta.settings.language, language));
    }
    let value = value(poll, key, language);
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
            form.fields[0].label = messages::tui_settings_value(language).into();
            form.setting = Some(key.into());
            match key {
                "run_mode" => {
                    form.fields[0].value = value
                        .as_str()
                        .context(messages::tui_settings_invalid_routing_mode(language))?
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
                        .context(messages::tui_settings_invalid_proxy_mode(language))?
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
                        messages::tui_settings_add_profile_first(language)
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
                        .map(|profile| (profile.id.clone(), profile.display_name(language).into()))
                        .collect();
                }
            }
        }
        "tui_node_sort" => choices(
            &mut form,
            1,
            &[
                ("name", messages::tui_settings_sort_node_name(language)),
                (
                    "latency_ascending",
                    node_sort_label(NodeSort::LatencyAscending, language),
                ),
                (
                    "latency_descending",
                    node_sort_label(NodeSort::LatencyDescending, language),
                ),
            ],
        ),
        key if key.starts_with("shortcut_") => {
            let shortcut = SHORTCUTS
                .iter()
                .find(|shortcut| shortcut.setting == key)
                .unwrap();
            let id = poll
                .meta
                .settings
                .active_profile
                .as_ref()
                .context(messages::tui_settings_profile_required(language))?;
            ensure!(
                poll.meta
                    .profiles
                    .iter()
                    .any(|profile| profile.id == *id && !profile.builtin),
                "{}",
                messages::tui_settings_shortcut_requires_profile(language)
            );
            form = Form::new_in(
                form::commands()
                    .find(|spec| spec.path == "rule shortcut")
                    .unwrap(),
                language,
            );
            form.setting = Some(key.into());
            form.fields[0].value = shortcut.command.into();
            form.fields[0].locked = true;
            form.fields[1].label = messages::tui_settings_value(language).into();
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
        .context(messages::tui_settings_missing_value(language))?;
    Ok(match args.path.as_str() {
        "config set" => {
            ensure!(
                KEYS.contains(&first.as_str())
                    && clash_client_core::config::MUTABLE_SETTINGS.contains(&first.as_str()),
                "{}",
                messages::tui_settings_use_dedicated_action(language)
            );
            let raw = args
                .positional
                .get(1)
                .context(messages::tui_settings_missing_value(language))?;
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
                .context(messages::tui_settings_missing_profile_id(language))?
                .into(),
            index: SHORTCUTS
                .iter()
                .position(|shortcut| shortcut.command == first.as_str())
                .context(messages::tui_settings_unknown_shortcut(language))?,
            enabled: args
                .positional
                .get(1)
                .context(messages::tui_settings_missing_switch_state(language))?
                == "on",
        },
        _ => anyhow::bail!(
            "{}",
            messages::tui_settings_requires_authorization(language)
        ),
    })
}
