use crate::config::Settings;
use anyhow::{Context, Result, bail};
use reqwest::{Client, Method, Url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};

// Go serializes nil slices as null, rather than []. Both are valid API responses.
fn null_default<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Clone)]
pub struct Api {
    pub client: Client,
    base: Url,
    secret: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Proxy {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub now: String,
    #[serde(default, deserialize_with = "null_default")]
    pub all: Vec<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub history: Vec<Delay>,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Delay {
    #[serde(default)]
    pub delay: u32,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Proxies {
    #[serde(default, deserialize_with = "null_default")]
    pub proxies: BTreeMap<String, Proxy>,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rule {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub payload: String,
    #[serde(default)]
    pub proxy: String,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rules {
    #[serde(default, deserialize_with = "null_default")]
    pub rules: Vec<Rule>,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Metadata {
    #[serde(default)]
    pub host: String,
    #[serde(rename = "destinationIP", default)]
    pub destination_ip: String,
    #[serde(rename = "destinationPort", default)]
    pub destination_port: String,
    #[serde(default)]
    pub network: String,
    #[serde(default)]
    pub process: String,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Connection {
    pub id: String,
    #[serde(default)]
    pub metadata: Metadata,
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
    #[serde(default, deserialize_with = "null_default")]
    pub chains: Vec<String>,
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub start: String,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Connections {
    #[serde(rename = "uploadTotal", default)]
    pub upload_total: u64,
    #[serde(rename = "downloadTotal", default)]
    pub download_total: u64,
    #[serde(default, deserialize_with = "null_default")]
    pub connections: Vec<Connection>,
}

impl Api {
    pub fn new(settings: &Settings) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(8))
                .build()?,
            base: Url::parse(&format!("http://127.0.0.1:{}/", settings.controller_port))?,
            secret: settings.secret.clone(),
        })
    }
    pub fn url(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("无效 API URL"))?
            .clear()
            .extend(segments);
        Ok(url)
    }
    pub fn request(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        self.client.request(method, url).bearer_auth(&self.secret)
    }
    async fn checked(response: reqwest::Response) -> Result<reqwest::Response> {
        let status = response.status();
        if !status.is_success() {
            // Do not include arbitrary server bodies or subscription URLs in diagnostics.
            bail!("mihomo API 返回 {status}");
        }
        Ok(response)
    }
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self
            .request(Method::GET, self.url(&[path])?)
            .send()
            .await
            .context("无法连接 mihomo 控制接口")?;
        Ok(Self::checked(response).await?.json().await?)
    }
    pub async fn patch(&self, body: Value) -> Result<()> {
        Self::checked(
            self.request(Method::PATCH, self.url(&["configs"])?)
                .json(&body)
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }
    pub async fn reload(&self, payload: &str) -> Result<()> {
        let mut url = self.url(&["configs"])?;
        url.query_pairs_mut().append_pair("force", "true");
        Self::checked(
            self.request(Method::PUT, url)
                .json(&json!({"payload": payload}))
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }
    pub async fn select(&self, group: &str, name: &str) -> Result<()> {
        Self::checked(
            self.request(Method::PUT, self.url(&["proxies", group])?)
                .json(&json!({"name": name}))
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }
    pub async fn delay(&self, name: &str) -> Result<u32> {
        let mut url = self.url(&["proxies", name, "delay"])?;
        url.query_pairs_mut()
            .append_pair("url", "https://www.gstatic.com/generate_204")
            .append_pair("timeout", "5000");
        let response = Self::checked(self.request(Method::GET, url).send().await?).await?;
        Ok(response.json::<Delay>().await?.delay)
    }

    pub async fn delay_group(&self, group: &str) -> Result<BTreeMap<String, u32>> {
        use futures_util::{StreamExt, stream};
        let proxies: Proxies = self.get("proxies").await?;
        let members = proxies
            .proxies
            .get(group)
            .filter(|p| !p.all.is_empty())
            .ok_or_else(|| anyhow::anyhow!("策略组不存在或没有节点"))?
            .all
            .clone();
        // Individual delay requests preserve pinned automatic-group selections.
        Ok(stream::iter(members.into_iter().map(|name| async move {
            let delay = self.delay(&name).await.unwrap_or(0);
            (name, delay)
        }))
        .buffer_unordered(6)
        .collect()
        .await)
    }

    pub async fn delay_all_nodes(&self) -> Result<BTreeMap<String, u32>> {
        use futures_util::{StreamExt, stream};
        let proxies: Proxies = self.get("proxies").await?;
        let names: std::collections::BTreeSet<_> = proxies
            .proxies
            .values()
            .flat_map(|proxy| proxy.all.iter())
            .filter(|name| {
                proxies.proxies.get(*name).is_some_and(|proxy| {
                    proxy.all.is_empty()
                        && !matches!(
                            proxy.kind.as_str(),
                            "Direct" | "Reject" | "Compatible" | "Pass"
                        )
                })
            })
            .cloned()
            .collect();
        Ok(stream::iter(names.into_iter().map(|name| async move {
            let delay = self.delay(&name).await.unwrap_or(0);
            (name, delay)
        }))
        .buffer_unordered(6)
        .collect()
        .await)
    }
    pub async fn close_connection(&self, id: &str) -> Result<()> {
        Self::checked(
            self.request(Method::DELETE, self.url(&["connections", id])?)
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }
    pub async fn logs(&self) -> Result<reqwest::Response> {
        let mut url = self.url(&["logs"])?;
        url.query_pairs_mut().append_pair("level", "info");
        let client = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .build()?;
        Self::checked(client.get(url).bearer_auth(&self.secret).send().await?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn group_delay_tests_every_member_and_keeps_failures() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let mut tasks = Vec::new();
            for _ in 0..10 {
                let (mut socket, _) = listener.accept().await.unwrap();
                tasks.push(tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0; 4096];
                    while !request.windows(4).any(|s| s == b"\r\n\r\n") {
                        let n = socket.read(&mut buffer).await.unwrap();
                        assert!(n > 0);
                        request.extend_from_slice(&buffer[..n]);
                    }
                    let request = String::from_utf8(request).unwrap();
                    assert!(request.to_lowercase().contains("authorization: bearer group-secret"));
                    let (status, body) = if request.starts_with("GET /proxies HTTP") {
                        ("200 OK", r#"{"proxies":{"test":{"all":["n0","n1","n2","n3","n4","n5","n6","n7","bad"]}}}"#)
                    } else {
                        assert!(request.contains("/delay?url="));
                        assert!(request.contains("timeout=5000"));
                        if request.starts_with("GET /proxies/bad/") { ("504 Gateway Timeout", "{}") } else { ("200 OK", r#"{"delay":42}"#) }
                    };
                    socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                }));
            }
            for task in tasks {
                task.await.unwrap();
            }
        });
        let api = Api::new(&Settings {
            controller_port: port,
            secret: "group-secret".into(),
            ..Default::default()
        })
        .unwrap();
        let delays = api.delay_group("test").await.unwrap();
        assert_eq!(delays.len(), 9);
        assert_eq!(delays["bad"], 0);
        assert_eq!(delays.values().filter(|v| **v == 42).count(), 8);
        server.await.unwrap();
    }
    #[test]
    fn accepts_go_nil_slices_in_idle_core() {
        let connections: Connections =
            serde_json::from_str(r#"{"uploadTotal":0,"downloadTotal":0,"connections":null}"#)
                .unwrap();
        assert!(connections.connections.is_empty());
        let proxy: Proxy =
            serde_json::from_str(r#"{"name":"DIRECT","type":"Direct","history":null,"all":null}"#)
                .unwrap();
        assert!(proxy.history.is_empty());
        assert!(proxy.all.is_empty());
    }

    #[tokio::test]
    async fn scheduled_delay_deduplicates_real_nodes_and_does_not_select() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0; 4096];
                let length = socket.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..length]).to_string();
                assert!(request.starts_with("GET "));
                let (status, body) = if request.starts_with("GET /proxies HTTP") {
                    (
                        "200 OK",
                        r#"{"proxies":{"Auto":{"type":"URLTest","now":"a","all":["a","b","DIRECT"]},"Manual":{"type":"Selector","all":["Auto","a","b"]},"a":{"type":"Shadowsocks","all":[]},"b":{"type":"Trojan","all":[]},"DIRECT":{"type":"Direct","all":[]}}}"#,
                    )
                } else if request.starts_with("GET /proxies/a/delay?") {
                    ("200 OK", r#"{"delay":99}"#)
                } else {
                    assert!(request.starts_with("GET /proxies/b/delay?"));
                    ("504 Gateway Timeout", "{}")
                };
                socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                requests.push(request);
            }
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request.starts_with("GET /proxies/a/"))
                    .count(),
                1
            );
        });
        let api = Api::new(&Settings {
            controller_port: port,
            ..Settings::default()
        })
        .unwrap();
        let delays = api.delay_all_nodes().await.unwrap();
        assert_eq!(delays.len(), 2);
        assert_eq!(delays["a"], 99);
        assert_eq!(delays["b"], 0);
        server.await.unwrap();
    }
    #[test]
    fn names_are_single_encoded_path_segments() {
        let api = Api::new(&Settings::default()).unwrap();
        let url = api.url(&["proxies", "香港 / #1?", "delay"]).unwrap();
        assert!(url.as_str().contains("%2F"));
        assert!(url.query().is_none());
        assert!(url.fragment().is_none());
    }
    #[tokio::test]
    async fn controller_auth_and_patch_contract() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = stream.read(&mut buffer).await.unwrap();
                bytes.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&bytes);
                if let Some(end) = text.find("\r\n\r\n") {
                    let size = text[..end]
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|s| s.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            let text = String::from_utf8(bytes).unwrap();
            assert!(text.starts_with("PATCH /configs HTTP/1.1"));
            assert!(
                text.to_lowercase()
                    .contains("authorization: bearer test-secret")
            );
            assert!(text.contains("\"mode\":\"global\""));
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let settings = Settings {
            controller_port: port,
            secret: "test-secret".into(),
            ..Settings::default()
        };
        Api::new(&settings)
            .unwrap()
            .patch(json!({"mode":"global"}))
            .await
            .unwrap();
        server.await.unwrap();
    }
}
