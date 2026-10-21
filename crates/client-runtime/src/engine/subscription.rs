use anyhow::Result;
use reqwest::Url;

pub(crate) async fn download(url: &Url, port: Option<u16>) -> Result<(String, Option<String>)> {
    clash_client_core::subscription::download_routes(
        url,
        &crate::network::available_routes(port, url.scheme()),
    )
    .await
}
