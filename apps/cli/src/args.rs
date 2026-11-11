//! One command catalogue drives parsing, help and shell completions.
use anyhow::{Result, bail, ensure};
use std::collections::{BTreeMap, BTreeSet};

pub struct Spec {
    pub path: &'static str,
    pub description: &'static str,
    pub usage: &'static str,
    pub min: usize,
    pub max: usize,
    pub options: &'static str,
}
macro_rules! command {
    ($path:literal, $description:literal, $usage:literal, $min:literal, $max:literal, $options:literal) => {
        Spec {
            path: $path,
            description: $description,
            usage: $usage,
            min: $min,
            max: $max,
            options: $options,
        }
    };
}
pub static COMMANDS: &[Spec] = &[
    command!(
        "run",
        "前台运行后台与内核；Ctrl+C 恢复网络后退出",
        "run [--no-core]",
        0,
        0,
        "no-core"
    ),
    command!(
        "start",
        "启动后台和内核，已有 GUI 时复用它",
        "start",
        0,
        0,
        ""
    ),
    command!(
        "stop",
        "停止内核并恢复网络；独立后台同时退出",
        "stop [--all-users]",
        0,
        0,
        "all-users"
    ),
    command!(
        "restart",
        "重启内核；--network-only 仅刷新出站与 DNS 并关闭旧连接",
        "restart [--network-only] [--yes]",
        0,
        0,
        "network-only"
    ),
    command!(
        "status",
        "显示内核、订阅、节点、模式及实时流量",
        "status [--watch] [--all-users]",
        0,
        0,
        "watch interval all-users"
    ),
    command!("version", "显示客户端版本与本机架构", "version", 0, 0, ""),
    command!(
        "profile list",
        "列出订阅；来源中的凭据默认脱敏",
        "profile list [--filter TEXT --all-users]",
        0,
        0,
        "filter all-users"
    ),
    command!(
        "profile add",
        "从 URL、YAML 路径或标准输入 - 导入订阅",
        "profile add NAME SOURCE",
        2,
        2,
        ""
    ),
    command!(
        "profile import",
        "从 YAML 文件或标准输入 - 导入订阅",
        "profile import FILE --name NAME",
        1,
        1,
        "name"
    ),
    command!(
        "profile update",
        "更新一份订阅或全部非内置订阅",
        "profile update [PROFILE] [--all]",
        0,
        1,
        "all"
    ),
    command!(
        "profile use",
        "按唯一名称或 ID 切换当前订阅",
        "profile use PROFILE",
        1,
        1,
        ""
    ),
    command!(
        "profile edit",
        "修改名称或来源；重新更新后载入新来源",
        "profile edit PROFILE [--name NAME] [--source SOURCE]",
        1,
        1,
        "name source"
    ),
    command!(
        "profile remove",
        "删除订阅；活动订阅会切换到默认直连",
        "profile remove PROFILE [--yes]",
        1,
        1,
        ""
    ),
    command!(
        "profile export",
        "导出缓存 YAML；导出完整凭据需 --show-secrets",
        "profile export PROFILE [--output FILE]",
        1,
        1,
        "output"
    ),
    command!(
        "profile directory",
        "输出订阅文件目录",
        "profile directory",
        0,
        0,
        ""
    ),
    command!(
        "node list",
        "查看策略组或成员，支持自然名称与延迟排序",
        "node list [GROUP] [--sort name|latency] [--reverse]",
        0,
        1,
        "filter sort reverse limit offset"
    ),
    command!(
        "node select",
        "选择策略组节点；保护期间禁止切换",
        "node select GROUP NODE",
        2,
        2,
        ""
    ),
    command!(
        "node test",
        "节点、单组或全部测速；自动组按共享策略重新选优",
        "node test [NODE] [--group GROUP | --all]",
        0,
        1,
        "group all"
    ),
    command!(
        "mode",
        "读取或切换规则、全局、直连模式",
        "mode [rule|global|direct]",
        0,
        1,
        ""
    ),
    command!(
        "proxy",
        "读取或切换系统代理、TUN、关闭状态",
        "proxy [system|tun|off] [--all-users]",
        0,
        1,
        "all-users"
    ),
    command!(
        "env",
        "输出供父终端显式加载的代理环境变量",
        "env [--shell bash|zsh|fish|powershell|cmd] [--unset]",
        0,
        0,
        "shell unset"
    ),
    command!(
        "rule list",
        "查看指定或当前订阅的有效规则、来源与稳定 ID",
        "rule list [--profile PROFILE] [--filter TEXT]",
        0,
        0,
        "profile filter limit offset"
    ),
    command!(
        "rule add",
        "新增自定义规则，校验后立即保存应用",
        "rule add 'TYPE,VALUE,POLICY' [--position N]",
        1,
        1,
        "profile position disabled"
    ),
    command!(
        "rule edit",
        "修改自定义规则；订阅原规则保持只读",
        "rule edit RULE 'TYPE,VALUE,POLICY'",
        2,
        2,
        "profile"
    ),
    command!(
        "rule remove",
        "删除自定义规则",
        "rule remove RULE [--yes]",
        1,
        1,
        "profile"
    ),
    command!(
        "rule move",
        "将任意规则移到从 1 开始的位置",
        "rule move RULE POSITION",
        2,
        2,
        "profile"
    ),
    command!(
        "rule enable",
        "启用或停用自定义规则",
        "rule enable RULE on|off",
        2,
        2,
        "profile"
    ),
    command!(
        "rule shortcut",
        "读写五项快捷规则；原订阅已有规则不重复添加",
        "rule shortcut [ads|private-domain|private-ip|cn-domain|cn-ip] [on|off]",
        0,
        2,
        "profile"
    ),
    command!(
        "connection list",
        "查看连接、双向速率与流量，支持筛选排序和连续采样",
        "connection list [--watch] [--sort download-rate]",
        0,
        0,
        "watch interval filter sort reverse limit offset"
    ),
    command!(
        "connection show",
        "查看完整连接详情，按需补充目标 IP",
        "connection show ID",
        1,
        1,
        ""
    ),
    command!(
        "connection close",
        "关闭一条或全部连接",
        "connection close [ID] [--all --yes]",
        0,
        1,
        "all"
    ),
    command!(
        "logs",
        "查看完整日志，支持等级、文本筛选与实时跟随",
        "logs [--follow] [--level warning] [--filter TEXT]",
        0,
        0,
        "follow interval level filter limit"
    ),
    command!(
        "logs clear",
        "清空客户端有界日志缓冲区",
        "logs clear [--yes]",
        0,
        0,
        ""
    ),
    command!(
        "logs export",
        "导出当前最多 500 条缓冲日志；排障现场用 diagnose --output",
        "logs export --output FILE",
        0,
        0,
        "output"
    ),
    command!(
        "config get",
        "读取设置；密钥默认脱敏",
        "config get [KEY]",
        0,
        1,
        ""
    ),
    command!(
        "config set",
        "校验并应用一个设置；网络模式使用 mode/proxy 命令",
        "config set KEY VALUE",
        2,
        2,
        ""
    ),
    command!(
        "config check",
        "用内核校验当前合并后的实际配置",
        "config check",
        0,
        0,
        ""
    ),
    command!(
        "config effective",
        "输出含覆盖规则的有效 YAML，默认移除凭据",
        "config effective [--output FILE]",
        0,
        0,
        "output"
    ),
    command!(
        "diagnose",
        "检查网络；--output 保存诊断报告，--offline 仅收集本地现场",
        "diagnose [--url URL] [--host DOMAIN] [--offline] [--output FILE] [--all-users]",
        0,
        0,
        "url host offline output all-users"
    ),
    command!(
        "dns",
        "查询系统或经内核代理的 DNS",
        "dns DOMAIN [--system]",
        1,
        1,
        "system"
    ),
    command!(
        "ip-check list",
        "列出内置 IP/地区检测项目",
        "ip-check list [--category CATEGORY]",
        0,
        0,
        "category filter"
    ),
    command!(
        "ip-check",
        "执行全部、分类或单项检测并生成总结",
        "ip-check [ID] [--category CATEGORY] [--output FILE]",
        0,
        1,
        "category output"
    ),
    command!(
        "claude check",
        "检测本机网络和 Claude 环境的地区线索",
        "claude check [--output FILE]",
        0,
        0,
        "output"
    ),
    command!(
        "claude audit",
        "扫描实际终端环境与已记录 Claude 项目配置",
        "claude audit [--offline] [--output FILE]",
        0,
        0,
        "offline output"
    ),
    command!(
        "claude browser",
        "创建浏览器真实采样入口并等待报告",
        "claude browser [--open] [--output FILE]",
        0,
        0,
        "open output"
    ),
    command!(
        "claude protect",
        "查看或开关 Claude 保护，开启需要确认",
        "claude protect [on|off] [--yes]",
        0,
        1,
        ""
    ),
    command!(
        "claude watch",
        "实时查看异常连接及拦截记录",
        "claude watch [--interval SECONDS]",
        0,
        0,
        "interval"
    ),
    command!("geo status", "查看 Geo 数据版本", "geo status", 0, 0, ""),
    command!(
        "geo update",
        "校验并更新全部 Geo 数据",
        "geo update",
        0,
        0,
        ""
    ),
    command!(
        "update check",
        "按已保存的更新通道检查客户端更新",
        "update check",
        0,
        0,
        ""
    ),
    command!(
        "update download",
        "下载并校验当前通道的更新安装包",
        "update download --output FILE",
        0,
        0,
        "output"
    ),
    command!(
        "update status",
        "查看本账户最近一次更新安装的结果，不启动后台",
        "update status",
        0,
        0,
        ""
    ),
    command!(
        "update install",
        "下载安装更新；Linux 终端等待结果，失败尝试恢复后台",
        "update install [--yes]",
        0,
        0,
        ""
    ),
    command!(
        "autostart",
        "读取或配置桌面程序登录后后台自启",
        "autostart [on|off]",
        0,
        1,
        ""
    ),
    command!(
        "service install",
        "安装当前用户的无界面后台自启",
        "service install [--yes]",
        0,
        0,
        ""
    ),
    command!(
        "service remove",
        "删除当前用户的 CLI 后台自启",
        "service remove [--yes]",
        0,
        0,
        ""
    ),
    command!(
        "service status",
        "查看 CLI 后台自启配置",
        "service status",
        0,
        0,
        ""
    ),
    command!(
        "notify",
        "调用与 Claude 拦截共用的系统通知入口",
        "notify [MESSAGE]",
        0,
        1,
        ""
    ),
    command!(
        "completion",
        "生成 Bash、Zsh、Fish 或 PowerShell 上下文补全脚本",
        "completion SHELL",
        1,
        1,
        ""
    ),
];

