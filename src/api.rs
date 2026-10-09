use crate::config::Settings;
use anyhow::{Context, Result, bail};
use reqwest::{Client, Method, Url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Read, time::Duration};

const JSON_LIMIT: usize = 32 * 1024 * 1024;

// Backpressure bounds queued response chunks. Dropping the request closes the
// sender, so a cancelled request also wakes the blocking JSON parser.
struct JsonReader {
    receiver: tokio::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>,
    chunk: std::io::Cursor<Vec<u8>>,
}

impl Read for JsonReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            let count = self.chunk.read(output)?;
            if count > 0 {
                return Ok(count);
            }
            match self.receiver.blocking_recv() {
                Some(chunk) => self.chunk = std::io::Cursor::new(chunk?),
                None => return Ok(0),
            }
        }
    }
}

async fn decode_json<T: DeserializeOwned + Send + 'static>(
    mut response: reqwest::Response,
) -> Result<T> {
    if response
        .content_length()
        .is_some_and(|size| size > JSON_LIMIT as u64)
    {
        bail!("mihomo API 响应超过 32 MiB 限制");
    }
    let (sender, receiver) = tokio::sync::mpsc::channel(2);
    let parser = tokio::task::spawn_blocking(move || {
        serde_json::from_reader(std::io::BufReader::with_capacity(
            64 * 1024,
            JsonReader {
                receiver,
                chunk: std::io::Cursor::new(Vec::new()),
            },
        ))
    });
    let mut size = 0;
    'response: loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                size += chunk.len();
                if size > JSON_LIMIT {
                    let _ = sender
                        .send(Err(std::io::Error::other("API 响应超过 32 MiB 限制")))
                        .await;
                    break;
                }
                for bytes in chunk.chunks(16 * 1024) {
                    if sender.send(Ok(bytes.to_vec())).await.is_err() {
                        break 'response;
                    }
                }
            }
            Ok(None) => break,
            Err(error) => {
                let _ = sender.send(Err(std::io::Error::other(error))).await;
                break;
            }
        }
    }
    drop(sender);
    Ok(parser.await??)
}

#[derive(Default, Deserialize)]
pub(crate) struct ConfigStatus {
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub tun: TunStatus,
}

#[derive(Default, Deserialize)]
pub(crate) struct TunStatus {
    #[serde(default)]
    pub enable: bool,
}

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

