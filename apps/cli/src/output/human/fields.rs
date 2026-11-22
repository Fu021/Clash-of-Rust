use chrono::{DateTime, Local, Utc};
use clash_client_core::config::Language;
use serde_json::Value;

/// Escape terminal control characters without changing stored names or JSON.
pub(super) fn clean(text: &str) -> String {
    text.chars().fold(String::new(), |mut out, ch| {
        if ch.is_control() {
            out.extend(ch.escape_default());
        } else {
            out.push(ch);
        }
        out
    })
}

pub(super) fn bytes(value: u64) -> String {
    let mut number = value as f64;
    let mut unit = "B";
    for next in ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"] {
        if number < 1024.0 {
            break;
        }
        number /= 1024.0;
        unit = next;
    }
    if unit == "B" {
        format!("{value} B")
    } else {
        format!("{number:.1} {unit}")
    }
}

pub(super) fn latency(value: &Value) -> String {
    match value.as_u64() {
        Some(0) => "超时".into(),
        Some(n) => format!("{n} ms"),
        None => "未测速".into(),
    }
}

pub(super) fn text(row: &Value, key: &str) -> String {
    scalar(key, &row[key])
}

pub(super) fn text_in(row: &Value, key: &str, language: Language) -> String {
    scalar_in(key, &row[key], language)
}

pub(super) fn scalar(key: &str, value: &Value) -> String {
    if key == "latency_ms" {
        return latency(value);
    }
    match value {
        Value::Null => {
            if matches!(key, "upload_rate" | "download_rate") {
                "尚未采样".into()
            } else {
                "—".into()
            }
        }
        Value::Bool(value) => match key {
            "running" => {
                if *value {
                    "运行中"
                } else {
                    "未运行"
                }
            }
            "enabled" | "protected" | "dns_override" | "ipv6" | "udp" | "tray_speed" | "dark" => {
                if *value {
                    "已开启"
                } else {
                    "已关闭"
                }
            }
            _ => {
                if *value {
                    "是"
                } else {
                    "否"
                }
            }
        }
        .into(),
        Value::Number(n) => {
            if let Some(n) = n.as_u64() {
                match key {
                    "upload" | "download" | "size" | "upload_total" | "download_total" => bytes(n),
                    "upload_rate" | "download_rate" => format!("{}/s", bytes(n)),
                    "millis" => format!("{n} ms"),
                    "updated" | "installed_at" if n == 0 => "尚未更新".into(),
                    "updated" | "installed_at" => i64::try_from(n)
                        .ok()
                        .and_then(|n| DateTime::<Utc>::from_timestamp(n, 0))
                        .map(|n| {
                            n.with_timezone(&Local)
                                .format("%Y-%m-%d %H:%M:%S")
                                .to_string()
                        })
                        .unwrap_or_else(|| n.to_string()),
                    "delay_interval_minutes"
                    | "subscription_update_interval_minutes"
                    | "update_check_interval_minutes"
                        if n == 0 =>
                    {
                        "已关闭".into()
                    }
                    "delay_interval_minutes"
                    | "subscription_update_interval_minutes"
                    | "update_check_interval_minutes" => {
                        format!("{n} 分钟")
                    }
                    "text_scale" => format!("{n}%"),
                    _ => n.to_string(),
                }
            } else {
                n.to_string()
            }
        }
        Value::String(s) if s.is_empty() => "—".into(),
        Value::String(s) => {
            if key == "country"
                && let Some(name) = clash_client_core::probe::country_name(s)
            {
                return clean(&format!("{name}（{s}）"));
            }
            let name = match (key, s.as_str()) {
                ("mode" | "run_mode", "rule") => "规则",
                ("mode" | "run_mode", "global") => "全局",
                ("mode" | "run_mode", "direct") => "直连",
                ("proxy" | "proxy_mode", "off") => "仅本地代理端口",
                ("proxy" | "proxy_mode", "system") => "系统代理",
                ("proxy" | "proxy_mode", "tun") => "TUN",
                ("owner", "gui") => "桌面程序",
                ("owner", "daemon") => "CLI 后台",
                ("source", "Custom") => "自定义",
                ("source", "Shortcut") => "快捷规则",
                ("source", "Subscription") => "订阅",
                ("source", "system") => "系统 DNS",
                ("source", "core/proxy") => "内核 / 代理 DNS",
                ("ip_source", "resolver") => "DNS 解析补充",
                ("theme_mode", "system") => "跟随系统",
                ("theme_mode", "dark") => "深色",
                ("theme_mode", "light") => "浅色",
                ("language", "system") => "跟随系统",
                ("language", "zh-CN") => "简体中文",
                ("language", "en") => "English",
                ("accent_color", "teal") => "青绿",
                ("accent_color", "blue") => "蓝色",
                ("accent_color", "purple") => "紫色",
                ("accent_color", "rose") => "玫瑰",
                ("accent_color", "amber") => "琥珀",
                ("accent_color", "slate") => "石板",
                ("node_sort" | "tui_node_sort", "name") => "名称排序",
                ("node_sort" | "tui_node_sort", "latency_ascending") => "延迟从低到高",
                ("node_sort" | "tui_node_sort", "latency_descending") => "延迟从高到低",
                ("evidence", "measured") => "实测",
                ("evidence", "inferred") => "辅助线索",
                ("evidence", "unknown") => "尚未确认",
                ("state", "Confirmed") => "确认可用",
                ("state", "Partial") => "部分可用",
                ("state", "Reachable") => "网页可达",
                ("state", "Identified") => "已识别信息",
                ("state", "Restricted") => "不可用或受限",
                ("state", "Unknown") => "结果未确认",
                ("state", "Failed") => "检测失败",
                ("state", "Off") => "保护未启用",
                ("state", "Active") => "接管与拦截就绪",
                ("state", "Checking") => "保护待确认",
                ("state", "Error") => "保护异常",
                ("level", "issue") => "配置问题",
                ("level", "review") => "需核对",
                ("level", "incomplete") => "未完成检查",
                ("scope", "current-user-logon") => "当前用户登录时启动",
                ("effective", "next-login") => "下次登录生效",
                _ => s,
            };
            if matches!(
                key,
                "theme_mode" | "language" | "accent_color" | "node_sort" | "tui_node_sort"
            ) && name != s
            {
                return clean(&format!("{name}（{s}）"));
            }
            if matches!(key, "start" | "generated_at")
                && let Ok(time) = DateTime::parse_from_rfc3339(s)
            {
                return time
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string();
            }
            clean(name)
        }
        _ => clean(&value.to_string()),
    }
}

