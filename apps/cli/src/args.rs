//! One command catalogue drives parsing, help and shell completions.
use crate::messages;
use anyhow::{Result, bail, ensure};
use clash_client_core::config::{Language, LanguagePreference, Store};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub struct Spec {
    pub path: &'static str,
    description: fn(Language) -> &'static str,
    pub usage: &'static str,
    pub min: usize,
    pub max: usize,
    pub options: &'static str,
}
impl Spec {
    pub fn localized_description(&self, language: Language) -> &'static str {
        (self.description)(language)
    }

    pub fn retired(&self) -> bool {
        matches!(
            self.path,
            "network explain"
                | "network rule"
                | "network config-diff"
                | "network traffic"
                | "network performance"
                | "network node"
        )
    }

    pub fn required_options(&self) -> &'static [(&'static str, &'static str)] {
        match self.path {
            "profile import" => &[("name", "NAME")],
            "logs export" | "update download" => &[("output", "FILE")],
            _ => &[],
        }
    }
}
macro_rules! command {
    ($path:literal, $description:ident, $usage:literal, $min:literal, $max:literal, $options:literal) => {
        Spec {
            path: $path,
            description: messages::$description,
            usage: $usage,
            min: $min,
            max: $max,
            options: $options,
        }
    };
}
pub static COMMANDS: &[Spec] = &[
    command!("ui", help_describe_ui, "ui [--demo]", 0, 0, "demo"),
    command!("run", help_describe_run, "run [--no-core]", 0, 0, "no-core"),
    command!("start", help_describe_start, "start", 0, 0, ""),
    command!(
        "stop",
        help_describe_stop,
        "stop [--all-users]",
        0,
        0,
        "all-users"
    ),
    command!(
        "restart",
        help_describe_restart,
        "restart [--network-only] [--yes]",
        0,
        0,
        "network-only"
    ),
    command!(
        "status",
        help_describe_status,
        "status [--watch] [--all-users]",
        0,
        0,
        "watch interval all-users"
    ),
    command!("version", help_describe_version, "version", 0, 0, ""),
    command!(
        "agent guide",
        help_describe_agent_guide,
        "agent guide",
        0,
        0,
        ""
    ),
    command!(
        "profile list",
        help_describe_profile_list,
        "profile list [--filter TEXT --all-users --output FILE]",
        0,
        0,
        "filter all-users output"
    ),
    command!(
        "profile add",
        help_describe_profile_add,
        "profile add NAME SOURCE",
        2,
        2,
        ""
    ),
    command!(
        "profile import",
        help_describe_profile_import,
        "profile import FILE --name NAME",
        1,
        1,
        "name"
    ),
    command!(
        "profile update",
        help_describe_profile_update,
        "profile update [PROFILE] [--all]",
        0,
        1,
        "all"
    ),
    command!(
        "profile use",
        help_describe_profile_use,
        "profile use PROFILE",
        1,
        1,
        ""
    ),
    command!(
        "profile edit",
        help_describe_profile_edit,
        "profile edit PROFILE [--name NAME] [--source SOURCE]",
        1,
        1,
        "name source"
    ),
    command!(
        "profile remove",
        help_describe_profile_remove,
        "profile remove PROFILE [--yes]",
        1,
        1,
        ""
    ),
    command!(
        "profile export",
        help_describe_profile_export,
        "profile export PROFILE [--output FILE]",
        1,
        1,
        "output"
    ),
    command!(
        "profile directory",
        help_describe_profile_directory,
        "profile directory",
        0,
        0,
        ""
    ),
    command!(
        "node list",
        help_describe_node_list,
        "node list [GROUP] [--sort name|latency] [--reverse]",
        0,
        1,
        "filter sort reverse limit offset"
    ),
    command!(
        "node select",
        help_describe_node_select,
        "node select [GROUP [NODE]]",
        0,
        2,
        ""
    ),
    command!(
        "node test",
        help_describe_node_test,
        "node test [NODE] [--group GROUP | --all]",
        0,
        1,
        "group all"
    ),
    command!(
        "mode",
        help_describe_mode,
        "mode [rule|global|direct]",
        0,
        1,
        ""
    ),
    command!(
        "proxy",
        help_describe_proxy,
        "proxy [system|tun|off] [--all-users]",
        0,
        1,
        "all-users"
    ),
    command!(
        "env",
        help_describe_env,
        "env [--shell bash|zsh|fish|powershell|cmd] [--unset]",
        0,
        0,
        "shell unset"
    ),
    command!(
        "rule list",
        help_describe_rule_list,
        "rule list [--profile PROFILE] [--filter TEXT]",
        0,
        0,
        "profile filter limit offset"
    ),
    command!(
        "rule add",
        help_describe_rule_add,
        "rule add 'TYPE,VALUE,POLICY' [--position N]",
        1,
        1,
        "profile position disabled"
    ),
    command!(
        "rule edit",
        help_describe_rule_edit,
        "rule edit RULE 'TYPE,VALUE,POLICY'",
        2,
        2,
        "profile"
    ),
    command!(
        "rule remove",
        help_describe_rule_remove,
        "rule remove RULE [--yes]",
        1,
        1,
        "profile"
    ),
    command!(
        "rule move",
        help_describe_rule_move,
        "rule move RULE POSITION",
        2,
        2,
        "profile"
    ),
    command!(
        "rule enable",
        help_describe_rule_enable,
        "rule enable RULE on|off",
        2,
        2,
        "profile"
    ),
    command!(
        "rule shortcut",
        help_describe_rule_shortcut,
        "rule shortcut [ads|private-domain|private-ip|cn-domain|cn-ip] [on|off]",
        0,
        2,
        "profile"
    ),
    command!(
        "connection list",
        help_describe_connection_list,
        "connection list [--watch] [--sort download-rate]",
        0,
        0,
        "watch interval filter sort reverse limit offset"
    ),
    command!(
        "connection show",
        help_describe_connection_show,
        "connection show ID",
        1,
        1,
        ""
    ),
    command!(
        "connection close",
        help_describe_connection_close,
        "connection close [ID] [--all --yes]",
        0,
        1,
        "all"
    ),
    command!(
        "logs",
        help_describe_logs,
        "logs [--follow] [--level warning] [--filter TEXT]",
        0,
        0,
        "follow interval level filter limit"
    ),
    command!(
        "logs clear",
        help_describe_logs_clear,
        "logs clear [--yes]",
        0,
        0,
        ""
    ),
    command!(
        "logs export",
        help_describe_logs_export,
        "logs export --output FILE",
        0,
        0,
        "output"
    ),
    command!(
        "config get",
        help_describe_config_get,
        "config get [KEY]",
        0,
        1,
        ""
    ),
    command!(
        "config set",
        help_describe_config_set,
        "config set KEY VALUE",
        2,
        2,
        ""
    ),
    command!(
        "config check",
        help_describe_config_check,
        "config check",
        0,
        0,
        ""
    ),
    command!(
        "config effective",
        help_describe_config_effective,
        "config effective [--output FILE]",
        0,
        0,
        "output"
    ),
    command!(
        "diagnose",
        help_describe_diagnose,
        "diagnose [--url URL] [--host DOMAIN] [--offline] [--output FILE] [--all-users]",
        0,
        0,
        "url host offline output all-users"
    ),
    command!(
        "dns",
        help_describe_dns,
        "dns DOMAIN [--system]",
        1,
        1,
        "system"
    ),
    command!(
        "network check",
        help_describe_network_check,
        "network check URL",
        1,
        1,
        ""
    ),
    command!(
        "network dns",
        help_describe_network_dns,
        "network dns HOST [--type ADDR|A|AAAA|CNAME|MX|TXT|NS|SOA|PTR|CAA]",
        1,
        1,
        "type output"
    ),
    command!(
        "network phase",
        help_describe_network_phase,
        "network phase URL [--seconds N]",
        1,
        1,
        "seconds output"
    ),
    command!(
        "network family",
        help_describe_network_family,
        "network family URL [--seconds N]",
        1,
        1,
        "seconds output"
    ),
    command!(
        "network explain",
        help_describe_network_explain,
        "network explain URL",
        1,
        1,
        "output"
    ),
    command!(
        "network tls",
        help_describe_network_tls,
        "network tls URL [--seconds N]",
        1,
        1,
        "seconds output"
    ),
    command!(
        "network stability",
        help_describe_network_stability,
        "network stability URL [--count N] [--interval-ms N] [--seconds N]",
        1,
        1,
        "count interval-ms seconds output"
    ),
    command!(
        "network udp",
        help_describe_network_udp,
        "network udp DNS_SERVER [--seconds N]",
        1,
        1,
        "seconds output"
    ),
    command!(
        "network ping",
        help_describe_network_ping,
        "network ping URL [--count N]",
        1,
        1,
        "count output"
    ),
    command!(
        "network trace",
        help_describe_network_trace,
        "network trace URL [--max-hops N]",
        1,
        1,
        "max-hops output"
    ),
    command!(
        "network download",
        help_describe_network_download,
        "network download URL [--max-bytes N] [--seconds N]",
        1,
        1,
        "max-bytes seconds output"
    ),
    command!(
        "network tcp",
        help_describe_network_tcp,
        "network tcp HOST[:PORT] [--seconds N]",
        1,
        1,
        "seconds output"
    ),
    command!(
        "network http",
        help_describe_network_http,
        "network http URL [--method HEAD|GET|OPTIONS] [--seconds N]",
        1,
        1,
        "method seconds output"
    ),
    command!(
        "network rule",
        help_describe_network_rule,
        "network rule URL",
        1,
        1,
        "output"
    ),
    command!(
        "network config-diff",
        help_describe_network_config_diff,
        "network config-diff",
        0,
        0,
        "output"
    ),
    command!(
        "network traffic",
        help_describe_network_traffic,
        "network traffic",
        0,
        0,
        "output"
    ),
    command!(
        "network performance",
        help_describe_network_performance,
        "network performance",
        0,
        0,
        "output"
    ),
    command!(
        "network node",
        help_describe_network_node,
        "network node NODE",
        1,
        1,
        "output"
    ),
    command!(
        "ip-check list",
        help_describe_ip_check_list,
        "ip-check list [--category CATEGORY]",
        0,
        0,
        "category filter"
    ),
    command!(
        "ip-check",
        help_describe_ip_check,
        "ip-check [ID] [--category CATEGORY] [--output FILE]",
        0,
        1,
        "category output"
    ),
    command!(
        "claude check",
        help_describe_claude_check,
        "claude check [--details] [--output FILE]",
        0,
        0,
        "details output"
    ),
    command!(
        "claude audit",
        help_describe_claude_audit,
        "claude audit [--offline] [--output FILE]",
        0,
        0,
        "offline output"
    ),
    command!(
        "claude browser",
        help_describe_claude_browser,
        "claude browser [--open] [--details] [--output FILE]",
        0,
        0,
        "open details output"
    ),
    command!(
        "claude protect",
        help_describe_claude_protect,
        "claude protect [on|off] [--yes]",
        0,
        1,
        ""
    ),
    command!(
        "claude watch",
        help_describe_claude_watch,
        "claude watch [--interval SECONDS]",
        0,
        0,
        "interval"
    ),
    command!(
        "geo status",
        help_describe_geo_status,
        "geo status",
        0,
        0,
        ""
    ),
    command!(
        "geo update",
        help_describe_geo_update,
        "geo update",
        0,
        0,
        ""
    ),
    command!(
        "update check",
        help_describe_update_check,
        "update check",
        0,
        0,
        ""
    ),
    command!(
        "update download",
        help_describe_update_download,
        "update download --output FILE",
        0,
        0,
        "output"
    ),
    command!(
        "update status",
        help_describe_update_status,
        "update status",
        0,
        0,
        ""
    ),
    command!(
        "update install",
        help_describe_update_install,
        "update install [--yes] [--no-start]",
        0,
        0,
        "no-start"
    ),
    command!(
        "autostart",
        help_describe_autostart,
        "autostart [on|off]",
        0,
        1,
        ""
    ),
    command!(
        "service install",
        help_describe_service_install,
        "service install [--yes]",
        0,
        0,
        ""
    ),
    command!(
        "service remove",
        help_describe_service_remove,
        "service remove [--yes]",
        0,
        0,
        ""
    ),
    command!(
        "service status",
        help_describe_service_status,
        "service status",
        0,
        0,
        ""
    ),
    command!("notify", help_describe_notify, "notify [MESSAGE]", 0, 1, ""),
    command!(
        "completion",
        help_describe_completion,
        "completion SHELL",
        1,
        1,
        ""
    ),
];

