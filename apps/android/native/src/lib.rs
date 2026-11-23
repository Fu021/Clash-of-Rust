#[cfg(target_os = "android")]
mod android;
#[cfg(any(target_os = "android", test))]
mod application_routing_mode;
#[cfg(any(target_os = "android", test))]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod claude_results;
#[cfg(any(target_os = "android", test))]
mod connection_detail;
#[cfg(any(target_os = "android", test))]
mod incident_delivery;
#[cfg(all(test, not(target_os = "android"), feature = "mobile-ui-tests"))]
#[path = "../tests/language_ui.rs"]
mod language_ui_tests;
#[cfg(any(target_os = "android", test))]
mod list_presentation;
#[cfg(any(target_os = "android", test))]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod network_probe;
#[cfg(any(target_os = "android", test))]
mod profile_picker;
#[cfg(any(target_os = "android", all(test, feature = "mobile-ui-tests")))]
mod protection_intent;
#[cfg(any(target_os = "android", test))]
mod protection_presentation;
#[cfg(target_os = "android")]
mod protection_service;
#[cfg(any(target_os = "android", test))]
mod protection_service_state;
#[cfg(any(target_os = "android", test))]
mod request_gate;
#[cfg(any(target_os = "android", test))]
mod session_worker;
#[cfg(any(target_os = "android", test))]
mod settings_input;
#[cfg(any(target_os = "android", test))]
mod site_report;
#[cfg(any(target_os = "android", test))]
mod startup_events;
#[cfg(any(target_os = "android", test))]
mod startup_presentation;
#[cfg(any(target_os = "android", test))]
mod startup_protection;
#[cfg(any(target_os = "android", test))]
mod update_presentation;
#[cfg(all(test, not(target_os = "android"), feature = "mobile-ui-tests"))]
mod test_ui {
    slint::include_modules!();
}
#[cfg(all(test, not(target_os = "android"), feature = "mobile-ui-tests"))]
mod ui_controls_tests;
