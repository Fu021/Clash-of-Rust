use super::{Group, Signal, SignalKind};
use crate::config::Language;

pub(super) fn title(value: &str, language: Language) -> String {
    if language == Language::SimplifiedChinese {
        return value.to_owned();
    }
    let translated = match value {
        "网络出口与分流" => "Network exit and routing",
        "浏览器地区特征" => "Browser region preferences",
        "DNS 地区" => "DNS region",
        "IP 归属与信誉" => "IP ownership and reputation",
        "连通性" => "Connectivity",
        "实时保护与异常拦截" => "Live protection and blocked connections",
        "Claude 连接与选路" => "Claude connections and routing",
        "出口与 IP 属性" => "Exit and IP properties",
        "DNS 与 WebRTC" => "DNS and WebRTC",
        "浏览器与配置" => "Browser and configuration",
        "Claude 域名选路" => "Claude domain routes",
        "出口 IP 属性" => "Exit IP properties",
        "IP 注册信息" => "IP registration",
        "全球连通性" => "Global connectivity",
        "Claude 官方支持地区" => "Claude supported region",
        "Cloudflare 网络环境" => "Cloudflare network",
        "IPv4 出口" => "IPv4 exit",
        "IPv6 出口" => "IPv6 exit",
        "浏览器 IPv4 出口" => "Browser IPv4 exit",
        "浏览器 IPv6 出口" => "Browser IPv6 exit",
        "Claude 网页实际出口" => "Claude website exit",
        "浏览器时区" => "Browser time zone",
        "系统时区" => "System time zone",
        "时区" => "Time zone",
        "UTC 偏移" => "UTC offset",
        "语言偏好" => "Language preferences",
        "系统语言" => "System language",
        "语言与地区格式" => "Language and regional format",
        "地区格式" => "Regional format",
        "Accept-Language 请求头" => "Accept-Language header",
        "平台与浏览器" => "Platform and browser",
        "可识别字体" => "Detected fonts",
        "Emoji 渲染" => "Emoji rendering",
        "Canvas 指纹摘要" => "Canvas fingerprint",
        "WebGL 厂商" => "WebGL vendor",
        "WebGL 渲染器" => "WebGL renderer",
        "浏览器 Cookie 功能" => "Browser cookies",
        "WebRTC 公网地址" => "WebRTC public address",
        "WebRTC 公网候选" => "WebRTC public candidate",
        "定位权限" => "Location permission",
        "主动检测的定位" => "Requested location",
        "操作系统" => "Operating system",
        "系统 HTTPS 代理" => "System HTTPS proxy",
        "Desktop 内部地区环境" => "Desktop region settings",
        "Claude 请求实时检测" => "Live Claude request monitoring",
        "浏览器递归 DNS" => "Browser recursive DNS",
        "实际递归 DNS" => "Observed recursive DNS",
        "系统递归 DNS" => "System recursive DNS",
        "公网 DNS 保护策略" => "Public DNS protection",
        "TUN 实际状态" => "TUN status",
        "严格路由实际状态" => "Strict routing status",
        "运行模式" => "Routing mode",
        "IPv6 实际状态" => "IPv6 status",
        _ => {
            if let Some(ip) = value.strip_prefix("出口 IP 质量 · ") {
                return format!("Exit IP quality · {ip}");
            }
            if let Some(host) = value.strip_suffix(" 出口")
                && host.is_ascii()
            {
                return format!("{host} exit");
            }
            if let Some(host) = value.strip_suffix(" 连通性")
                && host.is_ascii()
            {
                return format!("{host} connectivity");
            }
            value
        }
    };
    translated.to_owned()
}

