use super::*;
use serde_json::{Value, json};

fn request(action: Action) -> Request {
    Request {
        action,
        execute: action.mutating(),
        backend: Backend::WireGuard,
        distro: "COR-Egress".into(),
        profile_path: r"C:\Private profiles\egress.json".into(),
        ..Request::default()
    }
}

fn fixture(action: Action, generation: u64) -> Value {
    json!({
        "schema_version": 1,
        "operation_id": uuid::Uuid::new_v4().to_string(),
        "generation": generation,
        "action": action,
        "outcome": "succeeded",
        "protection": "verified",
        "phase": "acceptance_pending",
        "backend": "WireGuard",
        "message": "",
        "distributions": [{"name":"COR-Egress", "state":"running", "coverage":"verified"}],
        "checks": [
            {"id":"cold_boot", "result":"PASS", "detail":""},
            {"id":"host_policy", "result":"PASS", "detail":""},
            {"id":"all_wsl_coverage", "result":"PASS", "detail":""},
            {"id":"ipv4_only", "result":"PASS", "detail":""},
            {"id":"dns_via_exit", "result":"PASS", "detail":""},
            {"id":"windows_network_preserved", "result":"PASS", "detail":""},
            {"id":"interop_guarded", "result":"PASS", "detail":""},
            {"id":"failure_closed", "result":"PASS", "detail":""},
            {"id":"german_exit", "result":"PASS", "detail":""},
            {"id":"wireguard_connected", "result":"PASS", "detail":""},
            {"id":"functional_egress", "result":"PASS", "detail":""}
        ],
        "logs": []
    })
}

fn parse(value: Value) -> Report {
    Report::parse(&value.to_string()).unwrap()
}

#[test]
fn contract_accepts_actual_script_values_and_u64_generations() {
    for action in [
        Action::Doctor,
        Action::Status,
        Action::Install,
        Action::Start,
        Action::Stop,
        Action::Test,
        Action::Rollback,
        Action::Uninstall,
    ] {
        let mut raw = fixture(action, u64::MAX);
        raw["protection"] = json!("unknown");
        raw["outcome"] = json!("not_run");
        raw["phase"] = json!("installed_closed");
        raw["distributions"][0]["coverage"] = json!("not_enrolled");
        raw["checks"] = json!([
            {"id":"administrator", "result":"PASS", "detail":""},
            {"id":"host_policy", "result":"FAIL", "detail":""},
            {"id":"strict_tailscale_boot", "result":"BLOCKED", "detail":""},
            {"id":"cold_boot", "result":"NOT_RUN", "detail":""}
        ]);
        let report = parse(raw);
        assert_eq!(report.generation, u64::MAX);
        assert_eq!(report.action, action);
        assert!(!report.functional_ready());
        assert_eq!(report.effective_protection(), Protection::Blocked);
    }
}

#[test]
fn unambiguous_single_schema_object_is_required() {
    let valid = fixture(Action::Test, 1).to_string();
    for raw in [
        format!("{valid}\n{valid}"),
        format!("warning\n{valid}"),
        valid.replace(
            "\"schema_version\":1",
            "\"schema_version\":1,\"schema_version\":1",
        ),
        valid.replace("\"generation\":1", "\"generation\":-1"),
    ] {
        assert_eq!(Report::parse(&raw), Err(Error::InvalidReport));
    }
    let mut raw = fixture(Action::Test, 1);
    raw["schema_version"] = json!(2);
    assert_eq!(
        Report::parse(&raw.to_string()),
        Err(Error::UnsupportedSchema)
    );
    raw["schema_version"] = json!(1);
    raw.as_object_mut().unwrap().remove("logs");
    assert_eq!(Report::parse(&raw.to_string()), Err(Error::InvalidReport));
    assert_eq!(
        Report::parse(&" ".repeat(1024 * 1024 + 1)),
        Err(Error::InvalidReport)
    );
}

#[test]
fn current_unproven_script_report_cannot_claim_verified() {
    let mut raw = fixture(Action::Test, 1);
    raw["checks"] = json!([
        {"id":"host_policy", "result":"PASS", "detail":"Effective settings and endpoint rules only; not a cold-boot proof."},
        {"id":"cold_boot", "result":"NOT_RUN", "detail":"Requires an isolated Windows/WSL test environment."},
        {"id":"german_exit", "result":"NOT_RUN", "detail":"No target traffic is generated automatically."}
    ]);
    let report = parse(raw);
    assert_eq!(report.protection, Protection::Blocked);
    assert!(!report.functional_ready());
    for action in [
        Action::Doctor,
        Action::Status,
        Action::Install,
        Action::Start,
    ] {
        assert_eq!(parse(fixture(action, 1)).protection, Protection::Blocked);
    }
}

