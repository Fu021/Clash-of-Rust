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

pub fn installed_preview<'a>(
    current: &Version,
    releases: impl IntoIterator<Item = (&'a str, bool, bool)>,
) -> bool {
    !current.pre.is_empty()
        || releases.into_iter().any(|(tag, draft, preview)| {
            !draft
                && preview
                && Version::parse(tag.strip_prefix('v').unwrap_or(tag))
                    .is_ok_and(|version| version.cmp_precedence(current).is_eq())
        })
}

/// Stable users only move forward. A prerelease user who opts out may return
/// to the newest stable release, even when its version is numerically lower.
pub fn candidate(
    tag: &str,
    draft: bool,
    prerelease: bool,
    current: &Version,
    include_prereleases: bool,
    current_is_preview: bool,
) -> Option<Version> {
    if draft {
        return None;
    }
    let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
    let preview = prerelease || !version.pre.is_empty();
    if preview && !include_prereleases {
        return None;
    }
    (version.cmp_precedence(current).is_gt()
        || (!include_prereleases && current_is_preview && !preview))
        .then_some(version)
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

    fn eligible(
        tag: &str,
        draft: bool,
        preview: bool,
        current: &Version,
        include: bool,
    ) -> Option<Version> {
        candidate(
            tag,
            draft,
            preview,
            current,
            include,
            !current.pre.is_empty(),
        )
    }

    #[test]
    fn channels_filter_metadata_and_tags_and_allow_only_prereleases_to_return_to_stable() {
        let stable = Version::parse("0.6.2").unwrap();
        let preview = Version::parse("0.6.3-dev.2").unwrap();
        assert!(eligible("v0.6.3-dev.3", false, true, &stable, false).is_none());
        assert!(eligible("v0.6.3-dev.3", false, false, &stable, false).is_none());
        assert!(eligible("v0.6.3", false, true, &stable, false).is_none());
        assert!(eligible("v0.6.3-dev.3", true, true, &stable, true).is_none());
        assert!(eligible("not-a-version", false, false, &stable, true).is_none());
        assert!(eligible("v0.6.3-dev.3", false, true, &stable, true).is_some());
        assert!(eligible("v0.6.3-dev.1", false, true, &preview, true).is_none());
        assert!(eligible("v0.6.2", false, false, &preview, true).is_none());
        assert!(eligible("v0.6.2", false, false, &preview, false).is_some());
        assert!(eligible("v0.6.3", false, false, &preview, false).is_some());
        assert!(eligible("v0.6.1", false, false, &stable, false).is_none());
        assert!(eligible("v0.6.2+other-build", false, false, &stable, true).is_none());
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
    fn github_can_mark_a_plain_numeric_tag_as_a_prerelease() {
        let current = Version::parse("0.6.3").unwrap();
        let preview = installed_preview(&current, [("v0.6.3", false, true)]);
        assert!(preview);
        assert!(candidate("v0.6.2", false, false, &current, false, preview).is_some());
        assert!(candidate("v0.6.3", false, true, &current, true, preview).is_none());
        assert!(!installed_preview(&current, [("v0.6.3", true, true)]));
    }
}
