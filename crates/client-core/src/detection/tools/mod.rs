mod datagram;
mod error;
mod http;
#[cfg(test)]
mod inspection_tests;
mod process;
#[cfg(test)]
mod tests;
mod text;
mod transport;

use crate::{api::Api, config::Language, probe};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    Website,
    Dns,
    Phase,
    Family,
    Tls,
    Stability,
    Udp,
    Ping,
    Trace,
    Download,
    Tcp,
    Http,
}

impl Tool {
    pub const ALL: [Self; 12] = [
        Self::Website,
        Self::Dns,
        Self::Phase,
        Self::Family,
        Self::Tls,
        Self::Stability,
        Self::Udp,
        Self::Ping,
        Self::Trace,
        Self::Download,
        Self::Tcp,
        Self::Http,
    ];

    pub fn parse(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|tool| tool.id() == value)
            .context("未知网络工具")
    }

    pub fn at(index: i32) -> Result<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|index| Self::ALL.get(index).copied())
            .context("未知网络工具")
    }

    pub fn index(self) -> i32 {
        Self::ALL.iter().position(|tool| *tool == self).unwrap() as i32
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Website => "check",
            Self::Dns => "dns",
            Self::Phase => "phase",
            Self::Family => "family",
            Self::Tls => "tls",
            Self::Stability => "stability",
            Self::Udp => "udp",
            Self::Ping => "ping",
            Self::Trace => "trace",
            Self::Download => "download",
            Self::Tcp => "tcp",
            Self::Http => "http",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Website => "网站检查",
            Self::Dns => "DNS 查询",
            Self::Phase => "连接耗时",
            Self::Family => "IPv4 / IPv6",
            Self::Tls => "TLS 证书",
            Self::Stability => "短时稳定性",
            Self::Udp => "UDP 连通",
            Self::Ping => "Ping",
            Self::Trace => "路由追踪",
            Self::Download => "限量下载",
            Self::Tcp => "TCP 端口",
            Self::Http => "HTTP 检查",
        }
    }

    pub fn restricted(self) -> bool {
        matches!(self, Self::Udp | Self::Ping | Self::Trace)
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Website => "快速定位解析、响应和实际命中规则；详细分析使用专项工具",
            Self::Dns => "查询记录、TTL 和返回码，对照系统与内核解析",
            Self::Phase => "分段测量代理建连、TLS 与首字节；代理建连含代理侧处理",
            Self::Family => "经当前代理分别访问目标 IPv4 / IPv6，不代表本机地址族能力",
            Self::Tls => "仅检查 TLS 握手和证书，不发送 HTTP 请求",
            Self::Stability => "分别统计传输失败、HTTP 状态与耗时分布",
            Self::Udp => "经内核 UDP 中继向目标 DNS 服务器发送探针",
            Self::Ping => "系统 ICMP 探针，不代表代理节点延迟",
            Self::Trace => "系统逐跳探针，部分跳不响应属于常见情况",
            Self::Download => "限时限量测量接收吞吐，样本不代表线路最大带宽",
            Self::Tcp => "经当前代理连接指定端口，不发送应用数据",
            Self::Http => "分析方法、状态和安全响应头；Cookie 和认证头不展示",
        }
    }

    pub fn needs_target(self) -> bool {
        true
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub count: u16,
    pub interval_ms: u64,
    pub seconds: u64,
    pub max_bytes: u64,
    pub max_hops: u8,
    pub record_type: String,
    pub method: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            count: 5,
            interval_ms: 1000,
            seconds: 10,
            max_bytes: 8 * 1024 * 1024,
            max_hops: 8,
            record_type: "ADDR".into(),
            method: "HEAD".into(),
        }
    }
}

impl Options {
    pub fn from_fields(fields: &str) -> Self {
        if fields.is_empty() {
            return Self::default();
        }
        let parse = || -> Result<Self> {
            ensure!(fields.len() <= 128, "检查参数过长");
            let values: Vec<_> = fields.split('|').collect();
            ensure!(matches!(values.len(), 5 | 7), "检查参数格式无效");
            Ok(Self {
                count: values[0].parse()?,
                interval_ms: values[1].parse()?,
                seconds: values[2].parse()?,
                max_bytes: values[3]
                    .parse::<u64>()?
                    .checked_mul(1024 * 1024)
                    .context("下载上限溢出")?,
                max_hops: values[4].parse()?,
                record_type: values.get(5).copied().unwrap_or("ADDR").into(),
                method: values.get(6).copied().unwrap_or("HEAD").into(),
            })
        };
        parse().unwrap_or(Self {
            count: 0,
            ..Self::default()
        })
    }

    fn budget(&self, tool: Tool) -> Duration {
        let seconds = match tool {
            Tool::Stability => 120,
            Tool::Family => self.seconds * 2 + 10,
            Tool::Trace => u64::from(self.max_hops) * 3 + 10,
            Tool::Ping => u64::from(self.count) * 2 + 10,
            Tool::Website | Tool::Dns => 40,
            _ => self.seconds + 10,
        };
        Duration::from_secs(seconds)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_for_language(Language::SimplifiedChinese)
    }