pub(super) fn label(key: &str) -> &str {
    match key {
        "id" => "ID",
        "name" => "名称",
        "type" => "类型",
        "running" => "内核",
        "owner" => "控制方",
        "version" => "版本",
        "profile" => "订阅",
        "node" => "当前节点",
        "mode" | "run_mode" => "运行模式",
        "proxy" | "proxy_mode" => "代理方式",
        "protected" | "claude_protection" | "protection" => "Claude 保护",
        "connections" => "连接数",
        "upload_rate" => "上传速度",
        "download_rate" => "下载速度",
        "upload" => "上传量",
        "download" => "下载量",
        "source" => "来源",
        "updated" => "更新时间",
        "active" => "正在使用",
        "usage" => "订阅用量",
        "position" => "序号",
        "rule" => "匹配规则",
        "rule_payload" => "匹配内容",
        "enabled" => "启用状态",
        "duplicate" => "重复规则",
        "already_in_subscription" => "订阅已包含",
        "host" => "域名",
        "sniff_host" => "嗅探域名",
        "ip" => "目标 IP",
        "port" => "目标端口",
        "network" => "协议",
        "chain" => "代理链",
        "start" => "开始时间",
        "ip_source" => "IP 来源",
        "resolution_error" => "IP 查询失败原因",
        "controller_port" => "控制端口",
        "mixed_port" => "混合代理端口",
        "secret" => "控制接口密钥",
        "dark" => "深色外观",
        "theme_mode" => "主题",
        "language" => "语言",
        "accent_color" => "强调色",
        "text_scale" => "字号",
        "tray_speed" => "托盘网速",
        "dns_override" => "DNS 覆盖",
        "ipv6" => "IPv6",
        "udp" => "UDP",
        "delay_interval_minutes" => "自动测速间隔",
        "subscription_update_interval_minutes" => "订阅自动更新间隔",
        "update_check_interval_minutes" => "自动检查更新间隔",
        "active_profile" => "当前订阅 ID",
        "node_sort" => "节点排序",
        "tui_node_sort" => "终端节点排序",
        "rule_overrides" => "快捷规则",
        "rule_profiles" => "各订阅规则设置",
        "application_rules" => "各订阅应用出口规则",
        "path" => "文件位置",
        "url" => "链接",
        "size" => "大小",
        "sha256" => "SHA256",
        "available" => "发现新版本",
        "package" => "安装包",
        "installed" => "已配置自启",
        "matches_current" => "与当前配置一致",
        "scope" => "生效范围",
        "effective" => "生效时间",
        "source_profile_id" | "profile_id" => "订阅 ID",
        "source_group" => "锁定策略组",
        "target" => "锁定节点",
        "system_dns" => "系统 DNS",
        "core_dns" => "内核 DNS",
        "answers" => "解析结果",
        "http" => "HTTP 连通性",
        "ipv4" => "IPv4 出口",
        "error" => "错误",
        "status" => "HTTP 状态",
        "millis" => "耗时",
        "detail" => "详情",
        "category" => "分类",
        "state" => "检测状态",
        "summary" => "检测结果",
        "country" => "地区",
        "total" => "总数",
        "completed" => "已完成",
        "non_red" => "可用或部分可用",
        "confirmed_available" => "确认可用",
        "counts" => "状态统计",
        "categories" => "分类统计",
        "results" => "检测结果",
        "normal" => "正常项目",
        "risk" => "风险",
        "challenged" => "需要浏览器验证",
        "clues" => "线索",
        "report" => "检测报告",
        "terminal" => "终端环境",
        "projects" => "项目配置",
        "environment" => "检测环境",
        "generated_at" => "检测时间",
        "context" => "检测说明",
        "signals" => "检测线索",
        "routes" => "相关连接",
        "group" => "分类",
        "title" => "项目",
        "value" => "结果",
        "evidence" => "依据",
        "advice" => "建议",
        "time" => "时间",
        "outcome" => "处理结果",
        "level" => "级别",
        "field" => "配置项",
        "reason" => "原因",
        "files" => "配置文件数",
        "missing_projects" => "缺失项目数",
        "omitted" => "已省略",
        "incomplete" => "检查未完成",
        "findings" => "待处理项目",
        "clean_count" => "未发现风险的文件数",
        "clean_files" => "未发现风险的文件",
        "issues" => "配置问题数",
        "reviews" => "需核对项目数",
        "unreadable" => "无法读取的文件数",
        "guard" => "实时保护",
        "protection_status" => "保护接管状态",
        "observed" => "请求监测",
        "connected" => "监测已连接",
        "failures" => "失败数",
        "normal_closed" => "已关闭异常连接",
        "notifications" => "系统通知",
        "last_error" => "最近错误",
        "Ok" => "成功",
        "Err" => "失败",
        "Confirmed" => "确认可用",
        "Partial" => "部分可用",
        "Reachable" => "网页可达",
        "Identified" => "已识别信息",
        "Restricted" => "不可用或受限",
        "Unknown" => "结果未确认",
        "Failed" => "检测失败",
        _ => key,
    }
}

