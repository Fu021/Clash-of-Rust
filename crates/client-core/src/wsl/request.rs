use super::Error;
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backend {
    #[default]
    WireGuard,
    Tailscale,
}

impl Backend {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tailscale => "Tailscale",
            Self::WireGuard => "WireGuard",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    #[default]
    Doctor,
    Status,
    Install,
    Start,
    Stop,
    Test,
    Rollback,
    Uninstall,
}

impl Action {
    /// Install without Execute is the explicit installation preflight.
    pub const fn mutating(self) -> bool {
        matches!(
            self,
            Self::Install | Self::Start | Self::Stop | Self::Rollback | Self::Uninstall
        )
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Doctor => "Doctor",
            Self::Status => "Status",
            Self::Install => "Install",
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Test => "Test",
            Self::Rollback => "Rollback",
            Self::Uninstall => "Uninstall",
        }
    }
}

/// Safe preferences, also usable as a transient desktop input draft. Private
/// endpoint inputs are deliberately never serialized into settings.json.
/// Credentials and profile contents have no fields in this type.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WslSettings {
    pub backend: Backend,
    pub distro: String,
    #[serde(skip)]
    pub endpoint_ipv4: String,
    #[serde(skip)]
    pub endpoint_port: String,
    pub profile_path: String,
}

impl WslSettings {
    /// Validate only durable preferences. Ephemeral endpoint overrides are
    /// validated by Request before invocation and never persisted.
    pub fn validate(&self) -> Result<(), Error> {
        if !self.distro.is_empty() && !valid_distro(&self.distro) {
            return Err(Error::InvalidDistro);
        }
        if !self.profile_path.is_empty() {
            validate_profile_path(&self.profile_path)?;
        }
        Ok(())
    }

    pub fn request(&self, action: Action, execute: bool) -> Request {
        Request {
            action,
            backend: self.backend,
            endpoint_ipv4: self.endpoint_ipv4.clone(),
            endpoint_port: self.endpoint_port.clone(),
            distro: self.distro.clone(),
            profile_path: self.profile_path.clone(),
            execute,
            restore_original_network: false,
        }
    }
}

impl std::fmt::Debug for WslSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WslSettings")
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

/// An ephemeral invocation. Intentionally not serializable and Debug never
/// includes arguments, which may disclose a private endpoint or local path.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Request {
    pub action: Action,
    pub backend: Backend,
    pub endpoint_ipv4: String,
    pub endpoint_port: String,
    pub distro: String,
    pub profile_path: String,
    pub execute: bool,
    /// Set only after desktop has obtained the separate restore confirmation.
    pub restore_original_network: bool,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("action", &self.action)
            .field("backend", &self.backend)
            .field("execute", &self.execute)
            .field("restore_original_network", &self.restore_original_network)
            .finish_non_exhaustive()
    }
}

impl Request {
    pub fn validate(&self) -> Result<(), Error> {
        if self.restore_original_network
            && (!self.execute || !matches!(self.action, Action::Rollback | Action::Uninstall))
        {
            return Err(Error::UnexpectedRestore);
        }
        if !self.endpoint_ipv4.is_empty() {
            validate_ipv4(&self.endpoint_ipv4)?;
        }
        if !self.endpoint_port.is_empty() {
            validate_port(&self.endpoint_port)?;
        }
        if self.endpoint_ipv4.is_empty() != self.endpoint_port.is_empty() {
            return Err(Error::MissingEndpoint);
        }
        if !self.distro.is_empty() && !valid_distro(&self.distro) {
            return Err(Error::InvalidDistro);
        }
        if !self.profile_path.is_empty() {
            validate_profile_path(&self.profile_path)?;
        }
        if self.execute && !self.action.mutating() {
            return Err(Error::UnexpectedExecute);
        }
        if self.action.mutating() && !self.execute && self.action != Action::Install {
            return Err(Error::ExecuteRequired);
        }
        if matches!(self.action, Action::Install | Action::Start) && is_docker_desktop(&self.distro)
        {
            return Err(Error::DockerDesktopUnsupported);
        }
        // Doctor/Status/Stop/Rollback/Uninstall can run without an endpoint.
        // Install preflight also reports missing prerequisites in the helper.
        if (self.action == Action::Start || self.action == Action::Install && self.execute)
            && self.endpoint_ipv4.is_empty()
            && self.profile_path.is_empty()
        {
            return Err(Error::MissingEndpoint);
        }
        Ok(())
    }

    /// Argument values for Process::args, excluding the trusted executable and
    /// fixed script path. Do not concatenate these into a shell command string.
    pub fn arguments(&self) -> Result<Vec<String>, Error> {
        self.validate()?;
        let mut args = vec![
            "-Action".into(),
            self.action.as_str().into(),
            "-Backend".into(),
            self.backend.as_str().into(),
        ];
        for (flag, value) in [
            ("-EndpointIPv4", &self.endpoint_ipv4),
            ("-EndpointPort", &self.endpoint_port),
            ("-Distro", &self.distro),
            ("-ProfilePath", &self.profile_path),
        ] {
            if !value.is_empty() {
                args.extend([flag.to_owned(), value.clone()]);
            }
        }
        if self.execute {
            args.push("-Execute".into());
        }
        if self.restore_original_network {
            args.push("-RestoreOriginalNetwork".into());
        }
        Ok(args)
    }
}

fn validate_ipv4(raw: &str) -> Result<(), Error> {
    let ip: Ipv4Addr = raw.parse().map_err(|_| Error::InvalidEndpoint)?;
    let [a, b, _, _] = ip.octets();
    if ip.to_string() != raw
        || ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || a == 0
        || a >= 240
        || a == 100 && (64..=127).contains(&b)
        || a == 198 && (18..=19).contains(&b)
    {
        return Err(Error::InvalidEndpoint);
    }
    Ok(())
}

fn validate_port(raw: &str) -> Result<(), Error> {
    if raw.is_empty()
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
        || raw.starts_with('0')
        || raw.parse::<u16>().ok().is_none_or(|port| port == 0)
    {
        return Err(Error::InvalidPort);
    }
    Ok(())
}

pub(super) fn valid_distro(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        && !name.ends_with('.')
}

pub(super) fn is_docker_desktop(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "docker-desktop" || name.starts_with("docker-desktop-")
}

fn validate_profile_path(raw: &str) -> Result<(), Error> {
    let bytes = raw.as_bytes();
    // Cross-platform lexical validation of a local Windows drive path. Reject
    // UNC/device paths, ADS, traversal, interpolation and ambiguous components.
    if raw.len() > 1024
        || bytes.len() < 9
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || bytes[2] != b'\\'
        || raw.chars().any(char::is_control)
        || raw[2..].chars().any(|c| ":/\"'`$%;&|<>*?!".contains(c))
        || !raw.to_ascii_lowercase().ends_with(".json")
    {
        return Err(Error::InvalidProfilePath);
    }
    for component in raw[3..].split('\\') {
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with(['.', ' '])
            || component.starts_with(' ')
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9')
        {
            return Err(Error::InvalidProfilePath);
        }
    }
    Ok(())
}
