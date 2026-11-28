use super::{Language, overrides, source_view};
use serde::{Deserialize, Serialize};
use std::{fmt, io};

#[derive(Deserialize, Serialize)]
#[serde(tag = "action", content = "data", rename_all = "kebab-case")]
pub enum Action {
    OpenOverride,
    Source(source_view::Request),
    Check(overrides::Change),
    Apply(overrides::Change),
    OpenExternal(String),
    ReadExternal,
}

impl Action {
    pub fn operation(&self) -> Operation {
        match self {
            Self::OpenOverride => Operation::OpenOverride,
            Self::Source(request) => match (&request.position, request.view) {
                (source_view::Position::From(0), source_view::View::Runtime)
                    if request.revision.is_none() =>
                {
                    Operation::Runtime
                }
                (source_view::Position::From(0), _) if request.revision.is_none() => {
                    Operation::Original
                }
                _ => Operation::Navigate,
            },
            Self::Check(_) => Operation::Check,
            Self::Apply(_) => Operation::Apply,
            Self::OpenExternal(_) => Operation::OpenExternal,
            Self::ReadExternal => Operation::ReadExternal,
        }
    }
}

#[derive(Clone, Copy)]
pub enum Operation {
    OpenOverride,
    Original,
    Runtime,
    Navigate,
    Check,
    Apply,
    OpenExternal,
    ReadExternal,
}

impl Operation {
    fn code(self) -> &'static str {
        match self {
            Self::OpenOverride => "yaml_override_open_failed",
            Self::Original => "yaml_original_read_failed",
            Self::Runtime => "yaml_runtime_view_failed",
            Self::Navigate => "yaml_source_navigation_failed",
            Self::Check => "yaml_override_check_failed",
            Self::Apply => "yaml_override_apply_failed",
            Self::OpenExternal => "yaml_external_open_failed",
            Self::ReadExternal => "yaml_external_read_failed",
        }
    }

    fn summary(self, language: Language) -> &'static str {
        let (chinese, english) = match self {
            Self::OpenOverride => ("无法打开全局覆写", "Could not open the global override"),
            Self::Original => ("无法读取原始 YAML", "Could not read the original YAML"),
            Self::Runtime => ("无法查看运行 YAML", "Could not view the runtime YAML"),
            Self::Navigate => ("无法定位 YAML 内容", "Could not navigate the YAML document"),
            Self::Check => ("覆写校验失败", "Override validation failed"),
            Self::Apply => ("无法应用全局覆写", "Could not apply the global override"),
            Self::OpenExternal => ("无法启动外部编辑器", "Could not launch the external editor"),
            Self::ReadExternal => ("无法读回外部修改", "Could not load external edits"),
        };
        language.text(chinese, english)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Recovery {
    Restored,
    FilesFailed,
    RuntimeFailed,
    FilesAndRuntimeFailed,
}

impl Recovery {
    pub fn after_attempt(files_restored: bool, runtime_restored: bool) -> Self {
        match (files_restored, runtime_restored) {
            (true, true) => Self::Restored,
            (false, true) => Self::FilesFailed,
            (true, false) => Self::RuntimeFailed,
            (false, false) => Self::FilesAndRuntimeFailed,
        }
    }

    pub fn critical(self) -> bool {
        self != Self::Restored
    }

    fn detail(self, language: Language) -> &'static str {
        let (chinese, english) = match self {
            Self::Restored => (
                "配置提交失败，已恢复旧配置与订阅信息",
                "Could not commit the configuration. The previous configuration and subscription data were restored.",
            ),
            Self::FilesFailed => (
                "配置文件恢复失败，请导出诊断并检查配置状态",
                "Could not restore the configuration files. Export diagnostics and check the configuration state.",
            ),
            Self::RuntimeFailed => (
                "原运行配置恢复失败，请检查内核运行状态",
                "Could not restore the previous runtime configuration. Check the core status.",
            ),
            Self::FilesAndRuntimeFailed => (
                "配置文件和原运行配置均恢复失败，请导出诊断",
                "Could not restore the configuration files or the previous runtime configuration. Export diagnostics.",
            ),
        };
        language.text(chinese, english)
    }
}

impl fmt::Display for Recovery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail(Language::SimplifiedChinese))
    }
}

impl std::error::Error for Recovery {}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct SyntaxLocation {
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for SyntaxLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "全局覆写 YAML 格式错误 · {}:{}",
            self.line, self.column
        )
    }
}

impl std::error::Error for SyntaxLocation {}

pub struct Failure {
    operation: Operation,
    recovery: Option<Recovery>,
    syntax: Option<(usize, usize)>,
    io: Option<(io::ErrorKind, Option<i32>)>,
    reason: Option<&'static str>,
}