pub(super) fn label_in(key: &str, language: Language) -> &str {
    if language == Language::SimplifiedChinese {
        return label(key);
    }
    match key {
        "id" => "ID",
        "name" => "Name",
        "type" => "Type",
        "running" => "Core",
        "owner" => "Owner",
        "version" => "Version",
        "profile" => "Subscription",
        "node" => "Current node",
        "mode" | "run_mode" => "Routing mode",
        "proxy" | "proxy_mode" => "Proxy mode",
        "protected" | "claude_protection" | "protection" => "Claude protection",
        "connections" => "Connections",
        "upload_rate" => "Upload speed",
        "download_rate" => "Download speed",
        "upload" => "Uploaded",
        "download" => "Downloaded",
        "source" => "Source",
        "updated" => "Updated",
        "active" => "Active",
        "usage" => "Usage",
        "position" => "Position",
        "rule" => "Rule",
        "rule_payload" => "Rule value",
        "enabled" => "Enabled",
        "duplicate" => "Duplicate",
        "already_in_subscription" => "In subscription",
        "host" => "Host",
        "sniff_host" => "Sniffed host",
        "ip" => "IP",
        "port" => "Port",
        "network" => "Network",
        "chain" => "Route",
        "start" => "Started",
        "ip_source" => "IP source",
        "resolution_error" => "Resolution error",
        "controller_port" => "Controller port",
        "mixed_port" => "Mixed port",
        "secret" => "Controller secret",
        "dark" => "Dark appearance",
        "theme_mode" => "Theme",
        "language" => "Language",
        "accent_color" => "Accent color",
        "text_scale" => "Text scale",
        "tray_speed" => "Tray speed",
        "dns_override" => "DNS override",
        "ipv6" => "IPv6",
        "udp" => "UDP",
        "delay_interval_minutes" => "Automatic speed test interval",
        "subscription_update_interval_minutes" => "Subscription update interval",
        "update_check_interval_minutes" => "Update check interval",
        "active_profile" => "Active subscription ID",
        "node_sort" => "Node sort order",
        "tui_node_sort" => "Terminal node sort order",
        "rule_overrides" => "Rule shortcuts",
        "rule_profiles" => "Subscription rule settings",
        "application_rules" => "Application routing rules",
        "path" => "Path",
        "url" => "URL",
        "size" => "Size",
        "sha256" => "SHA256",
        "available" => "Update available",
        "package" => "Package",
        "installed" => "Installed",
        "matches_current" => "Matches current configuration",
        "scope" => "Scope",
        "effective" => "Effective",
        "source_profile_id" | "profile_id" => "Subscription ID",
        "source_group" => "Pinned group",
        "target" => "Target",
        "system_dns" => "System DNS",
        "core_dns" => "Core DNS",
        "answers" => "Answers",
        "http" => "HTTP connectivity",
        "ipv4" => "IPv4 exit",
        "error" => "Error",
        "status" => "Status",
        "millis" => "Duration",
        "detail" => "Details",
        "category" => "Category",
        "state" => "State",
        "summary" => "Result",
        "country" => "Region",
        "total" => "Total",
        "completed" => "Completed",
        "non_red" => "Available or partially available",
        "confirmed_available" => "Confirmed available",
        "counts" => "Counts",
        "categories" => "Category counts",
        "results" => "Results",
        "normal" => "Normal",
        "risk" => "Risks",
        "challenged" => "Browser verification required",
        "clues" => "Observations",
        "report" => "Report",
        "terminal" => "Terminal environment",
        "projects" => "Project configuration",
        "environment" => "Environment",
        "generated_at" => "Generated",
        "context" => "Context",
        "signals" => "Observations",
        "routes" => "Connections",
        "group" => "Category",
        "title" => "Item",
        "value" => "Result",
        "evidence" => "Evidence",
        "advice" => "Advice",
        "time" => "Time",
        "outcome" => "Outcome",
        "level" => "Level",
        "field" => "Field",
        "reason" => "Reason",
        "files" => "Configuration files",
        "missing_projects" => "Missing projects",
        "omitted" => "Omitted",
        "incomplete" => "Incomplete",
        "findings" => "Findings",
        "clean_count" => "Files without findings",
        "clean_files" => "Files without findings",
        "issues" => "Issues",
        "reviews" => "Items to review",
        "unreadable" => "Unreadable files",
        "guard" => "Live protection",
        "protection_status" => "Protection status",
        "observed" => "Requests observed",
        "connected" => "Connected",
        "failures" => "Failures",
        "normal_closed" => "Unexpected connections closed",
        "notifications" => "Notifications",
        "last_error" => "Last error",
        "Ok" => "Success",
        "Err" => "Failure",
        "Confirmed" => "Confirmed available",
        "Partial" => "Partially available",
        "Reachable" => "Website reachable",
        "Identified" => "Identified",
        "Restricted" => "Unavailable or restricted",
        "Unknown" => "Unconfirmed",
        "Failed" => "Failed",
        _ => key,
    }
}

