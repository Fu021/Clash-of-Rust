use super::*;

pub(crate) fn fixture(store: Store) -> Engine {
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
        unreachable_recovery: Default::default(),
        subscription_failure: None,
        subscription_failure_revision: 0,
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