// The pool belongs to one deserialized document, never to the process lifetime.
// Reader-backed serde can borrow its scratch string here, avoiding temporary
// allocations when a name/type already exists in this snapshot.
thread_local! {
    static TEXT_POOL: std::cell::RefCell<Option<std::collections::HashSet<std::sync::Arc<str>>>> = const { std::cell::RefCell::new(None) };
}
struct TextScope(Option<std::collections::HashSet<std::sync::Arc<str>>>);
impl TextScope {
    fn new() -> Self {
        Self(TEXT_POOL.with(|pool| pool.replace(Some(Default::default()))))
    }
}
impl Drop for TextScope {
    fn drop(&mut self) {
        TEXT_POOL.with(|pool| {
            pool.replace(self.0.take());
        });
    }
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Text(std::sync::Arc<str>);
impl Text {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl From<&str> for Text {
    fn from(value: &str) -> Self {
        TEXT_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            if let Some(pool) = pool.as_mut() {
                if let Some(found) = pool.get(value) {
                    return Self(found.clone());
                }
                let text: std::sync::Arc<str> = value.into();
                pool.insert(text.clone());
                Self(text)
            } else {
                Self(value.into())
            }
        })
    }
}
impl From<String> for Text {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}
impl Default for Text {
    fn default() -> Self {
        Self::from("")
    }
}
impl std::ops::Deref for Text {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
impl std::borrow::Borrow<str> for Text {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}
impl std::fmt::Display for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_str().fmt(f)
    }
}
impl PartialEq<&str> for Text {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
impl Serialize for Text {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self)
    }
}
impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = Text;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string")
            }
            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Text, E> {
                Ok(Text::from(text))
            }
            fn visit_string<E: serde::de::Error>(self, text: String) -> Result<Text, E> {
                Ok(Text::from(text))
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}
fn compact_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let mut values = null_default::<D, Vec<T>>(deserializer)?;
    values.shrink_to_fit();
    Ok(values)
}
fn last_delay<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Option<u32>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a delay array or null")
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut last = None;
            while let Some(delay) = seq.next_element::<Delay>()? {
                last = Some(delay.delay);
            }
            Ok(last)
        }
    }
    deserializer.deserialize_any(Visitor)
}
fn serialize_delay<S: serde::Serializer>(
    delay: &Option<u32>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(usize::from(delay.is_some())))?;
    if let Some(delay) = delay {
        seq.serialize_element(&Delay { delay: *delay })?;
    }
    seq.end()
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Proxy {
    #[serde(rename = "type", default)]
    pub kind: Text,
    #[serde(default)]
    pub now: Text,
    #[serde(default, deserialize_with = "compact_vec")]
    pub all: Vec<Text>,
    #[serde(
        rename = "history",
        default,
        deserialize_with = "last_delay",
        serialize_with = "serialize_delay"
    )]
    pub delay: Option<u32>,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Delay {
    #[serde(default)]
    pub delay: u32,
}
#[derive(Debug, Clone, Default)]
pub struct Proxies {
    pub proxies: BTreeMap<Text, Proxy>,
}
impl<'de> Deserialize<'de> for Proxies {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let _scope = TextScope::new();
        #[derive(Deserialize)]
        struct Document {
            #[serde(default, deserialize_with = "null_default")]
            proxies: BTreeMap<Text, Proxy>,
        }
        Ok(Self {
            proxies: Document::deserialize(deserializer)?.proxies,
        })
    }
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rule {
    #[serde(rename = "type", default)]
    pub kind: Text,
    #[serde(default)]
    pub payload: Box<str>,
    #[serde(default)]
    pub proxy: Text,
}
#[derive(Debug, Clone, Default)]
pub struct Rules {
    pub rules: Vec<Rule>,
}
impl<'de> Deserialize<'de> for Rules {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let _scope = TextScope::new();
        #[derive(Deserialize)]
        struct Document {
            #[serde(default, deserialize_with = "compact_vec")]
            rules: Vec<Rule>,
        }
        Ok(Self {
            rules: Document::deserialize(deserializer)?.rules,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Network {
    Tcp,
    Udp,
    Other(Box<str>),
}
impl Default for Network {
    fn default() -> Self {
        Self::Other(Box::default())
    }
}
impl Network {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Other(value) => value,
        }
    }
}
impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_str().fmt(f)
    }
}
impl<'de> Deserialize<'de> for Network {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = Network;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a network name")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Network, E> {
                Ok(match value {
                    "tcp" => Network::Tcp,
                    "udp" => Network::Udp,
                    _ => Network::Other(value.into()),
                })
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Metadata {
    #[serde(default)]
    pub host: Box<str>,
    #[serde(rename = "destinationIP", default)]
    pub destination_ip: Box<str>,
    #[serde(rename = "destinationPort", default)]
    pub destination_port: Text,
    #[serde(default)]
    pub network: Network,
    #[serde(default)]
    pub process: Text,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Connection {
    pub id: Box<str>,
    #[serde(default)]
    pub metadata: Metadata,
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
    #[serde(default, deserialize_with = "compact_vec")]
    pub chains: Vec<Text>,
    #[serde(default)]
    pub rule: Text,
    #[serde(default)]
    pub start: Box<str>,
}
#[derive(Debug, Clone, Default)]
pub struct Connections {
    pub upload_total: u64,
    pub download_total: u64,
    pub connections: Vec<Connection>,
}
impl<'de> Deserialize<'de> for Connections {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let _scope = TextScope::new();
        #[derive(Deserialize)]
        struct Document {
            #[serde(rename = "uploadTotal", default)]
            upload_total: u64,
            #[serde(rename = "downloadTotal", default)]
            download_total: u64,
            #[serde(default, deserialize_with = "compact_vec")]
            connections: Vec<Connection>,
        }
        let value = Document::deserialize(deserializer)?;
        Ok(Self {
            upload_total: value.upload_total,
            download_total: value.download_total,
            connections: value.connections,
        })
    }
}

/// Home needs counts and traffic totals, without allocating each connection's metadata.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ConnectionStats {
    #[serde(rename = "uploadTotal", default)]
    pub upload_total: u64,
    #[serde(rename = "downloadTotal", default)]
    pub download_total: u64,
    #[serde(
        rename = "connections",
        default,
        deserialize_with = "count_connections"
    )]
    pub count: usize,
}