const GLOBAL: &str =
    "help version json jsonl timeout data-dir resources user yes show-secrets quiet no-color lang";
pub(crate) const VALUES: &str = "timeout data-dir resources user lang filter name source output sort limit offset group profile position interval level shell url host category count interval-ms seconds max-bytes max-hops type method";

#[derive(Debug)]
pub struct Usage(pub String);
impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Usage {}

#[derive(Clone)]
pub struct Args {
    pub path: String,
    pub positional: Vec<String>,
    pub options: BTreeMap<String, String>,
    pub help: bool,
}
impl Args {
    pub fn flag(&self, key: &str) -> bool {
        self.options.contains_key(key)
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.options.get(key).map(String::as_str)
    }
    pub fn language(&self) -> Language {
        self.get("lang")
            .and_then(LanguagePreference::from_tag)
            .or_else(|| {
                let root = match self.get("data-dir") {
                    Some(path) => PathBuf::from(path),
                    None => Store::default_root().ok()?,
                };
                LanguagePreference::read(&root).ok()
            })
            .unwrap_or_default()
            .resolve_desktop()
    }
    pub fn number(&self, key: &str, default: usize, max: usize) -> Result<usize> {
        let value = self.get(key).map_or(Ok(default), str::parse::<usize>)?;
        ensure!(
            value <= max,
            messages::option_argument_exceeds_limit(self.language(), max, key)
        );
        Ok(value)
    }
    pub fn interval(&self) -> Result<std::time::Duration> {
        let seconds = self.get("interval").unwrap_or("1").parse::<f64>()?;
        ensure!(
            seconds.is_finite() && (0.25..=3600.).contains(&seconds),
            messages::option_interval_out_of_range(self.language())
        );
        Ok(std::time::Duration::from_secs_f64(seconds))
    }
}