#[test]
fn unsuccessful_outcomes_cannot_reuse_successful_proof() {
    for outcome in ["failed", "blocked", "not_run"] {
        let mut raw = fixture(Action::Test, 1);
        raw["outcome"] = json!(outcome);
        let report = parse(raw);
        assert_eq!(report.protection, Protection::Blocked);
        assert!(!report.functional_ready());
    }
}

#[test]
fn every_security_gate_is_necessary_and_unknown_results_fail_closed() {
    let good = fixture(Action::Test, 1);
    assert!(parse(good.clone()).functional_ready());
    for id in [
        "cold_boot",
        "host_policy",
        "all_wsl_coverage",
        "ipv4_only",
        "dns_via_exit",
        "windows_network_preserved",
        "interop_guarded",
        "failure_closed",
    ] {
        for result in ["FAIL", "BLOCKED", "NOT_RUN", "Passed", "future_value"] {
            let mut raw = good.clone();
            let check = raw["checks"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|check| check["id"] == id)
                .unwrap();
            check["result"] = json!(result);
            let report = parse(raw);
            assert_eq!(report.protection, Protection::Blocked, "{id}: {result}");
        }
    }
}

#[test]
fn connection_country_and_protection_are_distinct() {
    let mut raw = fixture(Action::Test, 1);
    raw["backend"] = json!("Tailscale");
    raw["checks"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"tailscale_connected", "result":"PASS", "detail":""}));
    let report = parse(raw.clone());
    assert!(report.tailscale_connected());
    assert!(report.german_exit_verified());
    assert_eq!(report.protection, Protection::Blocked); // Missing strict TS boot proof.
    raw["checks"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"strict_tailscale_boot", "result":"PASS", "detail":""}));
    raw["checks"]
        .as_array_mut()
        .unwrap()
        .retain(|check| check["id"] != "german_exit");
    let report = parse(raw);
    assert_eq!(report.protection, Protection::Verified);
    assert!(!report.german_exit_verified());
    assert!(!report.functional_ready());
}

#[test]
fn docker_blocking_is_safety_evidence_not_functional_acceptance() {
    let mut raw = fixture(Action::Test, 1);
    raw["distributions"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"docker-desktop", "state":"stopped", "coverage":"not_enrolled"}));
    assert_eq!(parse(raw.clone()).protection, Protection::Blocked);
    raw["distributions"][1]["coverage"] = json!("blocked");
    raw["checks"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"docker_desktop_isolated", "result":"PASS", "detail":""}));
    let report = parse(raw);
    assert_eq!(report.protection, Protection::Verified);
    assert!(!report.functional_ready());
}

#[test]
fn missing_or_duplicate_coverage_and_duplicate_checks_are_rejected() {
    let mut raw = fixture(Action::Test, 1);
    raw["distributions"] = json!([]);
    assert_eq!(parse(raw).protection, Protection::Blocked);
    let mut raw = fixture(Action::Test, 1);
    let duplicate = raw["distributions"][0].clone();
    raw["distributions"].as_array_mut().unwrap().push(duplicate);
    assert_eq!(Report::parse(&raw.to_string()), Err(Error::InvalidReport));
    let mut raw = fixture(Action::Test, 1);
    raw["checks"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"cold_boot", "result":"FAIL", "detail":""}));
    assert_eq!(Report::parse(&raw.to_string()), Err(Error::InvalidReport));
}

