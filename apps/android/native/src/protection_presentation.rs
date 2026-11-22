use clash_mobile_runtime::core::{config::Language, error_message, protection::status::Status};

pub(crate) fn project(status: &str, mut health: Status, language: Language) -> (String, Status) {
    let status = error_message::localize(status, language).into_owned();
    health.message = error_message::localize(&health.message, language).into_owned();
    (status, health)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_mobile_runtime::core::{claude_check::guard::Snapshot, protection::status::State};

    #[test]
    fn protection_notification_error_follows_language_without_changing_diagnostics() {
        let source = "连接已处理，但系统通知失败：通知已禁用 · example";
        let guard = Snapshot {
            connected: true,
            error: source.into(),
            ..Snapshot::default()
        };
        let diagnostics = serde_json::to_vec(&guard).unwrap();
        for (language, expected) in [
            (Language::SimplifiedChinese, source),
            (
                Language::English,
                "Connection handled; system notification failed: 通知已禁用 · example",
            ),
            (Language::SimplifiedChinese, source),
        ] {
            let health = Status::observe(
                true,
                false,
                true,
                true,
                Some((guard.connected, &guard.error)),
            );
            let (status, health) = project(&guard.error, health, language);
            assert_eq!(status, expected);
            assert_eq!(health.state, State::Error);
            assert_eq!(health.message, expected);
        }
        assert_eq!(serde_json::to_vec(&guard).unwrap(), diagnostics);
    }

    #[test]
    fn protection_readiness_message_follows_language_without_changing_state() {
        for (health, state, english) in [
            (
                Status::observe(true, true, true, true, None),
                State::Checking,
                "Switching protection; waiting for verification",
            ),
            (
                Status::observe(true, false, false, false, None),
                State::Error,
                "The core is not running; Claude protection is not active",
            ),
            (
                Status::observe(true, false, true, false, None),
                State::Error,
                "TUN is not connected; Claude protection is not ready",
            ),
            (
                Status::observe(true, false, true, true, None),
                State::Checking,
                "Checking the protection connection and live blocking",
            ),
            (
                Status::observe(true, false, true, true, Some((true, ""))),
                State::Active,
                "",
            ),
        ] {
            let (_, translated) = project("", health.clone(), Language::English);
            assert_eq!(translated.state, state);
            assert_eq!(translated.message, english);
            let (_, chinese) = project("", health.clone(), Language::SimplifiedChinese);
            assert_eq!(chinese, health);
        }
    }
}
