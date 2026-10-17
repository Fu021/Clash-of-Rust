//! Validate the public model contract without recompiling generated components.
use clash_client_ui::{models::ViewModels, *};
use slint::Model;

mod wsl;

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
