//! Stable model identities keep focus, scroll positions and unchanged rows intact.
use crate::*;
use slint::{Model, VecModel};
use std::{collections::BTreeMap, rc::Rc};

pub fn reconcile<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    let length = rows.len();
    for (index, row) in rows.into_iter().enumerate() {
        if index >= model.row_count() {
            model.push(row);
        } else if model.row_data(index).as_ref() != Some(&row) {
            model.set_row_data(index, row);
        }
    }
    while model.row_count() > length {
        model.remove(model.row_count() - 1);
    }
}

#[derive(Default)]
pub struct ViewModels {
    pub groups: Rc<VecModel<GroupItem>>,
    pub profiles: Rc<VecModel<ProfileItem>>,
    pub connections: Rc<VecModel<ConnectionItem>>,
    pub rules: Rc<VecModel<RuleItem>>,
    pub logs: Rc<VecModel<LogItem>>,
    pub categories: Rc<VecModel<CategoryPair>>,
    pub site_results: Rc<VecModel<ReportItem>>,
    pub claude_results: Rc<VecModel<ReportItem>>,
    nodes: BTreeMap<String, Rc<VecModel<NodePair>>>,
}
impl ViewModels {
    pub fn attach(&self, data: &UiData<'_>) {
        data.set_groups(self.groups.clone().into());
        data.set_profiles(self.profiles.clone().into());
        data.set_connections(self.connections.clone().into());
        data.set_rules(self.rules.clone().into());
        data.set_logs(self.logs.clone().into());
        data.set_categories(self.categories.clone().into());
        data.set_site_results(self.site_results.clone().into());
        data.set_claude_results(self.claude_results.clone().into());
    }
    pub fn set_groups(&mut self, groups: Vec<(GroupItem, Vec<NodePair>)>) {
        self.nodes
            .retain(|key, _| groups.iter().any(|(group, _)| group.id.as_str() == key));
        let mut rows = Vec::with_capacity(groups.len());
        for (mut group, nodes) in groups {
            if nodes.is_empty() {
                // A closing disclosure can own the old model for its short
                // fade. Drop the backend reference immediately, without
                // mutating the rows still being drawn by that disclosure.
                self.nodes.remove(group.id.as_str());
                group.nodes = Default::default();
            } else {
                let model = self.nodes.entry(group.id.to_string()).or_default();
                reconcile(model, nodes);
                group.nodes = model.clone().into();
            }
            rows.push(group);
        }
        reconcile(&self.groups, rows);
    }
    pub fn clear(&mut self) {
        self.set_groups(Vec::new());
        reconcile(&self.profiles, Vec::new());
        reconcile(&self.connections, Vec::new());
        reconcile(&self.rules, Vec::new());
        reconcile(&self.logs, Vec::new());
        reconcile(&self.categories, Vec::new());
        reconcile(&self.site_results, Vec::new());
        reconcile(&self.claude_results, Vec::new());
    }
}
