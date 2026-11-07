//! Desktop route discovery supplies native proxy preferences to shared downloads.
#[cfg(test)]
pub(crate) use clash_client_core::network::routes;
pub(crate) use clash_client_core::network::{DownloadPorts, Route, request_error};

pub(crate) fn available_routes(port: Option<DownloadPorts>, scheme: &str) -> Vec<Route> {
    clash_client_core::network::download_routes(
        port,
        crate::platform::configured_proxy(scheme).ok().flatten(),
    )
}
