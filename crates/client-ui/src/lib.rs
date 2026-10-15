//! Reusable UI contract. No network, file, proxy or privilege operations live here.
slint::include_modules!();
pub mod models;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
