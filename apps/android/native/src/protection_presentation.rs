use clash_mobile_runtime::core::{config::Language, error_message, protection::status::Status};
use serde_json::Value;

pub(crate) struct PendingVpnRestoration {
    pub(crate) request: String,
    pub(crate) connected: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VpnRestoration {
    Restored,
    Superseded,
}

pub(crate) fn observe_vpn_restoration(
    pending: &mut Option<PendingVpnRestoration>,
    platform: Option<&Value>,
) -> Option<VpnRestoration> {
    let restoration = pending.as_ref()?;
    let platform = platform?;
    let rollback = &platform["vpn_rollback"];
    let generation = platform["vpn_generation"].as_u64();
    let confirmation = if rollback["request"].as_str() != Some(restoration.request.as_str())
        || rollback["status"] != "complete"
        || (generation.is_some() && generation != rollback["generation"].as_u64())
    {
        VpnRestoration::Superseded
    } else if generation.is_some()
        && platform["vpn"].as_bool() == Some(restoration.connected)
        && platform["vpn_held"] == false
        && platform["vpn_transition_pending"] == false
        && rollback["error"].as_str() == Some("")
    {
        VpnRestoration::Restored
    } else {
        return None;
    };
    pending.take();
    Some(confirmation)
}

pub(crate) fn project(status: &str, mut health: Status, language: Language) -> (String, Status) {
    let status = error_message::localize(status, language).into_owned();
    health.message = error_message::localize(&health.message, language).into_owned();
    (status, health)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_mobile_runtime::core::{claude_check::guard::Snapshot, protection::status::State};
    use serde_json::json;

    fn pending_restoration(connected: bool) -> Option<PendingVpnRestoration> {
        Some(PendingVpnRestoration {
            request: "disable".into(),
            connected,
        })
    }

    fn restored_vpn(connected: bool) -> Value {
        json!({
            "vpn": connected, "vpn_held": false, "vpn_transition_pending": false,
            "vpn_error": "", "vpn_generation": 7,
            "vpn_rollback": {"request": "disable", "status": "complete", "generation": 7, "error": ""}
        })
    }

    #[test]
    fn committed_disconnect_is_confirmed_after_cleanup_and_a_fresh_observation() {
        let mut pending = pending_restoration(false);
        let mut platform = restored_vpn(false);
        platform["vpn_transition_pending"] = json!(true);
        assert_eq!(observe_vpn_restoration(&mut pending, Some(&platform)), None);
        assert_eq!(observe_vpn_restoration(&mut pending, None), None);
        platform["vpn_transition_pending"] = json!(false);
        platform["vpn_rollback"]["error"] = json!("VPN recovery error");
        assert_eq!(observe_vpn_restoration(&mut pending, Some(&platform)), None);
        assert!(pending.is_some());

        platform["vpn_rollback"]["error"] = json!("");
        platform["vpn_error"] = json!("Previous VPN startup error");
        assert_eq!(
            observe_vpn_restoration(&mut pending, Some(&platform)),
            Some(VpnRestoration::Restored)
        );
        assert!(pending.is_none());
        assert_eq!(observe_vpn_restoration(&mut pending, Some(&platform)), None);
    }

    #[test]
    fn committed_reconnect_is_confirmed_by_a_later_live_observation() {
        let mut pending = pending_restoration(true);
        assert_eq!(observe_vpn_restoration(&mut pending, None), None);
        let mut platform = restored_vpn(false);
        assert_eq!(observe_vpn_restoration(&mut pending, Some(&platform)), None);
        platform["vpn"] = json!(true);
        platform["vpn_held"] = json!(true);
        assert_eq!(observe_vpn_restoration(&mut pending, Some(&platform)), None);
        platform["vpn_held"] = json!(false);
        platform["vpn_transition_pending"] = json!(true);
        assert_eq!(observe_vpn_restoration(&mut pending, Some(&platform)), None);
        platform["vpn_transition_pending"] = json!(false);
        assert_eq!(
            observe_vpn_restoration(&mut pending, Some(&platform)),
            Some(VpnRestoration::Restored)
        );
        assert!(pending.is_none());
    }

    #[test]
    fn unreadable_guard_snapshot_does_not_prevent_disconnect_confirmation() {
        let mut pending = pending_restoration(false);
        let mut platform = restored_vpn(false);
        platform["protection"] = json!({
            "enabled": false,
            "snapshot": {"error": "保护状态不可用"}
        });
        assert!(
            serde_json::from_value::<Snapshot>(platform["protection"]["snapshot"].clone()).is_err()
        );
        assert_eq!(
            observe_vpn_restoration(&mut pending, Some(&platform)),
            Some(VpnRestoration::Restored)
        );
        assert!(pending.is_none());
    }

    #[test]
    fn a_new_vpn_request_or_generation_cannot_confirm_an_old_restoration() {
        for connected in [false, true] {
            for (field, value) in [
                ("request", json!("retry")),
                ("status", json!("failed")),
                ("generation", json!(6)),
            ] {
                let mut pending = pending_restoration(connected);
                let mut platform = restored_vpn(connected);
                platform["vpn_rollback"][field] = value;
                assert_eq!(
                    observe_vpn_restoration(&mut pending, Some(&platform)),
                    Some(VpnRestoration::Superseded)
                );
                assert!(pending.is_none());
            }
        }
    }

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
