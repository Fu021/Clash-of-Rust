use crate::*;
pub fn reconcile<T: Clone + PartialEq>(model: &List<T>, rows: Vec<T>) {
    if !model.with_rows(|old| old == rows.as_slice()) {
        model.set_vec(rows);
    }
}
#[derive(Default)]
pub struct ViewModels {
    pub groups: List<GroupItem>,
    pub profiles: List<ProfileItem>,
    pub connections: List<ConnectionItem>,
    pub rules: List<RuleItem>,
    pub logs: List<LogItem>,
    pub categories: List<CategoryPair>,
    pub site_results: List<ReportItem>,
    pub claude_results: List<ReportItem>,
}
impl ViewModels {
    pub fn attach(&self, data: &UiData) {
        data.set_groups(self.groups.clone());
        data.set_profiles(self.profiles.clone());
        data.set_connections(self.connections.clone());
        data.set_rules(self.rules.clone());
        data.set_logs(self.logs.clone());
        data.set_categories(self.categories.clone());
        data.set_site_results(self.site_results.clone());
        data.set_claude_results(self.claude_results.clone());
    }
    pub fn set_groups(&mut self, groups: Vec<(GroupItem, Vec<NodePair>)>) {
        reconcile(
            &self.groups,
            groups
                .into_iter()
                .map(|(mut group, nodes)| {
                    group.nodes = nodes.into();
                    group
                })
                .collect(),
        );
    }
    pub fn clear(&mut self) {
        self.groups.set_vec(Vec::new());
        self.profiles.set_vec(Vec::new());
        self.connections.set_vec(Vec::new());
        self.rules.set_vec(Vec::new());
        self.logs.set_vec(Vec::new());
        self.categories.set_vec(Vec::new());
        self.site_results.set_vec(Vec::new());
        self.claude_results.set_vec(Vec::new());
    }
}
