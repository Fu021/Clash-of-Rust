use super::{Action, Backend, Error, request::is_docker_desktop, request::valid_distro};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const SCHEMA_VERSION: u32 = 1;
const REPORT_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Blocked,
    Failed,
    NotRun,
}

impl Outcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::NotRun => "not_run",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protection {
    #[default]
    Unknown,
    Blocked,
    Verified,
}

impl Protection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Blocked => "blocked",
            Self::Verified => "verified",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Distribution {
    pub name: String,
    pub state: String,
    pub coverage: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub id: String,
    pub result: String,
    pub detail: String,
}

/// Single-object schema v1. Use parse rather than serde_json directly: it bounds
/// the document, validates identity/duplicates, redacts free text, and downgrades
/// unsupported protection claims. State::apply repeats these checks defensively.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema_version: u32,
    pub operation_id: String,
    pub generation: u64,
    pub action: Action,
    pub outcome: Outcome,
    pub protection: Protection,
    pub phase: String,
    pub backend: Backend,
    pub message: String,
    pub distributions: Vec<Distribution>,
    pub checks: Vec<Check>,
    pub logs: Vec<String>,
}

impl Report {
    pub fn parse(raw: &str) -> Result<Self, Error> {
        if raw.len() > REPORT_LIMIT {
            return Err(Error::InvalidReport);
        }
        let report: Self = serde_json::from_str(raw.trim_start_matches('\u{feff}'))
            .map_err(|_| Error::InvalidReport)?;
        report.prepare()
    }

    /// Unknown results, NOT_RUN and BLOCKED never count as positive evidence.
    pub fn check_passed(&self, id: &str) -> bool {
        let mut matches = self.checks.iter().filter(|check| check.id == id);
        matches
            .next()
            .is_some_and(|check| check.result.eq_ignore_ascii_case("PASS"))
            && matches.next().is_none()
    }

    pub fn tailscale_connected(&self) -> bool {
        self.backend == Backend::Tailscale
            && self.outcome != Outcome::Failed
            && self.check_passed("tailscale_connected")
    }

    pub fn german_exit_verified(&self) -> bool {
        self.outcome != Outcome::Failed
            && self.check_passed("german_exit")
            && self.check_passed("ipv4_only")
    }

    /// A readiness verdict, not a claim that this Rust module enforces a guard.
    /// Static policy, transport connectivity and location are independent facts.
    pub fn effective_protection(&self) -> Protection {
        if self.protection == Protection::Verified && self.has_protection_evidence() {
            Protection::Verified
        } else {
            Protection::Blocked
        }
    }

    pub fn functional_ready(&self) -> bool {
        let connected = match self.backend {
            Backend::Tailscale => self.tailscale_connected(),
            Backend::WireGuard => self.check_passed("wireguard_connected"),
        };
        self.effective_protection() == Protection::Verified
            && self.german_exit_verified()
            && connected
            && self.check_passed("functional_egress")
            && self.distributions.iter().all(|distro| {
                distro.coverage == "verified"
                    && (!is_docker_desktop(&distro.name)
                        || self.check_passed("docker_desktop_adapter")
                            && self.check_passed("docker_desktop_functional"))
            })
    }

    fn has_protection_evidence(&self) -> bool {
        self.action == Action::Test
            && self.outcome == Outcome::Succeeded
            && [
                "cold_boot",
                "host_policy",
                "all_wsl_coverage",
                "ipv4_only",
                "dns_via_exit",
                "windows_network_preserved",
                "interop_guarded",
                "failure_closed",
            ]
            .iter()
            .all(|id| self.check_passed(id))
            && (self.backend != Backend::Tailscale || self.check_passed("strict_tailscale_boot"))
            && !self.distributions.is_empty()
            && self.distributions.iter().all(|distro| {
                if is_docker_desktop(&distro.name) {
                    distro.coverage == "blocked" && self.check_passed("docker_desktop_isolated")
                        || distro.coverage == "verified"
                            && self.check_passed("docker_desktop_adapter")
                } else {
                    distro.coverage == "verified"
                }
            })
    }

