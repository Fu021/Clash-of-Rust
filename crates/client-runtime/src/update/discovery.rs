//! Release discovery tolerates unavailable API domains and stale proxy routes.
use super::*;
#[cfg(test)]
use crate::network::routes;
use crate::network::{Route, available_routes, request_error};

const LATEST_URL: &str = "https://github.com/Fu021/Clash-of-Rust/releases/latest";
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

pub(super) async fn check(
    port: Option<u16>,
    include_prereleases: bool,
) -> Result<Option<Available>> {
    // Reading settings does not change the desktop proxy, and an unavailable
    // desktop settings service must not prevent the other routes from working.
    let routes = available_routes(port, "https");
    clash_client_core::update::retry_check(|| {
        check_once_channel(
            &routes,
            API_URL,
            LATEST_URL,
            crate::VERSION,
            include_prereleases,
        )
    })
    .await
}

#[cfg(test)]
async fn check_once(
    routes: &[Route],
    api: &str,
    latest: &str,
    current: &str,
) -> Result<Option<Available>> {
    check_once_channel(routes, api, latest, current, false).await
}
async fn check_once_channel(
    routes: &[Route],
    api: &str,
    latest: &str,
    current: &str,
    include_prereleases: bool,
) -> Result<Option<Available>> {
    let (mut available, latest_route) = tokio::time::timeout(
        Duration::from_secs(60),
        discover_channel(routes, api, latest, current, include_prereleases),
    )
    .await
    .context("更新检查超时，请检查代理或稍后重试")??;
    if let (Some(available), Some(route)) = (&mut available, latest_route) {
        // A redirect only identifies the version. Probe both required assets
        // before publishing a downloadable update to the UI.
        available.package =
            tokio::time::timeout(REQUEST_TIMEOUT, latest_package(available, &route))
                .await
                .context("安装包信息查询超时")??;
    }
    if let Some(available) = &available
        && package_name(&available.version).is_some()
        && available.package.is_none()
    {
        bail!(
            "版本 {} 缺少本平台安装包或校验文件，请稍后重试或访问 {}",
            available.version,
            available.url
        );
    }
    Ok(available)
}