pub(crate) fn language_from_input(input: &[String]) -> Language {
    let mut options = BTreeMap::new();
    let mut words = input.iter();
    while let Some(word) = words.next() {
        if word == "--" {
            break;
        }
        let option = match word.as_str() {
            "-o" => "output",
            value => match value.strip_prefix("--") {
                Some(value) => value,
                None => continue,
            },
        };
        let (key, supplied) = option
            .split_once('=')
            .map_or((option, None), |(key, value)| (key, Some(value)));
        if VALUES.split_whitespace().any(|value| value == key) {
            let value = supplied.or_else(|| words.next().map(String::as_str));
            if matches!(key, "lang" | "data-dir")
                && let Some(value) = value
            {
                options.insert(key.to_owned(), value.to_owned());
            }
        }
    }
    Args {
        path: String::new(),
        positional: Vec::new(),
        options,
        help: false,
    }
    .language()
}

pub fn parse(input: impl IntoIterator<Item = String>) -> Result<Args> {
    let input: Vec<String> = input.into_iter().collect();
    let language = language_from_input(&input);
    parse_inner(input, language)
        .and_then(|args| {
            if !args.help {
                validate(&args)?;
            }
            Ok(args)
        })
        .map_err(|error| Usage(error.to_string()).into())
}
fn validate(args: &Args) -> Result<()> {
    let language = args.language();
    if args.path == "ui" {
        ensure!(
            !["json", "jsonl", "quiet", "output"]
                .iter()
                .any(|key| args.flag(key)),
            messages::command_ui_requires_terminal(language)
        );
    }
    ensure!(
        !args.flag("all-users") || (args.get("user").is_none() && args.get("data-dir").is_none()),
        messages::option_all_users_excludes_user_and_data_dir(language)
    );
    ensure!(
        !(args.flag("json") && args.flag("jsonl")),
        messages::option_json_and_jsonl_are_exclusive(language)
    );
    for (key, max) in [
        ("limit", 10_000),
        ("offset", usize::MAX),
        ("position", usize::MAX),
        ("timeout", 3600),
    ] {
        if args.get(key).is_some() {
            let value = args.number(key, 0, max)?;
            ensure!(
                !matches!(key, "timeout" | "position") || value > 0,
                messages::option_argument_positive(language, key)
            );
        }
    }
    if args.get("interval").is_some() {
        args.interval()?;
    }
    if let Some(spec) = COMMANDS.iter().find(|spec| spec.path == args.path) {
        for (key, placeholder) in spec.required_options() {
            ensure!(
                args.get(key).is_some(),
                messages::option_required(language, key, placeholder)
            );
        }
    }
    let enum_value = |value: &str, choices: &[&str]| -> Result<()> {
        ensure!(
            choices.contains(&value),
            messages::argument_choices(language, choices.join(" / "), &args.path)
        );
        Ok(())
    };
    match args.path.as_str() {
        "mode" => {
            if let Some(value) = args.positional.first() {
                enum_value(value, &["rule", "global", "direct"])?;
            }
        }
        "proxy" => {
            if let Some(value) = args.positional.first() {
                enum_value(value, &["system", "tun", "off"])?;
            }
            ensure!(
                !args.flag("all-users") || args.positional.is_empty(),
                messages::option_all_users_proxy_is_read_only(language)
            );
        }
        "claude protect" | "autostart" => {
            if let Some(value) = args.positional.first() {
                enum_value(value, &["on", "off"])?;
            }
        }
        "rule enable" => enum_value(&args.positional[1], &["on", "off"])?,
        "rule shortcut" => {
            if let Some(name) = args.positional.first() {
                enum_value(
                    name,
                    &["ads", "private-domain", "private-ip", "cn-domain", "cn-ip"],
                )?;
            }
            if let Some(value) = args.positional.get(1) {
                enum_value(value, &["on", "off"])?;
            }
        }
        "profile edit" => ensure!(
            args.get("name").is_some() || args.get("source").is_some(),
            messages::command_profile_edit_requires_changes(language)
        ),
        "profile update" => ensure!(
            !(args.flag("all") && !args.positional.is_empty()),
            messages::command_profile_update_requires_one_selection(language)
        ),
        "node test" => ensure!(
            usize::from(!args.positional.is_empty())
                + usize::from(args.get("group").is_some())
                + usize::from(args.flag("all"))
                == 1,
            messages::command_node_test_requires_one_selection(language)
        ),
        "connection close" => ensure!(
            args.flag("all") != !args.positional.is_empty(),
            messages::command_connection_close_requires_one_selection(language)
        ),
        "rule add" => {
            clash_client_core::rule_manager::parts(&args.positional[0])?;
        }
        "rule edit" => {
            clash_client_core::rule_manager::parts(&args.positional[1])?;
        }
        "rule move" => {
            ensure!(
                args.positional[1].parse::<usize>()? > 0,
                messages::command_rule_move_requires_positive_position(language)
            );
        }
        "completion" => enum_value(&args.positional[0], &["bash", "zsh", "fish", "powershell"])?,
        _ => {}
    }
    if let Some(sort) = args.get("sort") {
        enum_value(
            sort,
            if args.path == "node list" {
                &["name", "latency"]
            } else {
                &[
                    "name",
                    "rule",
                    "chain",
                    "upload",
                    "download",
                    "upload-rate",
                    "download-rate",
                    "start",
                    "none",
                ]
            },
        )?;
    }
    if let Some(level) = args.get("level") {
        enum_value(level, &["debug", "info", "warning", "error", "silent"])?;
    }
    if let Some(shell) = args.get("shell") {
        enum_value(shell, &["bash", "zsh", "fish", "powershell", "cmd"])?;
    }
    Ok(())
}
fn parse_inner(input: impl IntoIterator<Item = String>, language: Language) -> Result<Args> {
    let mut words = Vec::new();
    let mut options = BTreeMap::new();
    let mut literal = false;
    let mut input = input.into_iter();
    while let Some(value) = input.next() {
        if value == "--" && !literal {
            literal = true;
            continue;
        }
        if literal || !value.starts_with('-') || value == "-" {
            words.push(value);
            continue;
        }
        let value = match value.as_str() {
            "-h" => "--help",
            "-V" => "--version",
            "-y" => "--yes",
            "-q" => "--quiet",
            "-o" => "--output",
            _ => &value,
        };
        let value = value.strip_prefix("--").ok_or_else(|| {
            anyhow::anyhow!(messages::option_combined_short_unsupported(language, value))
        })?;
        let (key, supplied) = value
            .split_once('=')
            .map_or((value, None), |(k, v)| (k, Some(v.to_owned())));
        let needs_value = VALUES.split_whitespace().any(|v| v == key);
        let parameter = if needs_value {
            supplied
                .or_else(|| input.next())
                .ok_or_else(|| anyhow::anyhow!(messages::option_argument_missing(language, key)))?
        } else {
            ensure!(
                supplied.is_none(),
                messages::option_argument_unexpected(language, key)
            );
            "true".into()
        };
        ensure!(
            key != "lang" || LanguagePreference::from_tag(&parameter).is_some(),
            messages::option_language_choices(language, LanguagePreference::TAGS.join(" / "))
        );
        ensure!(
            options.insert(key.to_owned(), parameter).is_none(),
            messages::option_duplicate(language, key)
        );
    }
    let mut help = options.contains_key("help");
    if words.first().is_some_and(|s| s == "help") {
        words.remove(0);
        help = true;
    }
    if words.last().is_some_and(|word| word == "help") {
        let prefix = words[..words.len() - 1].join(" ");
        if !prefix.is_empty()
            && !COMMANDS.iter().any(|spec| spec.path == prefix)
            && COMMANDS
                .iter()
                .any(|spec| spec.path.starts_with(&(prefix.clone() + " ")))
        {
            words.pop();
            help = true;
        }
    }
    if options.contains_key("version") && words.is_empty() {
        words.push("version".into());
    }
    let selected = COMMANDS
        .iter()
        .filter(|spec| {
            let parts: Vec<_> = spec.path.split_whitespace().collect();
            words.len() >= parts.len() && words.iter().zip(parts).all(|(a, b)| a == b)
        })
        .max_by_key(|spec| spec.path.len());
    let Some(spec) = selected else {
        let prefix = words.join(" ");
        if prefix.is_empty()
            || (help && prefix == "all")
            || COMMANDS
                .iter()
                .any(|spec| spec.path.starts_with(&(prefix.clone() + " ")))
        {
            ensure!(
                options
                    .keys()
                    .all(|key| GLOBAL.split_whitespace().any(|g| g == key)),
                messages::help_group_excludes_subcommand_options(language)
            );
            return Ok(Args {
                path: prefix,
                positional: vec![],
                options,
                help: true,
            });
        }
        bail!(messages::command_unknown(language, words.join(" ")));
    };
    let positional = words[spec.path.split_whitespace().count()..].to_vec();
    for key in options.keys() {
        ensure!(
            GLOBAL
                .split_whitespace()
                .chain(spec.options.split_whitespace())
                .any(|allowed| allowed == key),
            messages::option_unsupported_by_command(language, spec.path, key)
        );
    }
    if !help {
        ensure!(
            (spec.min..=spec.max).contains(&positional.len()),
            "{}cor {}",
            messages::help_usage_prefix(language),
            spec.usage
        );
    }
    Ok(Args {
        path: spec.path.into(),
        positional,
        options,
        help,
    })
}

