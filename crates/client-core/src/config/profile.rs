use serde::{Deserialize, Deserializer, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub source: String,
    pub updated: u64,
    pub usage: Option<String>,
    pub builtin: bool,
}

impl Profile {
    pub fn is_default(&self) -> bool {
        self.builtin
    }

    pub fn is_local(&self) -> bool {
        !reqwest::Url::parse(&self.source).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
    }
}

impl<'de> Deserialize<'de> for Profile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Record {
            id: String,
            name: String,
            source: String,
            updated: u64,
            usage: Option<String>,
            #[serde(default)]
            builtin: Option<bool>,
        }
        let record = Record::deserialize(deserializer)?;
        let mut profile = Self {
            id: record.id,
            name: record.name,
            source: record.source,
            updated: record.updated,
            usage: record.usage,
            builtin: record.builtin.unwrap_or(false),
        };
        if record.builtin.is_none() {
            profile.builtin = profile.is_local()
                && profile.name == "默认直连"
                && Path::new(&profile.source)
                    .file_name()
                    .is_some_and(|name| name == "default.yaml");
        }
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_identity_migrates_once_and_ignores_later_display_changes() {
        let legacy = serde_json::json!({
            "id": "fixture", "name": "默认直连", "source": "/resources/default.yaml",
            "updated": 0, "usage": null
        });
        let mut profile: Profile = serde_json::from_value(legacy.clone()).unwrap();
        assert!(profile.is_default());
        profile.name = "Direct connection".into();
        profile.source = "/new/resources/default.yaml".into();
        let current = serde_json::to_value(&profile).unwrap();
        assert_eq!(current["builtin"], true);
        let loaded: Profile = serde_json::from_value(current).unwrap();
        assert!(loaded.is_default());
        for source in [
            "https://example.invalid/default.yaml",
            "/imports/other.yaml",
        ] {
            let mut imported = legacy.clone();
            imported["source"] = serde_json::json!(source);
            assert!(
                !serde_json::from_value::<Profile>(imported)
                    .unwrap()
                    .is_default()
            );
        }
        let mut explicit = legacy;
        explicit["builtin"] = serde_json::json!(false);
        assert!(
            !serde_json::from_value::<Profile>(explicit)
                .unwrap()
                .is_default()
        );
    }
}