#[cfg(test)]
async fn discover(
    routes: &[Route],
    api: &str,
    latest: &str,
    current: &str,
) -> Result<(Option<Available>, Option<Route>)> {
    discover_channel(routes, api, latest, current, false).await
}
async fn discover_channel(
    routes: &[Route],
    api: &str,
    latest: &str,
    current: &str,
    include_prereleases: bool,
) -> Result<(Option<Available>, Option<Route>)> {
    let mut api_error = None;
    for route in routes {
        let result = match client(route) {
            Ok(client) => fetch_channel(&client, api, current, include_prereleases).await,
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
    if include_prereleases {
        bail!(
            "暂时无法获取预发布版本列表：{}（已尝试可用代理和直连）",
            api_error.unwrap_or_default()
        );
    }
    let mut latest_error = None;
    for route in routes {
        let result = async {
            let response = client(route)?
                .get(latest)
                .header(reqwest::header::CACHE_CONTROL, "no-cache")
                .send()
                .await
                .map_err(|error| request_error(error, "Release 订阅"))?;
            latest_response(response, current).await
        }
        .await;
        match result {
            Ok(available) => return Ok((available, Some(route.clone()))),
            Err(error) => {
                if latest_error.is_none() {
                    latest_error = Some(error.to_string());
                }
            }
        }
    }
    bail!(
        "{}；{}（已尝试可用代理和直连）",
        api_error.unwrap_or_default(),
        latest_error.unwrap_or_default()
    );
}

// GitHub's /releases/latest excludes drafts and prereleases. Atom entries do
// not expose the prerelease flag, so they cannot safely select stable updates.
async fn latest_response(response: reqwest::Response, current: &str) -> Result<Option<Available>> {
    if !response.status().is_redirection() {
        release_bytes(response).await?;
        bail!("Release 订阅格式无效");
    }
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .context("Release 订阅格式无效")?;
    latest_release(location, current)
}

fn latest_release(location: &str, current: &str) -> Result<Option<Available>> {
    let base = Url::parse(RELEASES_URL).expect("constant release URL");
    let url = base.join(location).context("Release 订阅格式无效")?;
    // With no stable release, GitHub can redirect back to the release list.
    if url.as_str() == RELEASES_URL {
        return Ok(None);
    }
    let prefix = format!("{RELEASES_URL}/tag/");
    let tag = url
        .as_str()
        .strip_prefix(&prefix)
        .context("Release 订阅格式无效")?;
    newest_channel(
        vec![Release {
            tag_name: tag.to_owned(),
            draft: false,
            prerelease: false,
            assets: Vec::new(),
        }],
        current,
        false,
    )
}

async fn latest_package(available: &Available, route: &Route) -> Result<Option<Package>> {
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
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
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

    #[tokio::test]
    async fn check_refetches_missing_package_and_checksum_instead_of_publishing_broken_update() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/releases", listener.local_addr().unwrap());
        let name = package_name("0.6.3").expect("CI desktop platform");
        let package = serde_json::json!({"name":name,"size":42});
        let sidecar = serde_json::json!({"name":format!("{name}.sha256"),"size":64});
        let bodies = [vec![], vec![package.clone()], vec![package, sidecar]].map(|assets| {
            serde_json::json!([
                {"tag_name":"v0.6.3","draft":false,"prerelease":false,"assets":assets}
            ])
            .to_string()
        });
        let server = tokio::spawn(async move {
            for body in bodies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = [0; 8192];
                let mut count = 0;
                while !bytes[..count].ends_with(b"\r\n\r\n") {
                    let n = socket.read(&mut bytes[count..]).await.unwrap();
                    assert!(n > 0);
                    count += n;
                }
                let request = String::from_utf8_lossy(&bytes[..count]).to_ascii_lowercase();
                assert!(request.starts_with("get /releases http/1.1"));
                assert!(request.contains("cache-control: no-cache"));
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
            }
        });
        let routes = routes(None, None);
        let result = clash_client_core::update::retry_check(|| {
            check_once(&routes, &endpoint, &endpoint, "0.6.1")
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.version, "0.6.3");
        assert_eq!(result.package.unwrap().asset.name, name);
        server.await.unwrap();
    }

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

    #[test]
    fn latest_accepts_only_our_stable_release_tag_redirects() {
        for location in [
            format!("{RELEASES_URL}/tag/v0.4.9"),
            "/Fu021/Clash-of-Rust/releases/tag/v0.4.9".into(),
        ] {
            let available = latest_release(&location, "0.4.7").unwrap().unwrap();
            assert_eq!(available.version, "0.4.9");
            assert!(!available.preview);
            assert!(available.package.is_none());
            assert!(latest_release(&location, "0.4.10").unwrap().is_none());
        }
        assert!(latest_release(RELEASES_URL, "0.4.7").unwrap().is_none());
        assert!(
            latest_release(&format!("{RELEASES_URL}/tag/v0.5.0-beta.1"), "0.4.7")
                .unwrap()
                .is_none()
        );
        for location in [
            "https://evil.invalid/releases/tag/v99.0.0",
            "https://github.com/other/project/releases/tag/v99.0.0",
        ] {
            assert!(latest_release(location, "0.4.7").is_err());
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
        let (available, latest_route) = discover(
            &route_list,
            "http://api.invalid/releases",
            "http://feed.invalid/",
            "0.4.7",
        )
        .await
        .unwrap();
        assert_eq!(available.unwrap().version, "0.4.8");
        assert!(latest_route.is_none());
        assert!(
            task.await
                .unwrap()
                .starts_with("GET http://api.invalid/releases HTTP/1.1")
        );
    }

    #[tokio::test]
    async fn blocked_api_uses_stable_redirect_and_reports_both_errors_if_unavailable() {
        let route_list = routes(None, None);
        let (api, api_task) = server("403 Forbidden", b"", "").await;
        let redirect = format!("Location: {RELEASES_URL}/tag/v0.4.8\r\n");
        let (feed, feed_task) = server("302 Found", b"", &redirect).await;
        let (available, latest_route) = discover(&route_list, &api, &feed, "0.4.7").await.unwrap();
        assert_eq!(available.unwrap().version, "0.4.8");
        assert!(latest_route.unwrap().proxy.is_none());
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
    async fn fallback_never_accepts_a_preview_redirect_or_unverified_page() {
        for (status, body, headers, expected_none) in [
            (
                "302 Found",
                b"".as_slice(),
                format!("Location: {RELEASES_URL}/tag/v0.6.0-dev.1\r\n"),
                true,
            ),
            ("302 Found", b"".as_slice(), String::new(), false),
            (
                "200 OK",
                b"<html>Pre-release</html>".as_slice(),
                String::new(),
                false,
            ),
        ] {
            let (url, server) = server(status, body, &headers).await;
            let response = client(&Route {
                label: "直连",
                proxy: None,
            })
            .unwrap()
            .get(url)
            .send()
            .await
            .unwrap();
            let result = latest_response(response, "0.5.1").await;
            if expected_none {
                assert!(result.unwrap().is_none());
            } else {
                assert!(result.is_err());
            }
            server.await.unwrap();
        }
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
}
