use std::collections::BTreeSet;

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
