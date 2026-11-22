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
        (_, "core_unavailable", "内核未运行或代理端口不可用；请先启动内核") => {
            "Core is not running or its proxy port is unavailable"
        }
        (_, "controller_unavailable", "内核解析接口不可用") => {
            "Core DNS interface unavailable"
        }
        (_, "controller_query_failed", "内核未支持此查询或拒绝请求") => {
            "Core does not support or rejected this query"
        }
        (_, "controller_response_limit", "控制接口响应超过本次上限") => {
            "Core response exceeded the size limit"
        }
        (_, "controller_response_invalid", "控制接口响应格式无效") => {
            "Invalid core response"
        }
        (_, "dns_response_invalid", "DNS 响应缺少状态码") => {
            "DNS response has no status code"
        }
        (_, "dns_timeout", "内核解析超时") => "Core DNS timed out",
        (_, "dns_failed", "内核解析失败，请检查内核日志") => {
            "Core DNS failed; check the core log"
        }
        (_, "dns_no_address", "内核未返回目标地址") => "Core DNS returned no addresses",
        ("内核 DNS（独立查询）", "dns_not_observed", "内核解析接口不可用") => {
            "Core DNS interface unavailable"
        }
        ("内核 DNS（独立查询）", "dns_not_observed", "内核解析超时") => {
            "Core DNS timed out"
        }
        ("内核 DNS（独立查询）", "dns_not_observed", "内核解析失败，请检查内核日志") => {
            "Core DNS failed; check the core log"
        }
        ("内核 DNS（独立查询）", "dns_not_observed", "内核未返回目标地址") => {
            "Core DNS returned no addresses"
        }
        (_, "proxy_authentication", "本机代理拒绝无认证探针，未尝试其他出口") => {
            "Local proxy rejected the unauthenticated probe"
        }
        (_, "proxy_response_invalid", "SOCKS5 响应格式无效") => "Invalid SOCKS5 response",
        (_, "proxy_response_invalid", "SOCKS5 响应地址类型无效") => {
            "Invalid address type in SOCKS5 response"
        }
        (_, code, "本机代理未能连接目标，请查看内核日志") if code.starts_with("proxy_") => {
            "Local proxy could not connect to the target; check the core log"
        }
        (_, "connection_timeout", "TCP 连接或代理握手超时") => {
            "TCP connection or proxy handshake timed out"
        }
        (_, "connection_timeout", "本次连接达到时限，已停止检查") => {
            "Connection timed out; check stopped"
        }
        (_, "empty_response", "目标在响应前关闭连接") => {
            "Target closed the connection before responding"
        }
        (_, "tls_certificate_expired", "证书已过期，未绕过验证") => {
            "Certificate expired"
        }
        (_, "tls_certificate_not_yet_valid", "证书尚未生效，请核对系统时间") => {
            "Certificate is not yet valid; check the system clock"
        }
        (_, "tls_certificate_name", "证书与目标域名不匹配，未绕过验证") => {
            "Certificate does not match the target domain"
        }
        (_, "tls_certificate_issuer", "证书颁发机构不受信任或证书链不完整，未绕过验证") => {
            "Certificate issuer is untrusted or the chain is incomplete"
        }
        (_, "tls_certificate_invalid", "证书验证失败，未绕过验证") => {
            "Certificate validation failed"
        }
        (_, "tls_failed", "TLS 握手失败，未降级或绕过验证") => "TLS handshake failed",
        (_, "invalid_target", "请输入主机和端口，端口应为 1～65535") => {
            "Enter a host and port between 1 and 65535"
        }
        (_, "invalid_target", "UDP 探针需要 DNS 服务器地址，不支持网址路径或查询参数") => {
            "Enter a DNS server address without a URL path or query"
        }
        (_, "invalid_target", "DNS 服务器端口必须为 1～65535") => {
            "DNS server port must be between 1 and 65535"
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
        ("执行结果", "probe_timeout", "达到时限，已停止系统探针") => {
            "Time limit reached; system probe stopped"
        }
        ("追踪结果", "ok", "已观察到目标地址响应；中间跳的 * 不影响此结论") => {
            "Target responded"
        }
        (
            "追踪结果",
            "trace_destination_unconfirmed",
            "未识别到逐跳结果；请查看系统探针说明，无法确认到达目标",
        ) => "No hops identified; check the system probe output",
        (
            "追踪结果",
            "trace_destination_unconfirmed",
            "本次未确认到达目标，可能达到跳数上限或后续不回包；不等于网站或代理不可用",
        ) => "Target not confirmed; the hop limit or missing responses may be the cause",
        (
            "路径",
            "information",
            "系统 ICMP / 逐跳探针；系统代理不承载此流量，TUN 是否接管取决于系统路由。这不是代理节点延迟",
        ) => "System ICMP or trace probe; TUN routing depends on system routes",
        (
            "检查路径",
            "information",
            "强制目标 IP，经本机 SOCKS5 代理；HTTPS 验证原域名证书，HTTP 仅建立连接。IP 规则可能与域名请求不同；结果不代表本机或节点入口的地址族能力",
        ) => "Target IP via local SOCKS5; HTTPS validates the original domain certificate",
        ("IPv4" | "IPv6", "family_no_address", "未返回此地址族的目标地址，未发起连接") => {
            "No address returned for this family; connection not attempted"
        }
        ("系统 DNS" | "内核 DNS", "not_observed", "内核未运行或控制接口不可用") => {
            "Core is not running or its control interface is unavailable"
        }
        ("系统 DNS", "not_observed", "保护已开启或状态未知，未单独查询系统解析器") => {
            "System resolver not queried while protection is active or unknown"
        }
        ("系统 DNS" | "内核 DNS", "probe_failed", "未返回 IP 地址") => {
            "No IP addresses returned"
        }
        ("内核 DNS", "probe_failed", "未查到目标地址") => {
            "No addresses found for the target"
        }
        ("内核 DNS", "probe_failed", "查询超时") => "DNS query timed out",
        ("本机代理", "not_observed", "内核未运行") => "Core is not running",
        ("本机代理", "probe_failed", "端口连接超时") => "Proxy port connection timed out",
        ("HTTPS" | "HTTP", "not_observed", "内核未运行，未发起请求") => {
            "Core is not running; request not sent"
        }
        ("采用协议", "ok", "HTTP（HTTPS 不可达）") => "HTTP (HTTPS unreachable)",
        ("采用协议", "not_observed", "未检查") => "Not checked",
        ("采用协议", "probe_failed", "HTTPS 与 HTTP 均未连通") => {
            "Both HTTPS and HTTP failed"
        }
        ("实际线路", "not_observed", "未建立检测连接") => {
            "No test connection established"
        }
        ("实际线路", "not_observed", "未捕获到此请求的连接记录") => {
            "No connection record captured"
        }
        ("实际线路", "not_observed", "请求未完成，无法确认线路") => {
            "Request incomplete; route unavailable"
        }
        _ if label.starts_with("第 ")
            && code == "information"
            && value
                == "未收到该跳响应（*）；可能未回包、被过滤或超过等待时间，不能据此判定断网" =>
        {
            "No response from this hop (*)"
        }
        _ => return dynamic_detail(label, value, code).unwrap_or_else(|| value.to_owned()),
    };
    translated.to_owned()
}

