//! Keep network downloads outside the shared desktop owner lock.
use super::*;
use std::future::Future;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

pub(crate) struct UpdateGate {
    slot: Arc<Semaphore>,
    cancellation: watch::Sender<u64>,
}

impl Default for UpdateGate {
    fn default() -> Self {
        let (cancellation, _) = watch::channel(0);
        Self {
            slot: Arc::new(Semaphore::new(1)),
            cancellation,
        }
    }
}

pub(crate) struct UpdateJob {
    _permit: OwnedSemaphorePermit,
    cancellation: watch::Receiver<u64>,
}

impl UpdateGate {
    pub(crate) fn begin(&self, busy: &'static str) -> Result<UpdateJob> {
        let permit = self.slot.clone().try_acquire_owned().context(busy)?;
        Ok(UpdateJob {
            _permit: permit,
            cancellation: self.cancellation.subscribe(),
        })
    }

    pub(super) fn cancel(&self) {
        self.cancellation
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }
}

impl UpdateJob {
    pub(crate) async fn download<T>(
        &mut self,
        future: impl Future<Output = Result<T>>,
        disconnected: &mut Option<watch::Receiver<bool>>,
    ) -> Result<T> {
        self.ensure_current(disconnected)?;
        tokio::select! {
            result = future => result,
            _ = self.cancellation.changed() => bail!("内核已停止，更新已取消"),
            _ = wait_for_disconnect(disconnected) => bail!("请求已断开，更新已取消"),
        }
    }

    pub(crate) fn ensure_current(
        &self,
        disconnected: &Option<watch::Receiver<bool>>,
    ) -> Result<()> {
        ensure!(
            !self.cancellation.has_changed().unwrap_or(true),
            "内核已停止，更新已取消"
        );
        ensure!(
            disconnected
                .as_ref()
                .is_none_or(|receiver| !*receiver.borrow()),
            "请求已断开，更新已取消"
        );
        Ok(())
    }
}

async fn wait_for_disconnect(disconnected: &mut Option<watch::Receiver<bool>>) {
    match disconnected {
        Some(receiver) => {
            if !*receiver.borrow() {
                let _ = receiver.changed().await;
            }
        }
        None => std::future::pending().await,
    }
}