    pub fn validate_for_language(&self, language: Language) -> Result<()> {
        ensure!(
            (1..=20).contains(&self.count),
            "{}",
            language.text("检查次数应为 1～20", "Count must be between 1 and 20")
        );
        ensure!(
            (100..=5000).contains(&self.interval_ms),
            "{}",
            language.text(
                "请求间隔应为 100～5000 毫秒",
                "Interval must be between 100 and 5000 ms"
            )
        );
        ensure!(
            (1..=30).contains(&self.seconds),
            "{}",
            language.text(
                "单次时限应为 1～30 秒",
                "Timeout must be between 1 and 30 seconds"
            )
        );
        ensure!(
            (1024..=16 * 1024 * 1024).contains(&self.max_bytes),
            "{}",
            language.text(
                "下载上限应为 1 KiB～16 MiB",
                "Size limit must be between 1 KiB and 16 MiB"
            )
        );
        ensure!(
            (1..=16).contains(&self.max_hops),
            "{}",
            language.text(
                "最大跳数应为 1～16",
                "Maximum hops must be between 1 and 16"
            )
        );
        ensure!(
            matches!(
                self.record_type.as_str(),
                "ADDR" | "A" | "AAAA" | "CNAME" | "MX" | "TXT" | "NS" | "SOA" | "PTR" | "CAA"
            ),
            "{}",
            language.text("DNS 记录类型无效", "Invalid DNS record type")
        );
        ensure!(
            matches!(self.method.as_str(), "HEAD" | "GET" | "OPTIONS"),
            "{}",
            language.text(
                "HTTP 方法应为 HEAD、GET 或 OPTIONS",
                "HTTP method must be HEAD, GET, or OPTIONS"
            )
        );
        Ok(())
    }
}

#[derive(Clone)]
pub struct Environment {
    pub port: Option<u16>,
    pub api: Option<Api>,
    pub protected: bool,
    pub udp_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub label: String,
    pub detail: String,
    pub success: Option<bool>,
    pub code: &'static str,
}

impl Entry {
    fn new(
        label: &str,
        detail: impl Into<String>,
        success: Option<bool>,
        code: &'static str,
    ) -> Self {
        Self {
            label: label.into(),
            detail: bounded(&detail.into(), 2048),
            success,
            code,
        }
    }

    fn ok(label: &str, detail: impl Into<String>) -> Self {
        Self::new(label, detail, Some(true), "ok")
    }

    fn unknown(label: &str, detail: impl Into<String>, code: &'static str) -> Self {
        Self::new(label, detail, None, code)
    }

    fn error(label: &str, detail: impl Into<String>, code: &'static str) -> Self {
        Self::new(label, detail, Some(false), code)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub schema: u8,
    pub tool: Tool,
    pub target: String,
    pub elapsed_ms: u128,
    pub status: &'static str,
    pub entries: Vec<Entry>,
}

impl Report {
    pub fn lines(&self) -> Vec<String> {
        self.lines_in(Language::SimplifiedChinese)
    }

    pub fn lines_in(&self, language: Language) -> Vec<String> {
        [
            format!(
                "{} · {} · {} ms",
                language.text("检查状态", "Status"),
                match self.status {
                    "complete" => language.text("完成", "Complete"),
                    "partial" => language.text("部分完成", "Partial"),
                    "failed" => language.text("失败", "Failed"),
                    "unavailable" => language.text("不可用或未确认", "Unavailable or unconfirmed"),
                    _ => language.text("未知", "Unknown"),
                },
                self.elapsed_ms
            ),
            format!(
                "{} · {}",
                language.text("目标", "Target"),
                if self.target == "未检查"
                    && self
                        .entries
                        .iter()
                        .any(|entry| matches!(entry.code, "invalid_options" | "invalid_target"))
                {
                    language.text("未检查", "Not checked")
                } else {
                    &self.target
                }
            ),
        ]
        .into_iter()
        .chain(self.entries.iter().map(|entry| {
            format!(
                "{} · {}",
                entry_label_in(&entry.label, language),
                entry_detail_in(&entry.label, &entry.detail, entry.code, language)
            )
        }))
        .collect()
    }

