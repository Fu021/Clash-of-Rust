use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{io::ErrorKind, path::Path};

macro_rules! supported_languages {
    ($($variant:ident => ($tag:literal, $name:literal, $slint:literal, $aliases:expr, $excluded:expr)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
        pub enum LanguagePreference {
            #[default]
            #[serde(rename = "system")]
            System,
            $(
                #[serde(rename = $tag)]
                $variant,
            )+
        }

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Language {
            $($variant,)+
        }

        impl LanguagePreference {
            pub const TAGS: &[&str] = &["system", $($tag,)+];

            pub fn from_tag(tag: &str) -> Option<Self> {
                match tag {
                    "system" => Some(Self::System),
                    $($tag => Some(Self::$variant),)+
                    _ => None,
                }
            }

            pub const fn tag(self) -> &'static str {
                match self {
                    Self::System => "system",
                    $(Self::$variant => $tag,)+
                }
            }

            pub const fn explicit(self) -> Option<Language> {
                match self {
                    Self::System => None,
                    $(Self::$variant => Some(Language::$variant),)+
                }
            }
        }

        impl Language {
            pub const SUPPORTED: &[Self] = &[$(Self::$variant,)+];

            pub fn from_tag(tag: &str) -> Option<Self> {
                LanguagePreference::from_tag(tag)?.explicit()
            }

            pub const fn tag(self) -> &'static str {
                match self {
                    $(Self::$variant => $tag,)+
                }
            }

            pub const fn native_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }

            pub const fn slint_tag(self) -> &'static str {
                match self {
                    $(Self::$variant => $slint,)+
                }
            }

            fn system_aliases(self) -> (&'static [&'static str], &'static [&'static str]) {
                match self {
                    $(Self::$variant => ($aliases, $excluded),)+
                }
            }
        }
    };
}

include!("../../../../translations/languages.rs");

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

    #[cfg(target_os = "linux")]
    pub fn resolve_desktop(self) -> Language {
        if self != Self::System {
            return self.resolve([] as [&str; 0]);
        }
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.is_empty())
            .unwrap_or_else(|| "C".into());
        let base = locale.split(['.', '@']).next().unwrap_or("");
        if base.eq_ignore_ascii_case("C") || base.eq_ignore_ascii_case("POSIX") {
            return Language::English;
        }
        match std::env::var("LANGUAGE") {
            Ok(languages) if !languages.is_empty() => self.resolve(languages.split(':')),
            _ => self.resolve([locale]),
        }
    }

    #[cfg(not(any(target_os = "android", target_os = "linux")))]
    pub fn resolve_desktop(self) -> Language {
        self.resolve(std::iter::once_with(sys_locale::get_locales).flatten())
    }

    pub fn resolve(self, system_languages: impl IntoIterator<Item = impl AsRef<str>>) -> Language {
        self.explicit().unwrap_or_else(|| {
            system_languages
                .into_iter()
                .find_map(|tag| Language::from_system_tag(tag.as_ref()))
                .unwrap_or(Language::English)
        })
    }
}

impl Language {
    pub fn text<'a>(self, chinese: &'a str, english: &'a str) -> &'a str {
        match self {
            Self::SimplifiedChinese => chinese,
            _ => english,
        }
    }

    fn from_system_tag(tag: &str) -> Option<Self> {
        fn matches_prefix(tag: &str, prefix: &str) -> bool {
            tag == prefix
                || tag
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with('-'))
        }

        let mut tag = tag.trim().split(['.', '@']).next()?.replace('_', "-");
        tag.make_ascii_lowercase();
        Self::SUPPORTED
            .iter()
            .filter_map(|language| {
                let (aliases, excluded) = language.system_aliases();
                if excluded.iter().any(|prefix| matches_prefix(&tag, prefix)) {
                    return None;
                }
                aliases
                    .iter()
                    .filter(|prefix| matches_prefix(&tag, prefix))
                    .map(|prefix| prefix.len())
                    .max()
                    .map(|specificity| (specificity, *language))
            })
            .max_by_key(|(specificity, _)| *specificity)
            .map(|(_, language)| language)
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
        assert_eq!(
            preference.resolve(["fr", "C.UTF-8", "zh-CN"]),
            Language::English
        );
        assert_eq!(preference.resolve(["zh-HK", "zh-Hans-TW"]).tag(), "zh-CN");
        assert_eq!(LanguagePreference::from_tag("c"), None);
        assert_eq!(LanguagePreference::from_tag("posix"), None);
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
