#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
mod protection_service;
#[cfg(any(target_os = "android", test))]
mod request_gate;
