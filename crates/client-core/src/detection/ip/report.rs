//! A small aggregate over the single set of platform results.
use crate::config::Language;
use crate::ip_check::{self, CheckResult, State};
use std::{borrow::Cow, fmt};

pub type ResultSlot = Option<Result<CheckResult, String>>;
pub(crate) const NATIVE_REGION_DETAIL_PREFIX: &str = "原生地区检测结果：";

pub fn no_response_measurement_in(language: Language) -> &'static str {
    language.text("无响应测量", "No response measurement")
}

pub fn authentication_check_skipped_in(language: Language) -> &'static str {
    language.text("未执行认证检测", "Authentication check was not run")
}

pub fn country_in(country: &str, authentication_check_skipped: bool, language: Language) -> &str {
    if authentication_check_skipped && country == "未提供" {
        language.text("未提供", "Not provided")
    } else {
        country
    }
}

pub fn response_duration_in(result: &CheckResult, language: Language) -> String {
    if result.authentication_check_skipped {
        no_response_measurement_in(language).into()
    } else {
        result
            .response_millis()
            .map_or_else(String::new, |millis| format!("{millis} ms"))
    }
}

pub fn list_detail_in(
    service: &ip_check::Service,
    outcome: Option<&Result<CheckResult, String>>,
    language: Language,
) -> String {
    let exit = target_exit_detail_in(service, outcome, language);
    if matches!(outcome, Some(Ok(result)) if result.authentication_check_skipped) {
        with_target_exit(no_response_measurement_in(language), exit).into_owned()
    } else {
        exit.unwrap_or_default()
    }
}

pub fn detail_in<'a>(
    service: &ip_check::Service,
    outcome: &'a Result<CheckResult, String>,
    language: Language,
) -> Cow<'a, str> {
    match outcome {
        Ok(result) if result.authentication_check_skipped => with_target_exit(
            authentication_check_skipped_in(language),
            target_exit_detail_in(service, Some(outcome), language),
        ),
        Ok(result) if language == Language::English => {
            let observed = result.target_exit.as_ref();
            if let Some(host) = ip_check::target_host(service)
                && observed.is_none_or(|exit| exit.host == host)
                && let Some(evidence) = result
                    .detail
                    .strip_suffix(&ip_check::target_exit_evidence_suffix(&host, observed))
            {
                let evidence = english_check_detail(service, result, evidence);
                let value =
                    observed.map_or_else(|| "Unconfirmed".to_owned(), |exit| exit.ip.to_string());
                return Cow::Owned(format!(
                    "{evidence}\nSame-domain probe exit ({host}): {value}"
                ));
            }
            english_check_detail(service, result, &result.detail)
        }
        Ok(result) => Cow::Borrowed(&result.detail),
        Err(error) => crate::error_message::localize(error, language),
    }
}

fn english_check_detail<'a>(
    service: &ip_check::Service,
    result: &CheckResult,
    detail: &'a str,
) -> Cow<'a, str> {
    if result.state == State::Failed && result.summary == ip_check::PLATFORM_ERROR_WITH_EXIT_SUMMARY
    {
        return Cow::Borrowed(detail);
    }
    if let Some(evidence) = detail.strip_prefix(NATIVE_REGION_DETAIL_PREFIX) {
        return Cow::Owned(format!("Native region check result: {evidence}"));
    }
    if service.id == "github"
        && let Some((status, phrase)) = detail
            .strip_prefix("HTTP ")
            .and_then(|detail| detail.split_once(" · "))
        && reqwest::StatusCode::from_bytes(status.as_bytes()).is_ok()
    {
        let english = match phrase {
            ip_check::BASIC_HTTP_DETAIL => "Basic HTTP response check",
            ip_check::REGIONAL_HTTP_DETAIL => "Service reported a regional restriction",
            _ => return Cow::Borrowed(detail),
        };
        return Cow::Owned(format!("HTTP {status} · {english}"));
    }
    if service.id == "exit-ip"
        && let Some((family, status)) = detail
            .strip_prefix(ip_check::EXIT_IP_DETAIL_PREFIX)
            .and_then(|detail| detail.split_once(" · HTTP "))
        && matches!(family, "IPv4" | "IPv6")
        && reqwest::StatusCode::from_bytes(status.as_bytes()).is_ok_and(|code| code.is_success())
    {
        return Cow::Owned(format!(
            "Core egress IP observed by Cloudflare · {family} · HTTP {status}"
        ));
    }
    Cow::Borrowed(detail)
}

