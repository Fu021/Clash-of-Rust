use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{io::ErrorKind, path::Path};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LanguagePreference {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
    #[serde(rename = "en")]
    English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    SimplifiedChinese,
    English,
}

impl LanguagePreference {
    pub fn read(root: &Path) -> Result<Self> {
        #[derive(Deserialize)]
        struct StoredLanguage {
            #[serde(default)]
            language: LanguagePreference,
        }

        let path = root.join("settings.json");
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::System),
            Err(error) => {
                return Err(error).with_context(|| format!("无法读取设置文件：{}", path.display()));
            }
        };
        let settings: StoredLanguage = serde_json::from_slice(&bytes)
            .with_context(|| format!("设置文件格式错误：{}", path.display()))?;
        Ok(settings.language)
    }

    #[cfg(not(target_os = "android"))]
    pub fn resolve_desktop(self) -> Language {
        self.resolve(std::iter::once_with(sys_locale::get_locales).flatten())
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "system" => Some(Self::System),
            "zh-CN" => Some(Self::SimplifiedChinese),
            "en" => Some(Self::English),
            _ => None,
        }
    }

    pub const fn tag(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::SimplifiedChinese => "zh-CN",
            Self::English => "en",
        }
    }

    pub fn resolve(self, system_languages: impl IntoIterator<Item = impl AsRef<str>>) -> Language {
        match self {
            Self::System => system_languages
                .into_iter()
                .find_map(|tag| Language::from_system_tag(tag.as_ref()))
                .unwrap_or(Language::English),
            Self::SimplifiedChinese => Language::SimplifiedChinese,
            Self::English => Language::English,
        }
    }
}

impl Language {
    pub fn text<'a>(self, chinese: &'a str, english: &'a str) -> &'a str {
        match self {
            Self::SimplifiedChinese => chinese,
            Self::English => english,
        }
    }

    pub const fn tag(self) -> &'static str {
        match self {
            Self::SimplifiedChinese => "zh-CN",
            Self::English => "en",
        }
    }

    fn from_system_tag(tag: &str) -> Option<Self> {
        let mut subtags = tag.trim().split(['.', '@']).next()?.split(['-', '_']);
        match subtags.next()?.to_ascii_lowercase().as_str() {
            "en" => Some(Self::English),
            "zh" => match subtags.next().map(str::to_ascii_lowercase).as_deref() {
                Some("hant" | "tw" | "hk" | "mo") => None,
                _ => Some(Self::SimplifiedChinese),
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Settings, Store};

    #[test]
    fn fresh_and_legacy_settings_follow_the_first_supported_system_language() {
        let old_settings = serde_json::from_str::<Settings>(r#"{"dark":true}"#).unwrap();
        for settings in [Settings::default(), old_settings] {
            assert_eq!(settings.language, LanguagePreference::System);
            assert_eq!(
                settings.language.resolve(["fr-FR", "zh_CN.UTF-8", "en-GB"]),
                Language::SimplifiedChinese
            );
            assert_eq!(
                settings.language.resolve(["ja-JP", "en-AU", "zh-Hans"]),
                Language::English
            );
        }
    }

    #[test]
    fn unsupported_languages_fall_back_to_english() {
        let preference = LanguagePreference::System;
        assert_eq!(preference.resolve([] as [&str; 0]), Language::English);
        assert_eq!(
            preference.resolve(["", "C.UTF-8", "fr-FR", "zh-Hant-TW"]),
            Language::English
        );
        assert_eq!(preference.resolve(["zh-HK", "zh-Hans-TW"]).tag(), "zh-CN");
    }

    #[test]
    fn reading_language_is_read_only_for_a_fresh_install_and_a_legacy_file() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("profile");
        assert_eq!(
            LanguagePreference::read(&root).unwrap(),
            LanguagePreference::System
        );
        assert!(!root.exists());
        std::fs::create_dir(&root).unwrap();
        let path = root.join("settings.json");
        let legacy = r#"{"dark":true}"#;
        std::fs::write(&path, legacy).unwrap();
        assert_eq!(
            LanguagePreference::read(&root).unwrap(),
            LanguagePreference::System
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), legacy);
        std::fs::write(&path, r#"{"language":"en"}"#).unwrap();
        assert_eq!(
            LanguagePreference::read(&root).unwrap(),
            LanguagePreference::English
        );
    }

    #[test]
    fn explicit_language_round_trips_as_a_presentation_setting() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path().to_owned()).unwrap();
        let settings = store.load_settings().unwrap();
        for (preference, tag, expected, system_tag) in [
            (
                LanguagePreference::SimplifiedChinese,
                "zh-CN",
                Language::SimplifiedChinese,
                "en",
            ),
            (
                LanguagePreference::English,
                "en",
                Language::English,
                "zh-CN",
            ),
        ] {
            let next = Settings {
                language: preference,
                ..settings.clone()
            };
            store.save_settings(&next).unwrap();
            let contents = std::fs::read(store.root.join("settings.json")).unwrap();
            let serialized: serde_json::Value = serde_json::from_slice(&contents).unwrap();
            assert_eq!(serialized["language"], tag);
            let restored = store.load_settings().unwrap();
            assert_eq!(restored, next);
            assert_eq!(restored.language.resolve([system_tag]), expected);
            assert!(settings.presentation_change_only(&restored));
        }
        store.save_settings(&settings).unwrap();
        let system = store.load_settings().unwrap().language;
        assert_eq!(system.resolve(["en"]), Language::English);
        assert_eq!(system.resolve(["zh-CN"]), Language::SimplifiedChinese);
    }
}
