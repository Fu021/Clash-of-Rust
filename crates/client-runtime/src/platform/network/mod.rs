//! Read only the host's default routes, never probe an external service.
use anyhow::Result;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Route {
    pub interface: String,
    pub gateway: String,
}

pub(crate) type Routes = Vec<Route>;

pub(crate) fn snapshot() -> Result<Routes> {
    #[cfg(target_os = "linux")]
    let mut routes = linux::read()?;
    #[cfg(target_os = "macos")]
    let mut routes = macos::read()?;
    #[cfg(windows)]
    let mut routes = windows::read()?;
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let mut routes = Vec::new();
    routes.sort();
    routes.dedup();
    Ok(routes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_default_routes_can_be_read_without_changing_network_settings() {
        let routes = snapshot().expect("native default-route discovery failed");
        assert!(routes.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(routes.iter().all(|route| !route.interface.is_empty()));
    }
}