pub(super) fn scalar_in(key: &str, value: &Value, language: Language) -> String {
    if language == Language::SimplifiedChinese {
        return scalar(key, value);
    }
    match value {
        Value::Null if matches!(key, "upload_rate" | "download_rate") => "Not sampled".into(),
        Value::Bool(value) => match (key, *value) {
            ("running", true) => "Running",
            ("running", false) => "Stopped",
            (
                "enabled" | "protected" | "dns_override" | "ipv6" | "udp" | "tray_speed" | "dark",
                true,
            ) => "On",
            (
                "enabled" | "protected" | "dns_override" | "ipv6" | "udp" | "tray_speed" | "dark",
                false,
            ) => "Off",
            (_, true) => "Yes",
            (_, false) => "No",
        }
        .into(),
        Value::Number(number) if key == "latency_ms" && number.as_u64() == Some(0) => {
            "Timed out".into()
        }
        Value::Number(number)
            if matches!(key, "updated" | "installed_at") && number.as_u64() == Some(0) =>
        {
            "Not updated".into()
        }
        Value::Number(number)
            if matches!(
                key,
                "delay_interval_minutes"
                    | "subscription_update_interval_minutes"
                    | "update_check_interval_minutes"
            ) =>
        {
            match number.as_u64() {
                Some(0) => "Off".into(),
                Some(minutes) => format!("{minutes} min"),
                None => number.to_string(),
            }
        }
        Value::String(value) if !value.is_empty() => {
            if key == "country"
                && let Some(name) = clash_client_core::countries::english_name(value)
            {
                return clean(&format!("{name} ({value})"));
            }
            let translated = match (key, value.as_str()) {
                ("evidence", "measured") => "Measured",
                ("evidence", "inferred") => "Inferred",
                ("evidence", "unknown") => "Unconfirmed",
                ("environment", "core") => "Core",
                ("environment", "web") => "Browser",
                ("environment", "desktop") => "Desktop",
                ("environment", "code") => "Claude Code",
                ("group", "exit") => "Network exit",
                ("group", "browser") => "Browser preferences",
                ("group", "web-rtc") => "WebRTC",
                ("group", "dns") => "DNS",
                ("group", "desktop") => "Claude Desktop",
                ("group", "code") => "Claude Code",
                ("group", "quality") => "IP quality",
                ("group", "cloudflare") => "Cloudflare",
                ("group", "connectivity") => "Connectivity",
                ("mode" | "run_mode", "rule") => "Rule",
                ("mode" | "run_mode", "global") => "Global",
                ("mode" | "run_mode", "direct") => "Direct",
                ("proxy" | "proxy_mode", "off") => "Local port only",
                ("proxy" | "proxy_mode", "system") => "System proxy",
                ("proxy" | "proxy_mode", "tun") => "TUN",
                ("owner", "gui") => "Desktop",
                ("owner", "daemon") => "CLI background",
                ("source", "Custom") => "Custom",
                ("source", "Shortcut") => "Rule shortcut",
                ("source", "Subscription") => "Subscription",
                ("source", "system") => "System DNS",
                ("source", "core/proxy") => "Core / proxy DNS",
                ("ip_source", "resolver") => "Additional DNS lookup",
                ("theme_mode", "system") => "Follow system",
                ("theme_mode", "dark") => "Dark",
                ("theme_mode", "light") => "Light",
                ("language", "system") => "Follow system",
                ("language", "zh-CN") => "Simplified Chinese",
                ("language", "en") => "English",
                ("accent_color", "teal") => "Teal",
                ("accent_color", "blue") => "Blue",
                ("accent_color", "purple") => "Purple",
                ("accent_color", "rose") => "Rose",
                ("accent_color", "amber") => "Amber",
                ("accent_color", "slate") => "Slate",
                ("node_sort" | "tui_node_sort", "name") => "Name",
                ("node_sort" | "tui_node_sort", "latency_ascending") => "Latency ascending",
                ("node_sort" | "tui_node_sort", "latency_descending") => "Latency descending",
                ("state", "Confirmed") => "Confirmed available",
                ("state", "Partial") => "Partially available",
                ("state", "Reachable") => "Website reachable",
                ("state", "Identified") => "Identified",
                ("state", "Restricted") => "Unavailable or restricted",
                ("state", "Unknown") => "Unconfirmed",
                ("state", "Failed") => "Failed",
                ("state", "Off") => "Protection off",
                ("state", "Active") => "Protection ready",
                ("state", "Checking") => "Protection unconfirmed",
                ("state", "Error") => "Protection error",
                ("level", "issue") => "Configuration issue",
                ("level", "review") => "Review",
                ("level", "incomplete") => "Incomplete",
                ("scope", "current-user-logon") => "At current user login",
                ("effective", "next-login") => "At next login",
                _ => value,
            };
            if matches!(
                key,
                "theme_mode" | "language" | "accent_color" | "node_sort" | "tui_node_sort"
            ) && translated != value
            {
                return clean(&format!("{translated} ({value})"));
            }
            if matches!(key, "start" | "generated_at")
                && let Ok(time) = DateTime::parse_from_rfc3339(value)
            {
                return time
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string();
            }
            clean(translated)
        }
        Value::Null if key == "latency_ms" => "Not tested".into(),
        _ => scalar(key, value),
    }
}
