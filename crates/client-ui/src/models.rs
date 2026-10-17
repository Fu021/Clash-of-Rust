//! Stable model identities keep focus, scroll positions and unchanged rows intact.
use crate::*;
use slint::{Model, SharedString, VecModel};
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

pub const WSL_MAX_DISTRIBUTIONS: usize = 128;
pub const WSL_MAX_CHECKS: usize = 256;
pub const WSL_MAX_LOGS: usize = 256;
pub const WSL_MAX_TEXT_CHARS: usize = 2048;
const WSL_MAX_LABEL_CHARS: usize = 256;

/// Preserve the model's identity while bounding the number of retained rows.
pub fn reconcile_bounded<T: Clone + PartialEq + 'static>(
    model: &VecModel<T>,
    mut rows: Vec<T>,
    limit: usize,
) {
    rows.truncate(limit);
    reconcile(model, rows);
}

// Display bounds are not a credential filter: callers must supply safe text.
fn wsl_text(value: SharedString, limit: usize) -> SharedString {
    match value.char_indices().nth(limit) {
        Some((boundary, _)) => format!("{}…", &value[..boundary]).into(),
        None => value,
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
    pub wsl_distributions: Rc<VecModel<WslDistributionItem>>,
    pub wsl_checks: Rc<VecModel<DetailItem>>,
    pub wsl_logs: Rc<VecModel<SharedString>>,
    wsl_generation: Option<u64>,
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
        data.set_wsl_distributions(self.wsl_distributions.clone().into());
        data.set_wsl_checks(self.wsl_checks.clone().into());
        data.set_wsl_logs(self.wsl_logs.clone().into());
    }
    /// Replace all three lists atomically with respect to the generation check.
    /// Return false before changing any row when the snapshot is stale. The
    /// caller must also reject its scalar status fields and operation identity.
    /// Call with empty lists on configuration changes to invalidate old evidence.
    pub fn set_wsl_results(
        &mut self,
        generation: u64,
        mut distributions: Vec<WslDistributionItem>,
        mut checks: Vec<DetailItem>,
        logs: Vec<SharedString>,
    ) -> bool {
        if self
            .wsl_generation
            .is_some_and(|current| generation < current)
        {
            return false;
        }
        self.wsl_generation = Some(generation);

        // Never silently hide missing coverage behind an apparently complete list.
        if distributions.len() > WSL_MAX_DISTRIBUTIONS {
            let omitted = distributions.len() - (WSL_MAX_DISTRIBUTIONS - 1);
            distributions.truncate(WSL_MAX_DISTRIBUTIONS - 1);
            distributions.push(WslDistributionItem {
                name: format!("另有 {omitted} 个发行版未展示").into(),
                state: "display-truncated".into(),
                coverage: "unknown · 请检查完整覆盖报告".into(),
            });
        }
        if checks.len() > WSL_MAX_CHECKS {
            let omitted = checks.len() - (WSL_MAX_CHECKS - 1);
            checks.truncate(WSL_MAX_CHECKS - 1);
            checks.push(DetailItem {
                label: "display-truncated".into(),
                value: format!("另有 {omitted} 项检查未展示；不能由此列表推导完整验收。").into(),
            });
        }
        let distributions = distributions
            .into_iter()
            .map(|row| WslDistributionItem {
                name: wsl_text(row.name, WSL_MAX_LABEL_CHARS),
                state: wsl_text(row.state, WSL_MAX_LABEL_CHARS),
                coverage: wsl_text(row.coverage, WSL_MAX_TEXT_CHARS),
            })
            .collect();
        let checks = checks
            .into_iter()
            .map(|row| DetailItem {
                label: wsl_text(row.label, WSL_MAX_LABEL_CHARS),
                value: wsl_text(row.value, WSL_MAX_TEXT_CHARS),
            })
            .collect();
        // Logs are chronological; keep the newest entries, not the oldest ones.
        let skip = logs.len().saturating_sub(WSL_MAX_LOGS);
        let logs = logs
            .into_iter()
            .skip(skip)
            .map(|line| wsl_text(line, WSL_MAX_TEXT_CHARS))
            .collect();
        reconcile_bounded(
            &self.wsl_distributions,
            distributions,
            WSL_MAX_DISTRIBUTIONS,
        );
        reconcile_bounded(&self.wsl_checks, checks, WSL_MAX_CHECKS);
        reconcile_bounded(&self.wsl_logs, logs, WSL_MAX_LOGS);
        true
    }
    /// Release hidden rows without allowing a late response to lower generation.
    pub fn clear_wsl_results(&self) {
        reconcile(&self.wsl_distributions, Vec::new());
        reconcile(&self.wsl_checks, Vec::new());
        reconcile(&self.wsl_logs, Vec::new());
    }
    pub fn set_groups(&mut self, groups: Vec<(GroupItem, Vec<NodePair>)>) {
        self.nodes
            .retain(|key, _| groups.iter().any(|(group, _)| group.id.as_str() == key));
        let mut rows = Vec::with_capacity(groups.len());
        for (mut group, nodes) in groups {
            let model = self.nodes.entry(group.id.to_string()).or_default();
            reconcile(model, nodes);
            group.nodes = model.clone().into();
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
        self.clear_wsl_results();
    }
}
