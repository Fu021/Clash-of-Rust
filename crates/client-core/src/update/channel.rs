//! One release-selection policy for desktop, CLI and Android.
use semver::Version;

pub const PRERELEASE_WARNING: &str = "预发布版本可能存在尚未发现的风险与 Bug，可能导致程序崩溃、网络异常或配置兼容问题。建议先备份配置，确认后再下载安装。";
pub const PRERELEASE_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

pub struct Consent {
    version: String,
    opened: std::time::Instant,
}
impl Consent {
    pub fn new(version: String) -> Self {
        Self {
            version,
            opened: std::time::Instant::now(),
        }
    }
    pub fn remaining(&self) -> u64 {
        let remaining = PRERELEASE_WAIT.saturating_sub(self.opened.elapsed());
        remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
    }
    pub fn accepts(&self, version: &str) -> bool {
        self.version == version && self.remaining() == 0
    }
}

/// Stable mode follows the newest stable release, including a lower version.
/// Preview mode only offers versions newer than the installed version.
pub fn select<T>(
    releases: impl IntoIterator<Item = T>,
    current: &Version,
    include_prereleases: bool,
    metadata: impl Fn(&T) -> (&str, bool, bool),
) -> Option<(Version, T)> {
    releases
        .into_iter()
        .filter_map(|release| {
            let (tag, draft, prerelease) = metadata(&release);
            if draft {
                return None;
            }
            let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
            if !include_prereleases && (prerelease || !version.pre.is_empty()) {
                return None;
            }
            Some((version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp_precedence(right))
        // Compare only the channel maximum, or an equal installed version
        // could be filtered out and expose an older release as an update.
        .filter(|(target, _)| {
            let order = target.cmp_precedence(current);
            if include_prereleases {
                order.is_gt()
            } else {
                !order.is_eq()
            }
        })
}

pub fn description(version: &str, preview: bool, current: &str) -> String {
    if preview {
        format!("发现预发布版 {version}")
    } else if Version::parse(current)
        .ok()
        .zip(Version::parse(version).ok())
        .is_some_and(|(current, target)| target.cmp_precedence(&current).is_lt())
    {
        format!("可返回正式版 {version}")
    } else {
        format!("发现新版本 {version}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_is_delayed_bound_to_the_target_and_reset_for_each_new_dialog() {
        let mut consent = Consent::new("0.6.4-dev.1".into());
        assert_eq!(consent.remaining(), 3);
        assert!(!consent.accepts("0.6.4-dev.1"));
        consent.opened -= PRERELEASE_WAIT;
        assert_eq!(consent.remaining(), 0);
        assert!(consent.accepts("0.6.4-dev.1"));
        assert!(!consent.accepts("0.6.4-dev.2"));
        assert!(!Consent::new("0.6.4-dev.1".into()).accepts("0.6.4-dev.1"));
    }

    fn selected(releases: &[(&str, bool, bool)], current: &str, include: bool) -> Option<String> {
        select(
            releases,
            &Version::parse(current).unwrap(),
            include,
            |release| **release,
        )
        .map(|(version, _)| version.to_string())
    }

    #[test]
    fn channels_filter_drafts_invalid_tags_and_both_kinds_of_preview() {
        let releases = [
            ("v0.6.5", false, true),
            ("v0.6.6-dev.1", false, false),
            ("v0.6.4", false, false),
            ("v1.0.0", true, false),
            ("not-a-version", false, false),
        ];
        assert_eq!(
            selected(&releases, "0.6.3", false).as_deref(),
            Some("0.6.4")
        );
        assert_eq!(
            selected(&releases, "0.6.3", true).as_deref(),
            Some("0.6.6-dev.1")
        );
        assert!(selected(&[], "0.6.3", false).is_none());
        assert_eq!(
            description("0.6.2", false, "0.6.3-dev.2"),
            "可返回正式版 0.6.2"
        );
        assert_eq!(
            description("0.6.3-dev.3", true, "0.6.2"),
            "发现预发布版 0.6.3-dev.3"
        );
    }

    #[test]
    fn stable_channel_returns_high_local_versions_without_needing_release_metadata() {
        let releases = [
            ("v0.6.3", false, false),
            ("v0.6.5-dev.1.example", false, true),
            ("v0.6.4", false, false),
        ];
        for current in ["0.999.0", "0.999.0-fix.1.example", "0.6.5-dev.1.example"] {
            assert_eq!(
                selected(&releases, current, false).as_deref(),
                Some("0.6.4")
            );
            assert!(selected(&releases, current, true).is_none());
        }
        assert_eq!(description("0.6.4", false, "0.999.0"), "可返回正式版 0.6.4");
    }

    #[test]
    fn current_channel_maximum_never_exposes_an_older_release_or_metadata_only_change() {
        let mut releases = [
            ("v0.6.4+published", false, false),
            ("v0.6.3", false, false),
            ("v0.6.5-dev.1", false, true),
        ];
        for _ in 0..2 {
            assert!(selected(&releases, "0.6.4+local", false).is_none());
            assert!(selected(&releases, "0.6.5-dev.1", true).is_none());
            releases.reverse();
        }
    }
}
