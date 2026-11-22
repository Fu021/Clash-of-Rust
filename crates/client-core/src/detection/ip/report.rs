//! A small aggregate over the single set of platform results.
use crate::ip_check::{self, CheckResult, State};
use std::fmt;

pub type ResultSlot = Option<Result<CheckResult, String>>;

pub fn target_exit_detail(
    service: &ip_check::Service,
    result: Option<&Result<CheckResult, String>>,
) -> Option<String> {
    target_exit_detail_in(service, result, crate::config::Language::SimplifiedChinese)
}

pub fn target_exit_detail_in(
    service: &ip_check::Service,
    result: Option<&Result<CheckResult, String>>,
    language: crate::config::Language,
) -> Option<String> {
    let host = ip_check::target_host(service)?;
    let value = match result {
        None => language.text("待检测", "Pending").to_owned(),
        Some(Ok(result)) => result
            .target_exit
            .as_ref()
            .filter(|exit| exit.host == host)
            .map_or_else(
                || language.text("未确认", "Unconfirmed").to_owned(),
                |exit| match &exit.country {
                    Some(country) => {
                        let displayed = match language {
                            crate::config::Language::SimplifiedChinese => country.as_str(),
                            crate::config::Language::English => crate::countries::country_code(
                                crate::countries::country_text(country),
                            )
                            .and_then(crate::countries::english_name)
                            .unwrap_or(country),
                        };
                        format!("{} · {displayed}", exit.ip)
                    }
                    None => exit.ip.to_string(),
                },
            ),
        Some(Err(_)) => language.text("未确认", "Unconfirmed").to_owned(),
    };
    Some(format!(
        "{}{value}",
        language.text("同域出口：", "Same-domain exit: ")
    ))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CategoryFilter {
    #[default]
    All,
    Group(&'static str),
}

impl fmt::Display for CategoryFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => f.write_str("全部分类"),
            Self::Group(name) => f.write_str(name),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StatusFilter {
    #[default]
    All,
    Status(Status),
}

impl fmt::Display for StatusFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => f.write_str("全部结果"),
            Self::Status(status) => status.fmt(f),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RegionFilter {
    #[default]
    All,
    Unidentified,
    Country(&'static str),
}

impl fmt::Display for RegionFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => f.write_str("全部识别地区"),
            Self::Unidentified => f.write_str("未识别地区"),
            Self::Country(code) => {
                f.write_str(&crate::probe::country_name(code).unwrap_or_else(|| (*code).into()))
            }
        }
    }
}

pub fn region(result: Option<&Result<CheckResult, String>>) -> Option<&'static str> {
    let country = &result?.as_ref().ok()?.country;
    crate::countries::country_code(crate::countries::country_text(country))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Available,
    Partial,
    Reachable,
    Identified,
    Restricted,
    Unknown,
    Failed,
    Untested,
}

impl Status {
    pub const ALL: [Self; 8] = [
        Self::Available,
        Self::Partial,
        Self::Reachable,
        Self::Identified,
        Self::Restricted,
        Self::Unknown,
        Self::Failed,
        Self::Untested,
    ];

    pub fn of(result: Option<&Result<CheckResult, String>>) -> Self {
        match result {
            Some(Ok(result)) => match result.state {
                State::Confirmed => Self::Available,
                State::Partial => Self::Partial,
                State::Reachable => Self::Reachable,
                State::Identified => Self::Identified,
                State::Restricted => Self::Restricted,
                State::Unknown => Self::Unknown,
                State::Failed => Self::Failed,
            },
            Some(Err(_)) => Self::Failed,
            None => Self::Untested,
        }
    }

