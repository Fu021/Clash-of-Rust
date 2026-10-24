//! Automatic routes shared by update discovery, installers and subscriptions.
use anyhow::Result;
use reqwest::ClientBuilder;

/// Preserve the bundled public roots on every host, including Android without a Java verifier.
pub fn client_builder() -> ClientBuilder {
    reqwest::Client::builder().tls_certs_only(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().map(
        |cert| {
            reqwest::Certificate::from_der(cert.as_ref()).expect("bundled root certificate is DER")
        },
    ))
}

#[derive(Clone)]
pub struct Route {
    pub label: &'static str,
    pub proxy: Option<String>,
}

pub fn routes(port: Option<u16>, system: Option<String>) -> Vec<Route> {
    let mut routes = Vec::new();
    for (label, proxy) in [
        (
            "内核代理",
            port.map(|port| format!("http://127.0.0.1:{port}")),
        ),
        ("系统代理", system),
    ] {
        if let Some(proxy) = proxy
            && !routes
                .iter()
                .any(|route: &Route| route.proxy.as_ref() == Some(&proxy))
        {
            routes.push(Route {
                label,
                proxy: Some(proxy),
            });
        }
    }
    routes.push(Route {
        label: "直连",
        proxy: None,
    });
    routes
}

impl Route {
    pub fn apply(&self, builder: ClientBuilder) -> Result<ClientBuilder> {
        let builder = builder.no_proxy();
        Ok(match &self.proxy {
            Some(proxy) => builder.proxy(reqwest::Proxy::all(proxy)?),
            None => builder,
        })
    }
}

pub fn request_error(error: reqwest::Error, target: &str) -> anyhow::Error {
    use std::error::Error;
    let mut detail = error.to_string();
    let mut source = error.source();
    while let Some(error) = source {
        detail.push_str(&error.to_string());
        source = error.source();
    }
    let detail = detail.to_ascii_lowercase();
    let reason = if error.is_timeout() {
        "连接超时"
    } else if detail.contains("certificate") || detail.contains("cert verify") {
        "证书校验失败，请检查系统时间或代理证书"
    } else if detail.contains("dns") || detail.contains("lookup address") {
        "域名解析失败"
    } else if detail.contains("tunnel") || detail.contains("proxy") {
        "代理连接失败"
    } else {
        "网络连接失败"
    };
    anyhow::Error::new(error).context(format!("{target}{reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owned_system_proxy_is_only_attempted_once_before_direct() {
        let selected = routes(Some(7897), Some("http://127.0.0.1:7897".into()));
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].proxy.as_deref(), Some("http://127.0.0.1:7897"));
        assert!(selected[1].proxy.is_none());
    }
}
