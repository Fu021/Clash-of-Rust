//! Slint is confined to this process and its testable data adapter.
use clash_client_ui as ui;
use clash_desktop_protocol as wire;
use slint::{Model, ModelRc, VecModel};
mod activation;
mod motion;
mod process;
mod repaint;
mod view_state;
pub use process::run;
fn equal_rows<T: Clone + 'static, Source>(
    model: &ModelRc<T>,
    rows: &[Source],
    equal: impl Fn(&T, &Source) -> bool,
) -> bool {
    model.row_count() == rows.len()
        && rows
            .iter()
            .enumerate()
            .all(|(index, row)| model.row_data(index).is_some_and(|old| equal(&old, row)))
}
fn list<T: Clone + 'static, Source>(
    old: ModelRc<T>,
    rows: &[Source],
    equal: impl Fn(&T, &Source) -> bool,
    convert: impl Fn(&Source) -> T,
) -> ModelRc<T> {
    if equal_rows(&old, rows, equal) {
        old
    } else {
        ModelRc::new(VecModel::from(rows.iter().map(convert).collect::<Vec<_>>()))
    }
}
include!(concat!(env!("OUT_DIR"), "/bridge.rs"));

fn groups(models: &mut ui::models::ViewModels, rows: &[wire::GroupItem]) {
    let rows = rows
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let nodes = source.nodes.with_rows(|nodes| {
                models.group_nodes_with(source.id.as_str(), nodes, same_NodePair, convert_NodePair)
            });
            models
                .groups
                .row_data(index)
                .filter(|row| same_GroupItem(row, source) && row.nodes == nodes)
                .unwrap_or_else(|| {
                    let mut row = convert_GroupItem(source);
                    row.nodes = nodes;
                    row
                })
        })
        .collect();
    models.set_group_rows(rows);
}
pub fn invalidate_window(window: &slint::Window) {
    repaint::invalidate(window);
}

/// An in-flight frame must not overwrite input typed after it was produced.
pub fn apply_if_current(
    frame: &wire::Frame,
    data: &ui::UiData<'_>,
    models: &mut ui::models::ViewModels,
    accepted: u64,
    current: u64,
) -> bool {
    if accepted < current {
        return false;
    }
    apply(frame, data, models);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn unchanged_lists_skip_conversion_and_new_content_replaces_the_model() {
        let converted = Cell::new(0);
        let convert = |source: &&str| {
            converted.set(converted.get() + 1);
            slint::SharedString::from(*source)
        };
        let equal = |row: &slint::SharedString, source: &&str| row.as_str() == *source;
        let initial = list(ModelRc::default(), &["one", "two"], equal, convert);
        assert_eq!(converted.get(), 2);
        let unchanged = list(initial.clone(), &["one", "two"], equal, convert);
        assert_eq!(initial, unchanged);
        assert_eq!(converted.get(), 2);
        let changed = list(unchanged, &["one", "three"], equal, convert);
        assert_ne!(initial, changed);
        assert_eq!(changed.row_data(1).unwrap(), "three");
        let empty: &[&str] = &[];
        let cleared = list(changed, empty, equal, convert);
        assert_eq!(cleared.row_count(), 0);
        assert_eq!(initial.row_count(), 2);
    }

    #[test]
    fn group_projection_reuses_text_and_updates_selection_reordering_and_collapsing() {
        let mut models = ui::models::ViewModels::default();
        let make_group = |id: &str| wire::GroupItem {
            id: id.into(),
            title: id.into(),
            expanded: true,
            nodes: vec![wire::NodePair {
                left: wire::NodeItem {
                    name: "🚀 🇯🇵 日本".into(),
                    valid: true,
                    ..Default::default()
                },
                ..Default::default()
            }]
            .into(),
            ..Default::default()
        };
        let mut rows = [make_group("one"), make_group("two")];
        groups(&mut models, &rows);
        let first = models.groups.row_data(0).unwrap();
        let node = first.nodes.row_data(0).unwrap();
        groups(&mut models, &rows);
        let unchanged = models.groups.row_data(0).unwrap();
        assert_eq!(
            first.title.as_str().as_ptr(),
            unchanged.title.as_str().as_ptr()
        );
        assert_eq!(first.nodes, unchanged.nodes);
        assert_eq!(
            node.left.name.as_str().as_ptr(),
            unchanged
                .nodes
                .row_data(0)
                .unwrap()
                .left
                .name
                .as_str()
                .as_ptr()
        );
        let mut selected = rows[0].nodes.row_data(0).unwrap();
        selected.left.selected = true;
        rows[0].nodes = vec![selected].into();
        groups(&mut models, &rows);
        assert!(first.nodes.row_data(0).unwrap().left.selected);
        rows.swap(0, 1);
        groups(&mut models, &rows);
        assert_eq!(models.groups.row_data(1).unwrap().nodes, first.nodes);
        rows[1].expanded = false;
        rows[1].nodes = Default::default();
        groups(&mut models, &rows);
        assert_eq!(models.groups.row_data(1).unwrap().nodes.row_count(), 0);
        assert_eq!(first.nodes.row_count(), 1);
        rows[1] = make_group("one");
        groups(&mut models, &rows);
        let reopened = models.groups.row_data(1).unwrap().nodes;
        assert_ne!(first.nodes, reopened);
        assert!(!reopened.row_data(0).unwrap().left.selected);
        groups(&mut models, &[]);
        assert_eq!(models.groups.row_count(), 0);
    }
}
