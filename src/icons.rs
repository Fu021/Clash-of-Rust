//! Shared artwork for the sidebar, native window, and tray.
use std::sync::OnceLock;

pub const SIZE: u32 = 128;
const APP: &[u8] = include_bytes!("../resources/icons/app.rgba");
const SYSTEM: &[u8] = include_bytes!("../resources/icons/system.rgba");
const TUN: &[u8] = include_bytes!("../resources/icons/tun.rgba");

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

pub fn window() -> iced::window::Icon {
    iced::window::icon::from_rgba(APP.to_vec(), SIZE, SIZE)
        .expect("committed app icon must contain valid RGBA pixels")
}

pub fn sidebar() -> iced::widget::image::Handle {
    static HANDLE: OnceLock<iced::widget::image::Handle> = OnceLock::new();
    HANDLE
        .get_or_init(|| iced::widget::image::Handle::from_rgba(SIZE, SIZE, APP))
        .clone()
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
