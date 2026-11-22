use anyhow::{Context, Result, ensure};
#[cfg(test)]
use clash_mobile_runtime::core::config::Settings;
use clash_mobile_runtime::{
    PROTECTION_RECOVERY_FILE as RECOVERY_FILE, Session,
    core::{
        config::{Language, atomic_write},
        protection::Protection,
    },
    protection_request_id,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::ErrorKind,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(crate) fn active_probe_allowed(
    change_pending: bool,
    recovery: &SavedProtectionRecovery,
) -> bool {
    !change_pending && !recovery.is_active()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VpnBaseline {
    connected: bool,
    #[serde(deserialize_with = "required_option")]
    desired: Option<bool>,
}

fn required_option<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}

impl VpnBaseline {
    pub(crate) fn capture(session: &Session) -> Option<Self> {
        let state = &session.platform;
        if !session.running || state["vpn_transition_pending"].as_bool() != Some(false) {
            return None;
        }
        Some(Self {
            connected: state["vpn"].as_bool()?,
            desired: match state.get("vpn_desired")? {
                Value::Null => None,
                Value::Bool(desired) => Some(*desired),
                _ => return None,
            },
        })
    }

    pub(crate) fn target(connected: bool) -> Self {
        Self {
            connected,
            desired: Some(connected),
        }
    }

    pub(crate) fn save_for_disable(self, protection: &mut Protection) {
        use clash_mobile_runtime::core::{model::ProxyMode, protection::MobileVpnPlan};
        protection.previous.proxy_mode = if self.connected {
            ProxyMode::Tun
        } else {
            ProxyMode::Off
        };
        protection.previous_mobile_vpn_plan = Some(match self.desired {
            None => MobileVpnPlan::Unset,
            Some(false) => MobileVpnPlan::Disabled,
            Some(true) => MobileVpnPlan::Enabled,
        });
    }

    pub(crate) fn target_for_disable(protection: &Protection) -> Self {
        use clash_mobile_runtime::core::{model::ProxyMode, protection::MobileVpnPlan};
        let connected = protection.previous.proxy_mode == ProxyMode::Tun;
        let desired = match protection.previous_mobile_vpn_plan {
            None => Some(connected),
            Some(MobileVpnPlan::Unset) => None,
            Some(MobileVpnPlan::Disabled) => Some(false),
            Some(MobileVpnPlan::Enabled) => Some(true),
        };
        Self { connected, desired }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryRecord {
    version: u32,
    token: String,
    profile: String,
    run_mode: String,
    #[serde(deserialize_with = "required_option")]
    protection: Option<Arc<Protection>>,
    vpn: VpnBaseline,
}

impl RecoveryRecord {
    fn load(root: &Path) -> Result<Option<Self>> {
        let path = root.join(RECOVERY_FILE);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        ensure!(metadata.is_file() && metadata.len() <= 10 * 1024 * 1024);
        let record: Self = serde_json::from_slice(&fs::read(path)?)?;
        ensure!(
            record.version == 1
                && !record.token.is_empty()
                && !record.profile.is_empty()
                && matches!(record.run_mode.as_str(), "rule" | "global" | "direct")
                && record
                    .protection
                    .as_ref()
                    .is_none_or(|protection| protection.profile_id == record.profile)
        );
        if let Some(protection) = &record.protection {
            protection.validate()?;
        }
        Ok(Some(record))
    }
}

#[derive(Debug)]
pub(crate) struct VpnTransition {
    pub(crate) request: String,
    previous_generation: u64,
    target: VpnBaseline,
}

impl VpnTransition {
    fn start(session: &Session, live: Option<bool>, target: VpnBaseline) -> Result<Self> {
        let request = protection_request_id();
        let command = json!({
            "op":"restore-vpn", "request":request, "desired":target.desired, "connected":live,
        });
        let send = || {
            let response = session.native.request(command.clone())?;
            ensure!(
                response["request"] == request,
                "Unexpected VPN transition request"
            );
            response["generation"]
                .as_u64()
                .context("Missing VPN transition generation")
        };
        let previous_generation = send().or_else(|_| send()).context("restore-vpn")?;
        Ok(Self {
            request,
            previous_generation,
            target,
        })
    }

    pub(crate) fn apply(session: &Session, target: VpnBaseline) -> Result<Self> {
        Self::start(session, Some(target.connected), target)
    }

    pub(crate) fn target_connected(&self) -> bool {
        self.target.connected
    }

    pub(crate) fn poll(&self, session: &Session, protected: bool) -> RollbackProgress {
        let result = &session.platform["vpn_rollback"];
        if result["request"].as_str() != Some(&self.request)
            || session.platform["vpn_transition_pending"] != false
        {
            return RollbackProgress::Pending;
        }
        match result["status"].as_str() {
            Some("failed") => {
                return RollbackProgress::Failed(
                    result["error"]
                        .as_str()
                        .unwrap_or("VPN transition failed")
                        .to_owned(),
                );
            }
            Some("complete") => {}
            _ => return RollbackProgress::Pending,
        }
        let Some(observed) = VpnBaseline::capture(session) else {
            return RollbackProgress::Pending;
        };
        let held_for_disconnect =
            !self.target.connected && observed.connected && session.platform["vpn_held"] == true;
        if observed.desired != self.target.desired
            || (observed.connected != self.target.connected && !held_for_disconnect)
            || (observed.connected && session.platform["vpn_held"] != true)
            || !result["generation"].as_u64().is_some_and(|generation| {
                (!self.target.connected || generation > self.previous_generation)
                    && session.platform["vpn_generation"].as_u64() == Some(generation)
            })
        {
            return RollbackProgress::Failed(
                "The VPN changed before the transition could be confirmed".into(),
            );
        }
        let check_dns = self.target.connected && protected;
        let protection = &session.platform["protection"];
        if protection["enabled"].as_bool() != Some(check_dns)
            || protection["snapshot"]["error"].as_str() != Some("")
            || (check_dns && protection["snapshot"]["connected"] != true)
        {
            return RollbackProgress::Pending;
        }
        RollbackProgress::VerifyProtection {
            request: self.request.clone(),
            check_dns,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct SavedProtectionRecovery {
    rollback: Arc<Mutex<Option<Rollback>>>,
}

struct Rollback {
    record: Option<RecoveryRecord>,
    phase: RollbackPhase,
    last_error: Option<String>,
    finalizing: Option<String>,
}

enum RollbackPhase {
    Applying,
    ReleaseReservation {
        token: String,
    },
    Pending {
        transition: VpnTransition,
        deadline: Instant,
    },
    Failed,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RetryOutcome {
    ReservationReleased,
    FinalizationConfirmed,
    RollbackPending,
}

pub(crate) enum RollbackProgress {
    Idle,
    Pending,
    VerifyProtection { request: String, check_dns: bool },
    Restored,
    Failed(String),
}

impl RollbackProgress {
    pub(crate) fn verification_request(&self) -> Option<&str> {
        match self {
            Self::VerifyProtection { request, .. } => Some(request),
            _ => None,
        }
    }
}

impl SavedProtectionRecovery {
    pub(crate) fn for_process(root: &Path, language: Language) -> Self {
        static RECOVERY: std::sync::LazyLock<SavedProtectionRecovery> =
            std::sync::LazyLock::new(SavedProtectionRecovery::default);
        let recovery = RECOVERY.clone();
        recovery.reopen(root, language);
        recovery
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Rollback>> {
        self.rollback
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn is_active(&self) -> bool {
        self.lock().is_some()
    }

    pub(crate) fn last_error(&self) -> Option<String> {
        self.lock().as_ref()?.last_error.clone()
    }

    fn reopen(&self, root: &Path, language: Language) {
        let mut recovery = self.lock();
        if recovery
            .as_ref()
            .is_some_and(|rollback| !matches!(rollback.phase, RollbackPhase::Applying))
        {
            return;
        }
        *recovery = match RecoveryRecord::load(root) {
            Ok(None) => None,
            Ok(Some(record)) => Some(Rollback {
                record: Some(record),
                phase: RollbackPhase::Failed,
                last_error: Some(
                    language
                        .text(
                            "上次保护切换未完成，请重试恢复原网络状态",
                            "The previous protection change was interrupted. Retry to restore the previous network state.",
                        )
                        .into(),
                ),
                finalizing: None,
            }),
            Err(_) => Some(Rollback {
                record: None,
                phase: RollbackPhase::Failed,
                last_error: Some(
                    language
                        .text(
                            "无法读取保护恢复记录，已暂停 VPN 自动连接",
                            "The protection recovery record could not be read. Automatic VPN connection is paused.",
                        )
                        .into(),
                ),
                finalizing: None,
            }),
        };
    }

    pub(crate) fn begin(&self, session: &Session, vpn: VpnBaseline) -> Result<()> {
        let mut recovery = self.lock();
        ensure!(recovery.is_none(), "Protection recovery is already pending");
        let root = &session.store.root;
        if !matches!(RecoveryRecord::load(root), Ok(None)) {
            drop(recovery);
            self.reopen(root, session.settings.language.resolve(["en"]));
            anyhow::bail!("Protection recovery is already pending");
        }
        let profile = session
            .settings
            .active_profile
            .clone()
            .context("Missing active profile for protection recovery")?;
        let token = protection_request_id();
        let reservation = session
            .native
            .request(json!({
                "op": "reserve-vpn-recovery", "connected": vpn.connected, "desired": vpn.desired,
                "generation": session.platform["vpn_generation"], "token":token,
            }))
            .and_then(|response| {
                ensure!(
                    response["token"] == token,
                    "Missing protection recovery reservation"
                );
                Ok(())
            });
        if let Err(error) = reservation {
            if let Err(release) = Self::release_reservation(session, &token) {
                let message = format!("{error:#} · {release:#}");
                *recovery = Some(Rollback {
                    record: None,
                    phase: RollbackPhase::ReleaseReservation { token },
                    last_error: Some(message.clone()),
                    finalizing: None,
                });
                anyhow::bail!("{message}");
            }
            return Err(error);
        }
        let record = RecoveryRecord {
            version: 1,
            token: token.clone(),
            profile,
            run_mode: session.settings.run_mode.clone(),
            protection: session.settings.claude_protection.clone(),
            vpn,
        };
        let path = root.join(RECOVERY_FILE);
        let write = (|| {
            atomic_write(&path, &serde_json::to_vec(&record)?)
                .context("Could not save the protection recovery record")
        })();
        match (write, Self::release_reservation(session, &token)) {
            (Ok(()), Ok(())) => {
                *recovery = Some(Rollback {
                    record: Some(record),
                    phase: RollbackPhase::Applying,
                    last_error: None,
                    finalizing: None,
                });
                if let Err(error) = Self::hold_vpn(session) {
                    let rollback = recovery.as_mut().expect("new protection recovery");
                    rollback.phase = RollbackPhase::Failed;
                    rollback.last_error = Some(format!("{error:#}"));
                    return Err(error);
                }
                Ok(())
            }
            (Err(write), Ok(())) => Err(write),
            (write, Err(release)) => {
                let record = write.is_ok().then_some(record);
                let message = match write {
                    Ok(()) => format!("{release:#}"),
                    Err(write) => format!("{write:#} · {release:#}"),
                };
                *recovery = Some(Rollback {
                    record,
                    phase: RollbackPhase::ReleaseReservation { token },
                    last_error: Some(message.clone()),
                    finalizing: None,
                });
                anyhow::bail!("{message}");
            }
        }
    }

    fn release_reservation(session: &Session, token: &str) -> Result<()> {
        let response = session
            .native
            .request(json!({"op":"release-vpn-recovery","token":token}))
            .context("Could not release the VPN reservation. Retry protection recovery.")?;
        ensure!(
            response["ok"] == true,
            "Could not confirm that the VPN reservation was released"
        );
        Ok(())
    }

    fn hold_vpn(session: &Session) -> Result<()> {
        let response = session
            .native
            .request(json!({"op":"hold-vpn-recovery"}))
            .context("Could not pause VPN forwarding for protection recovery")?;
        ensure!(
            response["ok"] == true,
            "VPN forwarding pause was not confirmed"
        );
        Ok(())
    }

    pub(crate) fn is_applying(&self) -> bool {
        self.lock()
            .as_ref()
            .is_some_and(|rollback| matches!(rollback.phase, RollbackPhase::Applying))
    }

    #[cfg(test)]
    fn observe_snapshot(&self, session: &mut Session) -> Result<Option<(Settings, VpnBaseline)>> {
        session.observe_native()?;
        Ok(self.snapshot(session))
    }

    #[cfg(test)]
    fn snapshot(&self, session: &Session) -> Option<(Settings, VpnBaseline)> {
        session.settings.claude_protection.as_ref()?;
        Some((session.settings.clone(), VpnBaseline::capture(session)?))
    }

    pub(crate) fn rollback(&self, session: &mut Session) -> Result<()> {
        self.restore(session, false)
    }

    pub(crate) fn retry(&self, session: &mut Session) -> Result<RetryOutcome> {
        {
            let mut recovery = self.lock();
            let Some(rollback) = recovery.as_mut() else {
                return Ok(RetryOutcome::ReservationReleased);
            };
            ensure!(
                !matches!(rollback.phase, RollbackPhase::Applying),
                "A protection change is already running"
            );
            if let Some(request) = &rollback.finalizing
                && session.protection_recovery_completed(request)?
            {
                recovery.take();
                return Ok(RetryOutcome::FinalizationConfirmed);
            }
            if let RollbackPhase::ReleaseReservation { token } = &rollback.phase {
                if let Err(error) = Self::release_reservation(session, token) {
                    rollback.last_error = Some(format!("{error:#}"));
                    return Err(error);
                }
                rollback.phase = RollbackPhase::Failed;
                if rollback.record.is_none() {
                    match RecoveryRecord::load(&session.store.root) {
                        Ok(None) => {
                            recovery.take();
                            return Ok(RetryOutcome::ReservationReleased);
                        }
                        Ok(record) => rollback.record = record,
                        Err(error) => {
                            rollback.last_error = Some(format!("{error:#}"));
                            return Err(error);
                        }
                    }
                }
            }
        }
        self.restore(session, true)?;
        Ok(RetryOutcome::RollbackPending)
    }

    fn restore(&self, session: &mut Session, start_core: bool) -> Result<()> {
        let mut recovery = self.lock();
        let Some(rollback) = recovery.as_mut() else {
            return Ok(());
        };
        if matches!(rollback.phase, RollbackPhase::Pending { .. }) {
            return Ok(());
        }
        ensure!(
            !matches!(rollback.phase, RollbackPhase::ReleaseReservation { .. }),
            "The VPN recovery reservation must be released first"
        );
        let result = (|| {
            if rollback.record.is_none() {
                rollback.record = RecoveryRecord::load(&session.store.root)
                    .context("The protection recovery record could not be read")?;
            }
            let record = Self::validate_record(session, rollback)?;
            session.observe_native()?;
            ensure!(
                session.platform["vpn_transition_pending"] == false,
                "A VPN transition is still running. Retry protection recovery when it finishes."
            );
            Self::hold_vpn(session)?;
            rollback.finalizing = None;
            rollback_network(session, &record, start_core)
        })();
        match result {
            Ok(transition) => {
                rollback.phase = RollbackPhase::Pending {
                    transition,
                    deadline: Instant::now() + Duration::from_secs(120),
                };
                Ok(())
            }
            Err(error) => {
                rollback.phase = RollbackPhase::Failed;
                rollback.last_error = Some(format!("{error:#}"));
                Err(error)
            }
        }
    }

    fn validate_record(session: &Session, rollback: &Rollback) -> Result<RecoveryRecord> {
        let record = rollback
            .record
            .as_ref()
            .context("The protection recovery record could not be read")?;
        ensure!(
            RecoveryRecord::load(&session.store.root)
                .ok()
                .flatten()
                .as_ref()
                == Some(record),
            "The protection recovery record is missing or has changed"
        );
        ensure!(
            session.settings.active_profile.as_deref() == Some(&record.profile),
            "The active profile changed before protection recovery"
        );
        Ok(record.clone())
    }

    pub(crate) fn poll(&self, session: Option<&Session>) -> RollbackProgress {
        let mut recovery = self.lock();
        let Some(rollback) = recovery.as_mut() else {
            return RollbackProgress::Idle;
        };
        let (transition, deadline) = match &rollback.phase {
            RollbackPhase::Applying => return RollbackProgress::Pending,
            RollbackPhase::Pending {
                transition,
                deadline,
            } => (transition, *deadline),
            RollbackPhase::Failed | RollbackPhase::ReleaseReservation { .. } => {
                return RollbackProgress::Failed(
                    rollback
                        .last_error
                        .clone()
                        .expect("failed rollback has an error"),
                );
            }
        };
        let progress = if Instant::now() >= deadline {
            RollbackProgress::Failed("Timed out restoring the previous VPN state".into())
        } else {
            let Some(session) = session else {
                return RollbackProgress::Pending;
            };
            transition.poll(
                session,
                rollback
                    .record
                    .as_ref()
                    .is_some_and(|record| record.protection.is_some()),
            )
        };
        if let RollbackProgress::Failed(error) = &progress {
            rollback.last_error = Some(error.clone());
            rollback.phase = RollbackPhase::Failed;
        }
        progress
    }

    pub(crate) fn verified(
        &self,
        session: &Session,
        request: &str,
        result: Result<()>,
    ) -> RollbackProgress {
        let progress = self.poll(Some(session));
        if progress.verification_request() != Some(request) {
            return progress;
        }
        let mut recovery = self.lock();
        let Some(rollback) = recovery.as_mut() else {
            return RollbackProgress::Idle;
        };
        if !matches!(&rollback.phase, RollbackPhase::Pending { transition, .. } if transition.request == request)
        {
            return RollbackProgress::Pending;
        }
        let result = result.and_then(|()| {
            let record = Self::validate_record(session, rollback)?;
            ensure!(
                session.settings.claude_protection == record.protection
                    && session.settings.run_mode == record.run_mode,
                "The saved protection settings changed before rollback could be confirmed"
            );
            let RollbackPhase::Pending { transition, .. } = &rollback.phase else {
                unreachable!("matching recovery transition");
            };
            rollback.finalizing = Some(request.to_owned());
            session.finish_protection_recovery(request, transition.target_connected())
        });
        match result {
            Ok(()) => {
                recovery.take();
                RollbackProgress::Restored
            }
            Err(error) => {
                let message = format!("{error:#}");
                rollback.last_error = Some(message.clone());
                rollback.phase = RollbackPhase::Failed;
                RollbackProgress::Failed(message)
            }
        }
    }

    pub(crate) fn finish(
        &self,
        session: &Session,
        transition: &VpnTransition,
        dns_verified: bool,
    ) -> Result<()> {
        let mut recovery = self.lock();
        let rollback = recovery
            .as_mut()
            .context("Missing protection recovery record")?;
        ensure!(matches!(rollback.phase, RollbackPhase::Applying));
        let RollbackProgress::VerifyProtection { check_dns, .. } =
            transition.poll(session, session.settings.claude_protection.is_some())
        else {
            anyhow::bail!("The VPN changed before the transition could be confirmed");
        };
        ensure!(
            !check_dns || dns_verified,
            "Protection DNS has not been verified"
        );
        Self::validate_record(session, rollback)?;
        rollback.finalizing = Some(transition.request.clone());
        session.finish_protection_recovery(&transition.request, transition.target_connected())?;
        recovery.take();
        Ok(())
    }
}

fn rollback_network(
    session: &mut Session,
    record: &RecoveryRecord,
    start_core: bool,
) -> Result<VpnTransition> {
    let config = session
        .restore_protection(record.protection.clone(), record.run_mode.clone())
        .and_then(|()| {
            if start_core && !session.running {
                session.attach_or_initialize_for_protection_recovery()?;
            }
            ensure!(session.running, "Core is not running");
            Ok(())
        });
    let live = (!record.vpn.connected || (config.is_ok() && session.running))
        .then_some(record.vpn.connected);
    let network = VpnTransition::start(session, live, record.vpn);
    match (config, network) {
        (Ok(()), network) => network,
        (Err(config), Ok(_)) => Err(config),
        (Err(config), Err(network)) => anyhow::bail!("{config:#} · {network:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_mobile_runtime::{
        NativeHost,
        core::{
            config::LanguagePreference,
            model::ProxyMode,
            protection::{Previous, Protection},
        },
    };
    use std::{
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    #[derive(Default)]
    struct Network {
        plan: Option<bool>,
        vpn: bool,
        pending: bool,
        queue_reconnect: bool,
        generation: u64,
        reservation: Option<String>,
        rollback_status: Value,
        previous_generation: u64,
        completed_recovery: Option<String>,
        guard: bool,
        forwarding_held: bool,
        live_config: String,
        state: Option<Value>,
        cold: bool,
    }

    struct Host {
        root: PathBuf,
        calls: Mutex<Vec<Value>>,
        failures: Mutex<Vec<&'static str>>,
        network: Mutex<Network>,
    }

    impl Host {
        fn new(root: &Path) -> Self {
            Self {
                root: root.to_owned(),
                calls: Mutex::default(),
                failures: Mutex::default(),
                network: Mutex::default(),
            }
        }

        fn take_operations(&self) -> Vec<String> {
            std::mem::take(&mut *self.calls.lock().unwrap())
                .into_iter()
                .map(|request| request["op"].as_str().unwrap().to_owned())
                .collect()
        }
    }

    impl NativeHost for Host {
        fn request(&self, value: Value) -> Result<Value> {
            let operation = value["op"].as_str().unwrap();
            self.calls.lock().unwrap().push(value.clone());
            if self.failures.lock().unwrap().contains(&operation) {
                return clash_mobile_runtime::native_response(
                    json!({"error":format!("fixture {operation} failure")}),
                );
            }
            let mut network = self.network.lock().unwrap();
            match operation {
                "state" => {
                    return Ok(network.state.clone().unwrap_or_else(
                        || json!({
                            "running":!network.cold, "vpn":network.vpn, "vpn_pending":network.pending,
                            "vpn_held":network.forwarding_held,
                            "vpn_transition_pending":network.pending, "vpn_desired":network.plan,
                            "vpn_rollback":network.rollback_status,"vpn_generation":network.generation,
                            "protection":{"enabled":network.guard,"snapshot":{"connected":network.guard,"error":""}}
                        }),
                    ));
                }
                "reserve-vpn-recovery" => {
                    ensure!(network.reservation.is_none(), "fixture reservation is busy");
                    if self.failures.lock().unwrap().contains(&"record-write") {
                        fs::create_dir(self.root.join(RECOVERY_FILE))?;
                    }
                    let token = value["token"].as_str().unwrap().to_owned();
                    network.reservation = Some(token.clone());
                    if self.failures.lock().unwrap().contains(&"reserve-reply") {
                        anyhow::bail!("fixture lost reservation reply");
                    }
                    return Ok(json!({"token":token}));
                }
                "release-vpn-recovery" => {
                    let token = value["token"].as_str().unwrap();
                    if network.reservation.as_deref() == Some(token) {
                        network.reservation = None;
                    }
                    if self.failures.lock().unwrap().contains(&"release-reply") {
                        return Ok(json!({}));
                    }
                }
                "initialize" => {
                    network.cold = false;
                    network.live_config = value["config"].as_str().unwrap().to_owned();
                }
                "vpn-plan" => network.plan = value["enabled"].as_bool(),
                "reload" => network.live_config = value["config"].as_str().unwrap().to_owned(),
                "finish-protection-recovery" => {
                    if self
                        .failures
                        .lock()
                        .unwrap()
                        .contains(&"revoke-before-record-clear")
                    {
                        network.vpn = false;
                        network.guard = false;
                        network.rollback_status["status"] = json!("failed");
                    }
                    let matches = value["config"] == network.live_config;
                    let vpn_matches = network.rollback_status["request"] == value["request"]
                        && network.rollback_status["status"] == "complete"
                        && network.rollback_status["generation"] == value["generation"]
                        && value["generation"] == network.generation
                        && network.reservation.is_none()
                        && !network.pending
                        && if value["connected"] == true {
                            network.vpn && network.forwarding_held
                        } else {
                            !network.vpn || network.forwarding_held
                        }
                        && value["desired"] == json!(network.plan);
                    let guard_matches = value["protected"] == network.guard;
                    let clear_record = matches && vpn_matches && guard_matches;
                    if clear_record {
                        if value["connected"] == false && network.vpn {
                            if self
                                .failures
                                .lock()
                                .unwrap()
                                .contains(&"disconnect-finalize")
                            {
                                network.rollback_status["status"] = json!("failed");
                                anyhow::bail!("fixture disconnect failure");
                            }
                            network.vpn = false;
                            network.forwarding_held = false;
                        }
                        let path = self.root.join(RECOVERY_FILE);
                        let committed = path.with_file_name("protection-recovery-committed.json");
                        if self.failures.lock().unwrap().contains(&"record-delete") {
                            fs::create_dir(&committed)?;
                        }
                        fs::rename(path, &committed)
                            .context("Could not commit the verified protection recovery record")?;
                        network.forwarding_held = false;
                        network.vpn = value["connected"] == true;
                        network.completed_recovery = value["request"].as_str().map(str::to_owned);
                        fs::remove_file(committed)?;
                        if self.failures.lock().unwrap().contains(&"finish-reply") {
                            anyhow::bail!("fixture lost completion reply");
                        }
                    }
                    return Ok(json!({
                        "matches":matches, "vpn_matches":vpn_matches, "guard_matches":guard_matches,
                        "record_cleared":clear_record,
                    }));
                }
                "protection-recovery-receipt" => {
                    if self.failures.lock().unwrap().contains(&"receipt-reply") {
                        anyhow::bail!("fixture lost receipt reply");
                    }
                    return Ok(json!({"committed":
                        network.completed_recovery.as_deref() == value["request"].as_str()
                        && !self.root.join(RECOVERY_FILE).exists()
                    }));
                }
                "protection-pause" => network.guard = false,
                "hold-vpn-recovery" => network.forwarding_held = network.vpn,
                "restore-vpn" => {
                    ensure!(network.reservation.is_none(), "fixture reservation is busy");
                    let request = &value["request"];
                    if &network.rollback_status["request"] == request {
                        return Ok(
                            json!({"request":request,"generation":network.previous_generation}),
                        );
                    }
                    let previous = network.generation;
                    network.previous_generation = previous;
                    network.plan = value["desired"].as_bool();
                    if value["connected"] == true && network.queue_reconnect {
                        network.pending = true;
                        network.rollback_status = json!({"request":request,"status":"pending"});
                    } else {
                        if let Some(connected) = value["connected"].as_bool() {
                            if connected {
                                network.generation += 1;
                                network.vpn = true;
                                network.forwarding_held = true;
                            }
                            network.guard = connected
                                && self.root.join("settings.json").is_file()
                                && serde_json::from_slice::<Settings>(&std::fs::read(
                                    self.root.join("settings.json"),
                                )?)?
                                .claude_protection
                                .is_some();
                        }
                        network.rollback_status = json!({
                            "request":request,"status":"complete","generation":network.generation
                        });
                    }
                    if self.failures.lock().unwrap().contains(&"restore-reply") {
                        anyhow::bail!("fixture lost queued transition reply");
                    }
                    return Ok(json!({"request":request,"generation":previous}));
                }
                "protection-sync" => {
                    let settings: Settings =
                        serde_json::from_slice(&std::fs::read(self.root.join("settings.json"))?)?;
                    network.guard = network.vpn && settings.claude_protection.is_some();
                }
                "disconnect" => {
                    network.vpn = false;
                    network.guard = false;
                }
                _ => {}
            }
            Ok(json!({"ok":true}))
        }
    }

    async fn protected_session(root: &Path, host: Arc<Host>) -> Session {
        let mut session = Session::open(root.into(), host).unwrap();
        session
            .save_profile(
                None,
                "Fixture",
                "",
                Some(
                    "proxies: [{name: Test, type: socks5, server: 127.0.0.1, port: 1080}]\nrules: ['MATCH,Test']\n",
                ),
            )
            .await
            .unwrap();
        let profile = session.profiles.last().unwrap().id.clone();
        session.activate(&profile).unwrap();
        let mode = session.settings.run_mode.clone();
        let protection = Protection::new(
            profile,
            "Test".into(),
            Previous {
                mode: mode.clone(),
                proxy_mode: ProxyMode::Off,
                selectors: Vec::new(),
            },
        );
        session
            .commit_protection(Some(Arc::new(protection)), mode)
            .unwrap();
        session.attach_or_initialize().unwrap();
        session
    }

    #[tokio::test]
    async fn interrupted_protection_switch_is_recoverable_from_disk() {
        assert_recovery_after_process_death(
            false,
            VpnBaseline {
                connected: true,
                desired: Some(false),
            },
        )
        .await;
    }

    #[tokio::test]
    async fn process_death_after_config_rollback_still_restores_the_original_vpn_plan() {
        assert_recovery_after_process_death(
            true,
            VpnBaseline {
                connected: false,
                desired: None,
            },
        )
        .await;
    }

    async fn assert_recovery_after_process_death(
        config_already_restored: bool,
        baseline: VpnBaseline,
    ) {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        let protected = session.settings.clone();
        session.commit_protection(None, "global".into()).unwrap();
        {
            let mut network = host.network.lock().unwrap();
            network.plan = baseline.desired;
            network.vpn = baseline.connected;
        }
        let recovery = SavedProtectionRecovery::default();
        session.observe_native().unwrap();
        recovery
            .begin(&session, VpnBaseline::capture(&session).unwrap())
            .unwrap();
        let record = root.path().join(RECOVERY_FILE);
        let encoded: Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
        assert_eq!(encoded["version"], 1);
        assert_eq!(
            encoded["vpn"],
            json!({"connected":baseline.connected,"desired":baseline.desired})
        );
        assert_eq!(encoded["protection"], Value::Null);
        assert!(encoded.get("secret").is_none());
        session
            .commit_protection(protected.claude_protection, protected.run_mode)
            .unwrap();
        VpnTransition::apply(&session, VpnBaseline::target(true)).unwrap();
        assert_eq!(host.network.lock().unwrap().plan, Some(true));
        if config_already_restored {
            session.restore_protection(None, "global".into()).unwrap();
        }
        drop(session);
        drop(recovery);
        let host = Arc::new(Host::new(root.path()));
        {
            let mut network = host.network.lock().unwrap();
            network.cold = true;
            network.plan = Some(true);
        }
        let mut reopened = Session::open(root.path().into(), host.clone()).unwrap();
        let recovery = if config_already_restored {
            let recovery = SavedProtectionRecovery::default();
            recovery.reopen(root.path(), Language::English);
            recovery
        } else {
            SavedProtectionRecovery::for_process(root.path(), Language::English)
        };
        assert!(matches!(
            recovery.poll(Some(&reopened)),
            RollbackProgress::Failed(_)
        ));
        assert_eq!(
            reopened.settings.claude_protection.is_none(),
            config_already_restored
        );
        assert!(host.take_operations().is_empty());
        recovery.retry(&mut reopened).unwrap();
        let operations = host.take_operations();
        if config_already_restored {
            assert_eq!(
                operations,
                [
                    "state",
                    "hold-vpn-recovery",
                    "state",
                    "state",
                    "initialize",
                    "restore-vpn"
                ]
            );
        } else {
            assert_eq!(
                operations,
                [
                    "state",
                    "hold-vpn-recovery",
                    "protection-pause",
                    "state",
                    "state",
                    "initialize",
                    "restore-vpn"
                ]
            );
        }
        reopened.observe_native().unwrap();
        let progress = recovery.poll(Some(&reopened));
        let request = progress.verification_request().unwrap();
        assert!(matches!(
            recovery.verified(&reopened, request, Ok(())),
            RollbackProgress::Restored
        ));
        assert!(!record.exists());
        let network = host.network.lock().unwrap();
        assert_eq!(network.plan, baseline.desired);
        assert_eq!(network.vpn, baseline.connected);
        assert!(!network.forwarding_held);
        assert_eq!(reopened.settings.run_mode, "global");
        assert!(reopened.settings.claude_protection.is_none());
    }

    #[tokio::test]
    async fn a_failed_recovery_record_write_prevents_configuration_and_plan_changes() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        let settings = session.settings.clone();
        let runtime = fs::read(session.store.runtime().join("config.yaml")).unwrap();
        host.take_operations();
        *host.failures.lock().unwrap() = vec!["record-write"];
        session.observe_native().unwrap();
        let result = recovery_begin_and_disable(&mut session, &SavedProtectionRecovery::default());
        assert!(
            format!("{:#}", result.unwrap_err())
                .contains("Could not save the protection recovery record")
        );
        assert_eq!(
            host.take_operations(),
            ["state", "reserve-vpn-recovery", "release-vpn-recovery"]
        );
        assert_eq!(session.store.load_settings().unwrap(), settings);
        assert_eq!(
            fs::read(session.store.runtime().join("config.yaml")).unwrap(),
            runtime
        );
        assert_eq!(host.network.lock().unwrap().plan, None);
    }

    #[tokio::test]
    async fn reservation_retry_after_write_and_release_failures_only_releases_the_reservation() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        host.network.lock().unwrap().plan = Some(true);
        session.observe_native().unwrap();
        let settings = session.store.load_settings().unwrap();
        let runtime_path = session.store.runtime().join("config.yaml");
        let runtime = fs::read(&runtime_path).unwrap();
        let recovery = SavedProtectionRecovery::default();
        *host.failures.lock().unwrap() = vec!["record-write", "release-vpn-recovery"];
        recovery_begin_and_disable(&mut session, &recovery).unwrap_err();
        fs::remove_dir(root.path().join(RECOVERY_FILE)).unwrap();
        let reopened = recovery.clone();
        reopened.reopen(root.path(), Language::English);
        assert!(host.network.lock().unwrap().reservation.is_some());
        assert!(matches!(reopened.poll(None), RollbackProgress::Failed(_)));
        host.take_operations();
        *host.failures.lock().unwrap() = vec!["release-vpn-recovery"];
        reopened.retry(&mut session).unwrap_err();
        assert!(reopened.is_active());
        assert_eq!(host.take_operations(), ["release-vpn-recovery"]);

        host.failures.lock().unwrap().clear();
        assert_eq!(
            reopened.retry(&mut session).unwrap(),
            RetryOutcome::ReservationReleased
        );
        assert_eq!(host.take_operations(), ["release-vpn-recovery"]);
        assert!(matches!(reopened.poll(None), RollbackProgress::Idle));
        assert_eq!(session.store.load_settings().unwrap(), settings);
        assert_eq!(fs::read(runtime_path).unwrap(), runtime);
        assert_eq!(host.network.lock().unwrap().plan, Some(true));
        reopened
            .begin(&session, VpnBaseline::capture(&session).unwrap())
            .unwrap();
    }

    #[tokio::test]
    async fn reservation_retry_after_successful_write_releases_before_restoring_the_record() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        session.observe_native().unwrap();
        let recovery = SavedProtectionRecovery::default();
        *host.failures.lock().unwrap() = vec!["release-vpn-recovery"];
        recovery_begin_and_disable(&mut session, &recovery).unwrap_err();
        let path = root.path().join(RECOVERY_FILE);
        let record = fs::read(&path).unwrap();
        host.take_operations();
        recovery.retry(&mut session).unwrap_err();
        assert_eq!(host.take_operations(), ["release-vpn-recovery"]);
        assert_eq!(fs::read(&path).unwrap(), record);

        *host.failures.lock().unwrap() = vec!["release-reply"];
        recovery.retry(&mut session).unwrap_err();
        assert!(host.network.lock().unwrap().reservation.is_none());
        assert_eq!(host.take_operations(), ["release-vpn-recovery"]);
        assert_eq!(fs::read(&path).unwrap(), record);

        host.failures.lock().unwrap().clear();
        assert_eq!(
            recovery.retry(&mut session).unwrap(),
            RetryOutcome::RollbackPending
        );
        assert_eq!(
            host.take_operations(),
            [
                "release-vpn-recovery",
                "state",
                "hold-vpn-recovery",
                "reload",
                "protection-sync",
                "restore-vpn"
            ]
        );
        assert_eq!(fs::read(&path).unwrap(), record);
        session.observe_native().unwrap();
        let progress = recovery.poll(Some(&session));
        let request = progress.verification_request().unwrap();
        assert!(matches!(
            recovery.verified(&session, request, Ok(())),
            RollbackProgress::Restored
        ));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn revocation_before_record_clear_retains_failed_recovery() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        let settings = session.settings.clone();
        {
            let mut network = host.network.lock().unwrap();
            network.live_config = session.runtime(&settings).unwrap();
            network.plan = Some(false);
            network.vpn = true;
            network.guard = true;
        }
        session.observe_native().unwrap();
        let recovery = SavedProtectionRecovery::default();
        let baseline = VpnBaseline::capture(&session).unwrap();
        recovery.begin(&session, baseline).unwrap();
        let path = root.path().join(RECOVERY_FILE);
        let record = fs::read(&path).unwrap();
        let transition = VpnTransition::apply(&session, VpnBaseline::target(true)).unwrap();
        session.observe_native().unwrap();
        *host.failures.lock().unwrap() = vec!["revoke-before-record-clear"];
        let error = recovery.finish(&session, &transition, true).unwrap_err();
        assert!(error.to_string().contains("VPN changed"));
        assert_eq!(fs::read(&path).unwrap(), record);

        recovery.rollback(&mut session).unwrap();
        session.observe_native().unwrap();
        let progress = recovery.poll(Some(&session));
        let request = progress.verification_request().unwrap();
        assert!(matches!(
            recovery.verified(&session, request, Ok(())),
            RollbackProgress::Failed(error) if error.contains("VPN changed")
        ));
        assert_eq!(fs::read(&path).unwrap(), record);
        assert!(recovery.is_active());

        host.failures.lock().unwrap().clear();
        recovery.retry(&mut session).unwrap();
        session.observe_native().unwrap();
        let progress = recovery.poll(Some(&session));
        let retry_request = progress.verification_request().unwrap();
        assert_ne!(retry_request, request);
        assert!(matches!(
            recovery.verified(&session, retry_request, Ok(())),
            RollbackProgress::Restored
        ));
        assert_eq!(VpnBaseline::capture(&session), Some(baseline));
        assert_eq!(session.store.load_settings().unwrap(), settings);
        assert!(!path.exists());
    }

    fn recovery_begin_and_disable(
        session: &mut Session,
        recovery: &SavedProtectionRecovery,
    ) -> Result<VpnTransition> {
        recovery.begin(session, VpnBaseline::capture(session).unwrap())?;
        assert!(recovery.is_applying());
        session.commit_protection(None, "global".into())?;
        VpnTransition::apply(session, VpnBaseline::target(false))
    }

    #[tokio::test]
    async fn disconnect_failure_keeps_the_original_record_and_rolls_back() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        let original_settings = session.settings.clone();
        let profile = session.store.profile_path(&session.profile().id).unwrap();
        let original_profile = fs::read(&profile).unwrap();
        {
            let mut network = host.network.lock().unwrap();
            network.vpn = true;
            network.plan = Some(true);
            network.guard = true;
        }
        session.observe_native().unwrap();
        let recovery = SavedProtectionRecovery::default();
        let transition = recovery_begin_and_disable(&mut session, &recovery).unwrap();
        let path = root.path().join(RECOVERY_FILE);
        let original_record = fs::read(&path).unwrap();
        session.observe_native().unwrap();
        *host.failures.lock().unwrap() = vec!["disconnect-finalize"];
        let error = recovery.finish(&session, &transition, false).unwrap_err();
        assert!(format!("{error:#}").contains("fixture disconnect failure"));
        assert_eq!(fs::read(&path).unwrap(), original_record);
        assert!(recovery.is_active());
        assert!(
            !session
                .protection_recovery_completed(&transition.request)
                .unwrap()
        );
        assert!(host.network.lock().unwrap().forwarding_held);

        host.failures.lock().unwrap().clear();
        recovery.rollback(&mut session).unwrap();
        session.observe_native().unwrap();
        let request = recovery
            .poll(Some(&session))
            .verification_request()
            .unwrap()
            .to_owned();
        assert!(matches!(
            recovery.verified(&session, &request, Ok(())),
            RollbackProgress::Restored
        ));
        assert!(!path.exists());
        assert_eq!(
            session.store.load_settings().unwrap().claude_protection,
            original_settings.claude_protection
        );
        assert_eq!(
            session.store.load_settings().unwrap().run_mode,
            original_settings.run_mode
        );
        assert_eq!(fs::read(profile).unwrap(), original_profile);
        let network = host.network.lock().unwrap();
        assert!(network.vpn && network.guard && !network.forwarding_held);
        assert_eq!(network.plan, Some(true));
    }

    #[tokio::test]
    async fn successful_enable_then_disable_restores_live_and_saved_plan_independently() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        let protection = session.settings.claude_protection.clone().unwrap();
        let legacy = serde_json::to_value(protection.as_ref()).unwrap();
        assert!(legacy.get("previous_mobile_vpn_plan").is_none());
        for (mode, connected) in [("off", false), ("tun", true)] {
            let mut legacy = legacy.clone();
            legacy["previous"]["proxy_mode"] = json!(mode);
            let old: Protection = serde_json::from_value(legacy).unwrap();
            assert_eq!(
                VpnBaseline::target_for_disable(&old),
                VpnBaseline::target(connected)
            );
        }
        for (connected, desired) in [
            (false, Some(true)),
            (true, Some(false)),
            (false, None),
            (true, None),
        ] {
            session.commit_protection(None, "global".into()).unwrap();
            {
                let mut network = host.network.lock().unwrap();
                network.vpn = connected;
                network.plan = desired;
            }
            session.observe_native().unwrap();
            let recovery = SavedProtectionRecovery::default();
            let baseline = VpnBaseline::capture(&session).unwrap();
            recovery.begin(&session, baseline).unwrap();
            let mut prepared = (*protection).clone();
            baseline.save_for_disable(&mut prepared);
            session
                .commit_protection(Some(Arc::new(prepared)), "global".into())
                .unwrap();
            let enable = VpnTransition::apply(&session, VpnBaseline::target(true)).unwrap();
            session.observe_native().unwrap();
            recovery.finish(&session, &enable, true).unwrap();

            let saved = session
                .store
                .load_settings()
                .unwrap()
                .claude_protection
                .unwrap();
            let target = VpnBaseline::target_for_disable(&saved);
            session.observe_native().unwrap();
            recovery
                .begin(&session, VpnBaseline::capture(&session).unwrap())
                .unwrap();
            session
                .commit_protection(None, saved.previous.mode.clone())
                .unwrap();
            let disable = VpnTransition::apply(&session, target).unwrap();
            session.observe_native().unwrap();
            recovery.finish(&session, &disable, false).unwrap();
            assert_eq!(host.network.lock().unwrap().plan, desired);
            assert_eq!(host.network.lock().unwrap().vpn, connected);
        }
    }

    #[tokio::test]
    async fn stopped_guard_error_stops_blocking_disable_after_pause() {
        use crate::protection_service_state::acknowledge_stopped_pause;
        use clash_mobile_runtime::core::claude_check::guard::Snapshot;

        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host).await;
        session.observe_native().unwrap();
        let recovery = SavedProtectionRecovery::default();
        let transition = recovery_begin_and_disable(&mut session, &recovery).unwrap();
        session.observe_native().unwrap();
        let state = Mutex::new((
            false,
            Snapshot {
                error: "fixture worker stopped".into(),
                ..Default::default()
            },
        ));
        let observe = |session: &mut Session| {
            let (enabled, snapshot) = &*state.lock().unwrap();
            session.platform["protection"] = json!({"enabled":enabled, "snapshot":snapshot});
        };
        observe(&mut session);
        assert!(matches!(
            transition.poll(&session, false),
            RollbackProgress::Pending
        ));
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<()>();
        drop(receiver);
        assert!(acknowledge_stopped_pause(false, Some(&sender), &state).unwrap());
        observe(&mut session);
        recovery.finish(&session, &transition, false).unwrap();
        assert!(!recovery.is_active());
    }

    #[tokio::test]
    async fn a_damaged_recovery_record_keeps_the_saved_vpn_plan() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        let path = root.path().join(RECOVERY_FILE);
        let damaged = br#"{"version":1,"vpn":{"connected":false}}"#;
        fs::write(&path, damaged).unwrap();
        host.network.lock().unwrap().plan = Some(true);
        let reopened = SavedProtectionRecovery::default();
        reopened.reopen(root.path(), Language::English);
        assert!(matches!(reopened.poll(None), RollbackProgress::Failed(_)));
        host.take_operations();
        reopened.retry(&mut session).unwrap_err();
        assert!(host.take_operations().is_empty());
        assert_eq!(fs::read(path).unwrap(), damaged);
        assert_eq!(host.network.lock().unwrap().plan, Some(true));
    }

    #[tokio::test]
    async fn retry_rereads_a_repaired_recovery_record() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        session.observe_native().unwrap();
        SavedProtectionRecovery::default()
            .begin(&session, VpnBaseline::capture(&session).unwrap())
            .unwrap();
        let path = root.path().join(RECOVERY_FILE);
        let original = fs::read(&path).unwrap();
        fs::write(&path, b"{").unwrap();
        let recovery = SavedProtectionRecovery::default();
        recovery.reopen(root.path(), Language::English);
        assert!(recovery.retry(&mut session).is_err());
        fs::write(&path, &original).unwrap();
        assert_eq!(
            recovery.retry(&mut session).unwrap(),
            RetryOutcome::RollbackPending
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[tokio::test]
    async fn lost_reservation_reply_does_not_leave_the_host_reserved() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        session.observe_native().unwrap();
        let recovery = SavedProtectionRecovery::default();
        *host.failures.lock().unwrap() = vec!["reserve-reply"];
        let vpn = VpnBaseline::capture(&session).unwrap();
        assert!(recovery.begin(&session, vpn).is_err());
        assert!(!recovery.is_active());
        assert!(host.network.lock().unwrap().reservation.is_none());
        host.failures.lock().unwrap().clear();
        recovery.begin(&session, vpn).unwrap();
    }

    #[tokio::test]
    async fn lost_completion_reply_is_confirmed_without_rolling_back_a_committed_change() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        let recovery = SavedProtectionRecovery::default();
        session.observe_native().unwrap();
        let transition = recovery_begin_and_disable(&mut session, &recovery).unwrap();
        session.observe_native().unwrap();
        *host.failures.lock().unwrap() = vec!["finish-reply"];
        recovery.finish(&session, &transition, false).unwrap();
        assert!(!recovery.is_active());
        assert!(!root.path().join(RECOVERY_FILE).exists());
        assert!(
            session
                .store
                .load_settings()
                .unwrap()
                .claude_protection
                .is_none()
        );
    }

    #[tokio::test]
    async fn losing_both_completion_and_receipt_replies_can_be_reconciled_on_retry() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        session.observe_native().unwrap();
        let recovery = SavedProtectionRecovery::default();
        let transition = recovery_begin_and_disable(&mut session, &recovery).unwrap();
        session.observe_native().unwrap();
        *host.failures.lock().unwrap() = vec!["finish-reply", "receipt-reply"];
        assert!(recovery.finish(&session, &transition, false).is_err());
        assert!(recovery.rollback(&mut session).is_err());
        assert!(!active_probe_allowed(false, &recovery));
        host.failures.lock().unwrap().clear();
        host.take_operations();
        assert_eq!(
            recovery.retry(&mut session).unwrap(),
            RetryOutcome::FinalizationConfirmed
        );
        assert_eq!(host.take_operations(), ["protection-recovery-receipt"]);
        assert!(!recovery.is_active());
        assert!(session.settings.claude_protection.is_none());
    }

    #[tokio::test]
    async fn replay_of_a_lost_transition_receipt_applies_the_target_once() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        host.network.lock().unwrap().live_config = session.runtime(&session.settings).unwrap();
        session.observe_native().unwrap();
        let recovery = SavedProtectionRecovery::default();
        recovery
            .begin(&session, VpnBaseline::capture(&session).unwrap())
            .unwrap();
        *host.failures.lock().unwrap() = vec!["restore-reply"];
        host.take_operations();
        let transition = VpnTransition::apply(&session, VpnBaseline::target(true)).unwrap();
        {
            let calls = host.calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[0], calls[1]);
        }
        session.observe_native().unwrap();
        assert_eq!(session.platform["vpn_generation"], 1);
        assert!(recovery.is_active());
        recovery.finish(&session, &transition, true).unwrap();
        assert!(!recovery.is_active());
    }

    #[tokio::test]
    async fn interrupted_disable_blocks_probes_even_when_current_settings_are_unprotected() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host).await;
        let recovery = SavedProtectionRecovery::default();
        session.observe_native().unwrap();
        recovery_begin_and_disable(&mut session, &recovery).unwrap();
        assert!(session.settings.claude_protection.is_none());
        let reopened = SavedProtectionRecovery::default();
        reopened.reopen(root.path(), Language::English);
        assert!(active_probe_allowed(
            false,
            &SavedProtectionRecovery::default()
        ));
        assert!(!active_probe_allowed(
            true,
            &SavedProtectionRecovery::default()
        ));
        assert!(!active_probe_allowed(false, &reopened));
    }

    #[tokio::test]
    async fn forward_transition_requires_current_receipt_live_config_and_record_removal() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        {
            let mut network = host.network.lock().unwrap();
            network.vpn = true;
            network.guard = true;
        }
        session.observe_native().unwrap();
        let recovery = SavedProtectionRecovery::default();
        let transition = recovery_begin_and_disable(&mut session, &recovery).unwrap();
        let path = root.path().join(RECOVERY_FILE);
        session.observe_native().unwrap();
        let current_receipt = session.platform["vpn_rollback"].clone();
        session.platform["vpn_rollback"]["request"] = json!("stale");
        recovery.finish(&session, &transition, false).unwrap_err();
        assert!(path.is_file());

        session.platform["vpn_rollback"] = current_receipt;
        let live = std::mem::replace(
            &mut host.network.lock().unwrap().live_config,
            "mode: direct".into(),
        );
        let error = recovery.finish(&session, &transition, false).unwrap_err();
        assert!(error.to_string().contains("running core configuration"));
        assert!(path.is_file());
        host.network.lock().unwrap().live_config = live;

        *host.failures.lock().unwrap() = vec!["record-delete"];
        let error = recovery.finish(&session, &transition, false).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("commit the verified protection recovery record")
        );
        assert!(recovery.is_active());
        assert!(path.is_file());
        {
            let network = host.network.lock().unwrap();
            assert!(!network.vpn && !network.forwarding_held);
        }
        fs::remove_dir(path.with_file_name("protection-recovery-committed.json")).unwrap();
        host.failures.lock().unwrap().clear();
        recovery.finish(&session, &transition, false).unwrap();
        assert!(!path.exists());
        assert!(!recovery.is_active());
        assert!(!host.network.lock().unwrap().vpn);
    }

    #[tokio::test]
    async fn queued_rollback_reconnect_retains_the_baseline_after_late_failure() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        let recovery = SavedProtectionRecovery::default();
        {
            let mut network = host.network.lock().unwrap();
            network.vpn = true;
            network.queue_reconnect = true;
        }
        let (_, before_vpn) = recovery.observe_snapshot(&mut session).unwrap().unwrap();
        recovery.begin(&session, before_vpn).unwrap();
        recovery.rollback(&mut session).unwrap();
        session.observe_native().unwrap();
        assert!(matches!(
            recovery.poll(Some(&session)),
            RollbackProgress::Pending
        ));
        {
            let mut network = host.network.lock().unwrap();
            assert!(network.pending);
            network.pending = false;
            network.vpn = false;
            network.rollback_status["status"] = json!("failed");
            network.rollback_status["error"] = json!("fixture late reconnect failure");
        }
        session.observe_native().unwrap();
        assert!(
            matches!(recovery.poll(Some(&session)), RollbackProgress::Failed(error)
            if error == "fixture late reconnect failure")
        );
        host.network.lock().unwrap().queue_reconnect = false;
        recovery.retry(&mut session).unwrap();
        session.observe_native().unwrap();
        assert!(session.vpn);
        let RollbackProgress::VerifyProtection {
            request,
            check_dns: true,
        } = recovery.poll(Some(&session))
        else {
            panic!("expected the reconnected protection to require verification");
        };
        assert!(recovery.is_active());
        assert!(recovery.last_error().is_some());
        assert!(matches!(
            recovery.verified(&session, &request, Ok(())),
            RollbackProgress::Restored
        ));
        assert!(recovery.last_error().is_none());
    }

    #[tokio::test]
    async fn rollback_reload_failure_remains_visible_to_a_recreated_session() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        let recovery = SavedProtectionRecovery::default();
        host.network.lock().unwrap().vpn = true;
        let (before, before_vpn) = recovery.observe_snapshot(&mut session).unwrap().unwrap();
        recovery.begin(&session, before_vpn).unwrap();
        session
            .commit_protection(None, before.run_mode.clone())
            .unwrap();
        *host.failures.lock().unwrap() = vec!["reload"];
        recovery.rollback(&mut session).unwrap_err();
        drop(session);

        let recreated_recovery = SavedProtectionRecovery::default();
        recreated_recovery.reopen(root.path(), Language::English);
        let mut reopened = Session::open(root.path().into(), host.clone()).unwrap();
        assert!(recreated_recovery.is_active());
        reopened
            .attach_or_initialize_for_protection_recovery()
            .unwrap();
        assert!(
            !host.network.lock().unwrap().guard,
            "reattachment enabled the guard while the live configuration was still unprotected"
        );
        assert!(matches!(
            recreated_recovery.poll(Some(&reopened)),
            RollbackProgress::Failed(_)
        ));
        host.failures.lock().unwrap().clear();
        recreated_recovery.retry(&mut reopened).unwrap();
        assert!(recreated_recovery.last_error().is_some());
        reopened.observe_native().unwrap();
        let RollbackProgress::VerifyProtection {
            request,
            check_dns: true,
        } = recreated_recovery.poll(Some(&reopened))
        else {
            panic!("expected live verification after retry");
        };
        host.network.lock().unwrap().live_config = "mode: direct".into();
        assert!(matches!(
            recreated_recovery.verified(&reopened, &request, Ok(())),
            RollbackProgress::Failed(error) if error.contains("running core configuration")
        ));
        assert!(recreated_recovery.is_active());
        recreated_recovery.retry(&mut reopened).unwrap();
        reopened.observe_native().unwrap();
        assert!(recreated_recovery.last_error().is_some());
        let progress = recreated_recovery.poll(Some(&reopened));
        let request = progress.verification_request().unwrap();
        assert!(matches!(
            recreated_recovery.verified(&reopened, request, Ok(())),
            RollbackProgress::Restored
        ));
    }

    #[tokio::test]
    async fn rollback_preserves_a_saved_plan_when_the_vpn_was_off() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        let recovery = SavedProtectionRecovery::default();
        host.network.lock().unwrap().plan = Some(true);
        let (_, before_vpn) = recovery.observe_snapshot(&mut session).unwrap().unwrap();
        assert!(!before_vpn.connected);
        recovery.begin(&session, before_vpn).unwrap();
        host.network.lock().unwrap().plan = Some(false);
        recovery.rollback(&mut session).unwrap();
        {
            let network = host.network.lock().unwrap();
            assert!(!network.vpn);
            assert_eq!(network.plan, Some(true));
        }
        session.observe_native().unwrap();
        let RollbackProgress::VerifyProtection {
            request,
            check_dns: false,
        } = recovery.poll(Some(&session))
        else {
            panic!("an offline baseline still requires live core configuration verification");
        };
        assert!(matches!(
            recovery.verified(&session, &request, Ok(())),
            RollbackProgress::Restored
        ));
    }

    #[tokio::test]
    async fn rollback_rejects_old_vpn_and_requires_the_matching_new_generation() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = protected_session(root.path(), host.clone()).await;
        let recovery = SavedProtectionRecovery::default();
        {
            let mut network = host.network.lock().unwrap();
            network.vpn = true;
            network.guard = true;
            network.generation = 4;
            network.queue_reconnect = true;
        }
        let (_, vpn) = recovery.observe_snapshot(&mut session).unwrap().unwrap();
        recovery.begin(&session, vpn).unwrap();
        recovery.rollback(&mut session).unwrap();
        let request = {
            let mut network = host.network.lock().unwrap();
            let request = network.rollback_status["request"]
                .as_str()
                .unwrap()
                .to_owned();
            network.pending = false;
            network.rollback_status = json!({
                "request":"another-request","status":"complete","generation":5
            });
            request
        };
        session.observe_native().unwrap();
        assert!(matches!(
            recovery.poll(Some(&session)),
            RollbackProgress::Pending
        ));
        {
            let mut network = host.network.lock().unwrap();
            network.rollback_status = json!({"request":request,"status":"complete","generation":4});
        }
        session.observe_native().unwrap();
        assert!(session.vpn);
        assert!(matches!(
            recovery.poll(Some(&session)),
            RollbackProgress::Failed(_)
        ));
        host.network.lock().unwrap().queue_reconnect = false;
        recovery.retry(&mut session).unwrap();
        session.observe_native().unwrap();
        let progress = recovery.poll(Some(&session));
        let retry_request = progress.verification_request().unwrap();
        assert_ne!(retry_request, request);
        assert_eq!(session.platform["vpn_generation"], 5);
        assert!(matches!(
            recovery.verified(&session, &request, Ok(())),
            RollbackProgress::VerifyProtection { .. }
        ));
        assert!(recovery.is_active());
        host.network.lock().unwrap().vpn = false;
        assert!(matches!(
            recovery.verified(&session, retry_request, Ok(())),
            RollbackProgress::Failed(error) if error.contains("VPN changed")
        ));
        recovery.retry(&mut session).unwrap();
        session.observe_native().unwrap();
        let progress = recovery.poll(Some(&session));
        assert!(matches!(
            recovery.verified(&session, progress.verification_request().unwrap(), Ok(())),
            RollbackProgress::Restored
        ));
    }

    #[tokio::test]
    async fn saved_protection_rollback_keeps_a_live_vpn_with_historical_mode_off() {
        assert_saved_protection_rollback(ProxyMode::Off, true).await;
    }

    #[tokio::test]
    async fn saved_protection_rollback_leaves_a_stopped_vpn_off_with_historical_mode_tun() {
        assert_saved_protection_rollback(ProxyMode::Tun, false).await;
    }

    async fn assert_saved_protection_rollback(previous_mode: ProxyMode, live_vpn: bool) {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = Session::open(root.path().into(), host.clone()).unwrap();
        let recovery = SavedProtectionRecovery::default();
        session
            .save_profile(
                None,
                "Fixture",
                "",
                Some(
                    "proxies: [{name: Test, type: socks5, server: 127.0.0.1, port: 1080}]\nrules: ['MATCH,Test']\n",
                ),
            )
            .await
            .unwrap();
        let profile = session.profiles.last().unwrap().id.clone();
        session.activate(&profile).unwrap();
        let saved = Protection::new(
            profile.clone(),
            "Test".into(),
            Previous {
                mode: session.settings.run_mode.clone(),
                proxy_mode: previous_mode,
                selectors: Vec::new(),
            },
        );
        session
            .commit_protection(
                Some(Arc::new(saved.clone())),
                session.settings.run_mode.clone(),
            )
            .unwrap();
        session.attach_or_initialize().unwrap();
        assert!(session.running);
        assert!(recovery.snapshot(&session).is_none());
        {
            let mut network = host.network.lock().unwrap();
            network.vpn = !live_vpn;
            network.pending = true;
        }
        session.observe_native().unwrap();
        assert!(recovery.snapshot(&session).is_none());
        host.network.lock().unwrap().pending = false;
        session.observe_native().unwrap();
        assert_eq!(recovery.snapshot(&session).unwrap().1.connected, !live_vpn);

        host.take_operations();
        *host.failures.lock().unwrap() = vec!["state"];
        let error = recovery.observe_snapshot(&mut session).unwrap_err();
        assert!(format!("{error:#}").contains("fixture state failure"));
        host.failures.lock().unwrap().clear();
        for incomplete in [
            json!({"running":true,"vpn_transition_pending":false,"vpn_desired":null}),
            json!({"running":true,"vpn":live_vpn,"vpn_transition_pending":false}),
            json!({"vpn":live_vpn,"vpn_transition_pending":false,"vpn_desired":null}),
        ] {
            host.network.lock().unwrap().state = Some(incomplete);
            assert!(recovery.observe_snapshot(&mut session).unwrap().is_none());
        }
        assert_eq!(host.take_operations(), ["state"; 4]);
        {
            let mut network = host.network.lock().unwrap();
            network.state = None;
            network.vpn = live_vpn;
        }
        let (before, before_vpn) = recovery.observe_snapshot(&mut session).unwrap().unwrap();
        assert_eq!(before.claude_protection.as_deref(), Some(&saved));
        assert_eq!(before_vpn.connected, live_vpn);
        assert_eq!(before_vpn.desired, None);
        recovery.begin(&session, before_vpn).unwrap();
        let mut changed = session.settings.clone();
        changed.language = LanguagePreference::English;
        session.commit_settings(changed).unwrap();
        let raw = session.raw().unwrap();
        let runtime = std::fs::read(session.store.runtime().join("config.yaml")).unwrap();
        session
            .native
            .request(json!({"op":"vpn-plan","enabled":true}))
            .unwrap();
        *host.failures.lock().unwrap() = vec!["restore-vpn"];
        host.network.lock().unwrap().vpn = !live_vpn;
        host.take_operations();
        let error = recovery.rollback(&mut session).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("fixture restore-vpn failure"));
        assert_eq!(
            host.take_operations(),
            [
                "state",
                "hold-vpn-recovery",
                "reload",
                "protection-sync",
                "restore-vpn",
                "restore-vpn"
            ]
        );
        assert_eq!(session.store.load_settings().unwrap(), session.settings);
        assert_eq!(session.settings.claude_protection.as_deref(), Some(&saved));
        assert_eq!(
            std::fs::read(session.store.runtime().join("config.yaml")).unwrap(),
            runtime
        );

        session.observe_native().unwrap();
        assert_eq!(session.vpn, !live_vpn);
        let mut reopened = Session::open(root.path().into(), host.clone()).unwrap();
        let new_recovery = recovery.clone();
        reopened
            .attach_or_initialize_for_protection_recovery()
            .unwrap();
        assert!(new_recovery.snapshot(&reopened).is_none());
        reopened.observe_native().unwrap();
        assert!(matches!(
            new_recovery.poll(Some(&reopened)),
            RollbackProgress::Failed(_)
        ));
        host.failures.lock().unwrap().clear();
        host.take_operations();
        new_recovery.retry(&mut reopened).unwrap();
        assert_eq!(
            host.take_operations(),
            [
                "state",
                "hold-vpn-recovery",
                "reload",
                "protection-sync",
                "restore-vpn"
            ]
        );
        reopened.observe_native().unwrap();
        let RollbackProgress::VerifyProtection { request, check_dns } =
            new_recovery.poll(Some(&reopened))
        else {
            panic!("expected verification after reconnect or disconnect");
        };
        assert_eq!(check_dns, live_vpn);
        assert!(matches!(
            new_recovery.verified(&reopened, &request, Ok(())),
            RollbackProgress::Restored
        ));
        {
            let mut network = host.network.lock().unwrap();
            assert_eq!(
                (network.plan, network.vpn, network.guard),
                (None, live_vpn, live_vpn)
            );
            network.vpn = !live_vpn;
        }
        let (_, before_vpn) = recovery.observe_snapshot(&mut session).unwrap().unwrap();
        assert_eq!(before_vpn.connected, !live_vpn);
        recovery.begin(&session, before_vpn).unwrap();
        host.network.lock().unwrap().vpn = live_vpn;
        host.take_operations();
        recovery.rollback(&mut session).unwrap();
        assert_eq!(
            host.take_operations(),
            [
                "state",
                "hold-vpn-recovery",
                "reload",
                "protection-sync",
                "restore-vpn"
            ]
        );
        session.observe_native().unwrap();
        let progress = recovery.poll(Some(&session));
        assert!(matches!(
            recovery.verified(&session, progress.verification_request().unwrap(), Ok(())),
            RollbackProgress::Restored
        ));
        {
            let network = host.network.lock().unwrap();
            assert_eq!(
                (network.plan, network.vpn, network.guard),
                (None, !live_vpn, !live_vpn)
            );
        }
        let disk = session.store.load_settings().unwrap();
        assert_eq!(disk.claude_protection.as_deref(), Some(&saved));
        assert_eq!(disk.language, LanguagePreference::English);
        assert_eq!(
            session.settings.active_profile.as_deref(),
            Some(profile.as_str())
        );
        assert_eq!(session.raw().unwrap(), raw);
        assert_eq!(
            std::fs::read(session.store.runtime().join("config.yaml")).unwrap(),
            runtime
        );
    }

    #[tokio::test]
    async fn failed_new_protection_enable_restores_the_unprotected_settings() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = Session::open(root.path().into(), host).unwrap();
        session
            .save_profile(
                None,
                "Fixture",
                "",
                Some(
                    "proxies: [{name: Test, type: socks5, server: 127.0.0.1, port: 1080}]\nrules: ['MATCH,Test']\n",
                ),
            )
            .await
            .unwrap();
        let profile = session.profiles.last().unwrap().id.clone();
        session.activate(&profile).unwrap();
        let before = session.settings.clone();
        let saved = Protection::new(
            profile,
            "Test".into(),
            Previous {
                mode: before.run_mode.clone(),
                proxy_mode: ProxyMode::Off,
                selectors: Vec::new(),
            },
        );
        let recovery = SavedProtectionRecovery::default();
        session.observe_native().unwrap();
        recovery
            .begin(&session, VpnBaseline::capture(&session).unwrap())
            .unwrap();
        session
            .commit_protection(Some(Arc::new(saved)), before.run_mode.clone())
            .unwrap();
        recovery.rollback(&mut session).unwrap();
        assert!(session.settings.claude_protection.is_none());
        let disk: Settings =
            serde_json::from_slice(&std::fs::read(root.path().join("settings.json")).unwrap())
                .unwrap();
        assert!(disk.claude_protection.is_none());
    }

    #[tokio::test]
    async fn rollback_persists_original_protection_through_plan_reload_and_guard_failures() {
        for before_vpn in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let host = Arc::new(Host::new(root.path()));
            let mut session = Session::open(root.path().into(), host.clone()).unwrap();
            session
                .save_profile(
                    None,
                    "Fixture",
                    "",
                    Some(
                        "proxies: [{name: Test, type: socks5, server: 127.0.0.1, port: 1080}]\nrules: ['MATCH,Test']\n",
                    ),
                )
                .await
                .unwrap();
            let profile = session.profiles.last().unwrap().id.clone();
            session.activate(&profile).unwrap();
            session.running = true;
            let mode = session.settings.run_mode.clone();
            let protection = Arc::new(Protection::new(
                profile.clone(),
                "Test".into(),
                Previous {
                    mode: mode.clone(),
                    proxy_mode: if before_vpn {
                        ProxyMode::Tun
                    } else {
                        ProxyMode::Off
                    },
                    selectors: Vec::new(),
                },
            ));
            session
                .commit_protection(Some(protection.clone()), mode.clone())
                .unwrap();
            let vpn = VpnBaseline {
                connected: before_vpn,
                desired: Some(!before_vpn),
            };
            let recovery = SavedProtectionRecovery::default();
            session.observe_native().unwrap();
            recovery.begin(&session, vpn).unwrap();
            let runtime = session.store.runtime().join("config.yaml");
            let protected_config = std::fs::read_to_string(&runtime).unwrap();
            let profile_path = session.store.profile_path(&profile).unwrap();
            let yaml = std::fs::read(&profile_path).unwrap();

            session.commit_protection(None, mode).unwrap();
            let mut changed = session.settings.clone();
            changed.language = LanguagePreference::English;
            session.commit_settings(changed).unwrap();
            let live_unprotected = host.network.lock().unwrap().live_config.clone();
            assert_ne!(protected_config, live_unprotected);
            host.network.lock().unwrap().vpn = true;
            host.take_operations();

            *host.failures.lock().unwrap() = vec!["reload", "restore-vpn"];
            let error = recovery.rollback(&mut session).unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains("fixture restore-vpn failure"));
            assert!(message.contains("fixture reload failure"));
            assert_eq!(
                host.take_operations(),
                [
                    "state",
                    "hold-vpn-recovery",
                    "reload",
                    "restore-vpn",
                    "restore-vpn"
                ]
            );
            let assert_saved = |session: &Session| {
                assert_eq!(
                    session.settings.claude_protection.as_ref(),
                    Some(&protection)
                );
                let disk = session.store.load_settings().unwrap();
                assert_eq!(disk.claude_protection.as_ref(), Some(&protection));
                assert_eq!(disk.language, LanguagePreference::English);
                assert_eq!(std::fs::read_to_string(&runtime).unwrap(), protected_config);
                assert_eq!(std::fs::read(&profile_path).unwrap(), yaml);
            };
            assert_saved(&session);
            {
                let network = host.network.lock().unwrap();
                assert_eq!(network.live_config, live_unprotected);
                assert!(!network.guard);
                assert!(network.vpn);
                assert!(network.forwarding_held);
                assert_eq!(network.plan, None);
            }

            *host.failures.lock().unwrap() = vec!["protection-sync"];
            let error = recovery.retry(&mut session).unwrap_err();
            assert!(format!("{error:#}").contains("fixture protection-sync failure"));
            assert_eq!(
                host.take_operations(),
                [
                    "state",
                    "hold-vpn-recovery",
                    "reload",
                    "protection-sync",
                    "restore-vpn"
                ]
            );
            assert_saved(&session);
            assert!(!host.network.lock().unwrap().guard);

            host.failures.lock().unwrap().clear();
            recovery.retry(&mut session).unwrap();
            assert_eq!(
                host.take_operations(),
                [
                    "state",
                    "hold-vpn-recovery",
                    "reload",
                    "protection-sync",
                    "restore-vpn"
                ]
            );
            assert_saved(&session);
            session.observe_native().unwrap();
            let progress = recovery.poll(Some(&session));
            assert!(matches!(
                recovery.verified(&session, progress.verification_request().unwrap(), Ok(())),
                RollbackProgress::Restored
            ));
            let network = host.network.lock().unwrap();
            assert_eq!(network.plan, Some(!before_vpn));
            assert_eq!(network.vpn, before_vpn);
            assert_eq!(network.guard, before_vpn);
            assert_eq!(network.live_config, protected_config);
        }
    }

    #[test]
    fn stopped_core_is_not_initialized_or_given_a_new_runtime_config() {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new(root.path()));
        let mut session = Session::open(root.path().into(), host.clone()).unwrap();
        let path = session.store.runtime().join("config.yaml");
        let original = b"# exact existing runtime bytes\nmode: rule\n";
        std::fs::write(&path, original).unwrap();
        let record = RecoveryRecord {
            version: 1,
            token: "fixture".into(),
            profile: session.settings.active_profile.clone().unwrap(),
            run_mode: session.settings.run_mode.clone(),
            protection: None,
            vpn: VpnBaseline {
                connected: true,
                desired: None,
            },
        };
        let error = rollback_network(&mut session, &record, false).unwrap_err();
        assert!(format!("{error:#}").contains("Core is not running"));
        assert_eq!(host.take_operations(), ["restore-vpn"]);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}
