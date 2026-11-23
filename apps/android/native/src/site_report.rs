use clash_mobile_runtime::core::{
    config::Language,
    countries,
    ip_check::{CheckResult, Service},
    ip_report::{self, Status},
    probe,
};

pub(crate) fn category_row(
    category: &ip_report::Category,
    language: Language,
) -> (String, String, String, i32) {
    let counts = &category.counts;
    let has_failure = counts.count(Status::Restricted) + counts.count(Status::Failed) > 0;
    (
        category.name.into(),
        counts.total().to_string(),
        counts.summary_in(language),
        if has_failure { 2 } else { 1 },
    )
}

pub(crate) fn country_label(
    outcome: Option<&Result<CheckResult, String>>,
    country: &str,
    language: Language,
) -> String {
    let skipped = matches!(outcome, Some(Ok(result)) if result.authentication_check_skipped);
    let country = ip_report::country_in(country, skipped, language);
    match language {
        Language::SimplifiedChinese => {
            probe::country_name(country).unwrap_or_else(|| country.into())
        }
        Language::English => countries::country_code(countries::country_text(country))
            .and_then(countries::english_name)
            .unwrap_or(country)
            .into(),
    }
}

fn category_title(group: &str, language: Language) -> &str {
    if language == Language::SimplifiedChinese {
        return group;
    }
    match group {
        "IP信息" => "IP information",
        "全球" => "Global",
        "东南亚" => "Southeast Asia",
        "体育" => "Sports",
        "其他" => "Other",
        "北美" => "North America",
        "南美" => "South America",
        "台湾" => "Taiwan",
        "大洋洲" => "Oceania",
        "日本" => "Japan",
        "欧洲" => "Europe",
        "非洲" => "Africa",
        "韩国" => "South Korea",
        "香港" => "Hong Kong",
        _ => group,
    }
}