    pub fn label_in(self, language: crate::config::Language) -> &'static str {
        match self {
            Self::Available => language.text("确认可用", "Available"),
            Self::Partial => language.text("部分可用", "Partially available"),
            Self::Reachable => language.text("仅网页可达", "Website reachable"),
            Self::Identified => language.text("信息已识别", "Identified"),
            Self::Restricted => language.text("不可用或受限", "Unavailable or restricted"),
            Self::Unknown => language.text("结果未确认", "Unconfirmed"),
            Self::Failed => language.text("超时或失败", "Timed out or failed"),
            Self::Untested => language.text("未检测", "Not checked"),
        }
    }

    pub fn summary_is_generic(self, summary: &str) -> bool {
        summary == self.label_in(crate::config::Language::SimplifiedChinese)
            || (self == Self::Reachable && summary == "网页可达")
    }

    pub fn summary_in(self, summary: &str, language: crate::config::Language) -> &str {
        if language == crate::config::Language::English && self.summary_is_generic(summary) {
            self.label_in(language)
        } else {
            summary
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label_in(crate::config::Language::SimplifiedChinese))
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts(pub [usize; 8]);

impl Counts {
    pub fn count(&self, status: Status) -> usize {
        self.0[status as usize]
    }

    pub fn total(&self) -> usize {
        self.0.iter().sum()
    }

    pub fn completed(&self) -> usize {
        self.total() - self.count(Status::Untested)
    }

    /// Green and yellow outcomes; this is not a count of confirmed unlocks.
    pub fn non_red(&self) -> usize {
        self.count(Status::Available)
            + self.count(Status::Partial)
            + self.count(Status::Reachable)
            + self.count(Status::Identified)
            + self.count(Status::Unknown)
    }
}

pub struct Category {
    pub name: &'static str,
    pub counts: Counts,
}

pub struct Report {
    pub totals: Counts,
    pub categories: Vec<Category>,
}

impl Report {
    pub fn from_results(results: &[ResultSlot]) -> Self {
        let mut report = Self {
            totals: Counts::default(),
            categories: Vec::with_capacity(15),
        };
        for (index, service) in ip_check::services().iter().enumerate() {
            let status = Status::of(results.get(index).and_then(Option::as_ref));
            report.totals.0[status as usize] += 1;
            let category = match report
                .categories
                .iter()
                .position(|c| c.name == service.group)
            {
                Some(index) => &mut report.categories[index],
                None => {
                    report.categories.push(Category {
                        name: service.group.as_str(),
                        counts: Counts::default(),
                    });
                    report.categories.last_mut().unwrap()
                }
            };
            category.counts.0[status as usize] += 1;
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(state: State) -> ResultSlot {
        Some(Ok(CheckResult {
            state,
            summary: String::new(),
            country: String::new(),
            millis: 1,
            detail: String::new(),
            target_exit: None,
        }))
    }

    #[test]
    fn generic_status_summaries_translate_without_changing_custom_evidence() {
        use crate::config::Language;

        assert_eq!(
            Status::Reachable.summary_in("网页可达", Language::English),
            "Website reachable"
        );
        assert_eq!(
            Status::Available.summary_in("确认可用", Language::English),
            "Available"
        );
        assert_eq!(
            Status::Available.summary_in("自定义地区结果", Language::English),
            "自定义地区结果"
        );
        assert_eq!(
            Status::Reachable.summary_in("网页可达", Language::SimplifiedChinese),
            "网页可达"
        );
    }

    #[test]
    fn target_exit_detail_keeps_targets_pending_failures_and_addresses_distinct() {
        let mut results: Vec<ResultSlot> = std::iter::repeat_with(|| None)
            .take(ip_check::services().len())
            .collect();
        let index = |id| {
            ip_check::services()
                .iter()
                .position(|service| service.id == id)
                .unwrap()
        };
        let claude = index("AIUnlockTest_Claude");
        let chatgpt = index("MediaUnlockTest_ChatGPT");
        let mut measured = result(State::Reachable);
        measured.as_mut().unwrap().as_mut().unwrap().target_exit = Some(ip_check::TargetExit {
            host: "claude.ai".into(),
            ip: "8.8.8.8".parse().unwrap(),
            country: Some("美国".into()),
        });
        results[claude] = measured;
        results[chatgpt] = Some(Err("timeout".into()));
        assert_eq!(
            target_exit_detail(&ip_check::services()[claude], results[claude].as_ref()).as_deref(),
            Some("同域出口：8.8.8.8 · 美国")
        );
        assert_eq!(
            target_exit_detail_in(
                &ip_check::services()[claude],
                results[claude].as_ref(),
                crate::config::Language::English,
            )
            .as_deref(),
            Some("Same-domain exit: 8.8.8.8 · United States")
        );
        assert_eq!(
            target_exit_detail_in(
                &ip_check::services()[chatgpt],
                None,
                crate::config::Language::English,
            )
            .as_deref(),
            Some("Same-domain exit: Pending")
        );
        assert_eq!(
            target_exit_detail_in(
                &ip_check::services()[chatgpt],
                results[chatgpt].as_ref(),
                crate::config::Language::English,
            )
            .as_deref(),
            Some("Same-domain exit: Unconfirmed")
        );
        assert_eq!(
            target_exit_detail(&ip_check::services()[chatgpt], results[chatgpt].as_ref())
                .as_deref(),
            Some("同域出口：未确认")
        );
        assert_eq!(
            target_exit_detail(&ip_check::services()[chatgpt], None).as_deref(),
            Some("同域出口：待检测")
        );
        results[chatgpt] = result(State::Reachable);
        assert_eq!(
            target_exit_detail(&ip_check::services()[chatgpt], results[chatgpt].as_ref())
                .as_deref(),
            Some("同域出口：未确认")
        );
        let claude_result = results[claude].as_mut().unwrap().as_mut().unwrap();
        claude_result.target_exit = Some(ip_check::TargetExit {
            host: "chatgpt.com".into(),
            ip: "1.1.1.1".parse().unwrap(),
            country: None,
        });
        assert_eq!(
            target_exit_detail(&ip_check::services()[claude], results[claude].as_ref()).as_deref(),
            Some("同域出口：未确认")
        );
        assert_eq!(
            target_exit_detail(
                &ip_check::services()[index("AIUnlockTest_Gemini_location")],
                None
            )
            .as_deref(),
            Some("同域出口：待检测")
        );
        assert!(target_exit_detail(&ip_check::services()[index("exit-ip")], None).is_none());
    }

    #[test]
    fn status_labels_distinguish_failures_from_restrictions_and_unknown_results() {
        for (slot, label) in [
            (result(State::Restricted), "不可用或受限"),
            (result(State::Unknown), "结果未确认"),
            (result(State::Failed), "超时或失败"),
            (Some(Err("timeout".into())), "超时或失败"),
            (None, "未检测"),
        ] {
            assert_eq!(Status::of(slot.as_ref()).to_string(), label);
        }
    }

    #[test]
    fn reports_keep_identification_partial_access_failures_and_pending_distinct() {
        let mut results: Vec<ResultSlot> = std::iter::repeat_with(|| None)
            .take(ip_check::services().len())
            .collect();
        results[0] = result(State::Identified);
        results[1] = result(State::Confirmed);
        results[2] = result(State::Partial);
        results[3] = result(State::Reachable);
        results[4] = Some(Err("timeout".into()));
        results[5] = result(State::Restricted);
        results[6] = result(State::Unknown);
        let report = Report::from_results(&results);
        assert_eq!(report.totals.total(), 182);
        assert_eq!(report.totals.completed(), 7);
        assert_eq!(report.totals.count(Status::Available), 1);
        assert_eq!(report.totals.count(Status::Identified), 1);
        assert_eq!(report.totals.count(Status::Untested), 175);
        assert_eq!(report.categories.len(), 15);
        let ai = report.categories.iter().find(|c| c.name == "AI").unwrap();
        assert_eq!(ai.counts.total(), 4);
        assert_eq!(ai.counts.completed(), 4);
        assert_eq!(ai.counts.count(Status::Failed), 1);
        for status in Status::ALL {
            assert_eq!(
                report.totals.count(status),
                report
                    .categories
                    .iter()
                    .map(|c| c.counts.count(status))
                    .sum::<usize>()
            );
        }
    }

    #[test]
    fn fresh_report_covers_every_catalog_item_without_result_allocations() {
        let report = Report::from_results(&[]);
        assert_eq!(
            report.totals.count(Status::Untested),
            ip_check::services().len()
        );
        assert_eq!(report.totals.completed(), 0);
        assert_eq!(report.totals.non_red(), 0);
    }

    #[test]
    fn category_numerator_includes_green_and_yellow_but_excludes_red_and_pending() {
        let mut results: Vec<ResultSlot> = std::iter::repeat_with(|| None)
            .take(ip_check::services().len())
            .collect();
        for (index, state) in [State::Confirmed, State::Reachable, State::Identified]
            .into_iter()
            .enumerate()
        {
            results[index + 1] = result(state);
        }
        let report = Report::from_results(&results);
        let ai = report.categories.iter().find(|c| c.name == "AI").unwrap();
        assert_eq!(ai.counts.non_red(), 3);
        assert_eq!(ai.counts.total(), 4);
        assert_eq!(ai.counts.completed(), 3);
        results[4] = result(State::Restricted);
        let report = Report::from_results(&results);
        let ai = report.categories.iter().find(|c| c.name == "AI").unwrap();
        assert_eq!(ai.counts.non_red(), 3);
        assert_eq!(ai.counts.completed(), 4);
        let counts = Counts([1; 8]);
        assert_eq!(counts.non_red(), 5);
        assert_eq!(counts.completed(), 7);
        assert_eq!(counts.total(), 8);
    }
}