fn with_target_exit(text: &str, exit: Option<String>) -> Cow<'_, str> {
    match exit {
        Some(exit) => Cow::Owned(format!("{text}\n{exit}")),
        None => Cow::Borrowed(text),
    }
}

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
        if language != Language::English {
            return summary;
        }
        if self.summary_is_generic(summary) {
            return self.label_in(language);
        }
        match (self, summary) {
            (Self::Available, "可用") => self.label_in(language),
            (Self::Partial, "仅网页可用") => "Website only",
            (Self::Partial, "仅自制内容") => "Originals only",
            (Self::Reachable, "可达 · 解锁未确认") => "Reachable · Unlock unconfirmed",
            (Self::Identified, "已识别地区") => "Region identified",
            (Self::Identified, "已获取地区或网络信息") => {
                "Region or network information retrieved"
            }
            (Self::Restricted, "地区不支持") => "Unsupported region",
            (Self::Restricted, "出口网络受限") => "Exit network restricted",
            (Self::Restricted, "地区受限") => "Region restricted",
            (Self::Restricted, "访问受限") => "Access restricted",
            (Self::Restricted, "请求限流") => "Rate limited",
            (Self::Unknown, "检测失败或结果未确认") => {
                "Check failed or result unconfirmed"
            }
            (Self::Unknown, "请求被拦截，未确认") => "Request blocked · Unconfirmed",
            (Self::Unknown, "已返回检测信息") => "Check information returned",
            (Self::Unknown, "验证拦截，未确认") => "Browser verification blocked the check",
            (Self::Unknown, "请求被拒绝，未确认") => "Request denied · Unconfirmed",
            (Self::Unknown, "未确认可用性") => "Availability unconfirmed",
            (Self::Failed, "检测失败") => "Check failed",
            (Self::Failed, "请求失败，未确认") => "Request failed · Unconfirmed",
            (Self::Failed, "检测失败，未确认") => "Check failed · Unconfirmed",
            _ => summary,
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

    pub fn summary_in(&self, language: Language) -> String {
        Status::ALL
            .into_iter()
            .filter_map(|status| {
                let count = self.count(status);
                (count > 0).then(|| format!("{} {count}", status.label_in(language)))
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// Historical CLI JSON v1 aggregate.
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
            let outcome = results.get(index).and_then(Option::as_ref);
            let status = Status::of(outcome);
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
            millis: Some(1),
            authentication_check_skipped: false,
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
    fn application_check_summaries_are_translated_only_as_whole_known_values() {
        for (status, raw, english) in [
            (
                Status::Reachable,
                "可达 · 解锁未确认",
                "Reachable · Unlock unconfirmed",
            ),
            (
                Status::Identified,
                "已获取地区或网络信息",
                "Region or network information retrieved",
            ),
            (Status::Restricted, "地区受限", "Region restricted"),
            (Status::Restricted, "访问受限", "Access restricted"),
            (Status::Restricted, "请求限流", "Rate limited"),
            (
                Status::Unknown,
                "验证拦截，未确认",
                "Browser verification blocked the check",
            ),
            (
                Status::Unknown,
                "请求被拒绝，未确认",
                "Request denied · Unconfirmed",
            ),
            (Status::Unknown, "未确认可用性", "Availability unconfirmed"),
            (
                Status::Failed,
                "请求失败，未确认",
                "Request failed · Unconfirmed",
            ),
            (
                Status::Failed,
                "检测失败，未确认",
                "Check failed · Unconfirmed",
            ),
            (
                Status::Partial,
                "仅自制内容（上游摘要）",
                "仅自制内容（上游摘要）",
            ),
        ] {
            assert_eq!(status.summary_in(raw, Language::English), english);
            assert_eq!(status.summary_in(raw, Language::SimplifiedChinese), raw);
        }
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
    fn legacy_non_red_includes_identified_and_unknown_but_excludes_failures_and_pending() {
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

        results[3] = result(State::Unknown);
        results[4] = result(State::Unknown);
        let Some(Ok(skipped)) = &mut results[4] else {
            unreachable!();
        };
        skipped.authentication_check_skipped = true;
        let report = Report::from_results(&results);
        let ai = report.categories.iter().find(|c| c.name == "AI").unwrap();
        assert_eq!(ai.counts.count(Status::Unknown), 2);
        assert_eq!(ai.counts.completed(), 4);
        assert_eq!(ai.counts.non_red(), 4);
        assert_eq!(
            ai.counts.summary_in(Language::SimplifiedChinese),
            "确认可用 1 · 仅网页可达 1 · 结果未确认 2"
        );
    }
}