#[test]
fn inputs_are_validated_without_reading_profiles() {
    assert!(Request::default().validate().is_ok());
    assert!(request(Action::Start).validate().is_ok());
    for ip in [
        "::1",
        "localhost",
        "127.0.0.1",
        "10.0.0.1",
        "100.64.0.1",
        "169.254.1.1",
        "0.1.2.3",
        "224.0.0.1",
        "255.255.255.255",
        "1.2.3.4;exit",
        " 8.8.8.8",
    ] {
        let mut req = request(Action::Start);
        req.endpoint_ipv4 = ip.into();
        req.endpoint_port = "51820".into();
        assert_eq!(req.validate(), Err(Error::InvalidEndpoint), "{ip}");
    }
    for port in ["0", "65536", "-1", "+1", "1.0", " 80", "080", "80;exit"] {
        let mut req = request(Action::Start);
        req.endpoint_ipv4 = "203.0.113.8".into();
        req.endpoint_port = port.into();
        assert_eq!(req.validate(), Err(Error::InvalidPort), "{port}");
    }
    for distro in [
        "-d",
        "../Ubuntu",
        "Ubuntu;exit",
        "Ubuntu\n",
        "Ubuntu$(exit)",
    ] {
        let mut req = request(Action::Status);
        req.distro = distro.into();
        assert_eq!(req.validate(), Err(Error::InvalidDistro));
    }
    for path in [
        r"relative.json",
        r"\\server\share\profile.json",
        r"C:\a\..\b.json",
        r"C:\a.json:secret",
        r"C:\CON.json",
        r"C:\profile;.json",
        r"C:\profile.yaml",
        r"C:\a.\b.json",
        r"C:\a\%TOKEN%.json",
    ] {
        let mut req = request(Action::Start);
        req.profile_path = path.into();
        assert_eq!(req.validate(), Err(Error::InvalidProfilePath), "{path}");
    }
}

#[test]
fn explicit_execution_and_preflight_have_separate_arguments() {
    let mut req = request(Action::Install);
    req.execute = false;
    req.profile_path.clear();
    assert!(req.arguments().unwrap().iter().all(|arg| arg != "-Execute"));
    req.action = Action::Start;
    assert_eq!(req.validate(), Err(Error::ExecuteRequired));
    for action in [Action::Doctor, Action::Status, Action::Test] {
        let mut req = request(action);
        assert!(req.validate().is_ok());
        req.execute = true;
        assert_eq!(req.validate(), Err(Error::UnexpectedExecute));
    }
    let req = request(Action::Start);
    let args = req.arguments().unwrap();
    assert_eq!(
        args.iter()
            .position(|arg| arg == "-ProfilePath")
            .map(|index| args[index + 1].as_str()),
        Some(req.profile_path.as_str())
    );
    assert_eq!(args.last().unwrap(), "-Execute");
    let mut req = request(Action::Install);
    req.distro = "docker-desktop-data".into();
    assert_eq!(req.validate(), Err(Error::DockerDesktopUnsupported));
}

#[test]
fn original_network_restore_is_an_explicit_separate_capability() {
    for action in [
        Action::Doctor,
        Action::Status,
        Action::Install,
        Action::Start,
        Action::Stop,
        Action::Test,
        Action::Rollback,
        Action::Uninstall,
    ] {
        for execute in [false, true] {
            let mut req = request(action);
            req.execute = execute;
            req.restore_original_network = true;
            if matches!(action, Action::Rollback | Action::Uninstall) && execute {
                assert!(
                    req.arguments()
                        .unwrap()
                        .iter()
                        .any(|arg| arg == "-RestoreOriginalNetwork")
                );
            } else {
                assert_eq!(req.arguments(), Err(Error::UnexpectedRestore));
            }
        }
    }
    let req = WslSettings::default().request(Action::Rollback, true);
    assert!(!req.restore_original_network);
    assert!(
        !req.arguments()
            .unwrap()
            .iter()
            .any(|arg| arg == "-RestoreOriginalNetwork")
    );
}

#[test]
fn unicode_windows_profile_paths_are_single_literal_arguments() {
    let mut req = request(Action::Start);
    req.profile_path = r"E:\私密配置\网络配置.json".into();
    let args = req.arguments().unwrap();
    let index = args.iter().position(|arg| arg == "-ProfilePath").unwrap();
    assert_eq!(args[index + 1], req.profile_path);
    for path in [
        "E:\\配置\n\\网络.json",
        r"E:\配置\$(秘密).json",
        r"E:\配置\..\网络.json",
    ] {
        req.profile_path = path.into();
        assert_eq!(req.validate(), Err(Error::InvalidProfilePath));
    }
}

#[test]
fn exhausted_epoch_cannot_accept_an_in_flight_report_after_edit() {
    let mut state = State::default();
    state.generation = u64::MAX - 1;
    let op = state.begin(request(Action::Test)).unwrap();
    assert_eq!(state.invalidate(), Err(Error::GenerationExhausted));
    assert_eq!(
        state.apply(&op, parse(fixture(Action::Test, 10))),
        Err(Error::StaleOperation)
    );
    assert!(!state.functional_ready());
}

