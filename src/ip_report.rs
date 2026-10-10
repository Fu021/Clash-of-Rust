//! A small aggregate over the single set of platform results.
use crate::ip_check::{self, CheckResult, State};
use std::fmt;

pub type ResultSlot = Option<Result<CheckResult, String>>;

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
    crate::flags::country_code(crate::flags::country_text(country))
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
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Available => "确认可用",
            Self::Partial => "部分可用",
            Self::Reachable => "仅网页可达",
            Self::Identified => "信息已识别",
            Self::Restricted => "不可用或受限",
            Self::Unknown => "结果未确认",
            Self::Failed => "超时或失败",
            Self::Untested => "未检测",
        })
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
            categories: Vec::with_capacity(14),
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
        }))
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
        assert_eq!(report.totals.total(), 183);
        assert_eq!(report.totals.completed(), 7);
        assert_eq!(report.totals.count(Status::Available), 1);
        assert_eq!(report.totals.count(Status::Identified), 1);
        assert_eq!(report.totals.count(Status::Untested), 176);
        assert_eq!(report.categories.len(), 14);
        let ai = report.categories.iter().find(|c| c.name == "AI").unwrap();
        assert_eq!(ai.counts.total(), 5);
        assert_eq!(ai.counts.completed(), 5);
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
    }
}