    pub(super) fn prepare(mut self) -> Result<Self, Error> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(Error::UnsupportedSchema);
        }
        if uuid::Uuid::parse_str(&self.operation_id).is_err()
            || self.distributions.len() > 256
            || self.checks.len() > 256
            || self.logs.len() > 512
            || self.message.len() > 8192
            || self.phase.len() > 128
        {
            return Err(Error::InvalidReport);
        }
        // Canonicalize identity so hyphen/case variants cannot evade replay checks.
        self.operation_id = uuid::Uuid::parse_str(&self.operation_id)
            .map_err(|_| Error::InvalidReport)?
            .to_string();
        let mut names = HashSet::new();
        for distro in &mut self.distributions {
            if !valid_distro(&distro.name)
                || !names.insert(distro.name.to_ascii_lowercase())
                || distro.state.len() > 128
                || distro.coverage.len() > 128
            {
                return Err(Error::InvalidReport);
            }
            retain_known(&mut distro.state, &["running", "stopped", "unknown"]);
            retain_known(
                &mut distro.coverage,
                &[
                    "not_enrolled",
                    "unknown",
                    "blocked",
                    "verified",
                    "unsupported",
                ],
            );
        }
        let mut ids = HashSet::new();
        for check in &mut self.checks {
            if check.id.len() > 128
                || check.result.len() > 128
                || check.detail.len() > 8192
                || !ids.insert(check.id.to_ascii_lowercase())
            {
                return Err(Error::InvalidReport);
            }
            retain_known(&mut check.id, KNOWN_CHECKS);
            if !["PASS", "FAIL", "BLOCKED", "NOT_RUN", "NOT_SUPPORTED"]
                .iter()
                .any(|result| check.result.eq_ignore_ascii_case(result))
            {
                check.result = "unknown".into();
            }
            check.detail = redact(&check.detail);
        }
        retain_known(
            &mut self.phase,
            &[
                "uninstalled",
                "preflight",
                "preparing",
                "blocked",
                "failed",
                "installed_closed",
                "connected_unverified",
                "acceptance_pending",
                "failed_closed_requested",
                "restored",
                "verified",
                "unknown",
            ],
        );
        self.message = redact(&self.message);
        for log in &mut self.logs {
            if log.len() > 8192 {
                return Err(Error::InvalidReport);
            }
            *log = redact(log);
        }
        if self.protection == Protection::Verified && !self.has_protection_evidence() {
            self.protection = Protection::Blocked;
        }
        Ok(self)
    }
}

fn retain_known(value: &mut String, known: &[&str]) {
    if !known.contains(&value.as_str()) {
        *value = "unknown".into();
    }
}

const KNOWN_CHECKS: &[&str] = &[
    "administrator",
    "wsl_runtime",
    "host_policy",
    "strict_tailscale_boot",
    "cold_boot",
    "german_exit",
    "all_wsl_coverage",
    "ipv4_only",
    "dns_via_exit",
    "windows_network_preserved",
    "tailscale_connected",
    "wireguard_connected",
    "docker_desktop_isolated",
    "docker_desktop_adapter",
    "docker_desktop_functional",
    "functional_egress",
    "interop_guarded",
    "failure_closed",
];

/// Only known constant diagnostics cross the helper/UI boundary. A blacklist
/// cannot reliably recognize arbitrary keys, profile contents or endpoints in
/// exception text. Unrecognized text is omitted rather than partially masked.
pub fn redact(raw: &str) -> String {
    const SAFE: &[&str] = &[
        "",
        "[redacted]",
        "This host is not Windows.",
        "Network writes require an administrator.",
        "Read-only enumeration does not start stopped distributions.",
        "Installed policy inspected; end-to-end and cold-boot acceptance are separate.",
        "Not installed; no protection is claimed.",
        "Preview only. No files, network settings, or WSL runtime changed.",
        "No supported synchronous host/Linux guard lifetime binding has been verified.",
        "Run the fixed installed broker as administrator; no automatic elevation performed.",
        "Tailscale strict all-WSL cold-boot protection is not verified. Use the separately configured WireGuard backend.",
        "Effective settings and endpoint rules only; not a cold-boot proof.",
        "Requires an isolated Windows/WSL test environment.",
        "No target traffic is generated automatically.",
        "Host policy checked. Runtime acceptance is NOT_RUN until controlled receiver tests are supplied.",
        "Transport closed. Restoring original networking requires -RestoreOriginalNetwork and no running WSL distribution.",
        "Operation completed. Packet-level, exit-location, and reboot verification remain separate.",
        "Operation failed; inspect the protected local journal and restore only owned objects.",
    ];
    if SAFE.contains(&raw) {
        raw.to_owned()
    } else {
        "[redacted]".into()
    }
}