fn count_connections<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<usize, D::Error> {
    struct Counter;
    impl<'de> serde::de::Visitor<'de> for Counter {
        type Value = usize;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a connection array or null")
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<usize, E> {
            Ok(0)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<usize, A::Error> {
            let mut count = 0;
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                count += 1;
            }
            Ok(count)
        }
    }
    deserializer.deserialize_any(Counter)
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
    pub async fn get<T: DeserializeOwned + Send + 'static>(&self, path: &str) -> Result<T> {
        let response = self
            .request(Method::GET, self.url(&[path])?)
            .send()
            .await
            .context("无法连接 mihomo 控制接口")?;
        decode_json(Self::checked(response).await?).await
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
        #[derive(Serialize)]
        struct Payload<'a> {
            payload: &'a str,
        }
        let mut url = self.url(&["configs"])?;
        url.query_pairs_mut().append_pair("force", "true");
        Self::checked(
            self.request(Method::PUT, url)
                .json(&Payload { payload })
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
        let mut proxies: Proxies = self.get("proxies").await?;
        let members = proxies
            .proxies
            .remove(group)
            .filter(|p| !p.all.is_empty())
            .ok_or_else(|| anyhow::anyhow!("策略组不存在或没有节点"))?
            .all;
        drop(proxies);
        // Individual delay requests preserve pinned automatic-group selections.
        Ok(stream::iter(members.into_iter().map(|name| async move {
            let delay = self.delay(&name).await.unwrap_or(0);
            (name.to_string(), delay)
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
                proxies.proxies.get(name.as_str()).is_some_and(|proxy| {
                    proxy.all.is_empty()
                        && !matches!(
                            proxy.kind.as_str(),
                            "Direct" | "Reject" | "Compatible" | "Pass"
                        )
                })
            })
            .cloned()
            .collect();
        drop(proxies);
        Ok(stream::iter(names.into_iter().map(|name| async move {
            let delay = self.delay(&name).await.unwrap_or(0);
            (name.to_string(), delay)
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
    #[test]
    fn snapshots_share_repeated_text_and_release_the_intern_pool() {
        let connections: Connections = serde_json::from_str(r#"{"connections":[{"id":"1","metadata":{"network":"tcp","process":"browser"},"chains":["Auto","node"],"rule":"Domain"},{"id":"2","metadata":{"network":"udp","process":"browser"},"chains":["Auto","node"],"rule":"Domain"}]}"#).unwrap();
        let [a, b] = connections.connections.as_slice() else {
            panic!("missing connections")
        };
        assert!(std::sync::Arc::ptr_eq(
            &a.metadata.process.0,
            &b.metadata.process.0
        ));
        assert!(std::sync::Arc::ptr_eq(&a.chains[0].0, &b.chains[0].0));
        assert_eq!(a.metadata.network, Network::Tcp);
        assert_eq!(b.metadata.network, Network::Udp);
        assert!(TEXT_POOL.with(|pool| pool.borrow().is_none()));
        assert!(serde_json::from_str::<Connections>(r#"{"connections":[{"id":123}]}"#).is_err());
        assert!(TEXT_POOL.with(|pool| pool.borrow().is_none()));
        let proxies: Proxies = serde_json::from_str(r#"{"proxies":{"node":{"name":"ignored","type":"HTTP","history":[{"delay":42},{"delay":0}]},"group":{"type":"Selector","now":"node","all":["node"]}}}"#).unwrap();
        let key = proxies.proxies.get_key_value("node").unwrap().0;
        let group = &proxies.proxies["group"];
        assert!(std::sync::Arc::ptr_eq(&key.0, &group.all[0].0));
        assert!(std::sync::Arc::ptr_eq(&key.0, &group.now.0));
        assert_eq!(proxies.proxies["node"].delay, Some(0));
    }

    async fn mock_response(wire: Vec<u8>) -> (reqwest::Response, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            // Deliberately split UTF-8 and JSON tokens across HTTP body chunks.
            for chunk in wire.chunks(113) {
                if socket.write_all(chunk).await.is_err() {
                    break;
                }
            }
        });
        let response = Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/"))
            .send()
            .await
            .unwrap();
        (response, server)
    }

    #[tokio::test]
    async fn streamed_json_accepts_large_payloads_and_rejects_truncation() {
        let json = format!(
            r#"{{"rules":[{}]}}"#,
            vec![r#"{"type":"域名","payload":"example.com","proxy":"DIRECT"}"#; 5000].join(",")
        );
        let wire = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
            json.len()
        )
        .into_bytes();
        let (response, server) = mock_response(wire).await;
        let rules: Rules = decode_json(response).await.unwrap();
        assert_eq!(rules.rules.len(), 5000);
        assert_eq!(rules.rules[4999].kind, "域名");
        server.await.unwrap();
        for body in [r#"{"rules":["#, r#"{"rules":[]} trailing"#] {
            let wire = format!("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{body}").into_bytes();
            let (response, server) = mock_response(wire).await;
            assert!(decode_json::<Rules>(response).await.is_err());
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn streamed_json_enforces_limits_with_and_without_content_length() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let wire = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            JSON_LIMIT + 1
        )
        .into_bytes();
        let (response, server) = mock_response(wire).await;
        assert!(
            decode_json::<Value>(response)
                .await
                .unwrap_err()
                .to_string()
                .contains("32 MiB")
        );
        server.await.unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert!(socket.read(&mut [0; 4096]).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{\"ignored\":\"")
                .await
                .unwrap();
            for _ in 0..=JSON_LIMIT / 16384 {
                if socket.write_all(&[b'x'; 16384]).await.is_err() {
                    break;
                }
            }
        });
        let response = Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/"))
            .send()
            .await
            .unwrap();
        assert!(
            decode_json::<ConnectionStats>(response)
                .await
                .unwrap_err()
                .to_string()
                .contains("32 MiB")
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn closing_sender_wakes_a_waiting_json_reader() {
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        let parser = tokio::task::spawn_blocking(move || {
            let mut reader = JsonReader {
                receiver,
                chunk: std::io::Cursor::new(Vec::new()),
            };
            reader.read(&mut [0; 1]).unwrap()
        });
        drop(sender);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), parser)
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }

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
        assert_eq!(proxy.delay, None);
        assert!(proxy.all.is_empty());
    }
    #[test]
    fn home_stats_count_connections_without_metadata() {
        let stats: ConnectionStats = serde_json::from_str(
            r#"{"uploadTotal":42,"downloadTotal":73,"connections":[{"metadata":{"host":"a.example"},"chains":["node"]},{"metadata":{"host":"b.example"}}]}"#,
        ).unwrap();
        assert_eq!(
            (stats.count, stats.upload_total, stats.download_total),
            (2, 42, 73)
        );
        for json in [r#"{"connections":null}"#, r#"{"connections":[]}"#, "{}"] {
            assert_eq!(
                serde_json::from_str::<ConnectionStats>(json).unwrap().count,
                0
            );
        }
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
