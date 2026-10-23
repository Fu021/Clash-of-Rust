use anyhow::{Context, Result};
use std::net::TcpListener;

/// Reserve a loopback controller without contacting or stopping other listeners.
/// Keep the reservation until handing its port to the core.
pub fn reserve_controller(preferred: u16, mixed: u16) -> Result<TcpListener> {
    if preferred != 0 && preferred != mixed {
        match TcpListener::bind(("127.0.0.1", preferred)) {
            Ok(listener) => return Ok(listener),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
                ) => {}
            Err(error) => return Err(error).context("无法保留内核控制端口"),
        }
    }
    for _ in 0..16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).context("无法自动分配内核控制端口")?;
        if listener.local_addr()?.port() != mixed {
            return Ok(listener);
        }
    }
    anyhow::bail!("无法分配与代理端口不同的内核控制端口")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occupied_or_proxy_ports_are_replaced_without_touching_their_listener() {
        let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = occupied.local_addr().unwrap().port();
        for mixed in [0, port] {
            let selected = reserve_controller(port, mixed).unwrap();
            assert_ne!(selected.local_addr().unwrap().port(), port);
            assert!(TcpListener::bind(("127.0.0.1", port)).is_err());
        }
    }
}
