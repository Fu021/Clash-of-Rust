//! One command catalogue drives parsing, help and shell completions.
use anyhow::{Result, bail, ensure};
use clash_client_core::config::{Language, LanguagePreference, Store};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub struct Spec {
    pub path: &'static str,
    pub description: &'static str,
    english_description: &'static str,
    pub usage: &'static str,
    pub min: usize,
    pub max: usize,
    pub options: &'static str,
}
impl Spec {
    pub fn localized_description(&self, language: Language) -> &'static str {
        language.text(self.description, self.english_description)
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
    ($path:literal, $description:literal, $english:literal, $usage:literal, $min:literal, $max:literal, $options:literal) => {
        Spec {
            path: $path,
            description: $description,
            english_description: $english,
            usage: $usage,
            min: $min,
            max: $max,
            options: $options,
        }
    };
}
pub static COMMANDS: &[Spec] = &[
    command!(
        "ui",
        "打开终端界面；含设置与管理命令，退出保留后台和代理",
        "Open the terminal interface. The backend and proxy stay running when you exit.",
        "ui [--demo]",
        0,
        0,
        "demo"
    ),
    command!(
        "run",
        "前台运行后台与内核；Ctrl+C 恢复网络后退出",
        "Run the backend and core in the foreground. Ctrl+C restores networking and exits.",
        "run [--no-core]",
        0,
        0,
        "no-core"
    ),
    command!(
        "start",
        "启动后台和内核，已有 GUI 时复用它",
        "Start the backend and core, reusing the running desktop app if available.",
        "start",
        0,
        0,
        ""
    ),
    command!(
        "stop",
        "停止内核并恢复网络；独立后台同时退出",
        "Stop the core, restore networking, and exit a standalone backend.",
        "stop [--all-users]",
        0,
        0,
        "all-users"
    ),
    command!(
        "restart",
        "重启内核；--network-only 仅刷新出站与 DNS 并关闭旧连接",
        "Restart the core. Use --network-only to refresh outbound routes and DNS and close old connections.",
        "restart [--network-only] [--yes]",
        0,
        0,
        "network-only"
    ),
    command!(
        "status",
        "显示内核、订阅、节点、模式及实时流量",
        "Show the core status, subscription, nodes, mode, and live traffic.",
        "status [--watch] [--all-users]",
        0,
        0,
        "watch interval all-users"
    ),
    command!(
        "version",
        "显示客户端版本与本机架构",
        "Show the client version and machine architecture.",
        "version",
        0,
        0,
        ""
    ),
    command!(
        "profile list",
        "列出订阅；来源中的凭据默认脱敏",
        "List subscriptions. Source credentials are redacted by default.",
        "profile list [--filter TEXT --all-users --output FILE]",
        0,
        0,
        "filter all-users output"
    ),
    command!(
        "profile add",
        "从 URL、YAML 路径或标准输入 - 导入订阅",
        "Import a subscription from a URL, YAML path, or standard input (-).",
        "profile add NAME SOURCE",
        2,
        2,
        ""
    ),
    command!(
        "profile import",
        "从 YAML 文件或标准输入 - 导入订阅",
        "Import a subscription from a YAML file or standard input (-).",
        "profile import FILE --name NAME",
        1,
        1,
        "name"
    ),
    command!(
        "profile update",
        "更新一份订阅或全部非内置订阅",
        "Update one subscription or all user subscriptions.",
        "profile update [PROFILE] [--all]",
        0,
        1,
        "all"
    ),
    command!(
        "profile use",
        "按唯一名称或 ID 切换当前订阅",
        "Switch the active subscription by unique name or ID.",
        "profile use PROFILE",
        1,
        1,
        ""
    ),
    command!(
        "profile edit",
        "修改名称或来源；重新更新后载入新来源",
        "Edit the subscription name or source. Update it to load the new source.",
        "profile edit PROFILE [--name NAME] [--source SOURCE]",
        1,
        1,
        "name source"
    ),
    command!(
        "profile remove",
        "删除订阅；活动订阅会切换到默认直连",
        "Remove a subscription. Removing the active one switches to the built-in direct profile.",
        "profile remove PROFILE [--yes]",
        1,
        1,
        ""
    ),
    command!(
        "profile export",
        "导出缓存 YAML；导出完整凭据需 --show-secrets",
        "Export cached YAML. Use --show-secrets to include credentials.",
        "profile export PROFILE [--output FILE]",
        1,
        1,
        "output"
    ),
    command!(
        "profile directory",
        "输出订阅文件目录",
        "Print the subscription directory.",
        "profile directory",
        0,
        0,
        ""
    ),
    command!(
        "node list",
        "查看策略组或成员，支持自然名称与延迟排序",
        "List proxy groups or their members, sorted by name or latency.",
        "node list [GROUP] [--sort name|latency] [--reverse]",
        0,
        1,
        "filter sort reverse limit offset"
    ),
    command!(
        "node select",
        "选择策略组节点；省略参数时交互筛选，保护期间禁止切换",
        "Choose a node in a proxy group. Omit arguments for an interactive picker. Protection locks node selection.",
        "node select [GROUP [NODE]]",
        0,
        2,
        ""
    ),
    command!(
        "node test",
        "节点、单组或全部测速；自动组按共享策略重新选优",
        "Test latency for one node, one group, or all nodes. Automatic groups reselect their preferred node.",
        "node test [NODE] [--group GROUP | --all]",
        0,
        1,
        "group all"
    ),
    command!(
        "mode",
        "读取或切换规则、全局、直连模式",
        "Show or switch between rule, global, and direct modes.",
        "mode [rule|global|direct]",
        0,
        1,
        ""
    ),
    command!(
        "proxy",
        "读取或切换系统代理、TUN、关闭状态",
        "Show or switch between system proxy, TUN, and off.",
        "proxy [system|tun|off] [--all-users]",
        0,
        1,
        "all-users"
    ),
    command!(
        "env",
        "输出供父终端显式加载的代理环境变量",
        "Print proxy environment variables for the parent shell to load.",
        "env [--shell bash|zsh|fish|powershell|cmd] [--unset]",
        0,
        0,
        "shell unset"
    ),
    command!(
        "rule list",
        "查看指定或当前订阅的有效规则、来源与稳定 ID",
        "Show effective rules, sources, and stable IDs for the selected or active subscription.",
        "rule list [--profile PROFILE] [--filter TEXT]",
        0,
        0,
        "profile filter limit offset"
    ),
    command!(
        "rule add",
        "新增自定义规则，校验后立即保存应用",
        "Validate, save, and apply a new custom rule.",
        "rule add 'TYPE,VALUE,POLICY' [--position N]",
        1,
        1,
        "profile position disabled"
    ),
    command!(
        "rule edit",
        "修改自定义规则；订阅原规则保持只读",
        "Edit a custom rule. Subscription rules are read-only.",
        "rule edit RULE 'TYPE,VALUE,POLICY'",
        2,
        2,
        "profile"
    ),
    command!(
        "rule remove",
        "删除自定义规则",
        "Remove a custom rule.",
        "rule remove RULE [--yes]",
        1,
        1,
        "profile"
    ),
    command!(
        "rule move",
        "将任意规则移到从 1 开始的位置",
        "Move a rule to a position starting at 1.",
        "rule move RULE POSITION",
        2,
        2,
        "profile"
    ),
    command!(
        "rule enable",
        "启用或停用自定义规则",
        "Enable or disable a custom rule.",
        "rule enable RULE on|off",
        2,
        2,
        "profile"
    ),
    command!(
        "rule shortcut",
        "读写五项快捷规则；原订阅已有规则不重复添加",
        "Show or change five rule shortcuts. Existing subscription rules are reused.",
        "rule shortcut [ads|private-domain|private-ip|cn-domain|cn-ip] [on|off]",
        0,
        2,
        "profile"
    ),
    command!(
        "connection list",
        "查看连接、双向速率与流量，支持筛选排序和连续采样",
        "List connections with upload and download rates and totals. Supports filtering, sorting, and live sampling.",
        "connection list [--watch] [--sort download-rate]",
        0,
        0,
        "watch interval filter sort reverse limit offset"
    ),
    command!(
        "connection show",
        "查看完整连接详情，按需补充目标 IP",
        "Show connection details, resolving the destination IP when needed.",
        "connection show ID",
        1,
        1,
        ""
    ),
    command!(
        "connection close",
        "关闭一条或全部连接",
        "Close one or all connections.",
        "connection close [ID] [--all --yes]",
        0,
        1,
        "all"
    ),
    command!(
        "logs",
        "查看完整日志，支持等级、文本筛选与实时跟随",
        "View logs with level and text filters or follow live output.",
        "logs [--follow] [--level warning] [--filter TEXT]",
        0,
        0,
        "follow interval level filter limit"
    ),
    command!(
        "logs clear",
        "清空客户端有界日志缓冲区",
        "Clear the client log buffer.",
        "logs clear [--yes]",
        0,
        0,
        ""
    ),
    command!(
        "logs export",
        "导出当前最多 500 条缓冲日志；排障现场用 diagnose --output",
        "Export up to 500 buffered log entries. Use diagnose --output to capture diagnostics.",
        "logs export --output FILE",
        0,
        0,
        "output"
    ),
    command!(
        "config get",
        "读取设置；密钥默认脱敏",
        "Read settings. Secrets are redacted by default.",
        "config get [KEY]",
        0,
        1,
        ""
    ),
    command!(
        "config set",
        "校验并应用一个设置；网络模式使用 mode/proxy 命令",
        "Validate and apply a setting. Use mode or proxy to change network mode.",
        "config set KEY VALUE",
        2,
        2,
        ""
    ),
    command!(
        "config check",
        "用内核校验当前合并后的实际配置",
        "Use the core to validate the effective configuration.",
        "config check",
        0,
        0,
        ""
    ),
    command!(
        "config effective",
        "输出含覆盖规则的有效 YAML，默认移除凭据",
        "Print effective YAML with rule overrides. Credentials are redacted by default.",
        "config effective [--output FILE]",
        0,
        0,
        "output"
    ),
    command!(
        "diagnose",
        "检查网络；--output 保存诊断报告，--offline 仅收集本地现场",
        "Diagnose networking. Use --output to save a report and --offline to collect local data only.",
        "diagnose [--url URL] [--host DOMAIN] [--offline] [--output FILE] [--all-users]",
        0,
        0,
        "url host offline output all-users"
    ),
    command!(
        "dns",
        "查询内核或系统 DNS",
        "Look up addresses using core or system DNS.",
        "dns DOMAIN [--system]",
        1,
        1,
        "system"
    ),
    command!(
        "network check",
        "检查网址的 DNS、响应与本次实际代理线路",
        "Check a URL’s DNS, response, and the proxy route used.",
        "network check URL",
        1,
        1,
        ""
    ),
    command!(
        "network dns",
        "比较系统与内核 DNS",
        "Compare system DNS with the core’s DNS.",
        "network dns HOST [--type ADDR|A|AAAA|CNAME|MX|TXT|NS|SOA|PTR|CAA]",
        1,
        1,
        "type output"
    ),
    command!(
        "network phase",
        "检查 TCP、TLS 与首字节耗时",
        "Measure TCP, TLS, and time to first byte.",
        "network phase URL [--seconds N]",
        1,
        1,
        "seconds output"
    ),
    command!(
        "network family",
        "分别检查 IPv4 与 IPv6",
        "Check IPv4 and IPv6 separately.",
        "network family URL [--seconds N]",
        1,
        1,
        "seconds output"
    ),
    command!(
        "network explain",
        "已移除；请用 network check 查看规则与出口",
        "Removed. Use network check to see matched rules and outbound routes.",
        "network explain URL",
        1,
        1,
        "output"
    ),
    command!(
        "network tls",
        "检查 HTTPS 证书与信任链",
        "Inspect the HTTPS certificate and trust chain.",
        "network tls URL [--seconds N]",
        1,
        1,
        "seconds output"
    ),
    command!(
        "network stability",
        "检查短时请求稳定性",
        "Check whether repeated requests succeed over a short interval.",
        "network stability URL [--count N] [--interval-ms N] [--seconds N]",
        1,
        1,
        "count interval-ms seconds output"
    ),
    command!(
        "network udp",
        "经内核 UDP 中继检查 DNS 响应",
        "Check DNS responses over the core’s UDP relay.",
        "network udp DNS_SERVER [--seconds N]",
        1,
        1,
        "seconds output"
    ),
    command!(
        "network ping",
        "执行系统 ICMP 探针",
        "Run a system ICMP probe.",
        "network ping URL [--count N]",
        1,
        1,
        "count output"
    ),
    command!(
        "network trace",
        "执行系统逐跳探针",
        "Run a system traceroute.",
        "network trace URL [--max-hops N]",
        1,
        1,
        "max-hops output"
    ),
    command!(
        "network download",
        "限时限量下载并即时丢弃正文",
        "Run a time- and size-limited download and discard the response body.",
        "network download URL [--max-bytes N] [--seconds N]",
        1,
        1,
        "max-bytes seconds output"
    ),
    command!(
        "network tcp",
        "检查指定端口的 TCP 连接",
        "Check TCP connectivity to a specified port.",
        "network tcp HOST[:PORT] [--seconds N]",
        1,
        1,
        "seconds output"
    ),
    command!(
        "network http",
        "读取有界 HTTP 响应详情",
        "Inspect HTTP response details within the configured limits.",
        "network http URL [--method HEAD|GET|OPTIONS] [--seconds N]",
        1,
        1,
        "method seconds output"
    ),
    command!(
        "network rule",
        "已移除；请用 network check 查看实际命中",
        "Removed. Use network check to see the matched rule.",
        "network rule URL",
        1,
        1,
        "output"
    ),
    command!(
        "network config-diff",
        "已移除；请用 config get/check/effective",
        "Removed. Use config get, config check, or config effective.",
        "network config-diff",
        0,
        0,
        "output"
    ),
    command!(
        "network traffic",
        "已移除；请用 connection list 查看当前连接",
        "Removed. Use connection list to inspect current connections.",
        "network traffic",
        0,
        0,
        "output"
    ),
    command!(
        "network performance",
        "已移除；请用 status 或 diagnose",
        "Removed. Use status or diagnose.",
        "network performance",
        0,
        0,
        "output"
    ),
    command!(
        "network node",
        "已移除；节点信息位于代理页",
        "Removed. Node details are on the Proxies page.",
        "network node NODE",
        1,
        1,
        "output"
    ),
    command!(
        "ip-check list",
        "列出内置 IP/地区检测项目",
        "List the available IP and region checks.",
        "ip-check list [--category CATEGORY]",
        0,
        0,
        "category filter"
    ),
    command!(
        "ip-check",
        "执行全部、分类或单项检测并生成总结",
        "Run all checks, one category, or a single check and summarize the results.",
        "ip-check [ID] [--category CATEGORY] [--output FILE]",
        0,
        1,
        "category output"
    ),
    command!(
        "claude check",
        "检测本机网络和 Claude 环境的地区线索",
        "Check regional signals from the local network and Claude environment.",
        "claude check [--details] [--output FILE]",
        0,
        0,
        "details output"
    ),
    command!(
        "claude audit",
        "扫描实际终端环境与已记录 Claude 项目配置",
        "Inspect the current terminal environment and recorded Claude project settings.",
        "claude audit [--offline] [--output FILE]",
        0,
        0,
        "offline output"
    ),
    command!(
        "claude browser",
        "创建浏览器真实采样入口并等待报告",
        "Open a local browser probe and wait for its report.",
        "claude browser [--open] [--details] [--output FILE]",
        0,
        0,
        "open details output"
    ),
    command!(
        "claude protect",
        "查看或开关 Claude 保护，开启需要确认",
        "Show or toggle Claude protection. Enabling it requires confirmation.",
        "claude protect [on|off] [--yes]",
        0,
        1,
        ""
    ),
    command!(
        "claude watch",
        "实时查看异常连接及拦截记录",
        "Watch unusual connections and block events live.",
        "claude watch [--interval SECONDS]",
        0,
        0,
        "interval"
    ),
    command!(
        "geo status",
        "查看 Geo 数据版本",
        "Show Geo data versions.",
        "geo status",
        0,
        0,
        ""
    ),
    command!(
        "geo update",
        "校验并更新全部 Geo 数据",
        "Verify and update all Geo data.",
        "geo update",
        0,
        0,
        ""
    ),
    command!(
        "update check",
        "按已保存的更新通道检查客户端更新",
        "Check for client updates on the saved update channel.",
        "update check",
        0,
        0,
        ""
    ),
    command!(
        "update download",
        "下载并校验当前通道的更新安装包",
        "Download and verify an installer from the current update channel.",
        "update download --output FILE",
        0,
        0,
        "output"
    ),
    command!(
        "update status",
        "查看本账户最近一次更新安装的结果，不启动后台",
        "Show this account’s last update result without starting the backend.",
        "update status",
        0,
        0,
        ""
    ),
    command!(
        "update install",
        "下载安装更新；Linux 终端成功后启动，失败恢复原状态",
        "Download and install an update. On Linux, start the backend after success or restore its previous state on failure.",
        "update install [--yes] [--no-start]",
        0,
        0,
        "no-start"
    ),
    command!(
        "autostart",
        "读取或配置后台自启动",
        "Show or change backend autostart.",
        "autostart [on|off]",
        0,
        1,
        ""
    ),
    command!(
        "service install",
        "安装当前用户的无界面后台自启",
        "Install headless backend autostart for the current user.",
        "service install [--yes]",
        0,
        0,
        ""
    ),
    command!(
        "service remove",
        "删除当前用户的 CLI 后台自启",
        "Remove CLI backend autostart for the current user.",
        "service remove [--yes]",
        0,
        0,
        ""
    ),
    command!(
        "service status",
        "查看 CLI 后台自启配置",
        "Show the CLI backend autostart configuration.",
        "service status",
        0,
        0,
        ""
    ),
    command!(
        "notify",
        "调用与 Claude 拦截共用的系统通知入口",
        "Send a system notification through the Claude protection notification path.",
        "notify [MESSAGE]",
        0,
        1,
        ""
    ),
    command!(
        "completion",
        "生成 Bash、Zsh、Fish 或 PowerShell 上下文补全脚本",
        "Generate context-sensitive completion for Bash, Zsh, Fish, or PowerShell.",
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
            "{}",
            match self.language() {
                Language::SimplifiedChinese => format!("--{key} 不能超过 {max}"),
                Language::English => format!("--{key} cannot exceed {max}"),
            }
        );
        Ok(value)
    }
    pub fn interval(&self) -> Result<std::time::Duration> {
        let seconds = self.get("interval").unwrap_or("1").parse::<f64>()?;
        ensure!(
            seconds.is_finite() && (0.25..=3600.).contains(&seconds),
            "{}",
            self.language().text(
                "采样间隔应为 0.25–3600 秒",
                "The sampling interval must be 0.25–3600 seconds",
            )
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
            "{}",
            language.text(
                "终端界面需要交互终端；脚本请使用 status、node list 或 connection list",
                "The terminal interface requires an interactive terminal; scripts can use status, node list, or connection list",
            )
        );
    }
    ensure!(
        !args.flag("all-users") || (args.get("user").is_none() && args.get("data-dir").is_none()),
        "{}",
        language.text(
            "--all-users 不能与 --user 或 --data-dir 同时使用",
            "--all-users cannot be combined with --user or --data-dir",
        )
    );
    ensure!(
        !(args.flag("json") && args.flag("jsonl")),
        "{}",
        language.text(
            "--json 与 --jsonl 不能同时指定",
            "--json and --jsonl cannot be combined",
        )
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
                "{}",
                match language {
                    Language::SimplifiedChinese => format!("--{key} 必须大于 0"),
                    Language::English => format!("--{key} must be greater than 0"),
                }
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
                "{}",
                match language {
                    Language::SimplifiedChinese => format!("请提供 --{key} {placeholder}"),
                    Language::English => format!("Provide --{key} {placeholder}"),
                }
            );
        }
    }
    let enum_value = |value: &str, choices: &[&str]| -> Result<()> {
        ensure!(
            choices.contains(&value),
            "{}",
            match language {
                Language::SimplifiedChinese => {
                    format!("{} 的参数应为 {}", args.path, choices.join(" / "))
                }
                Language::English => {
                    format!("{} requires one of: {}", args.path, choices.join(" / "))
                }
            }
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
                "{}",
                language.text(
                    "--all-users 仅用于跨用户观察，不能同时切换代理",
                    "--all-users is read-only and cannot switch the proxy",
                )
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
            "{}",
            language.text("请提供 --name 或 --source", "Provide --name or --source")
        ),
        "profile update" => ensure!(
            !(args.flag("all") && !args.positional.is_empty()),
            "{}",
            language.text(
                "PROFILE 与 --all 不能同时指定",
                "PROFILE and --all cannot be combined"
            )
        ),
        "node test" => ensure!(
            usize::from(!args.positional.is_empty())
                + usize::from(args.get("group").is_some())
                + usize::from(args.flag("all"))
                == 1,
            "{}",
            language.text(
                "请选择 NODE、--group GROUP 或 --all 中的一项",
                "Choose one of NODE, --group GROUP, or --all"
            )
        ),
        "connection close" => ensure!(
            args.flag("all") != !args.positional.is_empty(),
            "{}",
            language.text("请选择 ID 或 --all", "Choose ID or --all")
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
                "{}",
                language.text("位置必须大于 0", "Position must be greater than 0")
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
        let value = value.strip_prefix("--").ok_or_else(|| match language {
            Language::SimplifiedChinese => anyhow::anyhow!("不支持组合短选项：{value}"),
            Language::English => anyhow::anyhow!("Combined short options are unsupported: {value}"),
        })?;
        let (key, supplied) = value
            .split_once('=')
            .map_or((value, None), |(k, v)| (k, Some(v.to_owned())));
        let needs_value = VALUES.split_whitespace().any(|v| v == key);
        let parameter = if needs_value {
            supplied
                .or_else(|| input.next())
                .ok_or_else(|| match language {
                    Language::SimplifiedChinese => anyhow::anyhow!("--{key} 缺少参数"),
                    Language::English => anyhow::anyhow!("--{key} requires a value"),
                })?
        } else {
            ensure!(
                supplied.is_none(),
                "{}",
                match language {
                    Language::SimplifiedChinese => format!("--{key} 不接受参数"),
                    Language::English => format!("--{key} does not take a value"),
                }
            );
            "true".into()
        };
        ensure!(
            key != "lang" || LanguagePreference::from_tag(&parameter).is_some(),
            "--lang: system / zh-CN / en"
        );
        ensure!(
            options.insert(key.to_owned(), parameter).is_none(),
            "{}",
            match language {
                Language::SimplifiedChinese => format!("--{key} 重复指定"),
                Language::English => format!("--{key} was specified more than once"),
            }
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
                "{}",
                language.text(
                    "分组帮助不接受命令专属选项",
                    "Group help does not accept command-specific options"
                )
            );
            return Ok(Args {
                path: prefix,
                positional: vec![],
                options,
                help: true,
            });
        }
        bail!(
            "{}",
            match language {
                Language::SimplifiedChinese => {
                    format!("未知命令：{}；运行 cor help 查看命令", words.join(" "))
                }
                Language::English => {
                    format!("Unknown command: {}; run cor help", words.join(" "))
                }
            }
        );
    };
    let positional = words[spec.path.split_whitespace().count()..].to_vec();
    for key in options.keys() {
        ensure!(
            GLOBAL
                .split_whitespace()
                .chain(spec.options.split_whitespace())
                .any(|allowed| allowed == key),
            "{}",
            match language {
                Language::SimplifiedChinese => format!("{} 不支持 --{key}", spec.path),
                Language::English => format!("{} does not support --{key}", spec.path),
            }
        );
    }
    if !help {
        ensure!(
            (spec.min..=spec.max).contains(&positional.len()),
            "{}cor {}",
            language.text("用法：", "Usage: "),
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
    let usage = language.text("用法：", "Usage: ");
    let command = language.text("命令", "command");
    let options = language.text("选项", "options");
    let mut out = format!("Clash of Rust CLI {}\n\n", clash_client_core::VERSION);
    if prefix.is_empty() {
        let common = language.text("常用命令：", "Common commands:");
        out += &format!("{usage}cor <{command}> [{options}]\n\n{common}\n");
        for path in [
            "ui", "start", "stop", "restart", "status", "proxy", "diagnose",
        ] {
            let spec = COMMANDS.iter().find(|spec| spec.path == path).unwrap();
            out += &format!(
                "  {:<25} {}\n",
                spec.path,
                spec.localized_description(language)
            );
        }
        out += language.text(
            "  profile                   订阅添加、切换与更新；cor profile help\n  node                      节点选择与测速；cor node help\n",
            "  profile                   Add, switch, and update subscriptions. See cor profile help.\n  node                      Pick and test nodes. See cor node help.\n",
        );
        let advanced = COMMANDS
            .iter()
            .map(|spec| spec.path.split_whitespace().next().unwrap())
            .filter(|path| {
                ![
                    "ui", "start", "stop", "restart", "status", "proxy", "diagnose", "profile",
                    "node",
                ]
                .contains(path)
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(language.text("、", ", "));
        let other = language.text("其他功能：", "Other commands: ");
        let full = language.text("完整命令与通用选项：", "All commands and global options: ");
        out += &format!("\n{other}{advanced}\n{full}cor help all\n");
    } else if prefix == "all" {
        let all = language.text("全部命令：", "All commands:");
        out += &format!("{usage}cor <{command}> [{options}]\n\n{all}\n");
    } else if let Some(spec) = COMMANDS.iter().find(|s| s.path == prefix) {
        out += &format!(
            "{}\n\n{usage}cor {}\n\n",
            spec.localized_description(language),
            spec.usage
        );
        if !spec.options.is_empty() {
            out += language.text("命令选项：\n", "Command options:\n");
            for key in spec.options.split_whitespace() {
                out += &option_help(key, language);
            }
            out += "\n";
        }
    } else {
        let heading = language.text("命令：", "Commands:");
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
        "profile add" => Some(language.text(
            "cor profile add 我的订阅 'https://example.com/subscription'",
            "cor profile add 'My subscription' 'https://example.com/subscription'",
        )),
        "profile import" => Some(language.text(
            "cat profile.yaml | cor profile import - --name 本地订阅",
            "cat profile.yaml | cor profile import - --name 'Local subscription'",
        )),
        "node list" => Some("cor node list Proxy --sort latency --limit 50"),
        "node select" => Some(language.text(
            "cor node select Proxy '节点 1'",
            "cor node select Proxy 'Node 1'",
        )),
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
        out += language.text("\n示例：\n", "\nExamples:\n");
        out += &format!("  {example}\n");
    }
    out += language.text("\n通用选项：\n", "\nGlobal options:\n");
    for key in (if prefix.is_empty() {
        "help version json data-dir user lang"
    } else {
        GLOBAL
    })
    .split_whitespace()
    {
        out += &option_help(key, language);
    }
    out += language.text(
        "\n详情：cor help <命令>；使用文档：docs/cli.md\n退出码：0 成功，1 操作失败，2 参数错误，3 后台不可达，124 请求超时，130 已取消。\n",
        "\nRun cor help <command> for details. See docs/cli.md for the CLI guide.\nExit codes: 0 success, 1 operation failed, 2 invalid arguments, 3 backend unavailable, 124 timed out, 130 cancelled.\n",
    );
    out
}
fn option_help(key: &str, language: Language) -> String {
    let chinese = match key {
        "help" => "显示当前命令及参数帮助（-h）",
        "lang" => "本次帮助或终端界面的语言：system、zh-CN、en",
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
        "demo" => "使用虚构数据预览终端界面，不连接后台或改变网络",
        "no-core" => "仅运行后台，暂不启动内核",
        "no-start" => "Linux 终端更新成功后保留原内核开关状态",
        "network-only" => "刷新当前内核出站和 DNS，关闭旧连接；不重启内核",
        "watch" | "follow" => "持续输出新采样/日志，Ctrl+C 结束",
        "interval" => "采样秒数，默认 1，最小 0.25",
        "count" => "检查次数，1～20，默认 5",
        "interval-ms" => "请求间隔毫秒，100～5000，默认 1000",
        "seconds" => "单次时限秒数，1～30，默认 10",
        "max-bytes" => "下载字节上限，1024～16777216，默认 8388608",
        "max-hops" => "追踪跳数上限，1～16，默认 8",
        "type" => "DNS 记录类型，默认 ADDR（A 与 AAAA）",
        "method" => "HTTP 方法 HEAD、GET 或 OPTIONS，默认 HEAD",
        "filter" => "按文本筛选",
        "sort" => {
            "排序字段；node: name/latency；connection: name/rule/chain/upload/download/upload-rate/download-rate/start"
        }
        "reverse" => "反转排序方向（速率/流量默认降序，名称默认升序）",
        "limit" => "最多输出条数，默认 100，最大 10000",
        "offset" => "跳过的条数，默认 0",
        "details" => "显示完整检测项目、说明和来源",
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
        "system" => "使用本机系统 DNS，默认使用内核 DNS",
        "category" => "检测分类，例如 AI、全球、日本",
        "offline" => "仅收集本机信息，不请求外部检测服务",
        "open" => "在默认浏览器打开本机采样入口",
        _ => "见当前命令说明",
    };
    let english = match key {
        "help" => "Show help for this command and its options (-h).",
        "lang" => "Choose the language for help or the terminal interface: system, zh-CN, or en.",
        "version" => "Show the client version (-V).",
        "json" => "Output machine-readable JSON only.",
        "jsonl" => "Output one JSON record per line, including live samples.",
        "timeout" => "Backend request timeout in seconds (default: 300).",
        "data-dir" => "Use a separate data directory.",
        "resources" => "Use a directory containing the core and Geo resources.",
        "user" => "On Linux, administer another user's settings and backend as that user.",
        "all-users" => "On Linux, let root view subscriptions or TUN processes across users.",
        "yes" => "Confirm removal, protection, installation, or similar operations (-y).",
        "show-secrets" => "Include full credentials in subscription and configuration output.",
        "quiet" => "Hide progress messages (-q).",
        "no-color" => "Use plain text. CLI output is plain by default.",
        "demo" => "Preview the terminal interface with mock data and no network changes.",
        "no-core" => "Start the backend without starting the core.",
        "no-start" => "Preserve the core's previous running state after a Linux update.",
        "network-only" => {
            "Refresh outbound routes and DNS and close old connections without restarting the core."
        }
        "watch" | "follow" => "Stream new samples or log entries until Ctrl+C.",
        "interval" => "Sampling interval in seconds (default: 1, minimum: 0.25).",
        "count" => "Number of checks (1–20, default: 5).",
        "interval-ms" => "Request interval in milliseconds (100–5000, default: 1000).",
        "seconds" => "Time limit per request in seconds (1–30, default: 10).",
        "max-bytes" => "Download limit in bytes (1024–16777216, default: 8388608).",
        "max-hops" => "Maximum traceroute hops (1–16, default: 8).",
        "type" => "DNS record type (default: ADDR, which checks A and AAAA).",
        "method" => "HTTP method: HEAD, GET, or OPTIONS (default: HEAD).",
        "filter" => "Filter by text.",
        "sort" => {
            "Sort by name or latency for nodes. Connections also support rule, chain, upload, download, upload-rate, download-rate, and start."
        }
        "reverse" => {
            "Reverse the sort order. Names ascend by default and rates and totals descend."
        }
        "limit" => "Maximum number of results (default: 100, maximum: 10000).",
        "offset" => "Number of results to skip (default: 0).",
        "details" => "Show all checks, explanations, and sources.",
        "output" => {
            "Write to a file (-o). Full logs and configurations are private to the current user."
        }
        "name" => "Subscription display name.",
        "source" => "Subscription URL or local YAML path.",
        "all" => "Apply to all items.",
        "group" => "Test only the specified proxy group.",
        "profile" => "Select a subscription by unique name or ID (default: active subscription).",
        "position" => "Rule position starting at 1 (default: first).",
        "disabled" => "Create the rule disabled.",
        "level" => "Log level: debug, info, warning, error, or silent.",
        "shell" => "Environment syntax: bash, zsh, fish, powershell, or cmd.",
        "unset" => "Print shell commands to clear proxy environment variables.",
        "url" => "HTTP or HTTPS diagnostics URL.",
        "host" => "Diagnostics hostname (default: github.com).",
        "system" => "Use system DNS. The default is core DNS.",
        "category" => "Check category, such as AI, 全球, or 日本.",
        "offline" => "Collect local information without contacting external check services.",
        "open" => "Open the local probe in the default browser.",
        _ => "See the command description.",
    };
    format!(
        "  --{:<20} {}{}\n",
        key,
        if VALUES.split_whitespace().any(|v| v == key) {
            language.text("<值>  ", "<value>  ")
        } else {
            ""
        },
        language.text(chinese, english)
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
        "language" => &["system", "zh-CN", "en"],
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
        "lang" => &["system", "zh-CN", "en"],
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
