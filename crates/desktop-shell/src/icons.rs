//! Shared artwork for the sidebar, native window, and tray.

pub const SIZE: u32 = 128;
const APP: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/icons/app.rgba"
));
const SYSTEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/icons/system.rgba"
));
const TUN: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/icons/tun.rgba"
));

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    App,
    SystemProxy,
    Tun,
}

pub fn pixels(kind: Kind) -> &'static [u8] {
    match kind {
        Kind::App => APP,
        Kind::SystemProxy => SYSTEM,
        Kind::Tun => TUN,
    }
}

pub fn tray_kind(system_proxy: bool, tun: bool) -> Kind {
    if tun {
        Kind::Tun
    } else if system_proxy {
        Kind::SystemProxy
    } else {
        Kind::App
    }
}