pub(super) fn message(value: &str, language: Language) -> &str {
    if language == Language::SimplifiedChinese {
        return value;
    }
    match value {
        "未确认" | "尚未确认" => "Unconfirmed",
        "未提供" => "Not provided",
        "允许 Cookie" => "Cookies enabled",
        "禁用 Cookie" => "Cookies disabled",
        "未配置；TUN 仍可能接管流量" => "Not configured; TUN may route traffic",
        "当前环境的设置值。" => "Setting reported by the current environment.",
        "未读取到设置值。" => "No setting was reported.",
        "按浏览器返回的优先顺序排列。" => {
            "Listed in the order reported by the browser."
        }
        "未读取到语言偏好。" => "No language preferences were reported.",
        "记录运行环境；操作系统本身不能确定地区。" => {
            "Records the operating system and architecture."
        }
        "未获得有效结果；请检查协议和连接状态。" => {
            "No valid result; check the protocol and connection."
        }
        "地区来自目标域名的 Cloudflare trace。" => {
            "Region from the target domain's Cloudflare trace."
        }
        "三次请求均未取得响应。" => "All three requests failed to receive a response.",
        "未取得实测公网出口 IP，未发起远端探测。" => {
            "No public exit IP was detected; remote probes were not started."
        }
        "远端探测超时、额度不足或服务不可用。" => {
            "Remote probes timed out, were rate limited, or were unavailable."
        }
        "未取得有效 ICMP 响应；请检查防火墙或探针服务状态。" => {
            "No valid ICMP response; check the firewall or probe service."
        }
        "HTTPS 请求耗时，包含连接、TLS 和服务器响应。" => {
            "HTTPS request time, including connection, TLS, and server response."
        }
        "HTTPS 请求耗时，包含连接、TLS 和服务器响应。 服务器要求 Cloudflare 人机验证。" => {
            "HTTPS request time includes connection, TLS, and server response. Cloudflare requested verification."
        }
        "可用浏览器打开 Claude，确认是否需要验证及能否正常使用。" => {
            "Open Claude in a browser to check verification and availability."
        }
        "如与预期不同，检查当前环境的地区、语言或时区设置。" => {
            "Check the region, language, and time zone settings if needed."
        }
        "运行时监测 Claude 及关联服务的连接，关闭偏离指定节点的存续连接。" => {
            "Monitors Claude and related connections and closes connections outside the selected route."
        }
        _ => value,
    }
}

pub(super) fn source(value: &str, language: Language) -> String {
    if language == Language::SimplifiedChinese {
        return value.to_owned();
    }
    let translated = match value {
        "内核事件与连接控制接口" => "Core events and connection control",
        "内核连接记录" => "Core connection records",
        "实时拦截 · 内核连接记录" => "Live blocking · core connection records",
        "第三方 IP 数据源" => "Third-party IP sources",
        "内核当前配置接口" => "Current core configuration",
        "保护运行时配置" => "Protection configuration",
        "通过内核发起轻量 HTTPS 请求" => "HTTPS requests through the core",
        "Globalping 远端探针" => "Globalping remote probes",
        "运行检测命令的终端进程" => "Terminal running the check",
        "本机系统；不是 Desktop 内部读取结果" => "Local system",
        "用户在 Claude 页面运行的浏览器采样" => "Browser sample from a Claude page",
        "浏览器本地检测页 · Claude 站点设置待确认" => {
            "Local browser test · Claude site settings unconfirmed"
        }
        _ => value,
    };
    translated.replace('、', ", ")
}

