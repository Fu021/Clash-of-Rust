//! Validate the public model contract without recompiling generated components.
use clash_client_ui::{models::ViewModels, *};
use slint::Model;
use std::cell::Cell;

#[test]
fn refresh_retains_nested_models_and_releases_removed_groups() {
    let mut models = ViewModels::default();
    let group = GroupItem {
        id: "Proxy".into(),
        ..Default::default()
    };
    let pair = NodePair {
        left: NodeItem {
            name: "node-a".into(),
            valid: true,
            ..Default::default()
        },
        ..Default::default()
    };
    models.set_groups(vec![(group.clone(), vec![pair.clone()])]);
    let nodes = models.groups.row_data(0).unwrap().nodes;
    models.set_groups(vec![(group, vec![pair])]);
    assert_eq!(nodes, models.groups.row_data(0).unwrap().nodes);
    assert_eq!(nodes.row_count(), 1);
    models.claude_results.push(ReportItem::default());
    models.clear();
    assert_eq!(models.groups.row_count(), 0);
    assert_eq!(models.claude_results.row_count(), 0);
}

#[test]
fn unchanged_rows_skip_conversion_and_changes_keep_the_model() {
    let model = slint::VecModel::default();
    let converted = Cell::new(0);
    let update = |rows: &[&str]| {
        models::reconcile_with(
            &model,
            rows,
            |old: &slint::SharedString, source| old.as_str() == *source,
            |source| {
                converted.set(converted.get() + 1);
                slint::SharedString::from(*source)
            },
        );
    };
    update(&["first", "second"]);
    assert_eq!(converted.get(), 2);
    let retained = model.row_data(0).unwrap();
    update(&["first", "second"]);
    assert_eq!(converted.get(), 2);
    update(&["first", "changed", "third"]);
    assert_eq!(converted.get(), 4);
    assert_eq!(
        retained.as_str().as_ptr(),
        model.row_data(0).unwrap().as_str().as_ptr()
    );
    update(&["first"]);
    assert_eq!(converted.get(), 4);
    assert_eq!(model.row_count(), 1);
    update(&[]);
    assert_eq!(model.row_count(), 0);
    update(&["reopened"]);
    assert_eq!(converted.get(), 5);
    assert_eq!(model.row_data(0).unwrap(), "reopened");
}

#[test]
fn collapsing_nodes_keeps_only_the_disclosure_copy_and_reopening_uses_a_new_model() {
    let mut models = ViewModels::default();
    let convert = |name: &&str| NodePair {
        left: NodeItem {
            name: (*name).into(),
            valid: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let equal = |row: &NodePair, name: &&str| row.left.name.as_str() == *name;
    let closing = models.group_nodes_with("group", &["node"], equal, convert);
    let same = models.group_nodes_with("group", &["node"], equal, |_| {
        panic!("unchanged nodes must not be converted")
    });
    assert_eq!(closing, same);
    let empty: &[&str] = &[];
    assert_eq!(
        models
            .group_nodes_with("group", empty, equal, convert)
            .row_count(),
        0
    );
    assert_eq!(closing.row_count(), 1);
    let reopened = models.group_nodes_with("group", &["new"], equal, convert);
    assert_ne!(closing, reopened);
    assert_eq!(closing.row_data(0).unwrap().left.name, "node");
    assert_eq!(reopened.row_data(0).unwrap().left.name, "new");
    models.set_group_rows(Vec::new());
    let fresh = models.group_nodes_with("group", &["fresh"], equal, convert);
    assert_ne!(fresh, reopened);
    assert_eq!(reopened.row_data(0).unwrap().left.name, "new");
}