#[test]
fn safe_preferences_roundtrip_but_inputs_and_secret_content_do_not() {
    let settings = WslSettings {
        backend: Backend::WireGuard,
        distro: "COR-Egress".into(),
        endpoint_ipv4: "203.0.113.8".into(),
        endpoint_port: "51820".into(),
        profile_path: r"C:\Private profiles\egress.json".into(),
    };
    let value = serde_json::to_value(&settings).unwrap();
    assert!(value.get("endpoint_ipv4").is_none());
    assert!(value.get("endpoint_port").is_none());
    let restored: WslSettings = serde_json::from_value(value).unwrap();
    assert_eq!(restored.profile_path, settings.profile_path);
    assert_eq!(restored.backend, settings.backend);
    assert!(restored.endpoint_ipv4.is_empty());
    assert!(!format!("{settings:?}").contains("Private profiles"));
    assert!(!format!("{:?}", settings.request(Action::Start, true)).contains("203.0.113.8"));
    let legacy: crate::config::Settings = serde_json::from_str("{}").unwrap();
    assert_eq!(legacy.wsl, WslSettings::default());
}

#[test]
fn legacy_application_settings_roundtrip_preserves_unrelated_preferences() {
    let original = crate::config::Settings {
        run_mode: "global".into(),
        text_scale: 125,
        delay_interval_minutes: 17,
        ..crate::config::Settings::default()
    };
    let mut legacy_json = serde_json::to_value(&original).unwrap();
    legacy_json.as_object_mut().unwrap().remove("wsl");
    let loaded: crate::config::Settings = serde_json::from_value(legacy_json.clone()).unwrap();
    loaded.validate().unwrap();
    assert_eq!(loaded.wsl, WslSettings::default());
    let saved = serde_json::to_string(&loaded).unwrap();
    let restored: crate::config::Settings = serde_json::from_str(&saved).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored.wsl, WslSettings::default());
    let mut restored_json = serde_json::to_value(restored).unwrap();
    restored_json.as_object_mut().unwrap().remove("wsl");
    assert_eq!(restored_json, legacy_json);
}

#[test]
fn application_settings_reject_unsafe_durable_wsl_fields_not_transient_overrides() {
    let mut settings = crate::config::Settings {
        wsl: WslSettings {
            distro: "COR-Egress".into(),
            profile_path: r"E:\私密配置\网络.json".into(),
            endpoint_ipv4: "unfinished override".into(),
            endpoint_port: String::new(),
            ..WslSettings::default()
        },
        ..crate::config::Settings::default()
    };
    assert!(settings.validate().is_ok());
    assert_eq!(
        settings.wsl.request(Action::Doctor, false).validate(),
        Err(Error::InvalidEndpoint)
    );
    settings.wsl.profile_path = r"E:\私密配置\..\网络.json".into();
    assert!(settings.validate().is_err());
    settings.wsl.profile_path = r"E:\私密配置\网络.json".into();
    settings.wsl.distro = "Ubuntu;invoke".into();
    assert!(settings.validate().is_err());
}

#[test]
fn all_free_text_channels_are_redacted_and_errors_are_fixed() {
    let private = "PrivateKey=synthetic-secret https://example.invalid/?token=synthetic 203.0.113.8 C:\\private\\profile.json";
    let mut raw = fixture(Action::Test, 1);
    raw["message"] = json!(private);
    raw["logs"] = json!([private]);
    raw["checks"][0]["detail"] = json!(private);
    raw["phase"] = json!("synthetic-secret");
    let serialized = serde_json::to_string(&parse(raw)).unwrap();
    assert!(!serialized.contains("synthetic"));
    assert!(!serialized.contains("203.0.113.8"));
    assert!(serialized.contains("[redacted]"));
    assert_eq!(Report::parse(private), Err(Error::InvalidReport));
    assert!(!Error::InvalidReport.to_string().contains(private));
}

#[test]
fn local_epoch_and_helper_generation_are_independent_and_cannot_regress() {
    let mut state = State::default();
    let req = request(Action::Test);
    let op = state.begin(&req).unwrap();
    state
        .apply(&op, parse(fixture(Action::Test, 1_792_000_000_000)))
        .unwrap();
    assert!(state.functional_ready());
    assert_eq!(state.service_generation(), Some(1_792_000_000_000));
    let op = state.begin(&req).unwrap();
    assert_eq!(state.protection(), Protection::Blocked);
    assert!(state.report().is_none());
    assert_eq!(
        state.apply(&op, parse(fixture(Action::Test, 1))),
        Err(Error::GenerationRegressed)
    );
    assert!(state.report().is_none());
    let op = state.begin(req).unwrap();
    state
        .apply(&op, parse(fixture(Action::Test, 1_792_000_000_000)))
        .unwrap();
    assert!(state.functional_ready()); // Same config generation, fresh operation.
}

