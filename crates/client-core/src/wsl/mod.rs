//! WSL helper contract and fail-closed UI state. This module never starts WSL,
//! reads a private profile, elevates privileges, or changes network state.
//!
//! Desktop must invoke a trusted installed helper with separate argument values.
//! A local operation ticket and the helper's configuration generation are
//! independent: a ticket rejects stale asynchronous callbacks; a monotonically
//! observed helper generation rejects rolled-back service evidence.
mod report;
mod request;
mod state;

pub use report::{Check, Distribution, Outcome, Protection, Report, SCHEMA_VERSION, redact};
pub use request::{Action, Backend, Request, WslSettings};
pub use state::{Operation, State};

/// No helper output or submitted input is included in an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidEndpoint,
    InvalidPort,
    InvalidDistro,
    InvalidProfilePath,
    MissingEndpoint,
    ExecuteRequired,
    UnexpectedExecute,
    UnexpectedRestore,
    DockerDesktopUnsupported,
    InvalidReport,
    UnsupportedSchema,
    Busy,
    GenerationExhausted,
    StaleOperation,
    ReportMismatch,
    GenerationRegressed,
    ReplayedReport,
    HelperUnavailable,
    HelperFailed,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidEndpoint => "Endpoint must be a public IPv4 literal.",
            Self::InvalidPort => "Endpoint port must be a decimal integer from 1 to 65535.",
            Self::InvalidDistro => "Distribution name is invalid.",
            Self::InvalidProfilePath => "Profile must be an absolute local Windows JSON path.",
            Self::MissingEndpoint => "An endpoint pair or a private profile is required.",
            Self::ExecuteRequired => "This action requires explicit execution.",
            Self::UnexpectedExecute => "A read-only action cannot request execution.",
            Self::UnexpectedRestore => "Restoring original networking requires an explicitly executed rollback or uninstall.",
            Self::DockerDesktopUnsupported => "Docker Desktop requires an independent adapter; external access must remain blocked.",
            Self::InvalidReport => "The WSL helper returned an invalid report.",
            Self::UnsupportedSchema => "The WSL helper report schema is unsupported.",
            Self::Busy => "A WSL operation is already in progress.",
            Self::GenerationExhausted => "The local operation generation is exhausted.",
            Self::StaleOperation => "The operation belongs to an obsolete local generation.",
            Self::ReportMismatch => "The helper report does not match the requested action or backend.",
            Self::GenerationRegressed => "The helper report generation has regressed.",
            Self::ReplayedReport => "The helper operation report has already been consumed.",
            Self::HelperUnavailable => "The trusted installed WSL helper is unavailable.",
            Self::HelperFailed => "The WSL helper could not complete the operation.",
        })
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests;
