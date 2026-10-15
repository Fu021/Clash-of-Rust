//! Reusable UI contract. No network, file, proxy or privilege operations live here.
// `--all-targets` overrides [lib].test=false. Keep that empty harness cheap:
// integration tests link the normal library and exercise the production models.
#[cfg(not(test))]
slint::include_modules!();
#[cfg(not(test))]
pub mod models;
