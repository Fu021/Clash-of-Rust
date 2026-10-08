//! Release discovery tolerates unavailable API domains and stale proxy routes.
use super::*;
#[cfg(test)]
use crate::network::routes;
use crate::network::{Route, available_routes, request_error};

const FEED_URL: &str = "https://github.com/Fu021/Clash-of-Rust/releases.atom";
const ATOM: &str = "http://www.w3.org/2005/Atom";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);

fn client(route: &Route) -> Result<Client> {
    let builder = Client::builder()
        .no_proxy()
        .user_agent(format!("Clash-of-Rust/{}", crate::VERSION))
        .connect_timeout(Duration::from_secs(5))
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    route
        .apply(builder)?
        .build()
        .context("无法创建更新检查连接")
}

pub(super) async fn check(port: Option<u16>) -> Result<Option<Available>> {
    // Reading settings does not change the desktop proxy, and an unavailable
    // desktop settings service must not prevent the other routes from working.
    let routes = available_routes(port, "https");
    let (mut available, feed_route) = tokio::time::timeout(
        Duration::from_secs(60),
        discover(&routes, API_URL, FEED_URL, crate::VERSION),
    )
    .await
    .context("更新检查超时，请检查代理或稍后重试")??;
    if let (Some(available), Some(route)) = (&mut available, feed_route) {
        // Asset discovery has its own deadline: a detected release must remain
        // visible even if its assets are absent or temporarily unreachable.
        available.package = tokio::time::timeout(REQUEST_TIMEOUT, feed_package(available, &route))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
    }
    Ok(available)
}

async fn discover(
    routes: &[Route],
    api: &str,
    feed: &str,
    current: &str,
) -> Result<(Option<Available>, Option<Route>)> {
    let mut api_error = None;
    for route in routes {
        let result = match client(route) {
            Ok(client) => fetch(&client, api, current).await,
            Err(error) => Err(error),
        };
        match result {
            Ok(available) => return Ok((available, None)),
            Err(error) => {
                if api_error.is_none() {
                    api_error = Some(format!("{}：{error}", route.label));
                }
            }
        }
    }
    let mut feed_error = None;
    for route in routes {
        let result = async {
            let response = client(route)?
                .get(feed)
                .send()
                .await
                .map_err(|error| request_error(error, "Release 订阅"))?;
            feed_release(&release_bytes(response).await?, current)
        }
        .await;
        match result {
            Ok(available) => return Ok((available, Some(route.clone()))),
            Err(error) => {
                if feed_error.is_none() {
                    feed_error = Some(error.to_string());
                }
            }
        }
    }
    bail!(
        "{}；{}（已尝试可用代理和直连）",
        api_error.unwrap_or_default(),
        feed_error.unwrap_or_default()
    );
}

fn feed_release(bytes: &[u8], current: &str) -> Result<Option<Available>> {
    let text = std::str::from_utf8(bytes).context("Release 订阅格式无效")?;
    let document = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 20_000,
        },
    )
    .context("Release 订阅格式无效")?;
    if !document.root_element().has_tag_name((ATOM, "feed")) {
        bail!("Release 订阅格式无效");
    }
    let prefix = format!("{RELEASES_URL}/tag/");
    let releases = document
        .root_element()
        .children()
        .filter(|node| node.has_tag_name((ATOM, "entry")))
        .filter_map(|entry| {
            let link = entry.children().find(|node| {
                node.has_tag_name((ATOM, "link")) && node.attribute("rel") == Some("alternate")
            })?;
            let tag = link.attribute("href")?.strip_prefix(&prefix)?;
            let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
            Some(Release {
                tag_name: tag.to_owned(),
                draft: false,
                prerelease: !version.pre.is_empty(),
                assets: Vec::new(),
            })
        })
        .collect();
    newest(releases, current)
}

async fn feed_package(available: &Available, route: &Route) -> Result<Option<Package>> {
    let Some(name) = package_name(&available.version) else {
        return Ok(None);
    };
    let tag = available
        .url
        .strip_prefix(&format!("{RELEASES_URL}/tag/"))
        .context("Release 地址无效")?;
    let base = format!("{RELEASES_URL}/download/{tag}/{name}");
    let client = download_client_via(route.proxy.as_deref(), REQUEST_TIMEOUT)?;
    let size = asset_size(&client, &base, PACKAGE_LIMIT).await?;
    let sidecar = format!("{base}.sha256");
    asset_size(&client, &sidecar, 1024).await?;
    Ok(Some(Package {
        asset: Asset {
            name,
            size,
            digest: None,
        },
        url: base,
        checksum_url: sidecar,
    }))
}

