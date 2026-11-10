//! Stable model identities keep focus, scroll positions and unchanged rows intact.
use crate::*;
use slint::{Model, VecModel};
use std::{collections::BTreeMap, rc::Rc};

pub fn reconcile<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    let length = rows.len();
    if length == 0 {
        if model.row_count() != 0 {
            model.set_vec(Vec::new());
        }
        return;
    }
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

pub fn reconcile_with<T: Clone + 'static, Source>(
    model: &VecModel<T>,
    rows: &[Source],
    equal: impl Fn(&T, &Source) -> bool,
    convert: impl Fn(&Source) -> T,
) {
    let length = rows.len();
    if length == 0 {
        if model.row_count() != 0 {
            // Removing rows retains Vec's backing allocation. An inactive page
            // has no rows to reuse; replace the Vec while keeping model identity.
            model.set_vec(Vec::new());
        }
        return;
    }
    for (index, row) in rows.iter().enumerate() {
        if index >= model.row_count() {
            model.push(convert(row));
        } else if !model.row_data(index).is_some_and(|old| equal(&old, row)) {
            model.set_row_data(index, convert(row));
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
        let mut rows = Vec::with_capacity(groups.len());
        for (mut group, nodes) in groups {
            group.nodes =
                self.group_nodes_with(group.id.as_str(), &nodes, PartialEq::eq, Clone::clone);
            rows.push(group);
        }
        self.set_group_rows(rows);
    }
    pub fn group_nodes_with<Source>(
        &mut self,
        id: &str,
        rows: &[Source],
        equal: impl Fn(&NodePair, &Source) -> bool,
        convert: impl Fn(&Source) -> NodePair,
    ) -> slint::ModelRc<NodePair> {
        if rows.is_empty() {
            self.nodes.remove(id);
            return Default::default();
        }
        let model = self.nodes.get(id).cloned().unwrap_or_else(|| {
            let model = Rc::new(VecModel::default());
            self.nodes.insert(id.to_owned(), model.clone());
            model
        });
        reconcile_with(&model, rows, equal, convert);
        model.into()
    }
    pub fn set_group_rows(&mut self, rows: Vec<GroupItem>) {
        self.nodes
            .retain(|key, _| rows.iter().any(|group| group.id.as_str() == key));
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