pub fn localized_help(prefix: &str, language: Language) -> String {
    let usage = messages::help_usage_prefix(language);
    let command = messages::help_command_token(language);
    let options = messages::help_options_token(language);
    let mut out = format!("Clash of Rust CLI {}\n\n", clash_client_core::VERSION);
    if prefix.is_empty() {
        let common = messages::help_common_commands(language);
        out += &format!("{usage}cor <{command}> [{options}]\n\n{common}\n");
        for path in [
            "ui",
            "start",
            "stop",
            "restart",
            "status",
            "proxy",
            "diagnose",
            "agent guide",
        ] {
            let spec = COMMANDS.iter().find(|spec| spec.path == path).unwrap();
            out += &format!(
                "  {:<25} {}\n",
                spec.path,
                spec.localized_description(language)
            );
        }
        out += messages::help_common_command_groups(language);
        let advanced = COMMANDS
            .iter()
            .map(|spec| spec.path.split_whitespace().next().unwrap())
            .filter(|path| {
                ![
                    "ui", "start", "stop", "restart", "status", "proxy", "diagnose", "profile",
                    "node", "agent",
                ]
                .contains(path)
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(messages::help_command_separator(language));
        let other = messages::help_other_commands(language);
        let full = messages::help_all_commands_and_options(language);
        out += &format!("\n{other}{advanced}\n{full}cor help all\n");
    } else if prefix == "all" {
        let all = messages::help_all_commands(language);
        out += &format!("{usage}cor <{command}> [{options}]\n\n{all}\n");
    } else if let Some(spec) = COMMANDS.iter().find(|s| s.path == prefix) {
        out += &format!(
            "{}\n\n{usage}cor {}\n\n",
            spec.localized_description(language),
            spec.usage
        );
        if !spec.options.is_empty() {
            out += messages::help_command_options_heading(language);
            for key in spec.options.split_whitespace() {
                out += &option_help(key, language);
            }
            out += "\n";
        }
    } else {
        let heading = messages::help_commands_heading(language);
        out += &format!("{usage}cor {prefix} <{command}> [{options}]\n\n{heading}\n");
    }
    for spec in COMMANDS.iter().filter(|spec| {
        prefix == "all"
            || (!prefix.is_empty()
                && spec.path.starts_with(&(prefix.to_owned() + " "))
                && spec.path != "profile import"
                && !spec.retired())
    }) {
        out += &format!(
            "  {:<25} {}\n",
            spec.path,
            spec.localized_description(language)
        );
    }
    let example = match prefix {
        "profile add" => Some(messages::help_example_profile_add(language)),
        "profile import" => Some(messages::help_example_profile_import(language)),
        "node list" => Some("cor node list Proxy --sort latency --limit 50"),
        "node select" => Some(messages::help_example_node_select(language)),
        "rule add" => Some("cor rule add 'DOMAIN-SUFFIX,example.com,DIRECT' --position 1"),
        "rule shortcut" => Some("cor rule shortcut ads on"),
        "connection list" => Some("cor connection list --watch --sort download-rate --jsonl"),
        "logs" => Some("cor logs --follow --level warning --filter DNS --jsonl"),
        "config set" => Some("cor config set dns_override true"),
        "ip-check" => Some("cor ip-check --category AI --output ai-report.json"),
        "claude audit" => Some("cor claude audit --offline --output claude-audit.json"),
        "env" => Some("eval \"$(cor env --shell bash)\""),
        "" => Some("cor start\n  cor proxy system\n  cor status --watch --jsonl"),
        _ => None,
    };
    if let Some(example) = example {
        out += messages::help_examples_heading(language);
        out += &format!("  {example}\n");
    }
    out += messages::help_global_options_heading(language);
    for key in (if prefix.is_empty() {
        "help version json data-dir user lang"
    } else {
        GLOBAL
    })
    .split_whitespace()
    {
        out += &option_help(key, language);
    }
    out += messages::help_footer(language);
    out
}
fn option_help(key: &str, language: Language) -> String {
    if key == "lang" {
        return format!(
            "  --{key:<20} {}{} [{}]\n",
            messages::help_option_value_token(language),
            messages::option_language_description(language),
            LanguagePreference::TAGS.join(" / ")
        );
    }
    let description = match key {
        "help" => messages::help_option_help(language),
        "version" => messages::help_option_version(language),
        "json" => messages::help_option_json(language),
        "jsonl" => messages::help_option_jsonl(language),
        "timeout" => messages::help_option_timeout(language),
        "data-dir" => messages::help_option_data_dir(language),
        "resources" => messages::help_option_resources(language),
        "user" => messages::help_option_user(language),
        "all-users" => messages::help_option_all_users(language),
        "yes" => messages::help_option_yes(language),
        "show-secrets" => messages::help_option_show_secrets(language),
        "quiet" => messages::help_option_quiet(language),
        "no-color" => messages::help_option_no_color(language),
        "demo" => messages::help_option_demo(language),
        "no-core" => messages::help_option_no_core(language),
        "no-start" => messages::help_option_no_start(language),
        "network-only" => messages::help_option_network_only(language),
        "watch" | "follow" => messages::help_option_watch(language),
        "interval" => messages::help_option_interval(language),
        "count" => messages::help_option_count(language),
        "interval-ms" => messages::help_option_interval_ms(language),
        "seconds" => messages::help_option_seconds(language),
        "max-bytes" => messages::help_option_max_bytes(language),
        "max-hops" => messages::help_option_max_hops(language),
        "type" => messages::help_option_type(language),
        "method" => messages::help_option_method(language),
        "filter" => messages::help_option_filter(language),
        "sort" => messages::help_option_sort(language),
        "reverse" => messages::help_option_reverse(language),
        "limit" => messages::help_option_limit(language),
        "offset" => messages::help_option_offset(language),
        "details" => messages::help_option_details(language),
        "output" => messages::help_option_output(language),
        "name" => messages::help_option_name(language),
        "source" => messages::help_option_source(language),
        "all" => messages::help_option_all(language),
        "group" => messages::help_option_group(language),
        "profile" => messages::help_option_profile(language),
        "position" => messages::help_option_position(language),
        "disabled" => messages::help_option_disabled(language),
        "level" => messages::help_option_level(language),
        "shell" => messages::help_option_shell(language),
        "unset" => messages::help_option_unset(language),
        "url" => messages::help_option_url(language),
        "host" => messages::help_option_host(language),
        "system" => messages::help_option_system(language),
        "category" => messages::help_option_category(language),
        "offline" => messages::help_option_offline(language),
        "open" => messages::help_option_open(language),
        _ => messages::help_option_generic(language),
    };
    format!(
        "  --{:<20} {}{description}\n",
        key,
        if VALUES.split_whitespace().any(|v| v == key) {
            messages::help_option_value_token(language)
        } else {
            ""
        },
    )
}

pub fn completion(shell: &str) -> Result<String> {
    Ok(match shell {
        "bash" => "_cor_complete() {\n  local IFS=$'\\n'\n  COMPREPLY=( $(cor __complete \"${COMP_WORDS[@]:1}\") )\n}\ncomplete -F _cor_complete cor\n".into(),
        "zsh" => "#compdef cor\n_cor() {\n  local -a candidates\n  candidates=(\"${(@f)$(cor __complete \"${(@)words[2,-1]}\")}\")\n  compadd -a candidates\n}\ncompdef _cor cor\n".into(),
        "fish" => "function __cor_complete\n  set -l tokens (commandline -opc)\n  set -e tokens[1]\n  cor __complete $tokens (commandline -ct)\nend\ncomplete -c cor -f -a '(__cor_complete)'\n".into(),
        "powershell" => "Register-ArgumentCompleter -Native -CommandName cor -ScriptBlock {\n  param($wordToComplete, $commandAst, $cursorPosition)\n  $words = @($commandAst.CommandElements | Select-Object -Skip 1 | ForEach-Object { $_.Extent.Text.Trim('\"').Trim(\"'\") })\n  if ($words.Count -eq 0 -or $words[-1] -ne $wordToComplete) { $words += $wordToComplete }\n  cor __complete @words | ForEach-Object { [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_) }\n}\n".into(),
        _ => bail!("补全支持 bash、zsh、fish、powershell"),
    })
}

/// Context-sensitive candidates for the hidden shell-completion entry point.
/// The final item is the incomplete word currently under the cursor.
pub fn complete(words: &[String]) -> String {
    let last = words.last().map(String::as_str).unwrap_or("");
    let completed = words
        .get(..words.len().saturating_sub(1))
        .unwrap_or_default();
    let tokens = completion_tokens(completed);
    let path = completion_path(&tokens);
    let path_parts = path.split_whitespace().count();
    let positional = tokens.get(path_parts..).unwrap_or_default();
    let spec = COMMANDS.iter().find(|spec| spec.path == path);

    let mut candidates = BTreeSet::new();
    if let Some((option, prefix)) = attached_option(last) {
        for value in option_values(&path, option) {
            candidates.insert(format!("{prefix}{value}"));
        }
    } else if let Some(option) = completed
        .last()
        .and_then(|value| value.strip_prefix("--"))
        .filter(|value| VALUES.split_whitespace().any(|name| name == *value))
    {
        candidates.extend(
            option_values(&path, option)
                .iter()
                .copied()
                .map(str::to_owned),
        );
    } else if last.starts_with("--") {
        for option in GLOBAL.split_whitespace().chain(
            spec.into_iter()
                .flat_map(|spec| spec.options.split_whitespace()),
        ) {
            candidates.insert(format!("--{option}"));
        }
    } else if spec.is_none() {
        candidates.extend(next_commands(&path));
    } else {
        candidates.extend(position_candidates(&path, positional));
    }
    let mut result = candidates
        .into_iter()
        .filter(|candidate| candidate.starts_with(last))
        .collect::<Vec<_>>()
        .join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    result
}

fn completion_tokens(words: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < words.len() {
        let word = &words[index];
        if word == "--" {
            result.extend(words[index + 1..].iter().cloned());
            break;
        }
        if let Some(option) = word.strip_prefix("--") {
            let (option, attached) = option
                .split_once('=')
                .map_or((option, false), |(option, _)| (option, true));
            index += if !attached && VALUES.split_whitespace().any(|name| name == option) {
                2
            } else {
                1
            };
            continue;
        }
        result.push(word.clone());
        index += 1;
    }
    result
}

fn completion_path(words: &[String]) -> String {
    let mut best = 0;
    for spec in COMMANDS {
        let parts = spec.path.split_whitespace().collect::<Vec<_>>();
        let shared = words
            .iter()
            .zip(parts)
            .take_while(|(word, part)| word == part)
            .count();
        best = best.max(shared);
    }
    words[..best].join(" ")
}

fn next_commands(path: &str) -> BTreeSet<String> {
    let prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{path} ")
    };
    let mut candidates = COMMANDS
        .iter()
        .filter(|spec| !spec.retired())
        .filter_map(|spec| {
            spec.path
                .strip_prefix(prefix.as_str())?
                .split_whitespace()
                .next()
                .map(str::to_owned)
        })
        .collect::<BTreeSet<_>>();
    if path.is_empty() {
        candidates.extend(["help".to_owned()]);
    }
    candidates
}

fn position_candidates(path: &str, positional: &[String]) -> Vec<String> {
    let index = positional.len();
    let values: &[&str] = match (path, index) {
        ("mode", 0) => &["rule", "global", "direct"],
        ("proxy", 0) => &["system", "tun", "off"],
        ("claude protect" | "autostart", 0) => &["on", "off"],
        ("rule enable", 1) => &["on", "off"],
        ("rule shortcut", 0) => &["ads", "private-domain", "private-ip", "cn-domain", "cn-ip"],
        ("rule shortcut", 1) => &["on", "off"],
        ("completion", 0) => &["bash", "zsh", "fish", "powershell"],
        ("config get", 0) => clash_client_core::config::READABLE_SETTINGS,
        ("config set", 0) => clash_client_core::config::MUTABLE_SETTINGS,
        ("config set", 1) => {
            return positional.first().map_or_else(Vec::new, |key| {
                setting_values(key)
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect()
            });
        }
        _ => &[],
    };
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn setting_values(key: &str) -> &'static [&'static str] {
    match key {
        "dark" | "tray_speed" | "update_prereleases" | "dns_override" | "fake_ip" | "ipv6"
        | "udp" => &["false", "true"],
        "theme_mode" => &["system", "light", "dark"],
        "language" => LanguagePreference::TAGS,
        "accent_color" => &["teal", "blue", "purple", "rose", "amber", "slate"],
        "node_sort" | "tui_node_sort" => &["latency_ascending", "latency_descending", "name"],
        _ => &[],
    }
}

fn attached_option(word: &str) -> Option<(&str, &str)> {
    let (option, value) = word.strip_prefix("--")?.split_once('=')?;
    VALUES
        .split_whitespace()
        .any(|name| name == option)
        .then_some((option, &word[..word.len() - value.len()]))
}

fn option_values(path: &str, option: &str) -> &'static [&'static str] {
    match option {
        "lang" => LanguagePreference::TAGS,
        "type" if path == "network dns" => &[
            "ADDR", "A", "AAAA", "CNAME", "MX", "TXT", "NS", "SOA", "PTR", "CAA",
        ],
        "method" if path == "network http" => &["HEAD", "GET", "OPTIONS"],
        "sort" if path == "node list" => &["name", "latency"],
        "sort" => &[
            "name",
            "rule",
            "chain",
            "upload",
            "download",
            "upload-rate",
            "download-rate",
            "start",
            "none",
        ],
        "level" => &["debug", "info", "warning", "error", "silent"],
        "shell" => &["bash", "zsh", "fish", "powershell", "cmd"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn help(path: &str) -> String {
        localized_help(path, Language::SimplifiedChinese)
    }

    fn args(line: &[&str]) -> Result<Args> {
        parse(line.iter().map(|s| (*s).into()))
    }
    #[test]
    fn flags_before_commands_and_literal_values_keep_their_meaning() {
        let parsed = args(&["--json", "profile", "add", "name", "--", "--not-an-option"]).unwrap();
        assert_eq!(parsed.path, "profile add");
        assert!(parsed.flag("json"));
        assert_eq!(parsed.positional[1], "--not-an-option");
        assert!(args(&["profile", "list", "--srot", "name"]).is_err());
        assert!(args(&["mode", "rule", "extra"]).is_err());
        assert!(args(&["--json", "--json", "status"]).is_err());
    }
    #[test]
    fn language_selection_does_not_interpret_literal_or_option_values_as_flags() {
        for words in [
            vec!["--lang=zh-CN", "--output", "--lang=en"],
            vec!["--lang=zh-CN", "--", "--lang=en"],
        ] {
            let input: Vec<String> = words.into_iter().map(str::to_owned).collect();
            assert_eq!(language_from_input(&input), Language::SimplifiedChinese);
        }
    }
    #[test]
    fn network_inspection_commands_validate_targets_and_options_independently() {
        for tool in clash_client_core::network_tools::Tool::ALL {
            let mut command = vec!["network", tool.id()];
            if tool.needs_target() {
                command.push("🇰🇷 韩国 01");
            }
            assert!(args(&command).is_ok(), "{command:?}");
        }
        assert!(args(&["network", "traffic", "unexpected"]).is_err());
        assert!(args(&["network", "node"]).is_err());
        assert!(args(&["network", "http", "example.test", "--method", "GET"]).is_ok());
        assert!(args(&["network", "dns", "example.test", "--type", "AAAA"]).is_ok());
        assert!(args(&["network", "tcp", "example.test:22", "--method", "GET"]).is_err());
        assert_eq!(
            option_values("network http", "method"),
            ["HEAD", "GET", "OPTIONS"]
        );
    }
    #[test]
    fn every_registered_command_has_help_and_invalid_input_stays_an_error() {
        for spec in COMMANDS {
            let words = std::iter::once("help".to_owned())
                .chain(spec.path.split_whitespace().map(str::to_owned));
            let parsed = parse(words).unwrap();
            assert!(parsed.help);
            assert!(help(&parsed.path).contains(spec.usage));
        }
        assert!(args(&["help", "does-not-exist"]).is_err());
        assert_eq!(args(&["profile"]).unwrap().path, "profile");
    }
    #[test]
    fn grouped_help_aliases_do_not_consume_a_literal_node_name() {
        for input in [
            vec!["node", "help"],
            vec!["help", "node"],
            vec!["node", "--help"],
        ] {
            let parsed = args(&input).unwrap();
            assert!(parsed.help);
            assert_eq!(parsed.path, "node");
            assert!(help(&parsed.path).contains("node list"));
        }
        let parsed = args(&["node", "list", "help"]).unwrap();
        assert!(!parsed.help);
        assert_eq!(parsed.positional, ["help"]);
        assert!(args(&["node", "unknown"]).is_err());
    }
    #[test]
    fn default_help_is_compact_and_full_help_keeps_compatible_commands() {
        let compact = help("");
        assert!(compact.contains("cor help all"));
        assert!(compact.contains("cor node help"));
        assert!(!compact.contains("profile import"));
        let full = args(&["help", "all"]).unwrap();
        assert!(full.help);
        for spec in COMMANDS {
            assert!(help(&full.path).contains(spec.usage.split_whitespace().next().unwrap()));
            assert!(help(&full.path).contains(spec.path));
        }
        assert!(!help("profile").contains("profile import"));
        assert!(args(&["profile", "import", "example.yaml", "--name", "example"]).is_ok());
    }
    #[test]
    fn network_refresh_is_an_explicit_restart_option() {
        let parsed = args(&["restart", "--network-only", "--yes"]).unwrap();
        assert!(parsed.flag("network-only"));
        assert!(parsed.flag("yes"));
        assert!(args(&["start", "--network-only"]).is_err());
        assert!(help("restart").contains("关闭旧连接"));
    }
    #[test]
    fn update_can_explicitly_preserve_a_stopped_core_without_changing_other_commands() {
        assert!(
            args(&["update", "install", "--no-start"])
                .unwrap()
                .flag("no-start")
        );
        assert!(args(&["update", "check", "--no-start"]).is_err());
        assert!(args(&["start", "--no-start"]).is_err());
        assert!(help("update install").contains("--no-start"));
    }
    #[test]
    fn diagnostics_can_export_local_evidence_without_starting_a_backend() {
        let parsed = args(&["diagnose", "--offline", "--output", "report.json"]).unwrap();
        assert!(parsed.flag("offline"));
        assert_eq!(parsed.get("output"), Some("report.json"));
        assert!(args(&["status", "--offline"]).is_err());
        assert!(help("diagnose").contains("--output"));
    }
    #[test]
    fn cross_user_inventory_cannot_be_confused_with_a_selected_data_directory() {
        assert!(args(&["profile", "list", "--all-users"]).is_ok());
        assert!(args(&["profile", "list", "--all-users", "--user", "alice"]).is_err());
        assert!(args(&["profile", "list", "--all-users", "--data-dir", "/tmp/data"]).is_err());
        assert!(args(&["profile", "use", "same-name", "--all-users"]).is_err());
        assert_eq!(
            args(&["--user=alice", "status"]).unwrap().get("user"),
            Some("alice")
        );
    }
    #[test]
    fn cross_user_tun_observation_is_explicit_and_limited_to_network_commands() {
        for command in ["status", "proxy", "stop", "diagnose"] {
            assert!(args(&[command, "--all-users"]).is_ok(), "{command}");
        }
        assert!(args(&["--all-users", "status"]).is_ok());
        assert!(args(&["status", "--watch", "--all-users"]).is_ok());
        assert!(args(&["proxy", "--all-users"]).is_ok());
        assert!(args(&["proxy", "off", "--all-users"]).is_err());
        assert!(args(&["node", "list", "--all-users"]).is_err());
        assert!(args(&["status", "--all-users", "--user", "alice"]).is_err());
    }
    #[test]
    fn contextual_completion_handles_commands_keys_values_and_options() {
        let complete = |line: &[&str]| {
            super::complete(
                &line
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect::<Vec<_>>(),
            )
        };
        assert!(complete(&["co"]).contains("completion\n"));
        assert!(complete(&["config", "g"]).contains("get\n"));
        assert!(complete(&["config", "set", "the"]).contains("theme_mode\n"));
        assert_eq!(
            complete(&["config", "set", "theme_mode", ""]),
            "dark\nlight\nsystem\n"
        );
        assert_eq!(complete(&["config", "set", "dns_override", "t"]), "true\n");
        assert!(complete(&["config", "set", "ipv6", "--"]).contains("--help\n"));
        assert_eq!(complete(&["node", "list", "--sort", ""]), "latency\nname\n");
        assert_eq!(complete(&["env", "--shell", "p"]), "powershell\n");
        assert!(complete(&["config", "get", ""]).contains("update_prereleases\n"));
        assert!(
            complete(&["config", "get", "update_"]).contains("update_check_interval_minutes\n")
        );
        assert!(
            complete(&["config", "set", "update_"]).contains("update_check_interval_minutes\n")
        );
        assert!(!complete(&["config", "get", ""]).contains("controller_port\n"));
        assert!(!complete(&["config", "set", ""]).contains("active_profile\n"));
    }
    #[test]
    fn retired_network_tools_keep_explicit_help_but_leave_menus_and_completion() {
        let menu = help("network");
        let candidates = complete(&["network".into(), "".into()]);
        for command in [
            "explain",
            "rule",
            "config-diff",
            "traffic",
            "performance",
            "node",
        ] {
            let path = format!("network {command}");
            assert!(!menu.contains(&path));
            assert!(!candidates.lines().any(|candidate| candidate == command));
            assert!(help(&path).contains("已移除"));
            assert!(help("all").contains(&path));
        }
        for tool in clash_client_core::network_tools::Tool::ALL {
            assert!(menu.contains(&format!("network {}", tool.id())));
            assert!(candidates.lines().any(|candidate| candidate == tool.id()));
        }
    }
    #[test]
    fn completion_scripts_delegate_to_the_contextual_entry_point() {
        for shell in ["bash", "zsh", "fish", "powershell"] {
            assert!(completion(shell).unwrap().contains("__complete"));
        }
    }
}