    fn finish(&mut self, start: Instant) {
        self.elapsed_ms = start.elapsed().as_millis();
        self.entries.truncate(48);
        let success = self.entries.iter().any(|entry| entry.success == Some(true));
        let failure = self
            .entries
            .iter()
            .any(|entry| entry.success == Some(false));
        let incomplete = self.entries.iter().any(|entry| {
            entry.success.is_none()
                && !matches!(
                    entry.code,
                    "information"
                        | "not_applicable"
                        | "literal_ip"
                        | "process_output"
                        | "output_truncated"
                        | "byte_limit"
                        | "time_limit"
                        | "completed"
                )
        });
        self.status = match (success, failure) {
            (true, true) => "partial",
            (true, false) if incomplete => "partial",
            (true, false) => "complete",
            (false, true) => "failed",
            (false, false) => "unavailable",
        };
    }
}

pub fn entry_label_in(label: &str, language: Language) -> String {
    text::label(label, language)
}

pub fn entry_detail_in(label: &str, detail: &str, code: &str, language: Language) -> String {
    text::detail(label, detail, code, language)
}

fn bounded(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .take(limit)
        .collect()
}

pub async fn run(
    tool: Tool,
    target: &str,
    context: Environment,
    mut options: Options,
) -> Result<Report> {
    options.record_type.make_ascii_uppercase();
    options.method.make_ascii_uppercase();
    let start = Instant::now();
    let mut report = Report {
        schema: 1,
        tool,
        target: "未检查".into(),
        elapsed_ms: 0,
        status: "unavailable",
        entries: Vec::new(),
    };
    if options.validate().is_err() {
        report.entries.push(Entry::error(
            "检查参数",
            "参数无效，请检查次数、间隔、时限、流量上限、跳数、DNS 类型或 HTTP 方法",
            "invalid_options",
        ));
        report.finish(start);
        return Ok(report);
    }
    let url = match probe::diagnostic_url(target) {
        Ok(url) => url,
        Err(_) => {
            report.entries.push(Entry::error(
                "目标地址",
                "请输入域名、IP 或 HTTP/HTTPS 地址；不支持嵌入账号密码",
                "invalid_target",
            ));
            report.finish(start);
            return Ok(report);
        }
    };
    report.target = url.origin().ascii_serialization();
    if context.protected && tool.restricted() {
        report.entries.push(Entry::unknown(
            "当前工具",
            "Claude 保护开启或状态未知，无法确认探针的受保护出口，未发起请求",
            "protection_restricted",
        ));
    } else if tool == Tool::Udp && !context.udp_enabled {
        report.entries.push(Entry::unknown(
            "UDP 探针",
            "设置中已关闭 UDP，未发送探针",
            "udp_disabled",
        ));
    } else {
        let operation = async {
            match tool {
                Tool::Website => {
                    let result = probe::diagnose(
                        target,
                        context.port,
                        context.api.clone(),
                        context.protected,
                    )
                    .await?;
                    for (label, check) in [
                        ("系统 DNS", result.system_dns),
                        ("内核 DNS", result.core_dns),
                        ("本机代理", result.proxy),
                        ("HTTPS", result.https),
                        ("HTTP", result.http),
                        ("采用协议", result.response),
                        ("实际线路", result.route),
                    ] {
                        report.entries.push(from_check(label, check));
                    }
                    Ok(())
                }
                Tool::Dns => http::dns(&url, &context, &options, &mut report).await,
                Tool::Tcp => transport::tcp(&url, target, &context, &options, &mut report).await,
                Tool::Http => http::inspect(&url, &context, &options, &mut report).await,
                Tool::Stability => http::stability(&url, &context, &options, &mut report).await,
                Tool::Download => http::download(&url, &context, &options, &mut report).await,
                Tool::Phase | Tool::Tls | Tool::Family => {
                    transport::run(tool, &url, &context, &options, &mut report).await
                }
                Tool::Udp => datagram::run(&url, target, &context, &options, &mut report).await,
                Tool::Ping | Tool::Trace => {
                    process::run(tool, &url, &context, &options, &mut report).await
                }
            }
        };
        match tokio::time::timeout(options.budget(tool), operation).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => report.entries.push(error::entry("检查失败", &error)),
            Err(_) => report.entries.push(Entry::error(
                "检查停止",
                "已达到本次检查时限",
                "probe_timeout",
            )),
        }
    }
    report.finish(start);
    Ok(report)
}

fn from_check(label: &str, check: probe::DiagnosticCheck) -> Entry {
    Entry::new(
        label,
        check.detail,
        check.success,
        match check.success {
            Some(true) => "ok",
            Some(false) => "probe_failed",
            None => "not_observed",
        },
    )
}

fn proxy_port(context: &Environment) -> Result<u16> {
    context.port.filter(|port| *port != 0).ok_or_else(|| {
        error::failure(
            "core_unavailable",
            "内核未运行或代理端口不可用；请先启动内核",
        )
    })
}

async fn resolve(url: &reqwest::Url, context: &Environment) -> Result<Vec<std::net::IpAddr>> {
    let host = url
        .host_str()
        .context("目标缺少主机名")?
        .trim_matches(['[', ']']);
    if let Ok(address) = host.parse() {
        return Ok(vec![address]);
    }
    let api = context
        .api
        .as_ref()
        .ok_or_else(|| error::failure("controller_unavailable", "内核解析接口不可用"))?;
    let mut addresses = tokio::time::timeout(Duration::from_secs(5), api.resolve_domain(host))
        .await
        .map_err(|_| error::failure("dns_timeout", "内核解析超时"))?
        .map_err(|_| error::failure("dns_failed", "内核解析失败，请检查内核日志"))?;
    addresses.sort();
    addresses.dedup();
    addresses.truncate(16);
    if addresses.is_empty() {
        return Err(error::failure("dns_no_address", "内核未返回目标地址"));
    }
    Ok(addresses)
}
