//! Attribute the download's own TCP socket; a selected group is not evidence
//! that a request actually used that node. Ignore other GitHub/browser traffic.
use crate::api::{Api, ConfigStatus};
use serde::{
    Deserialize,
    de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor},
};
use std::{
    fmt,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

type SharedOutbound = Arc<Mutex<Option<Arc<str>>>>;
pub struct Observer {
    outbound: SharedOutbound,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Observer {
    pub fn new(response: &reqwest::Response, api: Option<Api>, direct: bool) -> Self {
        let outbound = Arc::new(Mutex::new(None));
        let mut observer = Self {
            outbound: outbound.clone(),
            task: None,
        };
        let Some(api) = api else {
            if direct {
                *outbound.lock().unwrap() = Some(Arc::from("DIRECT"));
            }
            return observer;
        };
        let local = response
            .extensions()
            .get::<hyper_util::client::legacy::connect::HttpInfo>()
            .map(|info| info.local_addr());
        let host: Arc<str> = response.url().host_str().unwrap_or_default().into();
        observer.task = Some(tokio::spawn(async move {
            // A proxy-free request can still be captured by our TUN. Call it
            // DIRECT only after checking the real core configuration.
            if direct
                && let Ok(Ok(config)) = tokio::time::timeout(
                    Duration::from_millis(500),
                    api.get::<ConfigStatus>("configs"),
                )
                .await
                && !config.tun.enable
            {
                *outbound.lock().unwrap() = Some(Arc::from("DIRECT"));
                return;
            }
            let Some(local) = local else {
                return;
            };
            for _ in 0..5 {
                let host = host.clone();
                let lookup = api.get_decoded("connections", move |reader| {
                    let mut json = serde_json::Deserializer::from_reader(reader);
                    let result = Match { local, host: &host }.deserialize(&mut json)?;
                    json.end()?;
                    Ok(result)
                });
                if let Ok(Ok(Some(chain))) =
                    tokio::time::timeout(Duration::from_millis(500), lookup).await
                {
                    *outbound.lock().unwrap() = Some(chain);
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }));
        observer
    }
    pub fn outbound(&self) -> Option<Arc<str>> {
        self.outbound.lock().unwrap().clone()
    }
}
impl Drop for Observer {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[derive(Deserialize, Default)]
struct Metadata {
    #[serde(rename = "sourceIP", default)]
    ip: String,
    #[serde(rename = "sourcePort", default)]
    port: Port,
    #[serde(default)]
    host: String,
    #[serde(rename = "sniffHost", default)]
    sniff: String,
    #[serde(default)]
    network: String,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Port {
    Text(String),
    Number(u16),
}
impl Default for Port {
    fn default() -> Self {
        Self::Number(0)
    }
}
impl Port {
    fn number(&self) -> u16 {
        match self {
            Self::Text(s) => s.parse().unwrap_or(0),
            Self::Number(n) => *n,
        }
    }
}
#[derive(Deserialize)]
struct Row {
    #[serde(default)]
    metadata: Metadata,
    #[serde(default)]
    chains: Option<Vec<String>>,
}
#[derive(Clone, Copy)]
struct Match<'a> {
    local: SocketAddr,
    host: &'a str,
}
impl Match<'_> {
    fn accepts(self, row: &Row) -> bool {
        row.metadata.port.number() == self.local.port()
            && row
                .metadata
                .ip
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.to_canonical() == self.local.ip().to_canonical())
            && row.metadata.network == "tcp"
            && (row.metadata.host.eq_ignore_ascii_case(self.host)
                || row.metadata.sniff.eq_ignore_ascii_case(self.host)
                || (row.metadata.host.is_empty() && row.metadata.sniff.is_empty()))
            && row.chains.as_ref().is_some_and(|chains| {
                !chains.is_empty()
                    && chains.len() <= 16
                    && chains
                        .iter()
                        .all(|name| !name.is_empty() && name.len() <= 512)
            })
    }
}
impl<'de> DeserializeSeed<'de> for Match<'_> {
    type Value = Option<Arc<str>>;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for Match<'_> {
    type Value = Option<Arc<str>>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a connections document")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut result = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == "connections" {
                result = map.next_value_seed(Rows(self))?;
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(result)
    }
}
struct Rows<'a>(Match<'a>);
impl<'de> DeserializeSeed<'de> for Rows<'_> {
    type Value = Option<Arc<str>>;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Rows<'_> {
    type Value = Option<Arc<str>>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a connection list or null")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {
        let mut result = None;
        let mut ambiguous = false;
        while let Some(row) = seq.next_element::<Row>()? {
            if self.0.accepts(&row) {
                if result.is_some() {
                    ambiguous = true;
                } else {
                    let mut chain = row.chains.unwrap();
                    result = Some(if chain.first().is_some_and(|name| name == "DIRECT") {
                        Arc::from("DIRECT")
                    } else {
                        // Match the connection page: group → ... → actual node.
                        chain.reverse();
                        Arc::from(chain.join(" → "))
                    });
                }
            }
        }
        Ok(if ambiguous { None } else { result })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn response_socket_metadata_is_matched_against_the_authenticated_controller() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = origin.local_addr().unwrap();
        let controller = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = Api::new(&crate::config::Settings {
            controller_port: controller.local_addr().unwrap().port(),
            secret: "download-test-secret".into(),
            ..Default::default()
        })
        .unwrap();
        let (source, received) = tokio::sync::oneshot::channel();
        let (done, wait) = tokio::sync::oneshot::channel();
        let origin_task = tokio::spawn(async move {
            let (mut socket, peer) = origin.accept().await.unwrap();
            let mut request = [0; 8192];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            source.send(peer).unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nx")
                .await
                .unwrap();
            let _ = wait.await;
        });
        let controller_task = tokio::spawn(async move {
            let (mut socket, _) = controller.accept().await.unwrap();
            let mut request = [0; 8192];
            let mut count = 0;
            while !request[..count].ends_with(b"\r\n\r\n") {
                let n = socket.read(&mut request[count..]).await.unwrap();
                assert!(n > 0);
                count += n;
            }
            let request = String::from_utf8_lossy(&request[..count]).to_ascii_lowercase();
            assert!(request.starts_with("get /connections "));
            assert!(request.contains("authorization: bearer download-test-secret"));
            let source = received.await.unwrap();
            let body = serde_json::json!({"connections":[{"metadata":{
                "sourceIP":source.ip().to_string(), "sourcePort":source.port().to_string(),
                "network":"tcp","host":"127.0.0.1"},"chains":["真实节点","Auto"]}]})
            .to_string();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/update"))
            .send()
            .await
            .unwrap();
        let observer = Observer::new(&response, Some(api), false);
        tokio::time::timeout(Duration::from_secs(3), async {
            while observer.outbound().is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(observer.outbound().as_deref(), Some("Auto → 真实节点"));
        done.send(()).unwrap();
        origin_task.await.unwrap();
        controller_task.await.unwrap();
    }
    fn matched(value: serde_json::Value) -> Option<Arc<str>> {
        Match {
            local: "127.0.0.1:32100".parse().unwrap(),
            host: "release-assets.githubusercontent.com",
        }
        .deserialize(value)
        .unwrap()
    }
    #[test]
    fn actual_download_socket_wins_over_current_selection_and_browser_requests() {
        let row = |port, host, chain| serde_json::json!({"metadata":{"sourceIP":"::ffff:127.0.0.1","sourcePort":port,"network":"tcp","host":host},"chains":[chain,"Auto"]});
        let own = row("32100", "release-assets.githubusercontent.com", "真实节点");
        let browser = row(
            "32101",
            "release-assets.githubusercontent.com",
            "浏览器节点",
        );
        let other_host = row("32100", "example.com", "其他请求");
        let result = matched(serde_json::json!({"connections":[browser,other_host,own.clone()]}));
        assert_eq!(result.as_deref(), Some("Auto → 真实节点"));
        let mut direct = own.clone();
        direct["chains"] = serde_json::json!(["DIRECT"]);
        assert_eq!(
            matched(serde_json::json!({"connections":[direct]})).as_deref(),
            Some("DIRECT")
        );
        assert!(matched(serde_json::json!({"connections":[own.clone(),own]})).is_none());
        assert!(matched(serde_json::json!({"connections":null})).is_none());
    }
}
