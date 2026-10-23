use super::*;
use crate::control::{Command as ControlCommand, Meta, Poll, Remote};

impl Engine {
    pub fn view_revision(&self) -> u64 {
        self.view_revision
    }

    pub fn invalidate_views(&mut self) {
        self.view_revision = self.view_revision.wrapping_add(1);
    }

    pub fn is_remote(&self) -> bool {
        self.remote.is_some()
    }

    pub(super) fn attach(store: &Store, resources: &std::path::Path) -> Result<Option<Self>> {
        let Some(remote) = Remote::discover(store, Duration::from_secs(300))? else {
            return Ok(None);
        };
        let meta = match remote.call::<Meta>(ControlCommand::Meta) {
            Ok(meta) => meta,
            Err(error) => {
                // Only an unlocked directory can be recovered as a stale owner.
                // Never restore another live backend's proxy/DNS or steal its lock.
                if store.instance_lock().is_err() {
                    return Err(error);
                }
                return Ok(None);
            }
        };
        let api = Api::new(&meta.settings)?;
        Ok(Some(Self {
            store: store.clone(),
            settings: meta.settings,
            profiles: meta.profiles,
            api,
            geo_manifest: meta.geo,
            resources: resources.to_owned(),
            child: None,
            #[cfg(windows)]
            windows_tun: false,
            #[cfg(target_os = "macos")]
            mac_tun: None,
            #[cfg(target_os = "macos")]
            retained_startup_proxy: false,
            logs: Arc::new(Mutex::new(VecDeque::new())),
            readers: vec![],
            log_pump: None,
            last_totals: None,
            version: meta.version,
            node_root: Mutex::new(None),
            protection_guard: None,
            _instance_lock: None,
            remote: Some(remote),
            remote_running: meta.running,
            control_server: None,
            external_logs_until: None,
            view_revision: meta.view_revision,
        }))
    }

    fn apply_meta(&mut self, meta: Meta) -> Result<()> {
        self.api = Api::new(&meta.settings)?;
        self.settings = meta.settings;
        self.profiles = meta.profiles;
        self.geo_manifest = meta.geo;
        self.remote_running = meta.running;
        self.view_revision = meta.view_revision;
        Ok(())
    }

    pub(super) async fn remote_mutate(&mut self, operation: ControlCommand) -> Result<bool> {
        let Some(remote) = self.remote.clone() else {
            return Ok(false);
        };
        let _: Value = remote.request(operation).await?;
        self.apply_meta(remote.request(ControlCommand::Meta).await?)?;
        Ok(true)
    }

    pub(super) fn remote_mutate_sync(&mut self, operation: ControlCommand) -> Result<bool> {
        let Some(remote) = self.remote.clone() else {
            return Ok(false);
        };
        let _: Value = remote.call(operation)?;
        self.apply_meta(remote.call(ControlCommand::Meta)?)?;
        Ok(true)
    }

    pub(super) async fn remote_poll(&mut self, scope: Scope, header: bool) -> Result<Snapshot> {
        let remote = self.remote.clone().context("后台连接不可用")?;
        let mut result: Poll = match remote
            .request(ControlCommand::Snapshot { scope, header })
            .await
        {
            Ok(result) => result,
            Err(error) => {
                // An attached GUI must offer a usable restart after its
                // independent owner exits, rather than retaining "running".
                if self.store.instance_lock().is_ok() {
                    self.remote_running = false;
                }
                return Err(error);
            }
        };
        self.apply_meta(result.meta)?;
        if scope == Scope::Connections {
            result.snapshot.connections_sampled_at = Some(Instant::now());
        }
        Ok(result.snapshot)
    }

    pub(crate) fn retain_external_logs(&mut self) {
        self.external_logs_until = Some(Instant::now() + Duration::from_secs(10));
    }

    pub fn shutdown_signal(&self) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.control_server
            .as_ref()
            .map(crate::control::Server::shutdown)
    }

    pub fn exit_requested(&self) -> bool {
        self.shutdown_signal()
            .is_some_and(|signal| *signal.borrow())
    }

    pub async fn shutdown_owner(&mut self) -> Result<()> {
        if let Some(remote) = &self.remote {
            let result: Result<Value> = remote.request(ControlCommand::Quit).await;
            if let Err(error) = result
                && self.store.instance_lock().is_err()
            {
                return Err(error);
            }
            self.remote_running = false;
            return Ok(());
        }
        self.stop().await
    }

    pub(crate) fn effective_profile(&self, raw: &str, id: &str) -> Result<String> {
        self.profile_payload(raw, &self.settings, id)
    }
    pub(crate) async fn check_payload(&self, payload: &str) -> Result<()> {
        self.validate(payload).await
    }
}
