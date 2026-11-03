//! A single owner's timer; failed attempts never modify subscription metadata.
use crate::config::{Profile, Settings, Store, read_profile};
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Schedule {
    attempted: BTreeMap<String, (String, u64)>,
}

impl Schedule {
    pub fn next(&mut self, settings: &Settings, profiles: &[Profile], now: u64) -> Option<Profile> {
        self.attempted
            .retain(|id, _| profiles.iter().any(|profile| profile.id == *id));
        let minutes = settings.subscription_update_interval_minutes;
        if minutes == 0 {
            return None;
        }
        let profile = profiles.iter().find(|profile| {
            let remote = reqwest::Url::parse(&profile.source)
                .is_ok_and(|url| matches!(url.scheme(), "https" | "http"));
            let protected = settings
                .claude_protection
                .as_ref()
                .is_some_and(|protection| protection.profile_id == profile.id);
            let attempted = self
                .attempted
                .get(&profile.id)
                .filter(|(source, _)| *source == profile.source)
                .map_or(0, |(_, time)| *time);
            remote
                && !protected
                && now.saturating_sub(profile.updated.max(attempted)) >= u64::from(minutes) * 60
        })?;
        self.attempted
            .insert(profile.id.clone(), (profile.source.clone(), now));
        Some(profile.clone())
    }
}

pub struct PendingUpdate {
    pub profile: Profile,
    digest: [u8; 32],
}

impl PendingUpdate {
    pub fn capture(profile: Profile, store: &Store) -> Result<Self> {
        Ok(Self {
            digest: Self::digest(store, &profile.id)?,
            profile,
        })
    }

    pub fn is_current(
        &self,
        settings: &Settings,
        profiles: &[Profile],
        store: &Store,
    ) -> Result<bool> {
        Ok(settings.subscription_update_interval_minutes != 0
            && self.is_unchanged(settings, profiles, store)?)
    }

    pub fn is_unchanged(
        &self,
        settings: &Settings,
        profiles: &[Profile],
        store: &Store,
    ) -> Result<bool> {
        if settings
            .claude_protection
            .as_ref()
            .is_some_and(|protection| protection.profile_id == self.profile.id)
            || !profiles.iter().any(|profile| profile == &self.profile)
        {
            return Ok(false);
        }
        Ok(Self::digest(store, &self.profile.id)? == self.digest)
    }

    fn digest(store: &Store, id: &str) -> Result<[u8; 32]> {
        Ok(Sha256::digest(read_profile(&store.profile_path(id)?)?.as_bytes()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(source: &str) -> Profile {
        Profile {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Fixture".into(),
            source: source.into(),
            updated: 100,
            usage: None,
        }
    }

    #[test]
    fn skips_local_profiles_and_retries_failures_only_after_the_interval() {
        let mut settings = Settings {
            subscription_update_interval_minutes: 1,
            ..Default::default()
        };
        let mut profiles = vec![profile("default.yaml"), profile("https://example.test/sub")];
        let mut schedule = Schedule::default();
        assert!(schedule.next(&settings, &profiles, 159).is_none());
        assert_eq!(
            schedule.next(&settings, &profiles, 160),
            Some(profiles[1].clone())
        );
        assert!(schedule.next(&settings, &profiles, 219).is_none());
        assert!(schedule.next(&settings, &profiles, 220).is_some());
        profiles[1].updated = 230;
        assert!(schedule.next(&settings, &profiles, 289).is_none());
        settings.subscription_update_interval_minutes = 0;
        assert!(schedule.next(&settings, &profiles, 10000).is_none());
        assert_eq!(profiles[1].updated, 230);
    }

    #[test]
    fn pending_download_does_not_replace_deleted_edited_or_disabled_subscriptions() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::at(root.path().to_owned()).unwrap();
        let profiles = vec![profile("https://example.test/sub")];
        let path = store.profile_path(&profiles[0].id).unwrap();
        crate::config::atomic_write(&path, b"proxies: []\nrules: [MATCH,DIRECT]\n").unwrap();
        let pending = PendingUpdate::capture(profiles[0].clone(), &store).unwrap();
        let mut settings = Settings::default();
        assert!(pending.is_current(&settings, &profiles, &store).unwrap());
        settings.subscription_update_interval_minutes = 0;
        assert!(!pending.is_current(&settings, &profiles, &store).unwrap());
        assert!(pending.is_unchanged(&settings, &profiles, &store).unwrap());
        settings.subscription_update_interval_minutes = 1;
        assert!(!pending.is_current(&settings, &[], &store).unwrap());
        let mut changed = profiles.clone();
        changed[0].source = "https://example.test/replaced".into();
        assert!(!pending.is_current(&settings, &changed, &store).unwrap());
        crate::config::atomic_write(&path, b"proxies: []\nrules: [MATCH,REJECT]\n").unwrap();
        assert!(!pending.is_current(&settings, &profiles, &store).unwrap());
        settings.subscription_update_interval_minutes = 0;
        assert!(!pending.is_current(&settings, &profiles, &store).unwrap());
    }

    #[test]
    fn a_protection_lock_acquired_during_download_invalidates_the_result() {
        use crate::protection::{Previous, Protection};
        let root = tempfile::tempdir().unwrap();
        let store = Store::at(root.path().to_owned()).unwrap();
        let profiles = vec![profile("https://example.test/sub")];
        crate::config::atomic_write(
            &store.profile_path(&profiles[0].id).unwrap(),
            b"proxies: []\n",
        )
        .unwrap();
        let pending = PendingUpdate::capture(profiles[0].clone(), &store).unwrap();
        let mut settings = Settings::default();
        assert!(pending.is_current(&settings, &profiles, &store).unwrap());
        settings.claude_protection = Some(std::sync::Arc::new(Protection::new(
            profiles[0].id.clone(),
            "Fixture".into(),
            Previous {
                mode: "rule".into(),
                proxy_mode: crate::model::ProxyMode::Off,
                selectors: Default::default(),
            },
        )));
        assert!(!pending.is_current(&settings, &profiles, &store).unwrap());
    }

    #[test]
    fn protected_subscription_is_skipped_while_other_remote_profiles_can_update() {
        use crate::protection::{Previous, Protection};
        let profiles = vec![
            profile("https://example.test/locked"),
            profile("https://example.test/other"),
        ];
        let settings = Settings {
            subscription_update_interval_minutes: 1,
            claude_protection: Some(std::sync::Arc::new(Protection::new(
                profiles[0].id.clone(),
                "Fixture".into(),
                Previous {
                    mode: "rule".into(),
                    proxy_mode: crate::model::ProxyMode::Off,
                    selectors: Default::default(),
                },
            ))),
            ..Default::default()
        };
        let mut schedule = Schedule::default();
        assert_eq!(
            schedule.next(&settings, &profiles, 160),
            Some(profiles[1].clone())
        );
        assert!(schedule.next(&settings, &profiles, 161).is_none());
    }
}