pub(crate) fn detail_rows(
    service: &Service,
    outcome: Option<&Result<CheckResult, String>>,
    summary: &str,
    country: &str,
    language: Language,
) -> Vec<(String, String)> {
    let skipped = matches!(outcome, Some(Ok(result)) if result.authentication_check_skipped);
    let mut rows = vec![
        (
            language.text("平台", "Service").into(),
            service.name.clone(),
        ),
        (language.text("结果", "Result").into(), summary.into()),
    ];
    if let Some(Ok(result)) = outcome {
        let duration = ip_report::response_duration_in(result, language);
        if !duration.is_empty() {
            rows.push((language.text("耗时", "Duration").into(), duration));
        }
    }
    rows.extend([
        (
            language.text("详情", "Details").into(),
            outcome.map_or_else(String::new, |outcome| {
                ip_report::detail_in(service, outcome, language).into_owned()
            }),
        ),
        (
            language.text("分类", "Category").into(),
            category_title(&service.group, language).into(),
        ),
        (
            language.text("地区", "Region").into(),
            countries::localized_name(ip_report::country_in(country, skipped, language), language),
        ),
    ]);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_mobile_runtime::core::ip_check::{State, TargetExit, services};

    #[test]
    fn site_report_localizes_category_details_and_keeps_category_keys() {
        let report = ip_report::Report::from_results(&[]);
        for (group, english) in [
            ("北美", "North America"),
            ("全球", "Global"),
            ("IP信息", "IP information"),
            ("韩国", "South Korea"),
            ("AI", "AI"),
            ("自定义分类", "自定义分类"),
        ] {
            let service = Service {
                id: "fixture".into(),
                name: "上游服务".into(),
                group: group.into(),
                url: "https://fixture.example".into(),
            };
            let outcome = Err("上游原文".into());
            for (language, expected) in [
                (Language::SimplifiedChinese, group),
                (Language::English, english),
            ] {
                let rows = detail_rows(&service, Some(&outcome), "fixture", "", language);
                let value = |key| rows.iter().find(|row| row.0 == key).unwrap().1.as_str();
                assert_eq!(value(language.text("分类", "Category")), expected);
                assert_eq!(value(language.text("平台", "Service")), service.name);
                assert_eq!(value(language.text("详情", "Details")), "上游原文");
                if let Some(category) = report.categories.iter().find(|item| item.name == group) {
                    assert_eq!(category_row(category, language).0, group);
                }
            }
        }
    }

    #[test]
    fn site_report_localizes_native_region_details_and_keeps_upstream_text() {
        let service = &services()[0];
        let evidence = "No (Unsupported Region) · 上游原文";
        let outcome = Ok(CheckResult {
            state: State::Restricted,
            summary: "地区不支持".into(),
            country: "日本".into(),
            millis: Some(14),
            authentication_check_skipped: false,
            detail: format!("原生地区检测结果：{evidence}"),
            target_exit: None,
        });
        let result = outcome.as_ref().unwrap();
        for (language, summary, detail_prefix, region) in [
            (
                Language::SimplifiedChinese,
                "地区不支持",
                "原生地区检测结果：",
                "日本",
            ),
            (
                Language::English,
                "Unsupported region",
                "Native region check result: ",
                "Japan",
            ),
        ] {
            let value = Status::of(Some(&outcome)).summary_in(&result.summary, language);
            let rows = detail_rows(service, Some(&outcome), value, &result.country, language);
            assert!(rows.contains(&(language.text("结果", "Result").into(), summary.into())));
            assert!(rows.contains(&(
                language.text("详情", "Details").into(),
                format!("{detail_prefix}{evidence}"),
            )));
            assert!(rows.contains(&(language.text("地区", "Region").into(), region.into())));
        }
    }

    #[test]
    fn site_report_details_distinguish_skipped_authentication_from_real_zero() {
        let service = Service {
            id: "fixture".into(),
            name: "Fixture".into(),
            group: "Fixture".into(),
            url: "https://fixture.example".into(),
        };
        let catalog = services();
        let index = catalog
            .iter()
            .position(|item| item.id == "MediaUnlockTest_DisneyPlus")
            .unwrap();
        let mut results = vec![None; index + 1];
        for (language, duration_key, details_key, no_measurement, reason) in [
            (
                Language::SimplifiedChinese,
                "耗时",
                "详情",
                "无响应测量",
                "未执行认证检测",
            ),
            (
                Language::English,
                "Duration",
                "Details",
                "No response measurement",
                "Authentication check was not run",
            ),
        ] {
            for skipped in [true, false] {
                let outcome = Ok(CheckResult {
                    state: State::Unknown,
                    summary: "fixture".into(),
                    country: "未提供".into(),
                    millis: Some(0),
                    authentication_check_skipped: skipped,
                    detail: "fixture evidence".into(),
                    target_exit: Some(TargetExit {
                        host: "fixture.example".into(),
                        ip: "203.0.113.13".parse().unwrap(),
                        country: None,
                    }),
                });
                let rows = detail_rows(&service, Some(&outcome), "fixture", "未提供", language);
                let value = |key| rows.iter().find(|row| row.0 == key).unwrap().1.as_str();
                let list_detail = ip_report::list_detail_in(&service, Some(&outcome), language);
                let region = if skipped && language == Language::English {
                    "Not provided"
                } else {
                    "未提供"
                };
                assert_eq!(value(language.text("地区", "Region")), region);
                assert_eq!(country_label(Some(&outcome), "未提供", language), region);
                results[index] = Some(outcome.clone());
                let report = ip_report::Report::from_results(&results);
                let category = report
                    .categories
                    .iter()
                    .find(|category| category.name == catalog[index].group)
                    .unwrap();
                let (_, count, summary, _) = category_row(category, language);
                assert_eq!(count, category.counts.total().to_string());
                let pending = category.counts.total() - 1;
                assert_eq!(
                    summary,
                    match language {
                        Language::SimplifiedChinese => format!("结果未确认 1 · 未检测 {pending}"),
                        Language::English => format!("Unconfirmed 1 · Not checked {pending}"),
                    }
                );
                assert!(list_detail.contains("203.0.113.13"));
                if skipped {
                    assert_eq!(value(duration_key), no_measurement);
                    assert!(value(details_key).contains(reason));
                    assert!(value(details_key).contains("203.0.113.13"));
                    assert!(list_detail.contains(no_measurement));
                } else {
                    assert_eq!(value(duration_key), "0 ms");
                    assert_eq!(value(details_key), "fixture evidence");
                    assert!(!list_detail.contains(no_measurement));
                }
            }
        }
    }
}
