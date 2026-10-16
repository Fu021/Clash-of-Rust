//! Desktop environment inspection and background connection enforcement.
use anyhow::{Context, Result, bail};
pub use clash_client_core::claude_check::*;
use reqwest::Client;
use std::time::Duration;
pub mod audit;
pub mod system;