impl Failure {
    pub fn new(operation: Operation, error: &anyhow::Error) -> Self {
        let reason = match error.to_string().as_str() {
            "订阅不存在" => Some("订阅不存在"),
            "无法按已保存设置生成运行 YAML" => Some("无法按已保存设置生成运行 YAML"),
            "无法安全遮盖此 YAML 源码" => Some("无法安全遮盖此 YAML 源码"),
            "源码已变化，请重新打开" => Some("源码已变化，请重新打开"),
            "行号超出源码范围" => Some("行号超出源码范围"),
            "行号必须大于零" => Some("行号必须大于零"),
            "全局覆写已被另一客户端修改，请刷新后重试" => {
                Some("全局覆写已被另一客户端修改，请刷新后重试")
            }
            "设置已变更，请重新打开全局覆写" => {
                Some("设置已变更，请重新打开全局覆写")
            }
            "Claude 保护正在切换" => Some("Claude 保护正在切换"),
            crate::protection::READ_ONLY_NOTICE => Some(crate::protection::READ_ONLY_NOTICE),
            "内核拒绝全局覆写，请检查规则和规则集配置" => {
                Some("内核拒绝全局覆写，请检查规则和规则集配置")
            }
            _ => None,
        };
        Self {
            operation,
            recovery: error.downcast_ref::<Recovery>().copied(),
            syntax: error
                .downcast_ref::<SyntaxLocation>()
                .map(|location| (location.line, location.column)),
            io: error
                .downcast_ref::<io::Error>()
                .map(|error| (error.kind(), error.raw_os_error())),
            reason,
        }
    }

    pub fn critical(&self) -> bool {
        self.recovery.is_some_and(Recovery::critical)
    }

    pub fn message(&self, language: Language) -> String {
        let summary = self.operation.summary(language);
        let detail = if let Some(recovery) = self.recovery {
            recovery.detail(language).into()
        } else if let Some((line, column)) = self.syntax {
            let label = language.text("全局覆写 YAML 格式错误", "Invalid global override YAML");
            format!("{label} · {line}:{column}")
        } else if let Some(reason) = self.reason {
            crate::error_message::localize(reason, language).into_owned()
        } else if let Some((kind, _)) = self.io {
            io::Error::from(kind).to_string()
        } else {
            language
                .text(
                    "具体原因见编辑器内的提示。",
                    "See the message in the YAML view for details.",
                )
                .into()
        };
        format!("{summary}\n{detail}")
    }

    pub fn incident(&self) -> serde_json::Value {
        use serde_json::json;

        let mut event =
            crate::support::incident(&self.message(Language::SimplifiedChinese), self.critical());
        event["code"] = json!(self.operation.code());
        if let Some(recovery) = self.recovery {
            event["recovery"] = json!(recovery);
        }
        if let Some((line, column)) = self.syntax {
            event["location"] = json!({"line":line,"column":column});
        }
        if let Some((kind, os_code)) = self.io {
            event["io_kind"] = json!(format!("{kind:?}"));
            if let Some(os_code) = os_code {
                event["os_code"] = json!(os_code);
            }
        }
        event
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "reply", content = "data", rename_all = "kebab-case")]
pub enum Reply {
    Override(overrides::Document),
    Source(source_view::Page),
    Valid,
    Saved(String),
    ExternalOpened,
    ExternalDraft(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_keep_operation_location_and_recovery_without_document_contents() {
        let error = overrides::GlobalOverride::parse("prepend-rules: [")
            .err()
            .expect("malformed override");
        let bad_yaml = Failure::new(Operation::Check, &error);
        let diagnostic = bad_yaml.incident();
        assert_eq!(diagnostic["code"], "yaml_override_check_failed");
        assert_eq!(diagnostic["level"], "recoverable");
        assert_eq!(diagnostic["location"]["line"], 2);
        assert!(diagnostic["location"]["column"].as_u64().is_some());

        let device_error = anyhow::Error::new(io::Error::from_raw_os_error(13))
            .context("synthetic source /fixture/private/secret.yaml");
        let diagnostic = Failure::new(Operation::ReadExternal, &device_error).incident();
        assert_eq!(diagnostic["code"], "yaml_external_read_failed");
        assert_eq!(diagnostic["os_code"], 13);
        assert!(!diagnostic.to_string().contains("/fixture/private"));

        for (disk, runtime, level, recovery) in [
            (true, true, "recoverable", "restored"),
            (false, true, "critical", "files_failed"),
            (true, false, "critical", "runtime_failed"),
            (false, false, "critical", "files_and_runtime_failed"),
        ] {
            let error = Recovery::after_attempt(disk, runtime).into();
            let diagnostic = Failure::new(Operation::Apply, &error).incident();
            assert_eq!(diagnostic["code"], "yaml_override_apply_failed");
            assert_eq!(diagnostic["level"], level);
            assert_eq!(diagnostic["recovery"], recovery);
        }
    }
}
