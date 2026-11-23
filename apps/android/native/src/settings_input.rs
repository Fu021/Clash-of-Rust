use crate::messages;
use anyhow::{Context, ensure};
use clash_mobile_runtime::{Session, core::config::Language};
use serde_json::Value;
use std::str::FromStr;

pub(crate) fn parse_number<Number: FromStr>(input: &str) -> Result<Number, Number::Err> {
    input.trim().parse()
}

pub(crate) fn ipv6_change_available(platform: &Value) -> bool {
    ["vpn", "vpn_pending", "vpn_transition_pending", "vpn_held"]
        .into_iter()
        .all(|flag| platform[flag] == false)
}

pub(crate) fn save_ipv6(
    session: &mut Session,
    enabled: bool,
    language: Language,
) -> anyhow::Result<()> {
    if enabled == session.settings.ipv6 {
        return Ok(());
    }
    session
        .observe_native()
        .context(messages::vpn_disconnect_unconfirmed(language))?;
    ensure!(
        ipv6_change_available(&session.platform),
        "{}",
        messages::ipv6_requires_vpn_disconnect(language)
    );
    let mut settings = session.settings.clone();
    settings.ipv6 = enabled;
    session.commit_settings(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_mobile_runtime::NativeHost;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[test]
    fn ports_and_speed_test_intervals_accept_surrounding_whitespace() {
        assert_eq!(parse_number::<u16>(" \t7890 "), Ok(7890));
        assert_eq!(parse_number::<u32>(" 15\n"), Ok(15));
        assert_eq!(parse_number::<u32>(" 0 "), Ok(0));
        assert!(parse_number::<u16>(" 65536 ").is_err());
        assert!(parse_number::<u16>(" 78 90 ").is_err());
        assert!(parse_number::<u32>(" \t ").is_err());
        assert!(parse_number::<u32>(" -1 ").is_err());
    }

    #[test]
    fn ipv6_saves_only_after_a_fresh_disconnected_vpn_state() {
        #[derive(Default)]
        struct Host {
            state: Mutex<Value>,
            calls: Mutex<Vec<Value>>,
        }

        impl NativeHost for Host {
            fn request(&self, request: Value) -> anyhow::Result<Value> {
                let result = if request["op"] == "state" {
                    self.state.lock().unwrap().clone()
                } else {
                    json!({"ok":true})
                };
                self.calls.lock().unwrap().push(request);
                Ok(result)
            }
        }

        let disconnected = json!({
            "running": true, "vpn": false, "vpn_pending": false,
            "vpn_transition_pending": false, "vpn_held": false
        });
        let native = Arc::new(Host {
            state: Mutex::new(disconnected.clone()),
            ..Host::default()
        });
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::open(root.path().into(), native.clone()).unwrap();
        session.initialize().unwrap();
        native.calls.lock().unwrap().clear();
        let settings_path = session.store.root.join("settings.json");
        let runtime_path = session.store.runtime().join("config.yaml");
        let original_settings = std::fs::read(&settings_path).unwrap();
        let original_runtime = std::fs::read(&runtime_path).unwrap();
        let language = Language::English;
        let enabled = session.settings.ipv6;
        for flag in ["vpn", "vpn_pending", "vpn_transition_pending", "vpn_held"] {
            session.platform = disconnected.clone();
            session.vpn = false;
            let mut observed = disconnected.clone();
            observed[flag] = true.into();
            *native.state.lock().unwrap() = observed;
            let error = save_ipv6(&mut session, !enabled, language).unwrap_err();
            assert_eq!(error.to_string(), "Disconnect the VPN before changing IPv6");
            assert!(!ipv6_change_available(&session.platform));
            assert_eq!(session.settings.ipv6, enabled);
            assert_eq!(std::fs::read(&settings_path).unwrap(), original_settings);
            assert_eq!(std::fs::read(&runtime_path).unwrap(), original_runtime);
        }
        assert!(save_ipv6(&mut session, enabled, language).is_ok());
        *native.state.lock().unwrap() = json!({"running":true});
        assert!(save_ipv6(&mut session, !enabled, language).is_err());
        assert_eq!(
            *native.calls.lock().unwrap(),
            vec![json!({"op":"state"}); 5]
        );

        session.vpn = true;
        *native.state.lock().unwrap() = disconnected;
        save_ipv6(&mut session, !enabled, language).unwrap();
        assert!(ipv6_change_available(&session.platform));
        assert!(!session.vpn);
        assert_eq!(session.store.load_settings().unwrap().ipv6, !enabled);
        let calls = native.calls.lock().unwrap();
        assert_eq!(calls[5], json!({"op":"state"}));
        assert_eq!(calls[6]["op"], "reload");
        assert_eq!(calls[6]["ipv6"], !enabled);
    }
}
