#[cfg(target_os = "android")]
use crate::android::MobileIntent;
#[cfg(not(target_os = "android"))]
use crate::test_ui::MobileIntent;
use anyhow::{Result, ensure};
use clash_mobile_runtime::core::{config::Language, protection::READ_ONLY_NOTICE};

pub(crate) fn check(
    intent: MobileIntent,
    value: &str,
    configured: bool,
    pending: bool,
    language: Language,
) -> Result<()> {
    ensure!(
        !(configured || pending) || allowed(intent, value),
        "{}",
        language.text(
            READ_ONLY_NOTICE,
            "Disable Claude protection first; network changes and updates are locked while protection is active",
        )
    );
    Ok(())
}

fn allowed(intent: MobileIntent, value: &str) -> bool {
    matches!(
        intent,
        MobileIntent::Navigate
            | MobileIntent::RestartCore
            | MobileIntent::Appearance
            | MobileIntent::Accent
            | MobileIntent::TextScale
            | MobileIntent::Language
            | MobileIntent::NodeSort
            | MobileIntent::ClearLogs
            | MobileIntent::OverviewRefresh
            | MobileIntent::OverviewPrivacy
            | MobileIntent::Query
            | MobileIntent::NodePage
            | MobileIntent::GroupExpansion
            | MobileIntent::GroupPage
            | MobileIntent::GroupPagination
            | MobileIntent::ProfilePage
            | MobileIntent::ProfilePagination
            | MobileIntent::ProfileNew
            | MobileIntent::ProfileClose
            | MobileIntent::PickFile
            | MobileIntent::ConnectionPage
            | MobileIntent::RulePage
            | MobileIntent::RuleFilter
            | MobileIntent::RuleSelect
            | MobileIntent::ConnectionSort
            | MobileIntent::ConnectionDetail
            | MobileIntent::Copy
            | MobileIntent::LogLevel
            | MobileIntent::ExportLogs
            | MobileIntent::ExportDiagnostics
            | MobileIntent::CopyDiagnostics
            | MobileIntent::ReportIssue
            | MobileIntent::Probe
            | MobileIntent::NetworkTool
            | MobileIntent::ProbeCancel
            | MobileIntent::SiteStart
            | MobileIntent::SiteCancel
            | MobileIntent::SiteCategory
            | MobileIntent::SiteFilter
            | MobileIntent::SiteRetest
            | MobileIntent::SiteProbe
            | MobileIntent::SiteDetail
            | MobileIntent::ClaudeStart
            | MobileIntent::ClaudeCancel
            | MobileIntent::ExportReport
            | MobileIntent::NodeDelay
            | MobileIntent::GroupDelay
            | MobileIntent::DelayAll
            | MobileIntent::SystemNotification
            | MobileIntent::CancelUpdate
            | MobileIntent::OpenRelease
    ) || (intent == MobileIntent::ClaudeProtection && value == "false")
        || (intent == MobileIntent::ApplicationPolicy
            && matches!(
                value,
                "open" | "refresh" | "query" | "filter" | "system" | "edit" | "cancel" | "close"
            ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_close_is_blocked_while_protection_is_configured_or_changing() {
        for (configured, pending) in [(false, false), (true, false), (false, true), (true, true)] {
            let check =
                |intent, value| super::check(intent, value, configured, pending, Language::English);
            assert_eq!(
                check(MobileIntent::CloseConnection, "connection-id").is_ok(),
                !(configured || pending)
            );
            assert_eq!(
                check(MobileIntent::CloseAll, "").is_ok(),
                !(configured || pending)
            );
            assert!(check(MobileIntent::ConnectionDetail, "connection-id").is_ok());
            assert!(check(MobileIntent::Copy, "connection:connection-id").is_ok());
        }
    }
}
