use crate::config::Language;

pub(super) fn label(value: &str, language: Language) -> String {
    if language == Language::SimplifiedChinese {
        return value.to_owned();
    }
    let translated = match value {
        "检查参数" => "Parameters",
        "目标地址" => "Target address",
        "当前工具" => "Current tool",
        "UDP 探针" => "UDP probe",
        "系统 DNS" => "System DNS",
        "内核 DNS" => "Core DNS",
        "内核 DNS（独立查询）" => "Core DNS (separate query)",
        "解析方式" => "Resolution",
        "DNS 响应" => "DNS response",
        "记录" => "Record",
        "回答上限" => "Answer limit",
        "本机代理" => "Local proxy",
        "本机代理 TCP" => "Local proxy TCP",
        "代理建立目标连接" => "Proxy connection to target",
        "检查范围" => "Scope",
        "检查路径" | "路径" => "Route",
        "采用协议" => "Protocol",
        "实际线路" => "Route",
        "网站响应" => "Website response",
        "业务状态" => "Service status",
        "重定向" => "Redirect",
        "响应正文" => "Response body",
        "请求结果" => "Request results",
        "HTTP 状态分布" => "HTTP status distribution",
        "响应头耗时" => "Response header time",
        "接收中断" => "Receive interrupted",
        "已接收" => "Received",
        "平均速度" => "Average speed",
        "结束原因" => "Completion reason",
        "接收内容" => "Received content",
        "TLS 握手" => "TLS handshake",
        "TLS 协议" => "TLS protocol",
        "证书验证" => "Certificate validation",
        "证书详情" => "Certificate details",
        "等待首字节" => "Time to first byte",
        "证书主体" => "Certificate subject",
        "签发者" => "Issuer",
        "有效期（UTC）" => "Validity (UTC)",
        "证书 SHA256" => "Certificate SHA256",
        "系统探针" => "System probe",
        "系统探针结果" => "System probe result",
        "系统探针说明" => "System probe details",
        "系统探针原文" => "System probe output",
        "执行结果" => "Result",
        "追踪结果" => "Trace result",
        "探针设置" => "Probe settings",
        "检查失败" => "Check failed",
        "检查停止" => "Check stopped",
        _ => {
            if let Some(number) = value.strip_prefix("请求 ")
                && !number.is_empty()
                && number.bytes().all(|byte| byte.is_ascii_digit())
            {
                return format!("Request {number}");
            }
            if let Some(number) = value
                .strip_prefix("第 ")
                .and_then(|part| part.strip_suffix(" 跳"))
                && !number.is_empty()
                && number.bytes().all(|byte| byte.is_ascii_digit())
            {
                return format!("Hop {number}");
            }
            return value.to_owned();
        }
    };
    translated.to_owned()
}

