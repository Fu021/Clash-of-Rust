//! Desktop route discovery supplies native proxy preferences to shared downloads.
pub(crate) use clash_client_core::network::{Route, request_error, routes};

pub(crate) fn available_routes(port: Option<u16>, scheme: &str) -> Vec<Route> {
    routes(
        port,
        crate::platform::configured_proxy(scheme).ok().flatten(),
    )
}
