//! Only small, view-local values survive the window. Business drafts live in App.
use clash_client_ui::{AppWindow, UiData};
use slint::{ComponentHandle, Timer, winit_030::WinitWindowAccessor};
use std::time::Duration;

macro_rules! view_state {
    ($(($get:ident, $set:ident)),+ $(,)?) => {
        #[derive(Clone)]
        pub(super) struct State {
            page: i32,
            scroll: [f32; 13],
            accent: Option<i32>,
            position: slint::PhysicalPosition,
            maximized: bool,
            fullscreen: bool,
        }
        impl State {
            pub(super) fn capture(window: &AppWindow) -> Self {
                let data = window.global::<UiData>();
                Self {
                    page: data.get_page(),
                    scroll: [$(data.$get()),+],
                    accent: (data.get_modal() == 12).then(|| window.get_accent_draft()),
                    position: window.window().position(),
                    maximized: window.window().is_maximized(),
                    fullscreen: window.window().is_fullscreen(),
                }
            }
            pub(super) fn prepare(&self, window: &AppWindow) -> bool {
                    // Keep the old monitor when possible; a disconnected screen
                    // must not leave the recreated title bar unreachable.
                    let monitors = window.window().with_winit_window(|native| {
                        native.available_monitors().map(|monitor| {
                            let p = monitor.position(); let s = monitor.size();
                            (p.x, p.y, s.width, s.height)
                        }).collect::<Vec<_>>()
                    }).unwrap_or_default();
                    let size = window.window().size();
                    let (x, y) = fit_position((self.position.x, self.position.y),
                        (size.width, size.height), &monitors);
                    window.window().set_position(slint::PhysicalPosition::new(x, y));
                    window.window().set_maximized(self.maximized);
                    window.window().set_fullscreen(self.fullscreen);
                self.maximized || self.fullscreen
            }
            pub(super) fn restore(&self, window: &AppWindow) {
                // Populate models first, then let layout establish the scroll
                // ranges. Never retain the component through a delayed callback.
                let weak = window.as_weak();
                let state = self.clone();
                Timer::single_shot(Duration::from_millis(16), move || {
                    let Some(window) = weak.upgrade() else { return; };
                    let data = window.global::<UiData>();
                    if data.get_page() != state.page { return; }
                    let mut offsets = state.scroll.into_iter();
                    $(data.$set(offsets.next().unwrap_or_default().min(0.));)+
                    if let Some(accent) = state.accent.filter(|_| data.get_modal() == 12) {
                        window.set_accent_draft(accent);
                    }

                });
            }
        }
    };
}

fn fit_position(
    position: (i32, i32),
    size: (u32, u32),
    monitors: &[(i32, i32, u32, u32)],
) -> (i32, i32) {
    let (x, y) = (i64::from(position.0), i64::from(position.1));
    if monitors.is_empty()
        || monitors.iter().any(|&(left, top, width, height)| {
            x + i64::from(size.0) >= i64::from(left) + 64
                && x < i64::from(left) + i64::from(width) - 64
                && y >= i64::from(top)
                && y < i64::from(top) + i64::from(height) - 32
        })
    {
        return position;
    }
    let (left, top, width, height) = monitors[0];
    let x = x.clamp(
        i64::from(left),
        (i64::from(left) + i64::from(width) - i64::from(size.0)).max(i64::from(left)),
    );
    let y = y.clamp(
        i64::from(top),
        (i64::from(top) + i64::from(height) - i64::from(size.1)).max(i64::from(top)),
    );
    (
        x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
    )
}

view_state! {
    (get_scroll_home, set_scroll_home),
    (get_scroll_proxies, set_scroll_proxies),
    (get_scroll_profiles, set_scroll_profiles),
    (get_scroll_connections, set_scroll_connections),
    (get_scroll_rules, set_scroll_rules),
    (get_scroll_logs, set_scroll_logs),
    (get_scroll_network, set_scroll_network),
    (get_scroll_sites, set_scroll_sites),
    (get_scroll_site_categories, set_scroll_site_categories),
    (get_scroll_claude, set_scroll_claude),
    (get_scroll_settings, set_scroll_settings),
    (get_scroll_dialog, set_scroll_dialog),
    (get_scroll_navigation, set_scroll_navigation),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recreated_windows_keep_reachable_negative_coordinates_but_recover_removed_monitors() {
        let screens = [(-1920, 0, 1920, 1080), (0, 0, 1920, 1080)];
        assert_eq!(
            fit_position((-1800, 100), (950, 700), &screens),
            (-1800, 100)
        );
        assert_eq!(
            fit_position((-1800, 100), (950, 700), &screens[1..]),
            (0, 100)
        );
        assert_eq!(
            fit_position((4000, 4000), (950, 700), &screens[1..]),
            (970, 380)
        );
        assert_eq!(
            fit_position((4000, 4000), (950, 700), &[(0, 0, 800, 600)]),
            (0, 0)
        );
    }
}