pub(super) fn value(signal: &Signal, language: Language) -> String {
    let value = signal.display_value();
    if language == Language::SimplifiedChinese {
        return value.to_owned();
    }
    if signal.group == Group::Quality && signal.kind() == Some(SignalKind::IpQuality) {
        return value
            .split(" · ")
            .map(quality_summary)
            .collect::<Vec<_>>()
            .join(" · ");
    }
    if signal.group == Group::Desktop && signal.is_guard() {
        return value
            .split(" · ")
            .map(guard_summary)
            .collect::<Vec<_>>()
            .join(" · ");
    }
    if signal.group == Group::Connectivity {
        return value
            .split(" · ")
            .map(connectivity_summary)
            .collect::<Vec<_>>()
            .join(" · ");
    }
    if signal.title == "语言偏好" {
        return value.replace('、', ", ");
    }
    if let (Some(chinese), Some(english)) = (
        crate::probe::country_name(&signal.country),
        crate::countries::english_name(&signal.country),
    ) {
        if !signal.ip.is_empty() && value == format!("{} · {chinese}", signal.ip) {
            return format!("{} · {english}", signal.ip);
        }
        if signal.title == "Claude 官方支持地区" {
            if value == format!("{chinese} · 列于支持地区") {
                return format!("{english} · Listed as supported");
            }
            if value == format!("{chinese} · 未列于支持地区") {
                return format!("{english} · Not listed as supported");
            }
        }
    }
    message(value, language).to_owned()
}

fn connectivity_summary(value: &str) -> String {
    if let Some(counts) = value.strip_prefix("响应 ")
        && let Some((received, sent)) = counts.split_once('/')
        && received.parse::<u32>().is_ok()
        && sent.parse::<u32>().is_ok()
    {
        return format!("Responses {received}/{sent}");
    }
    if let Some(counts) = value.strip_suffix(" 个探针取得响应")
        && let Some((received, sent)) = counts.split_once('/')
        && received.parse::<u32>().is_ok()
        && sent.parse::<u32>().is_ok()
    {
        return format!("{received}/{sent} probes responded");
    }
    if let Some(millis) = value
        .strip_prefix("中位 ")
        .and_then(|value| value.strip_suffix(" ms"))
        && millis
            .parse::<f64>()
            .is_ok_and(|value| value.is_finite() && value >= 0.)
    {
        return format!("Median {millis} ms");
    }
    match value {
        "需要浏览器验证" => "Browser verification required",
        _ => value,
    }
    .to_owned()
}

pub(super) fn detail(signal: &Signal, language: Language) -> String {
    let detail = signal.detail.trim();
    if language == Language::SimplifiedChinese {
        return detail.to_owned();
    }
    if signal.group == Group::Quality && signal.kind() == Some(SignalKind::IpQuality) {
        return detail
            .lines()
            .map(quality_line)
            .collect::<Vec<_>>()
            .join("\n");
    }
    if signal.group == Group::Connectivity
        && signal.kind() == Some(SignalKind::Connectivity)
        && let Some((ip, latency)) = detail
            .strip_prefix("远端探针到出口 IP ")
            .and_then(|value| value.strip_suffix(" ms。"))
            .and_then(|value| value.split_once(" 的 ICMP 测量；有效探针平均延迟范围 "))
        && ip.parse::<std::net::IpAddr>().is_ok()
        && let Some((minimum, maximum)) = latency.split_once('–')
        && [minimum, maximum].into_iter().all(|value| {
            value
                .parse::<f64>()
                .is_ok_and(|value| value.is_finite() && value >= 0.)
        })
    {
        return format!(
            "ICMP from remote probes to exit IP {ip}; average latency range {minimum}–{maximum} ms."
        );
    }
    if signal.group == Group::Desktop
        && signal.is_guard()
        && let Some(error) =
            detail.strip_prefix("运行时监测 Claude 及关联服务的连接，关闭偏离指定节点的存续连接。")
    {
        let description = message(
            "运行时监测 Claude 及关联服务的连接，关闭偏离指定节点的存续连接。",
            language,
        );
        let error = error.trim();
        return if error.is_empty() {
            description.to_owned()
        } else {
            format!("{description} {}", guard_summary(error))
        };
    }
    message(detail, language).to_owned()
}

fn quality_field(value: &str) -> Option<&'static str> {
    match value {
        "机房或托管网络" => Some("Datacenter or hosting"),
        "VPN 地址" => Some("VPN"),
        "代理地址" => Some("Proxy"),
        "Tor 地址" => Some("Tor"),
        "爬虫地址" => Some("Crawler"),
        "滥用记录" => Some("Abuse record"),
        "移动网络" => Some("Mobile network"),
        _ => None,
    }
}