pub(super) fn detail(label: &str, value: &str, code: &str, language: Language) -> String {
    if language == Language::SimplifiedChinese {
        return value.to_owned();
    }
    let translated = match (label, code, value) {
        (_, "http_timeout", "网站请求超时，未自动切换线路") => {
            "Website request timed out"
        }
        (_, "http_connect_failed", "无法建立网站连接，请查看同时间内核日志") => {
            "Could not connect to the website; check the core log"
        }
        (_, "http_body_failed", "响应正文读取失败") => "Could not read the response body",
        (_, "http_failed", "网站请求失败，请查看同时间内核日志") => {
            "Website request failed; check the core log"
        }
        (_, "tls_failed", "TLS 握手或证书验证失败") => {
            "TLS handshake or certificate validation failed"
        }
        (_, "permission_denied", "系统拒绝此探针的访问权限") => {
            "System denied permission for this probe"
        }
        (_, "component_unavailable", "系统未提供此探针组件") => {
            "System probe component unavailable"
        }
        (_, "connection_refused", "连接被拒绝") => "Connection refused",
        (_, "connection_timeout", "连接超时") => "Connection timed out",
        (_, "connection_closed", "对端在响应完成前关闭连接") => {
            "Peer closed the connection before completing its response"
        }
        (_, "connection_interrupted", "连接中断") => "Connection interrupted",
        (_, "probe_io_failed", "探针通信失败，请查看同时间内核日志") => {
            "Probe communication failed; check the core log"
        }
        (_, "probe_failed", "检查未完成，请导出诊断并附上操作时间") => {
            "Check incomplete; export diagnostics and include the time"
        }
        (
            "检查参数",
            "invalid_options",
            "参数无效，请检查次数、间隔、时限、流量上限、跳数、DNS 类型或 HTTP 方法",
        ) => {
            "Invalid parameters; check count, interval, timeout, size, hops, DNS type, or HTTP method"
        }
        ("目标地址", "invalid_target", "请输入域名、IP 或 HTTP/HTTPS 地址；不支持嵌入账号密码") => {
            "Enter a domain, IP address, or HTTP/HTTPS URL without embedded credentials"
        }
        ("DNS", "literal_ip", "目标已经是 IP 地址，无需解析") => {
            "Target is already an IP address"
        }
        ("系统 DNS", "protection_restricted", "保护开启或状态未知，未单独查询系统解析器") => {
            "System resolver not queried while protection is active or unknown"
        }
        ("系统 DNS", "dns_failed", "系统解析失败或超时") => {
            "System DNS failed or timed out"
        }
        ("解析方式", "information", "内核查询返回真实上游地址；两种解析结果不同不单独视为故障") => {
            "Core DNS returns upstream addresses; results may differ from system DNS"
        }
        ("业务状态", "information", "已收到 HTTP 错误响应；传输连通不代表业务可用") => {
            "HTTP error response received"
        }
        ("业务状态", "information", "已收到响应；业务是否可用还取决于目标服务") => {
            "Response received"
        }
        ("content-security-policy", "ok", "已设置（策略正文已隐藏）") => {
            "Set; policy content hidden"
        }
        ("重定向", "information", "未跟随；Location、Cookie 和认证响应头不进入报告") => {
            "Redirect not followed; Location, Cookie, and authentication headers omitted"
        }
        ("检查范围", "information", "仅建立 TCP 连接，不发送应用数据；不代表 TLS 或服务可用") => {
            "TCP connection only; no application data sent"
        }
        ("TLS", "not_applicable", "HTTP 目标没有 TLS 握手") => {
            "HTTP target has no TLS handshake"
        }
        ("证书详情", "certificate_display_invalid", "握手已验证证书，但无法展示证书字段") => {
            "Certificate validated; fields unavailable"
        }
        (
            "路径",
            "information",
            "经本机 SOCKS5 代理，不切换模式、节点或 DNS；分阶段值与独立的 DNS 查询不直接相加",
        ) => "Via local SOCKS5; separate DNS timing is not included in connection timing",
        (
            "路径",
            "information",
            "经本机 SOCKS5 UDP 中继，不修改用户 UDP 设置或绕过当前出站规则",
        ) => "Via the local SOCKS5 UDP relay using the current outbound rules",
        (
            "接收内容",
            "information",
            "即时丢弃，不写文件或保留正文；平均吞吐包含建连等待，仅代表本次限量样本",
        ) => "Content discarded; average speed includes connection time",
        ("回答上限", "output_truncated", "仅显示前 16 条记录") => {
            "Showing the first 16 records"
        }
        ("记录", "dns_no_record", "查询成功，但没有此类型的回答") => {
            "Query succeeded; no records of this type"
        }
        ("DNS / UDP", "udp_invalid_response", "收到不匹配、截断或错误的 DNS 响应") => {
            "DNS response was mismatched, truncated, or invalid"
        }
        ("DNS / UDP", "udp_no_response", "本次探针未获响应；不代表所有 UDP 不可用") => {
            "No response to this UDP probe"
        }
        (
            "当前工具",
            "protection_restricted",
            "Claude 保护开启或状态未知，无法确认探针的受保护出口，未发起请求",
        ) => "Probe not sent while the protected route is active or unknown",
        ("UDP 探针", "udp_disabled", "设置中已关闭 UDP，未发送探针") => {
            "UDP is disabled in settings; probe not sent"
        }
        ("检查停止", "probe_timeout", "已达到本次检查时限") => {
            "Check reached its time limit"
        }
        ("结束原因", "byte_limit", "达到流量上限") => "Size limit reached",
        ("结束原因", "time_limit", "达到时间上限") => "Time limit reached",
        ("结束原因", "completed", "目标响应结束") => "Response completed",
        ("结束原因", "interrupted", "连接中断") => "Connection interrupted",
        ("HTTP 状态分布", "information", "未收到 HTTP 响应") => {
            "No HTTP response received"
        }
        ("执行结果", "ok", "系统命令已结束；目标是否到达请看探针结果") => {
            "System command finished; inspect the probe result"
        }
        ("执行结果", "ok", "系统命令已结束") => "System command finished",
        ("执行结果", "timeout", "达到时限，已停止系统探针") => {
            "Time limit reached; system probe stopped"
        }
        _ => return value.to_owned(),
    };
    translated.to_owned()
}
