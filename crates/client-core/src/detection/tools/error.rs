use super::Entry;
use std::{error::Error, fmt};

#[derive(Debug)]
struct Failure {
    code: &'static str,
    detail: &'static str,
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail)
    }
}

impl Error for Failure {}

pub(super) fn failure(code: &'static str, detail: &'static str) -> anyhow::Error {
    Failure { code, detail }.into()
}

pub(super) fn entry(label: &str, error: &anyhow::Error) -> Entry {
    for cause in error.chain() {
        if let Some(failure) = cause.downcast_ref::<Failure>() {
            return Entry::error(label, failure.detail, failure.code);
        }
        if let Some(http) = cause.downcast_ref::<reqwest::Error>() {
            let (code, detail) = if http.is_timeout() {
                ("http_timeout", "网站请求超时，未自动切换线路")
            } else if http.is_connect() {
                (
                    "http_connect_failed",
                    "无法建立网站连接，请查看同时间内核日志",
                )
            } else if http.is_body() || http.is_decode() {
                ("http_body_failed", "响应正文读取失败")
            } else {
                ("http_failed", "网站请求失败，请查看同时间内核日志")
            };
            return Entry::error(label, detail, code);
        }
        if cause
            .downcast_ref::<tokio_rustls::rustls::Error>()
            .is_some()
        {
            return Entry::error(label, "TLS 握手或证书验证失败", "tls_failed");
        }
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            let (code, detail) = match io.kind() {
                std::io::ErrorKind::PermissionDenied => {
                    ("permission_denied", "系统拒绝此探针的访问权限")
                }
                std::io::ErrorKind::NotFound => ("component_unavailable", "系统未提供此探针组件"),
                std::io::ErrorKind::ConnectionRefused => ("connection_refused", "连接被拒绝"),
                std::io::ErrorKind::TimedOut => ("connection_timeout", "连接超时"),
                std::io::ErrorKind::UnexpectedEof => {
                    ("connection_closed", "对端在响应完成前关闭连接")
                }
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe => {
                    ("connection_interrupted", "连接中断")
                }
                _ => ("probe_io_failed", "探针通信失败，请查看同时间内核日志"),
            };
            return Entry::error(label, detail, code);
        }
    }
    Entry::error(
        label,
        "检查未完成，请导出诊断并附上操作时间",
        "probe_failed",
    )
}
