use clash_client_ui::{
    DetailItem, WslDistributionItem,
    models::{
        ViewModels, WSL_MAX_CHECKS, WSL_MAX_DISTRIBUTIONS, WSL_MAX_LOGS, WSL_MAX_TEXT_CHARS,
        reconcile_bounded,
    },
};
use slint::{Model, SharedString, VecModel};
use std::rc::Rc;

fn distro(name: &str, coverage: &str) -> WslDistributionItem {
    WslDistributionItem {
        name: name.into(),
        state: "Stopped".into(),
        coverage: coverage.into(),
    }
}

fn check(value: &str) -> DetailItem {
    DetailItem {
        label: "strict-cold-start".into(),
        value: value.into(),
    }
}

#[test]
fn stale_generation_cannot_replace_any_list_or_survive_clearing() {
    let mut models = ViewModels::default();
    // Exercise u64 precisely, including values that Slint int/float cannot hold.
    let generation = u64::MAX - 1;
    assert!(models.set_wsl_results(
        generation,
        vec![distro("Ubuntu", "unknown")],
        vec![check("blocked")],
        vec!["current".into()],
    ));
    assert!(!models.set_wsl_results(
        generation - 1,
        vec![distro("old", "verified")],
        vec![check("succeeded")],
        vec!["stale".into()],
    ));
    assert_eq!(models.wsl_distributions.row_data(0).unwrap().name, "Ubuntu");
    assert_eq!(models.wsl_checks.row_data(0).unwrap().value, "blocked");
    assert_eq!(models.wsl_logs.row_data(0).unwrap(), "current");

    models.clear();
    assert!(!models.set_wsl_results(generation - 1, vec![], vec![], vec!["stale".into()]));
    assert_eq!(models.wsl_distributions.row_count(), 0);
    assert_eq!(models.wsl_checks.row_count(), 0);
    assert_eq!(models.wsl_logs.row_count(), 0);
    assert!(models.set_wsl_results(generation, vec![], vec![], vec!["reopened".into()]));
    assert!(models.set_wsl_results(u64::MAX, vec![], vec![], vec![]));
    assert!(!models.set_wsl_results(0, vec![], vec![], vec![]));
}

#[test]
fn new_generation_replaces_evidence_and_preserves_model_identity() {
    let mut models = ViewModels::default();
    let distributions = models.wsl_distributions.clone();
    let checks = models.wsl_checks.clone();
    let logs = models.wsl_logs.clone();
    assert!(models.set_wsl_results(
        1,
        vec![distro("Ubuntu", "verified"), distro("Debian", "verified")],
        vec![check("succeeded")],
        vec!["old evidence".into()],
    ));
    // An input/config change projects its new generation with empty evidence.
    assert!(models.set_wsl_results(2, vec![], vec![], vec![]));
    assert_eq!(distributions.row_count(), 0);
    assert_eq!(checks.row_count(), 0);
    assert_eq!(logs.row_count(), 0);
    assert!(models.set_wsl_results(
        2,
        vec![distro("Ubuntu", "blocked")],
        vec![check("not_run")],
        vec!["new evidence".into()],
    ));
    // Same-generation refreshes remain allowed; operation ordering is desktop-owned.
    assert!(models.set_wsl_results(2, vec![distro("Ubuntu", "unknown")], vec![], vec![]));
    assert_eq!(distributions.row_count(), 1);
    assert_eq!(distributions.row_data(0).unwrap().coverage, "unknown");
    assert_eq!(checks.row_count(), 0);
    assert_eq!(logs.row_count(), 0);
    assert!(Rc::ptr_eq(&distributions, &models.wsl_distributions));
    assert!(Rc::ptr_eq(&checks, &models.wsl_checks));
    assert!(Rc::ptr_eq(&logs, &models.wsl_logs));
}

#[test]
fn oversized_reports_bound_rows_and_explicitly_report_incomplete_display() {
    let mut models = ViewModels::default();
    let distributions = (0..WSL_MAX_DISTRIBUTIONS + 5)
        .map(|i| distro(&format!("distro-{i}"), "unknown"))
        .collect();
    let checks = vec![check("not_run"); WSL_MAX_CHECKS + 5];
    let logs = (0..WSL_MAX_LOGS + 5)
        .map(|i| SharedString::from(format!("line-{i}")))
        .collect();
    assert!(models.set_wsl_results(1, distributions, checks, logs));
    assert_eq!(models.wsl_distributions.row_count(), WSL_MAX_DISTRIBUTIONS);
    assert_eq!(models.wsl_checks.row_count(), WSL_MAX_CHECKS);
    assert_eq!(models.wsl_logs.row_count(), WSL_MAX_LOGS);
    assert_eq!(
        models.wsl_distributions.row_data(0).unwrap().name,
        "distro-0"
    );
    let last = models
        .wsl_distributions
        .row_data(WSL_MAX_DISTRIBUTIONS - 1)
        .unwrap();
    assert_eq!(last.state, "display-truncated");
    assert!(last.name.contains("6"));
    assert!(last.coverage.starts_with("unknown"));
    assert_eq!(
        models
            .wsl_checks
            .row_data(WSL_MAX_CHECKS - 1)
            .unwrap()
            .label,
        "display-truncated"
    );
    assert_eq!(models.wsl_logs.row_data(0).unwrap(), "line-5");
    assert_eq!(
        models.wsl_logs.row_data(WSL_MAX_LOGS - 1).unwrap(),
        format!("line-{}", WSL_MAX_LOGS + 4).as_str(),
    );
}

#[test]
fn display_text_limits_preserve_utf8_and_show_truncation() {
    let mut models = ViewModels::default();
    let long = "德🛡".repeat(WSL_MAX_TEXT_CHARS);
    let exact = "德".repeat(WSL_MAX_TEXT_CHARS);
    assert!(models.set_wsl_results(
        1,
        vec![distro(&long, &long)],
        vec![DetailItem {
            label: long.clone().into(),
            value: long.clone().into()
        }],
        vec![long.into(), exact.clone().into()],
    ));
    let row = models.wsl_distributions.row_data(0).unwrap();
    assert!(row.name.ends_with('…'));
    assert_eq!(row.coverage.chars().count(), WSL_MAX_TEXT_CHARS + 1);
    assert!(row.coverage.ends_with('…'));
    let row = models.wsl_checks.row_data(0).unwrap();
    assert!(row.label.ends_with('…'));
    assert_eq!(row.value.chars().count(), WSL_MAX_TEXT_CHARS + 1);
    assert_eq!(
        models.wsl_logs.row_data(0).unwrap().chars().count(),
        WSL_MAX_TEXT_CHARS + 1
    );
    assert_eq!(models.wsl_logs.row_data(1).unwrap(), exact.as_str());
}

#[test]
fn bounded_reconciliation_shrinks_and_zero_limit_releases_existing_rows() {
    let model = VecModel::from(vec![1, 2, 3, 4]);
    reconcile_bounded(&model, vec![8, 9, 10], 2);
    assert_eq!(model.iter().collect::<Vec<_>>(), vec![8, 9]);
    reconcile_bounded(&model, vec![11], 10);
    assert_eq!(model.iter().collect::<Vec<_>>(), vec![11]);
    reconcile_bounded(&model, vec![12], 0);
    assert_eq!(model.row_count(), 0);
}