impl Engine {
    pub(crate) async fn download_app_update_with<T, F, Fut>(
        shared: &Arc<tokio::sync::Mutex<Self>>,
        disconnected: &mut Option<watch::Receiver<bool>>,
        download: F,
    ) -> Result<(UpdateJob, T, bool)>
    where
        F: FnOnce(Option<u16>, bool, Api) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let (mut job, port, prereleases, api) = {
            let engine = shared.lock().await;
            (
                engine.app_updates.begin("客户端更新正在进行，请稍候")?,
                engine.running().then_some(engine.settings.mixed_port),
                engine.settings.update_prereleases,
                engine.api.clone(),
            )
        };
        let download = job
            .download(download(port, prereleases, api), disconnected)
            .await?;
        Ok((job, download, prereleases))
    }

    pub async fn import_shared(
        shared: &Arc<tokio::sync::Mutex<Self>>,
        name: String,
        source: String,
        existing: Option<String>,
    ) -> Result<()> {
        let expect_current = existing.is_some();
        Self::import_cancellable(shared, name, source, existing, expect_current, None).await
    }

    pub(crate) async fn import_cancellable(
        shared: &Arc<tokio::sync::Mutex<Self>>,
        name: String,
        source: String,
        existing: Option<String>,
        expect_current: bool,
        disconnected: Option<watch::Receiver<bool>>,
    ) -> Result<()> {
        ensure!(!expect_current || existing.is_some(), "订阅不存在");
        let remote = shared.lock().await.remote.clone();
        if let Some(remote) = remote {
            let _: Value = remote
                .request(ControlCommand::Import {
                    name,
                    source,
                    id: existing,
                    expect_current,
                })
                .await?;
            let mut engine = shared.lock().await;
            return engine.apply_meta(remote.request(ControlCommand::Meta).await?);
        }
        if !(source.starts_with("https://") || source.starts_with("http://")) {
            let mut engine = shared.lock().await;
            if expect_current {
                let id = existing.as_deref().context("订阅不存在")?;
                engine.require_current_import(id, &name, &source)?;
            }
            return engine.import(name, source, existing).await;
        }
        Self::import_remote_with(
            shared,
            name,
            source,
            existing,
            expect_current,
            disconnected,
            |url, port| async move { crate::subscription::download(&url, port).await },
        )
        .await
    }

    async fn import_remote_with<F, Fut>(
        shared: &Arc<tokio::sync::Mutex<Self>>,
        name: String,
        source: String,
        existing: Option<String>,
        expect_current: bool,
        mut disconnected: Option<watch::Receiver<bool>>,
        download: F,
    ) -> Result<()>
    where
        F: FnOnce(reqwest::Url, Option<u16>) -> Fut,
        Fut: Future<Output = Result<(String, Option<String>)>>,
    {
        let url = reqwest::Url::parse(&source).context("订阅 URL 无效")?;
        let (mut job, pending, port) = {
            let engine = shared.lock().await;
            let job = engine.subscription_updates.begin("订阅正在更新，请稍候")?;
            let pending = if let Some(id) = &existing {
                engine.require_unprotected_profile(id)?;
                let profile = engine
                    .profiles
                    .iter()
                    .find(|profile| &profile.id == id)
                    .context("订阅不存在")?;
                if expect_current {
                    engine.require_current_import(id, &name, &source)?;
                }
                Some(
                    clash_client_core::subscription::schedule::PendingUpdate::capture(
                        profile.clone(),
                        &engine.store,
                    )?,
                )
            } else {
                None
            };
            (
                job,
                pending,
                engine.running().then_some(engine.settings.mixed_port),
            )
        };
        let (raw, usage) = job
            .download(download(url, port), &mut disconnected)
            .await
            .context(if existing.is_some() {
                "订阅更新失败，已保留原配置与运行状态"
            } else {
                "订阅下载失败"
            })?;
        let mut engine = shared.lock().await;
        job.ensure_current(&disconnected)?;
        if let Some(pending) = pending {
            ensure!(
                pending.is_unchanged(&engine.settings, &engine.profiles, &engine.store)?,
                "订阅在下载期间已改变，已保留现有配置"
            );
        }
        engine
            .import_content(name, source, existing, raw, usage)
            .await
    }

    fn require_current_import(&self, id: &str, name: &str, source: &str) -> Result<()> {
        let profile = self
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .context("订阅不存在")?;
        ensure!(
            profile.name == name && profile.source == source,
            "订阅已修改，请刷新后重试"
        );
        Ok(())
    }

    /// The owner lock covers only preparation and installation, never downloads.
    pub async fn update_geo_shared(shared: &Arc<tokio::sync::Mutex<Self>>) -> Result<()> {
        Self::update_geo_cancellable(shared, None).await
    }

    pub(crate) async fn update_geo_cancellable(
        shared: &Arc<tokio::sync::Mutex<Self>>,
        disconnected: Option<watch::Receiver<bool>>,
    ) -> Result<()> {
        let remote = shared.lock().await.remote.clone();
        if let Some(remote) = remote {
            let _: Value = remote.request(ControlCommand::GeoUpdate).await?;
            let mut engine = shared.lock().await;
            return engine.apply_meta(remote.request(ControlCommand::Meta).await?);
        }
        Self::download_geo_with(shared, disconnected, |path, port| async move {
            assets::download_geo(&path, port).await.map(|_| ())
        })
        .await
    }

    async fn download_geo_with<F, Fut>(
        shared: &Arc<tokio::sync::Mutex<Self>>,
        mut disconnected: Option<watch::Receiver<bool>>,
        download: F,
    ) -> Result<()>
    where
        F: FnOnce(PathBuf, Option<u16>) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let (mut job, staged, port) = {
            let engine = shared.lock().await;
            let job = engine.geo_updates.begin("Geo 数据正在更新，请稍候")?;
            let staged = tempfile::Builder::new()
                .prefix(".geo-download-")
                .tempdir_in(engine.store.runtime())?;
            let port = engine.running().then_some(engine.settings.mixed_port);
            append(
                &engine.logs,
                "[客户端] 开始下载全部 Geo 数据，下载期间继续使用原数据",
            );
            (job, staged, port)
        };
        job.download(download(staged.path().to_owned(), port), &mut disconnected)
            .await?;
        let mut engine = shared.lock().await;
        job.ensure_current(&disconnected)?;
        engine.install_geo_staged(staged.path()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    fn fixture(store: Store) -> Engine {
        let settings = Settings::default();
        Engine {
            store,
            api: Api::new(&settings).unwrap(),
            settings,
            profiles: Vec::new(),
            geo_manifest: GeoManifest {
                version: "fixture".into(),
                updated: 0,
                installed_at: None,
                files: Vec::new(),
            },
            geo_updates: Default::default(),
            app_updates: Default::default(),
            subscription_updates: Default::default(),
            resources: PathBuf::new(),
            child: None,
            #[cfg(windows)]
            windows_tun: false,
            #[cfg(target_os = "macos")]
            mac_tun: None,
            #[cfg(target_os = "macos")]
            retained_startup_proxy: false,
            #[cfg(target_os = "macos")]
            mac_dns_error: None,
            logs: Arc::new(Mutex::new(VecDeque::new())),
            readers: Vec::new(),
            log_pump: None,
            last_totals: None,
            version: String::new(),
            node_root: Mutex::new(None),
            protection_guard: None,
            _instance_lock: None,
            remote: None,
            remote_running: false,
            control_server: None,
            external_logs_until: None,
            view_revision: 0,
            network_monitor: Default::default(),
        }
    }

    #[tokio::test]
    async fn slow_download_does_not_lock_owner_and_stop_cancels_it() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::at(root.path().to_owned()).unwrap();
        let shared = Arc::new(tokio::sync::Mutex::new(fixture(store.clone())));
        let (started, ready) = oneshot::channel();
        let running = shared.clone();
        let task = tokio::spawn(async move {
            Engine::download_geo_with(&running, None, |path, _| async move {
                std::fs::write(path.join("partial"), b"incomplete")?;
                started.send(()).unwrap();
                std::future::pending::<Result<()>>().await
            })
            .await
        });
        ready.await.unwrap();
        {
            let owner = shared.try_lock().expect("download held the owner lock");
            assert_eq!(owner.geo_manifest.version, "fixture");
            assert!(owner.geo_updates.begin("busy").is_err());
            owner.geo_updates.cancel();
        }
        let error = task.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("已取消"));
        assert!(
            std::fs::read_dir(store.runtime()).unwrap().next().is_none(),
            "cancelled temporary data must be removed"
        );
        assert!(shared.lock().await.geo_updates.begin("busy").is_ok());
    }

    #[tokio::test]
    async fn disconnected_client_cancels_download_before_installation() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::at(root.path().to_owned()).unwrap();
        let shared = Arc::new(tokio::sync::Mutex::new(fixture(store.clone())));
        let (cancel, disconnected) = watch::channel(false);
        let (started, ready) = oneshot::channel();
        let running = shared.clone();
        let task = tokio::spawn(async move {
            Engine::download_geo_with(&running, Some(disconnected), |path, _| async move {
                std::fs::write(path.join("partial"), b"incomplete")?;
                started.send(()).unwrap();
                std::future::pending::<Result<()>>().await
            })
            .await
        });
        ready.await.unwrap();
        cancel.send_replace(true);
        assert!(
            task.await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("断开")
        );
        assert!(std::fs::read_dir(store.runtime()).unwrap().next().is_none());
        assert!(shared.lock().await.geo_updates.begin("busy").is_ok());
    }

    #[tokio::test]
    async fn slow_client_update_does_not_block_owner_and_can_be_cancelled() {
        let root = tempfile::tempdir().unwrap();
        let shared = Arc::new(tokio::sync::Mutex::new(fixture(
            Store::at(root.path().to_owned()).unwrap(),
        )));
        let (started, ready) = oneshot::channel();
        let running = shared.clone();
        let task = tokio::spawn(async move {
            Engine::download_app_update_with(&running, &mut None, |_, _, _| async move {
                started.send(()).unwrap();
                std::future::pending::<Result<()>>().await
            })
            .await
        });
        ready.await.unwrap();
        {
            let owner = shared.try_lock().expect("download held the owner lock");
            assert!(owner.app_updates.begin("busy").is_err());
            owner.app_updates.cancel();
        }
        assert!(task.await.unwrap().is_err());
        assert!(shared.lock().await.app_updates.begin("busy").is_ok());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn update_preparation_releases_owner_and_rechecks_changes_before_commit() {
        use std::sync::atomic::{AtomicBool, Ordering};

        for scenario in 0..5 {
            let root = tempfile::tempdir().unwrap();
            let shared = Arc::new(tokio::sync::Mutex::new(fixture(
                Store::at(root.path().to_owned()).unwrap(),
            )));
            let job = shared.lock().await.app_updates.begin("busy").unwrap();
            let prereleases = shared.lock().await.settings.update_prereleases;
            let (disconnected_tx, disconnected) = watch::channel(false);
            let (started, ready) = oneshot::channel();
            let (finish, resume) = oneshot::channel();
            let cancelled = Arc::new(AtomicBool::new(false));
            let observed = cancelled.clone();
            let running = shared.clone();
            let mut gui_lock = None;
            let task = tokio::spawn(async move {
                crate::control::prepare_update_handoff(
                    &running,
                    &job,
                    &Some(disconnected),
                    prereleases,
                    move |_, _| async move {
                        started.send(()).unwrap();
                        resume.await.unwrap();
                        Ok(())
                    },
                    move |_: &()| observed.store(true, Ordering::SeqCst),
                )
                .await
                .map(|_| ())
            });
            ready.await.unwrap();
            {
                let mut owner = shared.try_lock().expect("preparation held the owner lock");
                match scenario {
                    1 => owner.app_updates.cancel(),
                    2 => owner.settings.update_prereleases = !prereleases,
                    3 => {
                        disconnected_tx.send_replace(true);
                    }
                    4 => {
                        let lock = std::fs::File::options()
                            .read(true)
                            .write(true)
                            .create(true)
                            .truncate(false)
                            .open(owner.store.root.join("gui-session.lock"))
                            .unwrap();
                        lock.try_lock().unwrap();
                        std::fs::write(
                            owner.store.root.join("gui-session.json"),
                            serde_json::to_vec(&serde_json::json!({
                                "pid": std::process::id(),
                                "nonce": "00000000-0000-0000-0000-000000000000"
                            }))
                            .unwrap(),
                        )
                        .unwrap();
                        gui_lock = Some(lock);
                    }
                    _ => {}
                }
            }
            finish.send(()).unwrap();
            let result = task.await.unwrap();
            assert_eq!(result.is_ok(), scenario == 0);
            assert_eq!(cancelled.load(Ordering::SeqCst), scenario != 0);
            drop(gui_lock);
        }
    }

    #[tokio::test]
    async fn explicit_import_can_change_source_but_stale_update_cannot() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let root = tempfile::tempdir().unwrap();
        let store = Store::at(root.path().to_owned()).unwrap();
        let profile = Profile {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Fixture".into(),
            source: "https://example.test/current".into(),
            updated: 1,
            usage: None,
        };
        let path = store.profile_path(&profile.id).unwrap();
        let original = "proxies: []\nrules: ['MATCH,DIRECT']\n";
        atomic_write(&path, original.as_bytes()).unwrap();
        let mut owner = fixture(store);
        owner.profiles.push(profile.clone());
        let shared = Arc::new(tokio::sync::Mutex::new(owner));
        let called = Arc::new(AtomicBool::new(false));

        for expect_current in [true, false] {
            let attempt_called = called.clone();
            let error = Engine::import_remote_with(
                &shared,
                profile.name.clone(),
                "https://example.test/replacement".into(),
                Some(profile.id.clone()),
                expect_current,
                None,
                move |_, _| async move {
                    attempt_called.store(true, Ordering::SeqCst);
                    bail!("HTTP 503")
                },
            )
            .await
            .unwrap_err();
            if expect_current {
                assert!(error.to_string().contains("订阅已修改"));
                assert!(!called.load(Ordering::SeqCst));
            } else {
                assert!(format!("{error:#}").contains("已保留原配置与运行状态"));
                assert!(called.load(Ordering::SeqCst));
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        }
    }

    #[tokio::test]
    async fn manual_subscription_download_preserves_an_edited_profile() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::at(root.path().to_owned()).unwrap();
        let profile = Profile {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Fixture".into(),
            source: "https://example.test/sub".into(),
            updated: 1,
            usage: None,
        };
        let path = store.profile_path(&profile.id).unwrap();
        let original = "proxies: []\nrules: ['MATCH,DIRECT']\n";
        atomic_write(&path, original.as_bytes()).unwrap();
        let mut owner = fixture(store);
        owner.profiles.push(profile.clone());
        owner.settings.subscription_update_interval_minutes = 0;
        let shared = Arc::new(tokio::sync::Mutex::new(owner));
        let (started, ready) = oneshot::channel();
        let (finish, resume) = oneshot::channel();
        let running = shared.clone();
        let task = tokio::spawn(async move {
            Engine::import_remote_with(
                &running,
                profile.name,
                profile.source,
                Some(profile.id),
                true,
                None,
                |_, _| async move {
                    started.send(()).unwrap();
                    resume.await.unwrap();
                    Ok(("proxies: []\nrules: ['MATCH,REJECT']\n".into(), None))
                },
            )
            .await
        });
        ready.await.unwrap();
        {
            let mut owner = shared.try_lock().expect("download held the owner lock");
            owner.profiles[0].source = "https://example.test/edited".into();
        }
        finish.send(()).unwrap();
        assert!(
            task.await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("已改变")
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    }
}
