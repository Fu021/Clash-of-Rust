//! Only small, view-local values survive the window. Business drafts live in App.
use clash_client_ui::{AppWindow, UiData};
use slint::{ComponentHandle, Timer};
use std::time::Duration;

macro_rules! view_state {
    ($(($get:ident, $set:ident)),+ $(,)?) => {
        #[derive(Clone)]
        pub(super) struct State {
            page: i32,
            scroll: [f32; 13],
            accent: Option<i32>,
        }
        impl State {
            pub(super) fn capture(window: &AppWindow) -> Self {
                let data = window.global::<UiData>();
                Self {
                    page: data.get_page(),
                    scroll: [$(data.$get()),+],
                    accent: (data.get_modal() == 12).then(|| window.get_accent_draft()),
                }
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