#[test]
fn editing_in_flight_invalidates_evidence_without_parallel_mutations() {
    let mut state = State::default();
    let req = request(Action::Test);
    let old = state.begin(&req).unwrap();
    state.invalidate().unwrap();
    assert!(state.busy());
    assert_eq!(state.begin(&req), Err(Error::Busy));
    assert_eq!(
        state.apply(&old, parse(fixture(Action::Test, 10))),
        Err(Error::StaleOperation)
    );
    assert!(!state.busy());
    assert!(state.report().is_none());
    let new = state.begin(&req).unwrap();
    assert_eq!(
        state.fail(&old, Error::HelperFailed),
        Err(Error::StaleOperation)
    );
    assert!(state.busy());
    state.fail(&new, Error::HelperUnavailable).unwrap();
    assert_eq!(state.last_error, Some(Error::HelperUnavailable));
    assert!(!state.busy());
}

#[test]
fn action_backend_replay_and_ticket_identity_are_checked() {
    let mut state = State::default();
    let op = state.begin(request(Action::Test)).unwrap();
    assert_eq!(
        state.apply(&op, parse(fixture(Action::Status, 10))),
        Err(Error::ReportMismatch)
    );
    let op = state.begin(request(Action::Test)).unwrap();
    let mut raw = fixture(Action::Test, 10);
    raw["backend"] = json!("Tailscale");
    assert_eq!(state.apply(&op, parse(raw)), Err(Error::ReportMismatch));
    // Doctor/Status inspect the installed backend rather than the UI draft.
    let mut req = request(Action::Status);
    req.backend = Backend::Tailscale;
    let op = state.begin(req).unwrap();
    let report = parse(fixture(Action::Status, 10));
    assert_ne!(op.operation_id, report.operation_id);
    state.apply(&op, report.clone()).unwrap();
    let op = state.begin(request(Action::Status)).unwrap();
    assert_eq!(state.apply(&op, report), Err(Error::ReplayedReport));
}

#[test]
fn identity_canonicalization_rejects_alternative_uuid_replays() {
    let mut state = State::default();
    let raw = fixture(Action::Status, 10);
    let op = state.begin(request(Action::Status)).unwrap();
    state.apply(&op, parse(raw.clone())).unwrap();
    let mut replay = raw;
    let id = uuid::Uuid::parse_str(replay["operation_id"].as_str().unwrap()).unwrap();
    replay["operation_id"] = json!(id.simple().to_string().to_ascii_uppercase());
    let op = state.begin(request(Action::Status)).unwrap();
    assert_eq!(state.apply(&op, parse(replay)), Err(Error::ReplayedReport));
}

#[test]
fn malformed_and_forged_reports_do_not_keep_the_ui_busy_or_restore_proof() {
    let mut state = State::default();
    let op = state.begin(request(Action::Test)).unwrap();
    let mut malformed = parse(fixture(Action::Test, 10));
    malformed.schema_version = 99;
    assert_eq!(state.apply(&op, malformed), Err(Error::UnsupportedSchema));
    assert!(!state.busy());
    assert!(state.report().is_none());
    let op = state.begin(request(Action::Test)).unwrap();
    let mut forged = op.clone();
    forged.request.backend = Backend::Tailscale;
    assert_eq!(
        state.apply(&forged, parse(fixture(Action::Test, 10))),
        Err(Error::StaleOperation)
    );
    assert!(state.busy());
    state.fail(&op, Error::HelperFailed).unwrap();
    assert_eq!(state.protection(), Protection::Blocked);
}

#[test]
fn failed_validation_and_exhausted_epoch_clear_prior_evidence() {
    let mut state = State::default();
    let op = state.begin(request(Action::Test)).unwrap();
    state.apply(&op, parse(fixture(Action::Test, 10))).unwrap();
    let mut invalid = request(Action::Start);
    invalid.profile_path = "secret-in-invalid-path".into();
    assert_eq!(state.begin(invalid), Err(Error::InvalidProfilePath));
    assert_eq!(state.protection(), Protection::Blocked);
    state.generation = u64::MAX;
    assert_eq!(state.invalidate(), Err(Error::GenerationExhausted));
    assert_eq!(state.generation, u64::MAX);
    assert!(state.report().is_none());
}
