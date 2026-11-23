use crate::messages;
use clash_mobile_runtime::core::{
    config::Language, error_message, messages as shared_messages, network_tools,
};

#[derive(Default)]
pub(crate) enum NetworkProbe {
    #[default]
    Empty,
    Paused,
    Cancelled,
    Failed(String),
    Complete(network_tools::Report),
}

impl NetworkProbe {
    pub(crate) fn lines(&self, language: Language) -> Vec<String> {
        match self {
            Self::Empty => Vec::new(),
            Self::Paused => vec![messages::network_check_paused(language).into()],
            Self::Cancelled => vec![shared_messages::network_tool_cancelled(language).into()],
            Self::Failed(error) => vec![shared_messages::network_tool_failed(
                language,
                error_message::localize(error, language),
            )],
            Self::Complete(report) => report.lines_in(language),
        }
    }

    pub(crate) fn codes(&self) -> Vec<&'static str> {
        match self {
            Self::Complete(report) => report.entries.iter().map(|entry| entry.code).collect(),
            Self::Cancelled => vec!["cancelled"],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_and_status_follow_language_without_losing_original_evidence() {
        let report = network_tools::Report {
            schema: 1,
            tool: network_tools::Tool::Dns,
            target: "example.test".into(),
            elapsed_ms: 12,
            status: "complete",
            entries: vec![network_tools::Entry {
                label: "测试项目".into(),
                detail: "原始数据".into(),
                success: Some(true),
                code: "ok",
                detail_english: Some("Original data".into()),
            }],
        };
        let probe = NetworkProbe::Complete(report);
        assert!(
            probe
                .lines(Language::English)
                .join("\n")
                .contains("Original data")
        );
        assert!(
            probe
                .lines(Language::SimplifiedChinese)
                .join("\n")
                .contains("原始数据")
        );
        assert_eq!(probe.codes(), ["ok"]);
        assert_eq!(
            NetworkProbe::Cancelled.lines(Language::English),
            ["Check cancelled"]
        );
        assert_eq!(
            NetworkProbe::Cancelled.lines(Language::SimplifiedChinese),
            ["检查已取消"]
        );
    }
}