const GLOBAL: &str =
    "help version json jsonl timeout data-dir resources user yes show-secrets quiet no-color";
pub(crate) const VALUES: &str = "timeout data-dir resources user filter name source output sort limit offset group profile position interval level shell url host category";

#[derive(Debug)]
pub struct Usage(pub String);
impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Usage {}

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
    pub fn number(&self, key: &str, default: usize, max: usize) -> Result<usize> {
        let value = self.get(key).map_or(Ok(default), str::parse::<usize>)?;
        ensure!(value <= max, "--{key} 不能超过 {max}");
        Ok(value)
    }
    pub fn interval(&self) -> Result<std::time::Duration> {
        let seconds = self.get("interval").unwrap_or("1").parse::<f64>()?;
        ensure!(
            seconds.is_finite() && (0.25..=3600.).contains(&seconds),
            "采样间隔应为 0.25–3600 秒"
        );
        Ok(std::time::Duration::from_secs_f64(seconds))
    }
}

pub fn parse(input: impl IntoIterator<Item = String>) -> Result<Args> {
    parse_inner(input)
        .and_then(|args| {
            if !args.help {
                validate(&args)?;
            }
            Ok(args)
        })
        .map_err(|error| Usage(error.to_string()).into())
}
fn validate(args: &Args) -> Result<()> {
    ensure!(
        !args.flag("all-users") || (args.get("user").is_none() && args.get("data-dir").is_none()),
        "--all-users 不能与 --user 或 --data-dir 同时使用"
    );
    ensure!(
        !(args.flag("json") && args.flag("jsonl")),
        "--json 与 --jsonl 不能同时指定"
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
                "--{key} 必须大于 0"
            );
        }
    }
    if args.get("interval").is_some() {
        args.interval()?;
    }
    if matches!(args.path.as_str(), "logs export" | "update download") {
        ensure!(args.get("output").is_some(), "请提供 --output FILE");
    }
    let enum_value = |value: &str, choices: &[&str]| -> Result<()> {
        ensure!(
            choices.contains(&value),
            "{} 的参数应为 {}",
            args.path,
            choices.join(" / ")
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
                "--all-users 仅用于跨用户观察，不能同时切换代理"
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
        "profile import" => ensure!(args.get("name").is_some(), "请提供 --name NAME"),
        "profile edit" => ensure!(
            args.get("name").is_some() || args.get("source").is_some(),
            "请提供 --name 或 --source"
        ),
        "profile update" => ensure!(
            !(args.flag("all") && !args.positional.is_empty()),
            "PROFILE 与 --all 不能同时指定"
        ),
        "node test" => ensure!(
            usize::from(!args.positional.is_empty())
                + usize::from(args.get("group").is_some())
                + usize::from(args.flag("all"))
                == 1,
            "请选择 NODE、--group GROUP 或 --all 中的一项"
        ),
        "connection close" => ensure!(
            args.flag("all") != !args.positional.is_empty(),
            "请选择 ID 或 --all"
        ),
        "rule add" => {
            clash_client_core::rule_manager::parts(&args.positional[0])?;
        }
        "rule edit" => {
            clash_client_core::rule_manager::parts(&args.positional[1])?;
        }
        "rule move" => {
            ensure!(args.positional[1].parse::<usize>()? > 0, "位置必须大于 0");
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
fn parse_inner(input: impl IntoIterator<Item = String>) -> Result<Args> {
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
        let value = value
            .strip_prefix("--")
            .ok_or_else(|| anyhow::anyhow!("不支持组合短选项：{value}"))?;
        let (key, supplied) = value
            .split_once('=')
            .map_or((value, None), |(k, v)| (k, Some(v.to_owned())));
        let needs_value = VALUES.split_whitespace().any(|v| v == key);
        let parameter = if needs_value {
            supplied
                .or_else(|| input.next())
                .ok_or_else(|| anyhow::anyhow!("--{key} 缺少参数"))?
        } else {
            ensure!(supplied.is_none(), "--{key} 不接受参数");
            "true".into()
        };
        ensure!(
            options.insert(key.to_owned(), parameter).is_none(),
            "--{key} 重复指定"
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
                "分组帮助不接受命令专属选项"
            );
            return Ok(Args {
                path: prefix,
                positional: vec![],
                options,
                help: true,
            });
        }
        bail!("未知命令：{}；运行 cor help 查看命令", words.join(" "));
    };
    let positional = words[spec.path.split_whitespace().count()..].to_vec();
    for key in options.keys() {
        ensure!(
            GLOBAL
                .split_whitespace()
                .chain(spec.options.split_whitespace())
                .any(|allowed| allowed == key),
            "{} 不支持 --{key}",
            spec.path
        );
    }
    if !help {
        ensure!(
            (spec.min..=spec.max).contains(&positional.len()),
            "用法：cor {}",
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

pub fn help(prefix: &str) -> String {
    let mut out = format!("Clash of Rust CLI {}\n\n", clash_client_core::VERSION);
    if prefix.is_empty() {
        out += "用法：cor <命令> [选项]\n\n常用命令：\n";
        for path in ["start", "stop", "restart", "status", "proxy", "diagnose"] {
            let spec = COMMANDS.iter().find(|spec| spec.path == path).unwrap();
            out += &format!("  {:<25} {}\n", spec.path, spec.description);
        }
        out += "  profile                   订阅添加、切换与更新；cor profile help\n";
        out += "  node                      节点选择与测速；cor node help\n";
        let advanced = COMMANDS
            .iter()
            .map(|spec| spec.path.split_whitespace().next().unwrap())
            .filter(|path| {
                ![
                    "start", "stop", "restart", "status", "proxy", "diagnose", "profile", "node",
                ]
                .contains(path)
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join("、");
        out += &format!("\n其他功能：{advanced}\n完整命令与通用选项：cor help all\n");
    } else if prefix == "all" {
        out += "用法：cor <命令> [选项]\n\n全部命令：\n";
    } else if let Some(spec) = COMMANDS.iter().find(|s| s.path == prefix) {
        out += &format!("{}\n\n用法：cor {}\n\n", spec.description, spec.usage);
        if !spec.options.is_empty() {
            out += "命令选项：\n";
            for key in spec.options.split_whitespace() {
                out += &option_help(key);
            }
            out += "\n";
        }
    } else {
        out += &format!(
            "用法：cor {}{}<命令> [选项]\n\n命令：\n",
            prefix,
            if prefix.is_empty() { "" } else { " " }
        );
    }
    for spec in COMMANDS.iter().filter(|spec| {
        prefix == "all"
            || (!prefix.is_empty()
                && spec.path.starts_with(&(prefix.to_owned() + " "))
                && spec.path != "profile import")
    }) {
        out += &format!("  {:<25} {}\n", spec.path, spec.description);
    }
    let example = match prefix {
        "profile add" => Some("cor profile add 我的订阅 'https://example.com/subscription'"),
        "profile import" => Some("cat profile.yaml | cor profile import - --name 本地订阅"),
        "node list" => Some("cor node list Proxy --sort latency --limit 50"),
        "node select" => Some("cor node select Proxy '节点 1'"),
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
        out += &format!("\n示例：\n  {example}\n");
    }
    out += "\n通用选项：\n";
    for key in (if prefix.is_empty() {
        "help version json data-dir user"
    } else {
        GLOBAL
    })
    .split_whitespace()
    {
        out += &option_help(key);
    }
    out += "\n详情：cor help <命令>；使用文档：docs/cli.md\n退出码：0 成功，1 操作失败，2 参数错误，3 后台不可达，124 请求超时，130 已取消。\n";
    out
}
fn option_help(key: &str) -> String {
    let description = match key {
        "help" => "显示当前命令及参数帮助（-h）",
        "version" => "显示客户端版本（-V）",
        "json" => "只输出机器可读 JSON",
        "jsonl" => "逐行输出 JSON，适合持续采样",
        "timeout" => "单次后台请求超时秒数，默认 300",
        "data-dir" => "指定独立数据目录",
        "resources" => "指定内核与 Geo 资源目录",
        "user" => "Linux 管理员以指定用户身份操作其配置与后台",
        "all-users" => "Linux root 跨用户只读观察；订阅列表或 TUN 持有进程",
        "yes" => "明确确认删除、保护或安装等操作（-y）",
        "show-secrets" => "显式允许输出订阅/配置中的完整凭据",
        "quiet" => "不显示进度提示（-q）",
        "no-color" => "纯文本输出；CLI 默认不使用颜色",
        "no-core" => "仅运行后台，暂不启动内核",
        "network-only" => "刷新当前内核出站和 DNS，关闭旧连接；不重启内核",
        "watch" | "follow" => "持续输出新采样/日志，Ctrl+C 结束",
        "interval" => "采样秒数，默认 1，最小 0.25",
        "filter" => "按文本筛选",
        "sort" => {
            "排序字段；node: name/latency；connection: name/rule/chain/upload/download/upload-rate/download-rate/start"
        }
        "reverse" => "反转排序方向（速率/流量默认降序，名称默认升序）",
        "limit" => "最多输出条数，默认 100，最大 10000",
        "offset" => "跳过的条数，默认 0",
        "output" => "写入目标文件（-o）；完整日志/配置文件仅当前用户可读写",
        "name" => "订阅显示名称",
        "source" => "订阅来源 URL 或本机 YAML 路径",
        "all" => "操作全部项目",
        "group" => "只测试指定策略组",
        "profile" => "按唯一名称或 ID 指定订阅，默认当前使用中",
        "position" => "规则位置，从 1 开始，默认放在最前",
        "disabled" => "新增规则暂不启用",
        "level" => "日志等级：debug/info/warning/error/silent",
        "shell" => "环境变量语法：bash/zsh/fish/powershell/cmd",
        "unset" => "输出清除代理环境变量的语句",
        "url" => "HTTP/HTTPS 诊断地址",
        "host" => "诊断域名，默认 github.com",
        "system" => "使用本机系统 DNS，默认使用内核/代理 DNS",
        "category" => "检测分类，例如 AI、全球、日本",
        "offline" => "仅收集本机信息，不请求外部检测服务",
        "open" => "在默认浏览器打开本机采样入口",
        _ => "见当前命令说明",
    };
    format!(
        "  --{:<20} {}{}\n",
        key,
        if VALUES.split_whitespace().any(|v| v == key) {
            "<值>  "
        } else {
            ""
        },
        description
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
        "accent_color" => &["teal", "blue", "purple", "rose", "amber", "slate"],
        "node_sort" => &["latency_ascending", "latency_descending", "name"],
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
    fn completion_scripts_delegate_to_the_contextual_entry_point() {
        for shell in ["bash", "zsh", "fish", "powershell"] {
            assert!(completion(shell).unwrap().contains("__complete"));
        }
    }
}