fn digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn decimal(value: &str) -> bool {
    let mut parts = value.split('.');
    parts.next().is_some_and(digits) && parts.next().is_none_or(digits) && parts.next().is_none()
}

fn dns_flag(value: &str) -> Option<&'static str> {
    match value {
        "是" => Some("yes"),
        "否" => Some("no"),
        "未提供" => Some("unavailable"),
        _ => None,
    }
}

fn dynamic_detail(label: &str, value: &str, code: &str) -> Option<String> {
    match (label, code) {
        ("DNS 响应", "ok") => {
            let (kind, rest) = value.split_once(" · ")?;
            let (count, rest) = rest.split_once(" 条回答 · AA=")?;
            let (authoritative, recursion) = rest.split_once(" · RA=")?;
            if kind.is_empty()
                || !kind.bytes().all(|byte| byte.is_ascii_alphanumeric())
                || !digits(count)
            {
                return None;
            }
            Some(format!(
                "{kind} · {count} answers · AA={} · RA={}",
                dns_flag(authoritative)?,
                dns_flag(recursion)?
            ))
        }
        ("记录", "ok") => {
            let rest = value.strip_prefix("类型 ")?;
            let (kind, rest) = rest.split_once(" · TTL ")?;
            let (ttl, data) = rest.split_once(" s · ")?;
            if !(digits(kind) || kind == "未提供") || !(digits(ttl) || ttl == "未提供") {
                return None;
            }
            let kind_text = if kind == "未提供" {
                "unavailable"
            } else {
                kind
            };
            let ttl_text = if ttl == "未提供" {
                "unavailable"
            } else {
                ttl
            };
            let data_text = if matches!(kind, "6" | "16") && data == "正文已隐藏" {
                "Content hidden"
            } else {
                data
            };
            Some(format!("Type {kind_text} · TTL {ttl_text} s · {data_text}"))
        }
        ("内核 DNS", "dns_failed") => {
            let (kind, status) = value.split_once(" · DNS 状态 ")?;
            if kind.is_empty()
                || !kind.bytes().all(|byte| byte.is_ascii_alphanumeric())
                || !digits(status)
            {
                return None;
            }
            Some(format!("{kind} · DNS status {status}"))
        }
        ("响应正文", "byte_limit" | "information") => {
            let size = value
                .strip_prefix("接收 ")?
                .strip_suffix(" B（最多 4096 B），已丢弃")?;
            digits(size).then(|| format!("Received {size} B (up to 4096 B); discarded"))
        }
        ("请求结果", "ok" | "http_failed") => {
            let rest = value.strip_prefix("收到响应 ")?;
            let (received, rest) = rest.split_once(" / ")?;
            let (total, failed) = rest.split_once(" · 传输失败 ")?;
            let failed = failed.strip_suffix("；HTTP 业务状态另列")?;
            (digits(received) && digits(total) && digits(failed)).then(|| {
                format!("Responses {received} / {total} · Transport failures {failed}; HTTP status listed separately")
            })
        }
        ("响应头耗时", "ok") => {
            let rest = value.strip_prefix("最低 ")?;
            let (minimum, rest) = rest.split_once(" · 中位 ")?;
            let (median, maximum) = rest.split_once(" · 最高 ")?;
            let maximum = maximum.strip_suffix(" ms")?;
            (digits(minimum) && digits(median) && digits(maximum))
                .then(|| format!("Min {minimum} · Median {median} · Max {maximum} ms"))
        }
        ("网站响应", "download_http_status") => {
            let status = value
                .strip_prefix("HTTP ")?
                .strip_suffix("，未接收正文；重定向不会自动跳转")?;
            digits(status)
                .then(|| format!("HTTP {status}; body not received; redirect not followed"))
        }
        ("已接收", "ok" | "empty_response") => {
            let (received, limit) = value.strip_suffix(" B 上限")?.split_once(" B / ")?;
            (digits(received) && digits(limit)).then(|| format!("{received} B / {limit} B limit"))
        }
        ("平均速度", "information") => {
            let (speed, seconds) = value.strip_suffix(" 秒")?.split_once(" KiB/s · ")?;
            (decimal(speed) && decimal(seconds)).then(|| format!("{speed} KiB/s · {seconds} s"))
        }
        ("DNS / UDP", "ok") => {
            let millis = value.strip_prefix("收到有效响应 · ")?.strip_suffix(" ms")?;
            digits(millis).then(|| format!("Valid response · {millis} ms"))
        }
        ("代理建立目标连接", "ok") => {
            if let Some(rest) = value.strip_prefix("端口 ") {
                let (port, millis) = rest.split_once(" · ")?;
                let millis = millis.strip_suffix(" ms（含代理侧 DNS、规则及出站处理）")?;
                return (digits(port) && digits(millis))
                    .then(|| format!("Port {port} · {millis} ms (includes proxy DNS, rules and outbound processing)"));
            }
            let millis = value.strip_suffix(" ms（含代理侧 DNS、规则及出站处理；沿用当前线路）")?;
            digits(millis)
                .then(|| format!("{millis} ms (includes proxy DNS, rules and outbound processing)"))
        }
        ("等待首字节", "ok") => {
            let millis = value.strip_suffix(" ms（请求写入后）")?;
            digits(millis).then(|| format!("{millis} ms after request"))
        }
        ("证书验证", "ok") => {
            let count = value
                .strip_prefix("域名、有效期和信任链验证通过 · 服务端提供 ")?
                .strip_suffix(" 张证书")?;
            digits(count)
                .then(|| format!("Domain, validity and chain verified · {count} certificates"))
        }
        ("内核 DNS（独立查询）", "ok") => {
            let (millis, count) = value
                .strip_suffix(" 个地址；不计入目标 TCP 耗时")?
                .split_once(" ms · ")?;
            (digits(millis) && digits(count)).then(|| {
                format!("{millis} ms · {count} addresses; separate from target TCP timing")
            })
        }
        ("IPv4" | "IPv6", "family_timeout") => {
            let address = value.strip_suffix(" · 连接超时")?;
            address.parse::<std::net::IpAddr>().ok()?;
            Some(format!("{address} · Connection timed out"))
        }
        ("系统 DNS" | "内核 DNS", "not_observed") => {
            let address = value.strip_suffix("（IP 目标，无需解析）")?;
            address.parse::<std::net::IpAddr>().ok()?;
            Some(format!("{address} (IP target; no DNS lookup)"))
        }
        ("系统 DNS" | "内核 DNS", "ok") => {
            let (addresses, extra) = match value.split_once("（另有 ") {
                Some((addresses, extra)) => {
                    let count = extra.strip_suffix(" 项）")?;
                    if !digits(count) || count.starts_with('0') {
                        return None;
                    }
                    (addresses, Some(count))
                }
                None => (value, None),
            };
            if !addresses
                .split('、')
                .all(|address| address.parse::<std::net::IpAddr>().is_ok())
            {
                return None;
            }
            let addresses = addresses.replace('、', ", ");
            Some(match extra {
                Some(count) => format!("{addresses} ({count} more)"),
                None => addresses,
            })
        }
        ("本机代理", "ok") => {
            let millis = value.strip_prefix("端口可连接 · ")?.strip_suffix(" ms")?;
            digits(millis).then(|| format!("Port reachable · {millis} ms"))
        }
        ("本机代理", "probe_failed") => {
            let error = value.strip_prefix("端口不可连接：")?;
            Some(format!("Port unreachable: {error}"))
        }
        ("探针设置", "information") => {
            let hops = value.strip_prefix("最多 ")?.strip_suffix(" 跳，每个响应等待 1 秒；星号表示等待期间未收到响应。可调整跳数后重试，网络设备也可能限制逐跳响应")?;
            digits(hops).then(|| {
                format!("Up to {hops} hops, 1 second per response; * indicates no response")
            })
        }
        ("执行结果", "process_failed") => {
            let status = value
                .strip_prefix("探针退出状态 ")?
                .strip_suffix("；可能为未响应或系统权限限制，请查看探针说明")?;
            Some(format!(
                "Probe exited with {status}; check system probe output"
            ))
        }
        _ => None,
    }
}