fn quality_summary(value: &str) -> String {
    if let Some((field, "是")) = value.split_once('：')
        && let Some(field) = quality_field(field)
    {
        return format!("{field}: Yes");
    }
    if let Some(field) = value.strip_suffix("来源不一致")
        && let Some(field) = quality_field(field)
    {
        return format!("{field}: Sources disagree");
    }
    if let Some(count) = value.strip_suffix(" 项待确认")
        && count.parse::<usize>().is_ok()
    {
        return format!("{count} unconfirmed");
    }
    if let Some(count) = value.strip_suffix(" 个来源查询未完成")
        && count.parse::<usize>().is_ok()
    {
        return format!("{count} sources incomplete");
    }
    match value {
        "各项均待确认" => "All properties unconfirmed",
        "有结果的属性暂未见标记" => "No flags in available results",
        _ => value,
    }
    .to_owned()
}

fn quality_line(line: &str) -> String {
    let heading = match line {
        "已标记" => Some("Flagged"),
        "来源不一致" => Some("Source disagreement"),
        "已有结果未标记" => Some("Not flagged in available results"),
        "尚未确认" => Some("Unconfirmed"),
        "信誉信息" => Some("Reputation"),
        "查询未完成" => Some("Incomplete queries"),
        _ => None,
    };
    if let Some(heading) = heading {
        return heading.to_owned();
    }
    let Some((key, value)) = line.split_once('：') else {
        return line.to_owned();
    };
    if let Some(field) = quality_field(key) {
        let values = value
            .split('、')
            .map(|item| {
                if item == "无有效响应" {
                    return "No valid responses".to_owned();
                }
                if let Some((provider, result)) = item.rsplit_once(' ') {
                    let result = match result {
                        "是" => "Yes",
                        "否" => "No",
                        "未提供" => "Not provided",
                        _ => result,
                    };
                    return format!("{provider}: {result}");
                }
                item.to_owned()
            })
            .collect::<Vec<_>>()
            .join(", ");
        return format!("{field}: {values}");
    }
    if let Some(provider) = key.strip_suffix(" 信誉") {
        let reputation = match value {
            "允许名单" => "Allowlist",
            "灰名单" => "Greylist",
            "阻止名单" => "Blocklist",
            _ => value,
        };
        return format!("{provider} reputation: {reputation}");
    }
    match value {
        "请求超时" => format!("{key}: Timed out"),
        "达到查询限额" => format!("{key}: Rate limited"),
        "服务暂不可用" => format!("{key}: Service unavailable"),
        "响应未通过校验" => format!("{key}: Invalid response"),
        _ => line.to_owned(),
    }
}

fn guard_summary(value: &str) -> String {
    for (prefix, suffix, singular, plural) in [
        (
            "已观察 ",
            " 次请求",
            "request observed",
            "requests observed",
        ),
        (
            "已关闭 ",
            " 条异常连接",
            "unexpected connection closed",
            "unexpected connections closed",
        ),
        (
            "系统提示请求 ",
            " 次",
            "notification request",
            "notification requests",
        ),
    ] {
        if let Some(count) = value
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix(suffix))
            && let Ok(number) = count.parse::<u64>()
        {
            return format!("{count} {}", if number == 1 { singular } else { plural });
        }
    }
    match value {
        "上次保护记录" => "Previous protection record",
        "控制接口暂不可用，实时拦截未确认；保护规则仍保留。" => {
            "Control interface unavailable; protection rules retained."
        }
        "实时拦截已停止，请重新开启 Claude 保护。" => {
            "Live blocking stopped; restart Claude protection."
        }
        "异常连接关闭失败" => "Failed to close an unexpected connection",
        "异常连接关闭失败，请检查内核状态。" => {
            "Failed to close an unexpected connection; check the core."
        }
        _ => value,
    }
    .to_owned()
}
