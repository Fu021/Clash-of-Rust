use chrono::{DateTime, Local, Utc};
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
                    "updated" if n == 0 => "尚未更新".into(),
                    "updated" => i64::try_from(n)
                        .ok()
                        .and_then(|n| DateTime::<Utc>::from_timestamp(n, 0))
                        .map(|n| {
                            n.with_timezone(&Local)
                                .format("%Y-%m-%d %H:%M:%S")
                                .to_string()
                        })
                        .unwrap_or_else(|| n.to_string()),
                    "delay_interval_minutes" if n == 0 => "已关闭".into(),
                    "delay_interval_minutes" => format!("{n} 分钟"),
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
                ("accent_color", "teal") => "青绿",
                ("accent_color", "blue") => "蓝色",
                ("accent_color", "purple") => "紫色",
                ("accent_color", "rose") => "玫瑰",
                ("accent_color", "amber") => "琥珀",
                ("accent_color", "slate") => "石板",
                ("node_sort", "name") => "名称排序",
                ("node_sort", "latency_ascending") => "延迟从低到高",
                ("node_sort", "latency_descending") => "延迟从高到低",
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
                ("level", "issue") => "配置问题",
                ("level", "review") => "需核对",
                ("level", "incomplete") => "未完成检查",
                ("scope", "current-user-logon") => "当前用户登录时启动",
                ("effective", "next-login") => "下次登录生效",
                _ => s,
            };
            if matches!(key, "theme_mode" | "accent_color" | "node_sort") && name != s {
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
        "accent_color" => "强调色",
        "text_scale" => "字号",
        "tray_speed" => "托盘网速",
        "dns_override" => "DNS 覆盖",
        "ipv6" => "IPv6",
        "udp" => "UDP",
        "ip_reputation_key" => "IP 信誉服务密钥",
        "delay_interval_minutes" => "自动测速间隔",
        "active_profile" => "当前订阅 ID",
        "node_sort" => "节点排序",
        "rule_overrides" => "快捷规则",
        "rule_profiles" => "各订阅规则设置",
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
        "normal" => "实测项目",
        "risk" => "风险",
        "challenged" => "需要浏览器验证",
        "clues" => "其他线索",
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
