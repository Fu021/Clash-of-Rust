//! Shared wire representation for desktop proxy preferences and the Mac helper.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProxyValue {
    Text(String),
    Number(u32),
}
pub type ProxyState = BTreeMap<String, Option<ProxyValue>>;
