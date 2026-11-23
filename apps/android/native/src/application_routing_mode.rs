use clash_mobile_runtime::core::config::Language;
use serde_json::Value;
use std::collections::BTreeSet;

use crate::messages;

pub(super) fn scope_label(scope: &Value, language: Language) -> String {
    let selected = scope["selected"].as_u64().unwrap_or_default();
    match scope["mode"].as_str() {
        Some("only") => messages::routing_scope_only(language, selected),
        Some("exclude") => messages::routing_scope_exclude(language, selected),
        _ => messages::routing_scope_all(language).into(),
    }
}

pub(super) fn selection_after_mode_change(
    current: &str,
    next: &str,
    selected: &BTreeSet<String>,
    available: &BTreeSet<String>,
) -> BTreeSet<String> {
    match (current, next) {
        ("all", "only") => available.clone(),
        ("all", "exclude") => BTreeSet::new(),
        ("only", "exclude") | ("exclude", "only") => {
            available.difference(selected).cloned().collect()
        }
        _ => selected.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_scope_renders_in_either_language_without_a_native_refresh() {
        for (mode, selected, chinese, english) in [
            ("all", 5, "全部应用", "All apps"),
            ("only", 2, "仅选中 2 个应用", "Only 2 selected"),
            ("exclude", 1, "排除 1 个应用", "1 excluded"),
        ] {
            let scope = serde_json::json!({"mode": mode, "selected": selected});
            assert_eq!(scope_label(&scope, Language::SimplifiedChinese), chinese);
            assert_eq!(scope_label(&scope, Language::English), english);
            assert_eq!(scope_label(&scope, Language::SimplifiedChinese), chinese);
        }
        assert_eq!(scope_label(&Value::Null, Language::English), "All apps");
    }

    fn captured(
        mode: &str,
        selected: &BTreeSet<String>,
        available: &BTreeSet<String>,
    ) -> BTreeSet<String> {
        match mode {
            "only" => selected.intersection(available).cloned().collect(),
            "exclude" => available.difference(selected).cloned().collect(),
            _ => available.clone(),
        }
    }

    #[test]
    fn reversing_modes_preserves_application_routes_and_includes_system_components() {
        let available: BTreeSet<String> = BTreeSet::from([
            "org.example.browser".into(),
            "org.example.mail".into(),
            "org.example.hidden.system".into(),
        ]);
        for selected in [
            BTreeSet::new(),
            BTreeSet::from(["org.example.browser".into()]),
            available.clone(),
        ] {
            for (current, next) in [("only", "exclude"), ("exclude", "only")] {
                let changed = selection_after_mode_change(current, next, &selected, &available);
                assert_eq!(
                    captured(current, &selected, &available),
                    captured(next, &changed, &available)
                );
                assert_eq!(
                    selection_after_mode_change(next, current, &changed, &available),
                    selected
                );
            }
        }
    }

    #[test]
    fn leaving_all_apps_preserves_full_capture_in_either_representation() {
        let available: BTreeSet<String> =
            BTreeSet::from(["org.example.browser".into(), "org.example.mail".into()]);
        let previous = BTreeSet::from(["org.example.browser".into()]);
        for next in ["only", "exclude"] {
            let selected = selection_after_mode_change("all", next, &previous, &available);
            assert_eq!(captured(next, &selected, &available), available);
        }
        assert_eq!(
            selection_after_mode_change("only", "all", &previous, &available),
            previous
        );
        assert_eq!(
            selection_after_mode_change("only", "only", &previous, &available),
            previous
        );
    }

    #[test]
    fn repeated_mode_changes_preserve_routes_for_every_selection() {
        let available: BTreeSet<String> = BTreeSet::from([
            "org.example.browser".into(),
            "org.example.mail".into(),
            "org.example.system".into(),
            "org.example.hidden".into(),
        ]);
        for mask in 0..16 {
            let original = available
                .iter()
                .enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, package)| package.clone())
                .collect::<BTreeSet<_>>();
            for initial in ["only", "exclude"] {
                let mut mode = initial;
                let mut selected = original.clone();
                let expected = captured(initial, &selected, &available);
                for _ in 0..8 {
                    let next = if mode == "only" { "exclude" } else { "only" };
                    selected = selection_after_mode_change(mode, next, &selected, &available);
                    assert_eq!(captured(next, &selected, &available), expected);
                    mode = next;
                }
                assert_eq!(selected, original);
                let dormant = selection_after_mode_change(mode, "all", &selected, &available);
                for next in ["only", "exclude"] {
                    let restored = selection_after_mode_change("all", next, &dormant, &available);
                    assert_eq!(captured(next, &restored, &available), available);
                }
            }
        }
    }
}
