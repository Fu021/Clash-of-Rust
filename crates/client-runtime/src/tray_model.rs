//! Small native-menu model shared without loading a tray implementation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Nodes {
    pub group: String,
    pub selected: String,
    pub names: Vec<String>,
    pub total: usize,
    pub locked: bool,
}