async fn asset_size(client: &Client, url: &str, limit: u64) -> Result<u64> {
    // Read one byte (or only headers when a server ignores Range), never the
    // installer body. Redirects retain the download client's host restrictions.
    let response = client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .await
        .map_err(|error| request_error(error, "安装包信息"))?;
    let size = match response.status() {
        StatusCode::OK => response.content_length(),
        StatusCode::PARTIAL_CONTENT => response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("bytes 0-0/"))
            .and_then(|total| total.parse().ok()),
        status => bail!("安装包信息返回 HTTP {status}"),
    }
    .context("无法读取安装包大小")?;
    if size == 0 || size > limit {
        bail!("安装包大小无效");
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn server(
        status: &str,
        body: &[u8],
        extra: &str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let wire = [
            format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}\r\n",
                body.len()
            )
            .as_bytes(),
            body,
        ]
        .concat();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 8192];
            let count = socket.read(&mut request).await.unwrap();
            socket.write_all(&wire).await.unwrap();
            String::from_utf8_lossy(&request[..count]).into_owned()
        });
        (address, task)
    }

    fn feed(link: &str) -> String {
        format!(
            r#"<feed xmlns="{ATOM}"><entry><link rel="alternate" href="{link}"/></entry></feed>"#
        )
    }

    #[test]
    fn feed_uses_only_this_repositories_semantic_release_tags() {
        let mut xml = feed(&format!("{RELEASES_URL}/tag/v0.4.10-beta.1"));
        xml = xml.replace("</feed>", &format!(r#"<entry><link rel="alternate" href="{RELEASES_URL}/tag/v0.4.9"/></entry><entry><link rel="alternate" href="https://evil.invalid/releases/tag/v99.0.0"/></entry></feed>"#));
        let available = feed_release(xml.as_bytes(), "0.4.7").unwrap().unwrap();
        assert_eq!(available.version, "0.4.10-beta.1");
        assert!(available.preview);
        assert!(available.package.is_none());
        assert!(feed_release(xml.as_bytes(), "0.4.10").unwrap().is_none());
        for xml in [
            "<html>challenge</html>",
            "<feed/>",
            "<!DOCTYPE feed [<!ENTITY a 'unsafe'>]><feed xmlns='http://www.w3.org/2005/Atom'>&a;</feed>",
        ] {
            assert!(feed_release(xml.as_bytes(), "0.4.7").is_err());
        }
    }

    #[tokio::test]
    async fn unavailable_core_falls_back_to_system_proxy_without_origin_dns() {
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed_port = closed.local_addr().unwrap().port();
        drop(closed);
        let body = br#"[{"tag_name":"v0.4.8","draft":false,"prerelease":false}]"#;
        let (proxy, task) = server("200 OK", body, "").await;
        let route_list = routes(Some(closed_port), Some(proxy));
        let (available, feed_route) = discover(
            &route_list,
            "http://api.invalid/releases",
            "http://feed.invalid/",
            "0.4.7",
        )
        .await
        .unwrap();
        assert_eq!(available.unwrap().version, "0.4.8");
        assert!(feed_route.is_none());
        assert!(
            task.await
                .unwrap()
                .starts_with("GET http://api.invalid/releases HTTP/1.1")
        );
    }

    #[tokio::test]
    async fn blocked_api_uses_release_feed_and_reports_both_errors_if_unavailable() {
        let route_list = routes(None, None);
        let (api, api_task) = server("403 Forbidden", b"", "").await;
        let body = feed(&format!("{RELEASES_URL}/tag/v0.4.8"));
        let (feed, feed_task) = server("200 OK", body.as_bytes(), "").await;
        let (available, feed_route) = discover(&route_list, &api, &feed, "0.4.7").await.unwrap();
        assert_eq!(available.unwrap().version, "0.4.8");
        assert!(feed_route.unwrap().proxy.is_none());
        api_task.await.unwrap();
        feed_task.await.unwrap();

        let (api, api_task) = server("429 Too Many Requests", b"", "").await;
        let (feed, feed_task) = server("502 Bad Gateway", b"", "").await;
        let error = discover(&route_list, &api, &feed, "0.4.7")
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("限制请求") && error.contains("502"));
        api_task.await.unwrap();
        feed_task.await.unwrap();
    }

    #[tokio::test]
    async fn asset_probe_uses_range_total_and_rejects_missing_or_oversized_files() {
        let client = download_client_via(None, REQUEST_TIMEOUT).unwrap();
        for (status, extra, expected) in [
            (
                "206 Partial Content",
                "Content-Range: bytes 0-0/12345\r\n",
                Some(12345),
            ),
            ("200 OK", "", Some(1)),
            (
                "206 Partial Content",
                "Content-Range: bytes 0-0/*\r\n",
                None,
            ),
            (
                "206 Partial Content",
                "Content-Range: bytes 0-0/999999999\r\n",
                None,
            ),
            ("404 Not Found", "", None),
        ] {
            let (url, task) = server(status, b"x", extra).await;
            let result = asset_size(&client, &url, PACKAGE_LIMIT).await;
            match expected {
                Some(size) => assert_eq!(result.unwrap(), size),
                None => assert!(result.is_err()),
            }
            assert!(
                task.await
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains("range: bytes=0-0")
            );
        }
    }

    #[tokio::test]
    async fn system_proxy_works_without_a_running_core() {
        let (proxy, task) = server("200 OK", b"[]", "").await;
        let (result, fallback) = discover(
            &routes(None, Some(proxy)),
            "http://api.invalid/releases",
            "http://feed.invalid/",
            "0.4.7",
        )
        .await
        .unwrap();
        assert!(result.is_none() && fallback.is_none());
        assert!(
            task.await
                .unwrap()
                .starts_with("GET http://api.invalid/releases HTTP/1.1")
        );
    }

    #[tokio::test]
    #[ignore = "checks official GitHub feed and asset headers without installing"]
    async fn official_feed_detects_version_and_installer_without_api() {
        let route = Route {
            label: "系统代理",
            proxy: crate::platform::configured_proxy("https").unwrap(),
        };
        let response = client(&route).unwrap().get(FEED_URL).send().await.unwrap();
        let available = feed_release(&release_bytes(response).await.unwrap(), "0.0.0")
            .unwrap()
            .unwrap();
        let package = feed_package(&available, &route).await.unwrap().unwrap();
        assert!(package.asset.size > 0 && package.asset.size <= PACKAGE_LIMIT);
        assert_eq!(
            package.asset.name,
            package_name(&available.version).unwrap()
        );
        eprintln!(
            "Feed-only discovery: {} ({} bytes)",
            available.version, package.asset.size
        );
    }
}
