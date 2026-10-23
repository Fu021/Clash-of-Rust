mod control;
use crate::control::Command as ControlCommand;
use crate::{
    api::{Api, ConfigStatus, ConnectionStats, Proxies},
    assets::{self, GeoManifest},
    config::{
        Profile, RuleOverrides, Settings, Store, atomic_write, parse_profile, read_profile,
        read_runtime_config, runtime_config,
    },
    platform,
};
use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs::File,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    task::JoinHandle,
};

pub use crate::proxy::ProxyMode;
use clash_client_core::runtime::logging::{LOG_FRAME_LIMIT, LogFrames, append, read_core_logs};
pub use clash_client_core::runtime::logging::{LOG_LIMIT, LogBuffer};

#[cfg(target_os = "macos")]
fn mac_port_probe(port: u16) -> std::io::Result<tokio::net::TcpListener> {
    // Match Go's listener behavior. A graceful core shutdown can leave API
    // connections in TIME_WAIT; these do not prevent mihomo from rebinding.
    // Listen as well as bind so an active listener still fails the probe.
    let listener = platform::macos::tun::probe_port(port)?;
    listener.set_nonblocking(true)?;
    tokio::net::TcpListener::from_std(listener)
}

#[cfg(target_os = "macos")]
async fn wait_for_mac_port(
    port: u16,
    logs: &LogBuffer,
    shutdown_grace: Duration,
) -> std::io::Result<()> {
    // A prior TUN core can leave root-owned TIME_WAIT sockets after it exits.
    // Darwin denies reuse by the new ordinary-user core until those expire.
    // Ordinary starts reject listeners immediately. Update handoffs allow only
    // the short explicit shutdown grace; never stop the listener's owner.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let shutdown_deadline = tokio::time::Instant::now() + shutdown_grace;
    let mut reported = false;
    loop {
        let error = match mac_port_probe(port) {
            Ok(_) => return Ok(()),
            Err(error) => error,
        };
        if error.kind() != std::io::ErrorKind::AddrInUse {
            return Err(error);
        }
        match tokio::time::timeout(
            Duration::from_millis(200),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await
        {
            Ok(Err(ref refused)) if refused.kind() == std::io::ErrorKind::ConnectionRefused => {}
            Ok(Ok(_)) if tokio::time::Instant::now() < shutdown_deadline => {}
            _ => return Err(error),
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(error);
        }
        if !reported {
            append(
                logs,
                format!("[客户端] 等待端口 {port} 的退出交接或连接状态释放"),
            );
            reported = true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn capture_output(mut reader: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut captured = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let count = reader.read(&mut chunk).await?;
        if count == 0 {
            return Ok(captured);
        }
        let keep = count.min(LOG_FRAME_LIMIT - captured.len());
        captured.extend_from_slice(&chunk[..keep]);
    }
}

pub use clash_client_core::runtime::snapshot::{Scope, Snapshot};

pub struct Engine {
    pub store: Store,
    pub settings: Settings,
    pub profiles: Vec<Profile>,
    pub api: Api,
    pub geo_manifest: GeoManifest,
    resources: PathBuf,
    child: Option<Child>,
    #[cfg(windows)]
    windows_tun: bool,
    #[cfg(target_os = "macos")]
    mac_tun: Option<platform::macos::tun::Session>,
    #[cfg(target_os = "macos")]
    retained_startup_proxy: bool,
    logs: LogBuffer,
    readers: Vec<JoinHandle<()>>,
    log_pump: Option<JoinHandle<()>>,
    last_totals: Option<(Instant, u64, u64)>,
    version: String,
    node_root: Mutex<Option<String>>,
    protection_guard: Option<crate::claude_check::guard::Guard>,
    // Held for the entire application lifetime; prevents two clients racing on recovery.
    _instance_lock: Option<File>,
    remote: Option<crate::control::Remote>,
    remote_running: bool,
    pub(crate) control_server: Option<crate::control::Server>,
    external_logs_until: Option<Instant>,
    pub(crate) view_revision: u64,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct RestartState {
    mode: String,
    tun: bool,
    system_proxy: bool,
    selectors: Vec<(String, String)>,
}

impl Engine {
    pub fn new(store: Store) -> Result<Self> {
        Self::with_resources(store, assets::discover()?)
    }

    pub fn with_resources(store: Store, resources: PathBuf) -> Result<Self> {
        if let Some(engine) = Self::attach(&store, &resources)? {
            return Ok(engine);
        }
        let instance = match store.instance_lock() {
            Ok(instance) => instance,
            Err(error) => {
                // The other owner may still be seeding resources before its
                // endpoint is published. Attach without recovering its journals.
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    if let Some(engine) = Self::attach(&store, &resources)? {
                        return Ok(engine);
                    }
                    if let Ok(instance) = store.instance_lock() {
                        break instance;
                    }
                    if Instant::now() >= deadline {
                        return Err(error);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        };
        crate::profile_transaction::recover(&store.root)?;
        let geo_manifest = assets::seed(&resources, &store.runtime())?;
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        let settings = store.load_settings();
        #[cfg(target_os = "macos")]
        let keep_proxy = settings.as_ref().is_ok_and(|settings| {
            settings.proxy_mode == ProxyMode::System
                && platform::is_owned_at_port(
                    &store.root.join("system-proxy.json"),
                    settings.mixed_port,
                )
                .unwrap_or(false)
        });
        #[cfg(not(target_os = "macos"))]
        let keep_proxy = false;
        match if keep_proxy {
            append(
                &logs,
                "[客户端] 继续使用上次运行的系统代理，保留原设置恢复记录",
            );
            Ok(false)
        } else {
            platform::restore(&store.root.join("system-proxy.json"))
        } {
            Ok(true) => append(&logs, "[客户端] 已恢复上次运行遗留的系统代理"),
            Ok(false) => {}
            Err(e) => append(&logs, format!("[客户端] 代理恢复失败：{e}")),
        }
        let mut settings = settings?;
        #[cfg(target_os = "macos")]
        {
            platform::macos::service::protection_dns(false)
                .context("恢复上次运行的系统 DNS 失败")?;
            if let Some(protection) = &mut settings.claude_protection
                && protection.system_dns.is_empty()
            {
                Arc::make_mut(protection).system_dns = platform::macos::dns::system_servers()?;
                store.save_settings(&settings)?;
            }
        }
        let mut profiles = store.profiles()?;
        if profiles.is_empty() {
            let raw = read_profile(&resources.join("default.yaml"))
                .context("默认配置缺失，请重新安装")?;
            parse_profile(&raw)?;
            let id = uuid::Uuid::new_v4().to_string();
            atomic_write(&store.profile_path(&id)?, raw.as_bytes())?;
            profiles.push(Profile {
                id: id.clone(),
                name: "默认直连".into(),
                source: resources.join("default.yaml").display().to_string(),
                updated: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                usage: None,
            });
            store.save_profiles(&profiles)?;
            settings.active_profile = Some(id);
            store.save_settings(&settings)?;
        }
        let api = Api::new(&settings)?;
        Ok(Self {
            store,
            settings,
            profiles,
            api,
            resources,
            geo_manifest,
            child: None,
            #[cfg(windows)]
            windows_tun: false,
            #[cfg(target_os = "macos")]
            mac_tun: None,
            #[cfg(target_os = "macos")]
            retained_startup_proxy: keep_proxy,
            logs,
            readers: vec![],
            log_pump: None,
            last_totals: None,
            version: String::new(),
            node_root: Mutex::new(None),
            protection_guard: None,
            _instance_lock: Some(instance),
            remote: None,
            remote_running: false,
            control_server: None,
            external_logs_until: None,
            view_revision: 0,
        })
    }

    pub fn running(&self) -> bool {
        if self.is_remote() {
            self.remote_running
        } else {
            self.child.is_some()
        }
    }
    fn journal(&self) -> PathBuf {
        self.store.root.join("system-proxy.json")
    }
    fn core(&self) -> Result<PathBuf> {
        assets::core_path(&self.resources)
    }
    fn command(&self) -> Result<Command> {
        #[cfg(windows)]
        if self.windows_tun {
            return platform::windows_service::command();
        }
        #[cfg(target_os = "macos")]
        if let Some(session) = &self.mac_tun {
            let mut command =
                session.command(self.settings.controller_port, self.settings.mixed_port)?;
            command.kill_on_drop(true);
            return Ok(command);
        }
        let core = self.core()?;
        #[cfg(target_os = "linux")]
        let executable = platform::core_launcher(&core);
        #[cfg(not(target_os = "linux"))]
        let executable = &core;
        #[cfg(target_os = "macos")]
        let mut command = {
            // Locate the driver alongside these resources, not alongside an
            // integration test executable (which may have a same-named harness).
            let parent = self.resources.parent().context("无法定位内核启动器")?;
            let launcher = if parent.file_name() == Some(std::ffi::OsStr::new("Resources"))
                && parent.parent().and_then(|path| path.file_name())
                    == Some(std::ffi::OsStr::new("Contents"))
            {
                parent.parent().unwrap().join("MacOS/clash-tun-launcher")
            } else {
                parent.join("clash-tun-launcher")
            };
            if launcher.is_file() {
                let mut command = Command::new(launcher);
                command.arg("--macos-user-core").arg(&core);
                command
            } else {
                Command::new(executable)
            }
        };
        #[cfg(not(target_os = "macos"))]
        let mut command = Command::new(executable);
        command.kill_on_drop(!cfg!(target_os = "macos"));
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        Ok(command)
    }
    fn validation_command(&self) -> Result<Command> {
        // Configuration checks do not create TUN interfaces. Keep them away
        // from privilege launchers on every platform, including root CLI use.
        let mut command = Command::new(self.core()?);
        command.kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        Ok(command)
    }
    async fn validate(&self, payload: &str) -> Result<()> {
        self.validate_in(payload, &self.store.runtime()).await
    }

    async fn validate_in(&self, payload: &str, directory: &std::path::Path) -> Result<()> {
        let candidate = directory.join("candidate.yaml");
        atomic_write(&candidate, payload.as_bytes())?;
        let mut child = self
            .validation_command()?
            .arg("-t")
            .arg("-d")
            .arg(directory)
            .arg("-f")
            .arg(&candidate)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("无法启动内核校验程序：{}", self.resources.display()))?;
        let stdout = child.stdout.take().context("无法读取校验输出")?;
        let stderr = child.stderr.take().context("无法读取校验错误输出")?;
        let (status, stdout, stderr) = tokio::time::timeout(Duration::from_secs(30), async {
            tokio::try_join!(child.wait(), capture_output(stdout), capture_output(stderr))
        })
        .await
        .context("内核配置校验超时")??;
        if !status.success() {
            let error = format!(
                "{}\n{}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            )
            .replace(&self.settings.secret, "[密钥]");
            // Keep separate records so a fatal line after startup information
            // is not lost to the per-record log limit. Return a bounded tail
            // directly to callers, including a CLI with no log page.
            for line in error.lines().filter(|line| !line.trim().is_empty()) {
                append(&self.logs, format!("[配置校验] {line}"));
            }
            let detail = validation_error_detail(&error);
            bail!("mihomo 配置校验失败（{status}）：\n{detail}");
        }
        Ok(())
    }
    async fn healthy(&self) -> Result<String> {
        let version: Value = self.api.get("version").await?;
        Ok(version["version"].as_str().unwrap_or("未知").to_owned())
    }
    pub async fn start(&mut self) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::Start).await?;
            return Ok(());
        }
        #[cfg(windows)]
        if !self.running()
            && (self.settings.proxy_mode == ProxyMode::Tun
                || self.settings.claude_protection.is_some())
        {
            let interactive = !std::env::args_os().any(|arg| arg == "--background");
            self.ensure_windows_service(interactive).await?;
            self.windows_tun = true;
        }

        let result = self.start_core().await;
        #[cfg(target_os = "macos")]
        if std::mem::take(&mut self.retained_startup_proxy) {
            return match result {
                Err(error) => match platform::restore(&self.journal()) {
                    Ok(_) => Err(error),
                    Err(restore) => {
                        Err(error.context(format!("启动失败后恢复系统代理也失败：{restore:#}")))
                    }
                },
                Ok(()) => Ok(()),
            };
        }
        result
    }

    async fn start_core(&mut self) -> Result<()> {
        if self.running() {
            bail!("内核已启动");
        }
        #[cfg(target_os = "macos")]
        if self
            .mac_tun
            .as_ref()
            .is_some_and(|session| !session.is_available())
        {
            self.mac_tun = None;
        }
        let controller = crate::config::reserve_controller(
            self.settings.controller_port,
            self.settings.mixed_port,
        )?;
        let port = controller.local_addr()?.port();
        if port != self.settings.controller_port {
            let mut settings = self.settings.clone();
            settings.controller_port = port;
            let api = Api::new(&settings)?;
            self.store.save_settings(&settings)?;
            self.settings = settings;
            self.api = api;
            self.view_revision = self.view_revision.wrapping_add(1);
            append(&self.logs, "[客户端] 已自动分配可用的内核控制端口");
        }
        {
            let port = self.settings.mixed_port;
            #[cfg(target_os = "macos")]
            if self.mac_tun.is_none() {
                // Old updater versions can relaunch before their parent-watch
                // helper finishes its bounded 2s core shutdown. Give that
                // handoff 3s; ordinary starts still reject listeners at once.
                let grace = if std::env::args_os().any(|arg| arg == "--update-result") {
                    Duration::from_secs(3)
                } else {
                    Duration::ZERO
                };
                wait_for_mac_port(port, &self.logs, grace)
                    .await
                    .with_context(|| format!("无法绑定端口 {port}，请检查占用或系统端口限制"))?;
            }
            #[cfg(not(target_os = "macos"))]
            let _listener = std::net::TcpListener::bind(("127.0.0.1", port))
                .with_context(|| format!("无法绑定端口 {port}，请检查占用或系统端口限制"))?;
        }
        let id = self
            .settings
            .active_profile
            .as_ref()
            .context("请先导入并选择一个订阅配置")?;
        let raw = read_profile(&self.store.profile_path(id)?)?;
        let payload = self.profile_payload(&raw, &self.settings, id)?;
        drop(raw);
        self.validate(&payload).await?;
        let path = self.store.runtime().join("config.yaml");
        atomic_write(&path, payload.as_bytes())?;
        self.node_root.lock().expect("header route lock").take();
        drop(payload);
        drop(controller);
        let mut child = self
            .command()?
            .arg("-d")
            .arg(self.store.runtime())
            .arg("-f")
            .arg(path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("无法启动内核程序：{}", self.resources.display()))?;
        if let Some(stdout) = child.stdout.take() {
            let logs = self.logs.clone();
            self.readers
                .push(tokio::spawn(read_core_logs(stdout, logs)));
        }
        if let Some(stderr) = child.stderr.take() {
            let logs = self.logs.clone();
            self.readers
                .push(tokio::spawn(read_core_logs(stderr, logs)));
        }
        self.child = Some(child);
        for _ in 0..30 {
            if let Some(status) = self.child.as_mut().unwrap().try_wait()? {
                self.child = None;
                for reader in self.readers.drain(..) {
                    reader.abort();
                }
                bail!("内核启动后退出：{status}，请查看日志");
            }
            // mihomo starts its controller before applying rules and listeners.
            // A responsive /version alone does not mean the profile is ready.
            let ready = async {
                let version = self.healthy().await?;
                let config: Value = self.api.get("configs").await?;
                if config["mixed-port"].as_u64() != Some(u64::from(self.settings.mixed_port)) {
                    bail!("内核代理端口仍在初始化");
                }
                Ok::<_, anyhow::Error>(version)
            };
            if let Ok(Ok(version)) = tokio::time::timeout(Duration::from_millis(400), ready).await {
                self.version = version;
                append(&self.logs, "[客户端] mihomo 已启动，控制接口可用");
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        self.stop().await?;
        bail!("内核未通过启动健康检查，请查看日志");
    }

    pub async fn stop(&mut self) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::Stop).await?;
            return Ok(());
        }
        platform::restore(&self.journal()).context("恢复系统代理失败，暂不停止内核以避免断网")?;
        self.stop_process().await
    }

    async fn stop_process(&mut self) -> Result<()> {
        self.protection_guard = None;
        #[cfg(target_os = "macos")]
        self.mac_protection_dns(false).await?;
        if self.running()
            && let Ok(config) = self.api.get::<Value>("configs").await
            && config["tun"]["enable"].as_bool() == Some(true)
        {
            self.tun(false).await.context("停止内核前关闭 TUN 失败")?;
        }
        self.external_logs_until = None;
        self.set_logs(false);
        if let Some(mut child) = self.child.take() {
            if child.try_wait()?.is_none() {
                #[cfg(target_os = "macos")]
                if let Some(pid) = child.id() {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }
                #[cfg(target_os = "macos")]
                if tokio::time::timeout(Duration::from_secs(3), child.wait())
                    .await
                    .is_err()
                {
                    child.kill().await?;
                }
                #[cfg(not(target_os = "macos"))]
                child.kill().await?;
            }
            child.wait().await?;
            #[cfg(target_os = "macos")]
            if let Some(session) = &self.mac_tun {
                session.wait_idle()?;
            }
        }
        #[cfg(windows)]
        if self.windows_tun {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                if std::net::TcpListener::bind(("127.0.0.1", self.settings.controller_port)).is_ok()
                    && std::net::TcpListener::bind(("127.0.0.1", self.settings.mixed_port)).is_ok()
                {
                    break;
                }
                ensure!(
                    tokio::time::Instant::now() < deadline,
                    "服务内核退出后端口仍被占用，请稍后重试"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        for reader in self.readers.drain(..) {
            reader.abort();
        }
        self.last_totals = None;
        self.version.clear();
        append(&self.logs, "[客户端] 内核已停止");
        Ok(())
    }

    pub async fn save_ports(&mut self, mixed_port: u16) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::Ports { mixed: mixed_port })
                .await?;
            return Ok(());
        }
        let controller = if mixed_port == self.settings.controller_port {
            Some(crate::config::reserve_controller(0, mixed_port)?)
        } else {
            None
        };
        let controller_port = match &controller {
            Some(listener) => listener.local_addr()?.port(),
            None => self.settings.controller_port,
        };
        drop(controller);
        self.save_settings(Settings {
            controller_port,
            mixed_port,
            ..self.settings.clone()
        })
        .await
    }

    fn profile_payload(&self, raw: &str, settings: &Settings, id: &str) -> Result<String> {
        let builtin = self.profiles.iter().any(|p| p.id == id && p.is_default());
        crate::config::runtime_config_with_rules(
            raw,
            settings,
            settings.rule_profiles.get(id).map(AsRef::as_ref),
            builtin,
        )
    }

    pub fn rule_document(&self, id: &str) -> Result<crate::rule_manager::Document> {
        if let Some(remote) = &self.remote {
            return remote.call(ControlCommand::RuleDocument { id: id.into() });
        }
        let profile = self
            .profiles
            .iter()
            .find(|p| p.id == id)
            .context("订阅不存在")?;
        let raw = read_profile(&self.store.profile_path(id)?)?;
        crate::rule_manager::Document::from_raw(id, profile.is_default(), &raw)
    }

    pub async fn save_profile_rules(
        &mut self,
        id: &str,
        rules: crate::rule_manager::ProfileRules,
    ) -> Result<()> {
        self.save_profile_rules_checked(id, rules, None).await
    }

    pub async fn save_profile_rules_checked(
        &mut self,
        id: &str,
        rules: crate::rule_manager::ProfileRules,
        expected: Option<String>,
    ) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::SaveRules {
                id: id.into(),
                rules: rules.clone(),
                expected,
            })
            .await?;
            return Ok(());
        }
        self.require_unprotected_profile(id)?;
        if let Some(expected) = expected {
            let builtin = self.profiles.iter().any(|p| p.id == id && p.is_default());
            let current =
                crate::rule_manager::ProfileRules::for_profile(&self.settings, id, builtin);
            ensure!(
                crate::rule_manager::revision(&current)? == expected,
                "规则已被另一客户端修改，草稿已保留；请刷新并检查修改后重新应用"
            );
        }
        self.profiles
            .iter()
            .find(|p| p.id == id)
            .context("订阅不存在")?;
        rules.validate()?;
        let mut settings = self.settings.clone();
        settings
            .rule_profiles
            .insert(id.to_owned(), Arc::new(rules));
        settings.validate()?;
        let raw = read_profile(&self.store.profile_path(id)?)?;
        let payload = self.profile_payload(&raw, &settings, id)?;
        drop(raw);
        let mut transaction = crate::profile_transaction::Transaction::new(&self.store.root)?;
        transaction.stage(
            &self.store.root.join("settings.json"),
            &serde_json::to_vec_pretty(&settings)?,
        )?;
        let active = self.settings.active_profile.as_deref() == Some(id);
        self.commit_profile(transaction, &payload, active).await?;
        self.settings = settings;
        append(&self.logs, "[客户端] 规则已校验并保存；原始订阅文件保留");
        Ok(())
    }

    pub async fn save_rule_overrides(&mut self, rule_overrides: RuleOverrides) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::RuleOverrides {
                overrides: rule_overrides,
            })
            .await?;
            return Ok(());
        }
        if self.settings.claude_protection.is_some() {
            bail!("请先关闭 Claude 保护，再修改快捷规则");
        }
        let settings = Settings {
            rule_overrides,
            ..self.settings.clone()
        };
        settings.validate()?;
        let id = settings
            .active_profile
            .as_deref()
            .context("请先选择订阅配置")?;
        let raw = read_profile(&self.store.profile_path(id)?)?;
        let payload = self.profile_payload(&raw, &settings, id)?;
        drop(raw);
        let mut transaction = crate::profile_transaction::Transaction::new(&self.store.root)?;
        transaction.stage(
            &self.store.root.join("settings.json"),
            &serde_json::to_vec_pretty(&settings)?,
        )?;
        self.commit_profile(transaction, &payload, true).await?;
        self.settings = settings;
        append(&self.logs, "[客户端] 订阅附加规则已保存并应用");
        Ok(())
    }

    pub async fn save_settings(&mut self, settings: Settings) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::Settings {
                settings: settings.clone(),
                expected: Some(serde_json::to_value(&self.settings)?),
            })
            .await?;
            return Ok(());
        }
        settings.validate()?;
        if self.settings.claude_protection.is_some()
            && (settings.dns_override != self.settings.dns_override
                || settings.ipv6 != self.settings.ipv6
                || settings.udp != self.settings.udp
                || settings.active_profile != self.settings.active_profile
                || settings.run_mode != self.settings.run_mode
                || settings.proxy_mode != self.settings.proxy_mode)
        {
            bail!("请先关闭 A÷检测中的 Claude 保护，再更改网络设置");
        }
        if settings.controller_port == self.settings.controller_port
            && settings.mixed_port == self.settings.mixed_port
            && settings.secret == self.settings.secret
        {
            if (settings.dns_override != self.settings.dns_override
                || settings.ipv6 != self.settings.ipv6
                || settings.udp != self.settings.udp)
                && let Some(id) = settings.active_profile.as_deref()
            {
                let raw = read_profile(&self.store.profile_path(id)?)?;
                let payload = self.profile_payload(&raw, &settings, id)?;
                drop(raw);
                let mut transaction =
                    crate::profile_transaction::Transaction::new(&self.store.root)?;
                transaction.stage(
                    &self.store.root.join("settings.json"),
                    &serde_json::to_vec_pretty(&settings)?,
                )?;
                self.commit_profile(transaction, &payload, true).await?;
            } else {
                self.store.save_settings(&settings)?;
            }
            self.settings = settings;
            return Ok(());
        }
        // Detect occupied new ports before interrupting the current core.
        let mut reservations = Vec::new();
        for port in [settings.controller_port, settings.mixed_port] {
            if !self.running()
                || ![self.settings.controller_port, self.settings.mixed_port].contains(&port)
            {
                reservations.push(
                    std::net::TcpListener::bind(("127.0.0.1", port))
                        .with_context(|| format!("端口 {port} 已被占用，设置未改变"))?,
                );
            }
        }
        let old = self.settings.clone();
        let state = self.restart_state().await?;
        drop(reservations);
        if state.is_some() {
            self.stop().await?;
        }
        let changed = async {
            self.store.save_settings(&settings)?;
            self.api = Api::new(&settings)?;
            self.settings = settings;
            if let Some(state) = &state {
                self.resume(state).await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = changed {
            self.stop().await.context("设置应用失败，停止新内核失败")?;
            self.store.save_settings(&old)?;
            self.api = Api::new(&old)?;
            self.settings = old;
            if let Some(state) = &state {
                self.resume(state)
                    .await
                    .context("设置已恢复，但旧内核重新启动失败")?;
            }
            return Err(error.context("设置应用失败，已恢复旧端口与运行状态"));
        }
        Ok(())
    }

    async fn restart_state(&self) -> Result<Option<RestartState>> {
        if !self.running() {
            return Ok(None);
        }
        let config: Value = self.api.get("configs").await?;
        let proxies: Proxies = self.api.get("proxies").await?;
        Ok(Some(RestartState {
            mode: config["mode"].as_str().unwrap_or("rule").into(),
            tun: config["tun"]["enable"].as_bool().unwrap_or(false),
            system_proxy: platform::is_owned(&self.journal())?,
            selectors: proxies
                .proxies
                .into_iter()
                .filter(|(_, p)| p.kind == "Selector" && !p.now.is_empty())
                .map(|(name, p)| (name.to_string(), p.now.to_string()))
                .collect(),
        }))
    }

    /// Editing source metadata does not replace the cached YAML. The explicit
    /// Update action downloads and validates the new source transactionally.
    pub fn edit_profile(&mut self, id: &str, name: &str, source: &str) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate_sync(ControlCommand::EditProfile {
                id: id.into(),
                name: name.into(),
                source: source.into(),
            })?;
            return Ok(());
        }
        let name = name.trim();
        let source = source.trim();
        if name.is_empty() || source.is_empty() {
            bail!("配置名称与来源不能为空");
        }
        if source.starts_with("http://") || source.starts_with("https://") {
            reqwest::Url::parse(source).context("订阅 URL 无效")?;
        } else if source.contains("://") {
            bail!("订阅链接仅支持 HTTP 或 HTTPS");
        }
        let mut profiles = self.profiles.clone();
        let profile = profiles
            .iter_mut()
            .find(|p| p.id == id)
            .context("订阅已不存在")?;
        if profile.is_default() {
            bail!("默认直连配置不能编辑");
        }
        profile.name = name.to_owned();
        profile.source = source.to_owned();
        self.store.save_profiles(&profiles)?;
        self.profiles = profiles;
        Ok(())
    }

    fn default_route(&self) -> Result<String> {
        if let Some(root) = self.node_root.lock().expect("header route lock").clone() {
            return Ok(root);
        }
        let raw = read_runtime_config(&self.store.runtime().join("config.yaml"))?;
        let root = crate::active_route::root(&raw)?;
        *self.node_root.lock().expect("header route lock") = Some(root.clone());
        Ok(root)
    }

    /// Retain only one selectable decision group for the native menu. Fetching
    /// the full proxies response every second would waste memory while hidden.
    pub async fn tray_nodes(&self) -> Result<Option<crate::tray_model::Nodes>> {
        if let Some(remote) = &self.remote {
            return remote.request(ControlCommand::TrayNodes).await;
        }
        if !self.running() {
            return Ok(None);
        }
        let config: ConfigStatus = self.api.get("configs").await?;
        let root = self.default_route()?;
        let mut name = crate::active_route::start(&config.mode, &root).to_owned();
        let mut visited = std::collections::BTreeSet::new();
        for _ in 0..crate::active_route::MAX_DEPTH {
            if crate::active_route::terminal(&name) || !visited.insert(name.clone()) {
                return Ok(None);
            }
            let proxy = self.api.proxy_nodes(&name).await?;
            if proxy.kind == "Selector" && !proxy.all.is_empty() {
                let total = proxy.all.len();
                return Ok(Some(crate::tray_model::Nodes {
                    group: name,
                    selected: proxy.now.to_string(),
                    names: proxy
                        .all
                        .into_iter()
                        .take(200)
                        .map(|n| n.to_string())
                        .collect(),
                    total,
                    locked: self.settings.claude_protection.is_some(),
                }));
            }
            if proxy.now.is_empty() {
                return Ok(None);
            }
            name = proxy.now.to_string();
        }
        Ok(None)
    }

    pub async fn import(
        &mut self,
        name: String,
        source: String,
        existing: Option<String>,
    ) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::Import {
                name: name.clone(),
                source: source.clone(),
                id: existing.clone(),
            })
            .await?;
            return Ok(());
        }
        if let Some(id) = existing.as_deref() {
            self.require_unprotected_profile(id)?;
        }
        if source.trim().is_empty() {
            bail!("请输入订阅 URL 或本地 YAML 路径");
        }
        let (raw, usage) = if source.starts_with("https://") || source.starts_with("http://") {
            let url = reqwest::Url::parse(&source).context("订阅 URL 无效")?;
            crate::subscription::download(&url, self.running().then_some(self.settings.mixed_port))
                .await?
        } else {
            (read_profile(std::path::Path::new(&source))?, None)
        };
        let id = existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let path = self.store.profile_path(&id)?;
        let payload = self.profile_payload(&raw, &self.settings, &id)?;
        let active = self.settings.active_profile.as_ref() == Some(&id);
        let profile = Profile {
            id: id.clone(),
            name: if name.trim().is_empty() {
                "未命名订阅".into()
            } else {
                name
            },
            source,
            usage,
            updated: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        };
        let mut profiles = self.profiles.clone();
        if let Some(old) = profiles.iter_mut().find(|p| p.id == id) {
            *old = profile;
        } else {
            profiles.push(profile);
        }
        let mut settings = self.settings.clone();
        if settings.active_profile.is_none() {
            settings.active_profile = Some(id);
        }
        let mut transaction = crate::profile_transaction::Transaction::new(&self.store.root)?;
        transaction.stage(&path, raw.as_bytes())?;
        transaction.stage(
            &self.store.root.join("profiles.json"),
            &serde_json::to_vec_pretty(&profiles)?,
        )?;
        transaction.stage(
            &self.store.root.join("settings.json"),
            &serde_json::to_vec_pretty(&settings)?,
        )?;
        drop(raw);
        self.commit_profile(transaction, &payload, active).await?;
        self.profiles = profiles;
        self.settings = settings;
        append(&self.logs, "[客户端] 配置已导入/更新");
        Ok(())
    }

    async fn commit_profile(
        &self,
        mut transaction: crate::profile_transaction::Transaction,
        payload: &str,
        active: bool,
    ) -> Result<()> {
        let running = active && self.running();
        let (previous, next) = if running {
            let path = self.store.runtime().join("config.yaml");
            let current: ConfigStatus = self.api.get("configs").await?;
            let preserved = |raw: &str| -> Result<String> {
                let mut value: serde_yaml::Value = serde_yaml::from_str(raw)?;
                value["mode"] = serde_yaml::Value::from(current.mode.clone());
                value["tun"]["enable"] = serde_yaml::Value::from(current.tun.enable);
                Ok(serde_yaml::to_string(&value)?)
            };
            let previous = preserved(&read_runtime_config(&path)?)?;
            let next = preserved(payload)?;
            self.validate(&next).await?;
            transaction.stage(&path, next.as_bytes())?;
            (Some(previous), Some(next))
        } else {
            self.validate(payload).await?;
            (None, None)
        };
        transaction.prepare()?;
        let changed = async {
            if let Some(next) = &next {
                self.api.reload(next).await?;
                self.healthy().await?;
            }
            transaction.commit()?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = changed {
            // Always attempt both recoveries, even if the controller is unreachable.
            let runtime = if let Some(previous) = previous {
                async {
                    self.api.reload(&previous).await?;
                    self.healthy().await?;
                    Ok::<_, anyhow::Error>(())
                }
                .await
            } else {
                Ok(())
            };
            let disk = transaction.rollback();
            match (runtime, disk) {
                (Ok(()), Ok(())) => {
                    return Err(error.context("配置提交失败，已恢复旧配置与订阅信息"));
                }
                (runtime, disk) => bail!(
                    "配置提交失败：{error}；运行配置恢复：{}；文件恢复：{}",
                    runtime.err().map_or("成功".into(), |e| e.to_string()),
                    disk.err().map_or("成功".into(), |e| e.to_string())
                ),
            }
        }
        if active {
            self.node_root.lock().expect("header route lock").take();
        }
        Ok(())
    }

    pub async fn activate(&mut self, id: String) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::Activate { id: id.clone() })
                .await?;
            return Ok(());
        }
        if self.settings.claude_protection.is_some() {
            bail!("请先关闭 A÷检测中的 Claude 保护，再切换订阅");
        }
        if !self.profiles.iter().any(|p| p.id == id) {
            bail!("配置不存在");
        }
        let raw = read_profile(&self.store.profile_path(&id)?)?;
        let payload = self.profile_payload(&raw, &self.settings, &id)?;
        let settings = Settings {
            active_profile: Some(id),
            ..self.settings.clone()
        };
        let mut transaction = crate::profile_transaction::Transaction::new(&self.store.root)?;
        transaction.stage(
            &self.store.root.join("settings.json"),
            &serde_json::to_vec_pretty(&settings)?,
        )?;
        self.commit_profile(transaction, &payload, true).await?;
        self.settings = settings;
        Ok(())
    }

    pub async fn update_geo(&mut self) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::GeoUpdate).await?;
            return Ok(());
        }
        let runtime = self.store.runtime();
        let staged = tempfile::Builder::new()
            .prefix(".geo-download-")
            .tempdir_in(&runtime)?;
        append(
            &self.logs,
            "[客户端] 开始下载全部 Geo 数据，下载期间继续使用原数据",
        );
        assets::download_geo(
            staged.path(),
            self.running().then_some(self.settings.mixed_port),
        )
        .await?;
        self.install_geo(staged.path()).await
    }

    async fn resume(&mut self, state: &RestartState) -> Result<()> {
        self.start().await?;
        self.mode(&state.mode).await?;
        for (group, node) in &state.selectors {
            self.api.select(group, node).await?;
        }
        if state.tun {
            self.tun(true).await?;
            #[cfg(target_os = "macos")]
            self.mac_protection_dns(self.settings.claude_protection.is_some())
                .await?;
        }
        if state.system_proxy && !platform::is_owned(&self.journal())? {
            self.system_proxy(true).await?;
        }
        if state.tun {
            self.start_protection_guard();
        }
        Ok(())
    }

    fn require_unprotected_profile(&self, id: &str) -> Result<()> {
        if self
            .settings
            .claude_protection
            .as_ref()
            .is_some_and(|p| p.profile_id == id)
        {
            bail!("请先关闭 A÷检测中的 Claude 保护，再修改正在使用的订阅与规则");
        }
        Ok(())
    }
    fn start_protection_guard(&mut self) {
        if let Some(protection) = &self.settings.claude_protection
            && self.running()
            && self.protection_guard.is_none()
        {
            self.protection_guard = Some(crate::claude_check::guard::Guard::start(
                self.api.clone(),
                protection.node.clone(),
                |title, message| async move { platform::notification::show(&title, &message).await },
            ));
        }
    }

    async fn prepare_protection_tun(&mut self, interactive: bool) -> Result<()> {
        #[cfg(windows)]
        self.prepare_windows_tun(interactive).await?;
        #[cfg(target_os = "linux")]
        self.prepare_linux_tun(interactive).await?;
        #[cfg(target_os = "macos")]
        self.prepare_mac_tun(interactive).await?;
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        let _ = interactive;
        Ok(())
    }

    /// The GUI, restart and disposable native tests all use this operation.
    /// A successful Windows UAC handoff returns true so the old GUI can exit.
    pub async fn claude_protection(&mut self, enabled: bool) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::Protect { enabled })
                .await?;
            return Ok(());
        }
        use crate::claude_check::protection::{Previous, Protection};
        if enabled == self.settings.claude_protection.is_some() {
            return Ok(());
        }
        if !enabled {
            self.apply_claude_protection(None, None).await?;
            return Ok(());
        }
        let previous = self
            .restart_state()
            .await?
            .context("请先启动内核并选择代理节点")?;
        let mode = if previous.mode == "direct" {
            "rule"
        } else {
            &previous.mode
        };
        let root = self.default_route()?;
        let mut node = crate::active_route::start(mode, &root).to_owned();
        let mut visited = std::collections::BTreeSet::new();
        let mut provider = None;
        let mut route_groups = std::collections::BTreeMap::new();
        loop {
            ensure!(
                mode != "global" || node != "DIRECT",
                "当前全局出口为 DIRECT，请先选择代理节点再开启 Claude 保护"
            );
            if crate::active_route::terminal(&node)
                || visited.len() >= crate::active_route::MAX_DEPTH
                || !visited.insert(node.clone())
            {
                bail!("请先选择具体代理节点；Claude 保护不能使用直连或未确定节点的代理组");
            }
            match self.api.proxy_nodes(&node).await {
                Ok(proxy) if !proxy.now.is_empty() => {
                    if matches!(proxy.kind.as_str(), "URLTest" | "Fallback" | "LoadBalance") {
                        route_groups.insert(node.clone(), proxy.now.to_string());
                    }
                    node = proxy.now.to_string();
                }
                Ok(proxy) => {
                    ensure!(
                        proxy.all.is_empty()
                            && !matches!(
                                proxy.kind.as_str(),
                                "" | "Direct"
                                    | "Reject"
                                    | "Pass"
                                    | "Compatible"
                                    | "Selector"
                                    | "URLTest"
                                    | "Fallback"
                                    | "LoadBalance"
                            ),
                        "请选择具体代理节点"
                    );
                    break;
                }
                Err(_) => {
                    let (source, kind) = self
                        .api
                        .provider_leaf(&node)
                        .await?
                        .context("无法确定当前代理节点，请先选择节点")?;
                    ensure!(
                        !matches!(
                            kind.as_str(),
                            "Direct"
                                | "Reject"
                                | "Pass"
                                | "Selector"
                                | "URLTest"
                                | "Fallback"
                                | "LoadBalance"
                        ),
                        "请选择具体代理节点"
                    );
                    provider = Some(source);
                    break;
                }
            }
        }
        let mut protection = Protection::new(
            self.settings
                .active_profile
                .clone()
                .context("请先选择订阅")?,
            node,
            Previous {
                mode: previous.mode.clone(),
                proxy_mode: if previous.tun {
                    ProxyMode::Tun
                } else if previous.system_proxy {
                    ProxyMode::System
                } else {
                    ProxyMode::Off
                },
                selectors: previous.selectors.clone(),
            },
        );
        protection.provider = provider;
        protection.capture_automatic_groups(&self.api.get("proxies").await?);
        protection.locked_groups.extend(route_groups);
        #[cfg(target_os = "macos")]
        {
            protection.system_dns = platform::macos::dns::system_servers()?;
        }
        protection.validate()?;
        self.apply_claude_protection(Some(Arc::new(protection)), None)
            .await?;
        Ok(())
    }

    async fn apply_claude_protection(
        &mut self,
        protection: Option<Arc<crate::claude_check::protection::Protection>>,
        recovery: Option<RestartState>,
    ) -> Result<()> {
        let old = self.settings.clone();
        let previous = recovery.or(self.restart_state().await?);
        let candidate = Settings {
            claude_protection: protection.clone(),
            ..old.clone()
        };
        candidate.validate()?;
        let id = candidate
            .active_profile
            .as_deref()
            .context("请先选择订阅")?;
        let raw = read_profile(&self.store.profile_path(id)?)?;
        let mut next: serde_yaml::Value =
            serde_yaml::from_str(&self.profile_payload(&raw, &candidate, id)?)?;
        drop(raw);
        let desired = if let Some(protection) = &protection {
            let mut selectors = previous
                .as_ref()
                .map_or_else(Vec::new, |s| s.selectors.clone());
            if protection.run_mode() == "global" {
                selectors.retain(|(group, _)| group != "GLOBAL");
                selectors.push(("GLOBAL".into(), protection.node.clone()));
            }
            RestartState {
                mode: protection.run_mode().into(),
                tun: true,
                system_proxy: false,
                selectors,
            }
        } else {
            let saved = &old
                .claude_protection
                .as_ref()
                .context("Claude 保护恢复记录缺失")?
                .previous;
            RestartState {
                mode: saved.mode.clone(),
                tun: saved.proxy_mode == ProxyMode::Tun,
                system_proxy: saved.proxy_mode == ProxyMode::System,
                selectors: saved.selectors.clone(),
            }
        };
        next["mode"] = desired.mode.clone().into();
        next["tun"]["enable"] = serde_yaml::Value::from(self.running() && desired.tun);
        let next = serde_yaml::to_string(&next)?;
        self.validate(&next).await?;
        // Authorization is requested before the durable transaction is opened.
        // Cancellation therefore cannot leave an enabled preference behind.
        if protection.is_some() {
            self.prepare_protection_tun(true).await?;
        }
        #[cfg(windows)]
        if protection.is_none() && self.running() && desired.tun {
            self.prepare_windows_tun(true).await?;
        }
        #[cfg(target_os = "macos")]
        if protection.is_none() && self.running() {
            // This is an explicit user action, even in a login-started GUI.
            // A saved protection can outlive the old network helper after an
            // upgrade; repair it before restoring the user's original mode.
            if desired.tun {
                self.prepare_mac_tun(true).await?;
            } else if desired.system_proxy {
                self.prepare_mac_system(true).await?;
            }
        }
        let path = self.store.runtime().join("config.yaml");
        let mut original: serde_yaml::Value = serde_yaml::from_str(&read_runtime_config(&path)?)?;
        if let Some(previous) = &previous {
            original["mode"] = previous.mode.clone().into();
            original["tun"]["enable"] = previous.tun.into();
        }
        let original = serde_yaml::to_string(&original)?;
        let mut transaction = crate::profile_transaction::Transaction::new(&self.store.root)?;
        transaction.stage(
            &self.store.root.join("settings.json"),
            &serde_json::to_vec_pretty(&candidate)?,
        )?;
        transaction.stage(&path, next.as_bytes())?;
        // An interrupted enable recovers the old preferences; an interrupted
        // disable recovers the protection and its *original* restoration record.
        transaction.prepare()?;
        self.protection_guard = None;
        let changed = async {
            if self.running() {
                self.apply_protection_runtime(&next, &desired, protection.is_some())
                    .await?;
                if protection.is_some() {
                    // /version and tun.enable do not prove network availability.
                    // Use the core's actual upstream DNS path (not fake-IP or the
                    // host resolver) before committing a full-route takeover.
                    self.api
                        .wait_for_dns("www.cloudflare.com", Duration::from_secs(30))
                        .await
                        .context("保护节点或加密 DNS 无法连通，请检查节点和节点 DNS 配置")?;
                } else if let Some(saved) = &old.claude_protection {
                    self.api
                        .restore_automatic_groups(&saved.locked_groups, &saved.fixed_groups)
                        .await?;
                }
            } else {
                #[cfg(target_os = "macos")]
                self.mac_protection_dns(false).await?;
                platform::restore(&self.journal())?;
            }
            transaction.commit()?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = changed {
            let runtime = async {
                if let Some(previous) = &previous {
                    self.apply_protection_runtime(
                        &original,
                        previous,
                        old.claude_protection.is_some(),
                    )
                    .await?;
                    if old.claude_protection.is_none()
                        && let Some(saved) = &protection
                    {
                        self.api
                            .restore_automatic_groups(&saved.locked_groups, &saved.fixed_groups)
                            .await?;
                    }
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            let disk = transaction.rollback();
            self.start_protection_guard();
            return Err(error.context(format!(
                "Claude 保护切换失败；运行状态恢复：{}；配置恢复：{}",
                runtime.err().map_or("成功".into(), |e| e.to_string()),
                disk.err().map_or("成功".into(), |e| e.to_string())
            )));
        }
        self.settings = candidate;
        self.start_protection_guard();
        self.node_root.lock().expect("header route lock").take();
        append(
            &self.logs,
            if protection.is_some() {
                "[客户端] Claude 保护已启用：TUN、模式与节点锁定、代理 DoH；原设置恢复记录已保存"
            } else {
                "[客户端] Claude 保护已关闭，原代理模式、DNS 与规则已恢复"
            },
        );
        Ok(())
    }

    async fn apply_protection_runtime(
        &self,
        payload: &str,
        state: &RestartState,
        protected: bool,
    ) -> Result<()> {
        #[cfg(target_os = "macos")]
        self.mac_protection_dns(false).await?;
        #[cfg(not(target_os = "macos"))]
        let _ = protected;
        self.proxy_mode(ProxyMode::Off).await?;
        self.api.reload(payload).await?;
        self.healthy().await?;
        for (group, node) in &state.selectors {
            self.api.select(group, node).await?;
        }
        self.mode(&state.mode).await?;
        self.proxy_mode(if state.tun {
            ProxyMode::Tun
        } else if state.system_proxy {
            ProxyMode::System
        } else {
            ProxyMode::Off
        })
        .await?;
        #[cfg(target_os = "macos")]
        self.mac_protection_dns(protected && state.tun).await?;
        Ok(())
    }

    pub async fn delete_profile(&mut self, id: &str) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::DeleteProfile { id: id.into() })
                .await?;
            return Ok(());
        }
        self.require_unprotected_profile(id)?;
        let profile = self
            .profiles
            .iter()
            .find(|p| p.id == id)
            .context("订阅不存在")?;
        if profile.is_default() {
            bail!("默认直连配置不能删除");
        }
        let active = self.settings.active_profile.as_deref() == Some(id);
        let profiles: Vec<_> = self
            .profiles
            .iter()
            .filter(|p| p.id != id)
            .cloned()
            .collect();
        let mut settings = self.settings.clone();
        settings.rule_profiles.remove(id);
        let payload = if active {
            let default = self
                .profiles
                .iter()
                .find(|p| p.is_default())
                .context("默认配置缺失")?;
            let raw = read_profile(&self.store.profile_path(&default.id)?)?;
            settings.active_profile = Some(default.id.clone());
            Some(self.profile_payload(&raw, &settings, &default.id)?)
        } else {
            None
        };
        let mut transaction = crate::profile_transaction::Transaction::new(&self.store.root)?;
        transaction.stage_remove(&self.store.profile_path(id)?)?;
        transaction.stage(
            &self.store.root.join("profiles.json"),
            &serde_json::to_vec_pretty(&profiles)?,
        )?;
        transaction.stage(
            &self.store.root.join("settings.json"),
            &serde_json::to_vec_pretty(&settings)?,
        )?;
        if let Some(payload) = payload {
            self.commit_profile(transaction, &payload, true).await?;
        } else {
            transaction.prepare()?;
            if let Err(error) = transaction.commit() {
                transaction
                    .rollback()
                    .context("订阅删除失败，恢复旧文件失败")?;
                return Err(error.context("订阅删除失败，已恢复旧文件"));
            }
        }
        self.settings = settings;
        self.profiles = profiles;
        Ok(())
    }

    pub async fn install_geo(&mut self, staged: &std::path::Path) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::GeoInstall {
                path: staged.to_owned(),
            })
            .await?;
            return Ok(());
        }
        let runtime = self.store.runtime();
        let manifest = assets::verify_geo(staged)?;
        let geo_settings = Settings {
            rule_overrides: RuleOverrides::default(),
            claude_protection: None,
            ..self.settings.clone()
        };
        // Ask the actual core to read DAT, MMDB and ASN data before replacing anything.
        for mode in [true, false] {
            let raw = format!(
                "proxies: []\ngeodata-mode: {mode}\nrules:\n  - GEOSITE,cn,DIRECT\n  - GEOIP,CN,DIRECT\n  - IP-ASN,13335,DIRECT\n  - MATCH,DIRECT\n"
            );
            let payload = runtime_config(&raw, &geo_settings)?;
            self.validate_in(&payload, staged).await?;
        }
        let state = if self.running() {
            let config: Value = self.api.get("configs").await?;
            let proxies: Proxies = self.api.get("proxies").await?;
            Some(RestartState {
                mode: config["mode"].as_str().unwrap_or("rule").into(),
                tun: config["tun"]["enable"].as_bool().unwrap_or(false),
                system_proxy: platform::is_owned(&self.journal())?,
                selectors: proxies
                    .proxies
                    .into_iter()
                    .filter(|(_, p)| p.kind == "Selector" && !p.now.is_empty())
                    .map(|(name, p)| (name.to_string(), p.now.to_string()))
                    .collect(),
            })
        } else {
            None
        };
        // MMDB uses mmap and Windows may deny replacing open database files.
        // Stop only after all downloads and validation, then restart with live state.
        if state.is_some() {
            self.stop_process().await?;
        }
        let result = async {
            assets::begin_install(staged, &runtime)?;
            if let Some(state) = &state {
                self.resume(state).await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            if self.running() {
                self.stop_process().await?;
            }
            assets::recover(&runtime)?;
            if let Some(state) = &state
                && let Err(restart) = self.resume(state).await
            {
                platform::restore(&self.journal())?;
                bail!("Geo 文件已恢复，但内核重新启动失败：{restart}");
            }
            return Err(error.context("Geo 更新失败，已恢复旧数据"));
        }
        assets::commit(&runtime)?;
        self.geo_manifest = manifest;
        append(&self.logs, "[客户端] 全部 Geo 数据更新完成并通过校验");
        Ok(())
    }

    // Only explicit selections persist preferences. Cleanup and temporary restarts
    // use the lower-level mode/proxy_mode operations without clearing them.
    fn stage_mode_settings(
        &self,
        settings: &Settings,
    ) -> Result<crate::profile_transaction::Transaction> {
        settings.validate()?;
        let mut transaction = crate::profile_transaction::Transaction::new(&self.store.root)?;
        transaction.stage(
            &self.store.root.join("settings.json"),
            &serde_json::to_vec_pretty(settings)?,
        )?;
        transaction.prepare()?;
        Ok(transaction)
    }

    pub async fn select_node(&self, group: &str, node: &str) -> Result<()> {
        if let Some(remote) = &self.remote {
            return remote
                .request(ControlCommand::Select {
                    group: group.into(),
                    node: node.into(),
                })
                .await;
        }
        ensure!(
            self.settings.claude_protection.is_none(),
            "Claude 保护已锁定当前节点，请先关闭保护再切换节点"
        );
        self.api.select(group, node).await
    }

    pub async fn select_mode(&mut self, mode: &str) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::Mode { mode: mode.into() })
                .await?;
            return Ok(());
        }
        if self.settings.claude_protection.is_some() {
            bail!("Claude 保护已锁定当前运行模式，请先关闭保护再切换模式");
        }
        let current: ConfigStatus = self.api.get("configs").await?;
        let settings = Settings {
            run_mode: mode.into(),
            ..self.settings.clone()
        };
        let mut transaction = self.stage_mode_settings(&settings)?;
        let result = async {
            self.mode(mode).await?;
            transaction.commit()?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            let runtime = self.mode(&current.mode).await;
            let disk = transaction.rollback();
            return Err(error.context(format!(
                "运行模式保存失败；运行状态恢复：{}；文件恢复：{}",
                runtime.err().map_or("成功".into(), |e| e.to_string()),
                disk.err().map_or("成功".into(), |e| e.to_string())
            )));
        }
        self.settings = settings;
        Ok(())
    }

    pub async fn select_proxy_mode(&mut self, mode: ProxyMode) -> Result<()> {
        if self.remote.is_some() {
            self.remote_mutate(ControlCommand::Proxy { mode }).await?;
            return Ok(());
        }
        if self.settings.claude_protection.is_some() {
            bail!("Claude 保护正在接管 TUN，请先在 A÷检测中关闭保护");
        }
        #[cfg(target_os = "macos")]
        if mode == ProxyMode::System {
            self.prepare_mac_system(true).await?;
        }
        #[cfg(windows)]
        if mode == ProxyMode::Tun {
            self.prepare_windows_tun(true).await?;
        }
        #[cfg(target_os = "linux")]
        if mode == ProxyMode::Tun {
            self.prepare_linux_tun(true).await?;
        }
        #[cfg(target_os = "macos")]
        if mode == ProxyMode::Tun {
            self.prepare_mac_tun(true).await?;
        }
        let previous = crate::proxy::current(self).await?;
        let settings = Settings {
            proxy_mode: mode,
            ..self.settings.clone()
        };
        let mut transaction = self.stage_mode_settings(&settings)?;
        let result = async {
            self.proxy_mode(mode).await?;
            transaction.commit()?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            let runtime = self.proxy_mode(previous).await;
            let disk = transaction.rollback();
            return Err(error.context(format!(
                "代理模式保存失败；运行状态恢复：{}；文件恢复：{}",
                runtime.err().map_or("成功".into(), |e| e.to_string()),
                disk.err().map_or("成功".into(), |e| e.to_string())
            )));
        }
        self.settings = settings;
        Ok(())
    }

    pub async fn restore_proxy_mode(&mut self) -> Result<()> {
        if self.is_remote() {
            return Ok(());
        }
        if let Some(protection) = self.settings.claude_protection.clone() {
            let interactive = !std::env::args_os().any(|arg| arg == "--background");
            self.prepare_protection_tun(interactive).await?;
            self.mode(protection.run_mode()).await?;
            self.proxy_mode(ProxyMode::Tun).await?;
            #[cfg(target_os = "macos")]
            self.mac_protection_dns(true).await?;
            self.start_protection_guard();
            append(
                &self.logs,
                format!(
                    "[客户端] 已恢复 Claude 保护；原状态恢复记录保留，使用节点 {}",
                    protection.node
                ),
            );
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        if self.settings.proxy_mode == ProxyMode::System {
            let interactive = !std::env::args_os().any(|arg| arg == "--background");
            self.prepare_mac_system(interactive).await?;
        }
        #[cfg(windows)]
        if self.settings.proxy_mode == ProxyMode::Tun {
            let interactive = !std::env::args_os().any(|arg| arg == "--background");
            self.prepare_windows_tun(interactive).await?;
        }
        #[cfg(target_os = "linux")]
        if self.settings.proxy_mode == ProxyMode::Tun {
            // Background startup must not repeatedly request administrator access.
            let interactive = !std::env::args_os().any(|arg| arg == "--background");
            self.prepare_linux_tun(interactive).await?;
        }
        #[cfg(target_os = "macos")]
        if self.settings.proxy_mode == ProxyMode::Tun {
            let interactive = !std::env::args_os().any(|arg| arg == "--background");
            self.prepare_mac_tun(interactive).await?;
        }
        self.proxy_mode(self.settings.proxy_mode).await?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    async fn prepare_mac_system(&self, interactive: bool) -> Result<()> {
        let journal = self.journal();
        tokio::task::spawn_blocking(move || {
            platform::macos::service::ensure_available(interactive)?;
            platform::adopt_owned_proxy(&journal)
        })
        .await??;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    async fn prepare_mac_tun(&mut self, interactive: bool) -> Result<()> {
        if self
            .mac_tun
            .as_ref()
            .is_some_and(|session| session.is_available())
        {
            return Ok(());
        }
        self.mac_tun = None;
        tokio::task::spawn_blocking(move || {
            platform::macos::service::ensure_available(interactive)
        })
        .await??;
        let previous = self.restart_state().await?.context("请先启动内核")?;
        let session =
            tokio::task::spawn_blocking(platform::macos::tun::Session::authorize).await??;
        self.stop().await?;
        self.mac_tun = Some(session);
        if let Err(error) = self.resume(&previous).await {
            self.stop_process().await?;
            self.mac_tun = None;
            self.resume(&previous)
                .await
                .context("TUN 授权失败后恢复原内核失败")?;
            return Err(error.context("TUN 授权后内核重启失败，已恢复原模式"));
        }
        Ok(())
    }

    #[cfg(windows)]
    async fn ensure_windows_service(&self, interactive: bool) -> Result<()> {
        let helper = self
            .resources
            .parent()
            .context("无法定位 Windows 网络助手")?
            .join("clash-tun-launcher.exe");
        tokio::task::spawn_blocking(move || {
            platform::windows_service::ensure_available(&helper, interactive)
        })
        .await??;
        Ok(())
    }

    #[cfg(windows)]
    async fn prepare_windows_tun(&mut self, interactive: bool) -> Result<()> {
        if self.windows_tun {
            return Ok(());
        }
        // Complete installation/repair before interrupting the current route.
        self.ensure_windows_service(interactive).await?;
        let previous = self.restart_state().await?.context("请先启动内核")?;
        self.stop().await?;
        self.windows_tun = true;
        if let Err(error) = self.resume(&previous).await {
            self.stop_process().await?;
            self.windows_tun = false;
            self.resume(&previous)
                .await
                .context("服务启动失败后恢复原内核失败")?;
            return Err(error.context("Windows 网络助手启动失败，已恢复原代理模式"));
        }
        append(
            &self.logs,
            "[客户端] Windows 网络服务已接管内核，桌面界面保持普通用户运行",
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    async fn prepare_linux_tun(&mut self, interactive: bool) -> Result<()> {
        platform::check_tun_environment()?;
        let pid = self
            .child
            .as_ref()
            .and_then(Child::id)
            .context("请先启动内核")?;
        if platform::core_has_tun_permissions(pid)? {
            return Ok(());
        }
        if !interactive {
            bail!("TUN 需要网络权限，请打开客户端并点击 TUN 模式完成授权");
        }
        let previous = self.restart_state().await?.context("请先启动内核")?;
        // Authorize before interrupting the old core. Cancellation leaves routing intact.
        let core = self.core()?;
        tokio::task::spawn_blocking(move || platform::authorize_tun(&core)).await??;
        let result = async {
            self.stop().await?;
            self.resume(&previous).await?;
            let pid = self
                .child
                .as_ref()
                .and_then(Child::id)
                .context("内核重启失败")?;
            if !platform::core_has_tun_permissions(pid)? {
                bail!("内核没有获得 TUN 网络权限，请检查文件系统挂载选项与启动环境");
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            let rollback = async {
                self.stop().await?;
                self.resume(&previous).await
            }
            .await;
            return match rollback {
                Ok(()) => Err(error.context("TUN 授权后重启失败，已恢复原代理模式")),
                Err(rollback) => {
                    Err(error.context(format!("TUN 重启失败，恢复原模式也失败：{rollback:#}")))
                }
            };
        }
        append(
            &self.logs,
            "[客户端] 已授权并重启 mihomo，桌面界面保持普通用户运行",
        );
        Ok(())
    }

    pub async fn mode(&self, mode: &str) -> Result<()> {
        if let Some(remote) = &self.remote {
            return remote
                .request(ControlCommand::Mode { mode: mode.into() })
                .await;
        }
        if !matches!(mode, "rule" | "global" | "direct") {
            bail!("模式无效");
        }
        self.healthy().await?;
        self.api.patch(json!({"mode": mode})).await
    }

    pub async fn system_proxy(&self, enabled: bool) -> Result<()> {
        if let Some(remote) = &self.remote {
            return remote
                .request(ControlCommand::Proxy {
                    mode: if enabled {
                        ProxyMode::System
                    } else {
                        ProxyMode::Off
                    },
                })
                .await;
        }
        if enabled {
            self.proxy_mode(ProxyMode::System).await
        } else {
            platform::restore(&self.journal())?;
            Ok(())
        }
    }

    pub async fn tun(&self, enabled: bool) -> Result<()> {
        if let Some(remote) = &self.remote {
            return remote
                .request(ControlCommand::Proxy {
                    mode: if enabled {
                        ProxyMode::Tun
                    } else {
                        ProxyMode::Off
                    },
                })
                .await;
        }
        if enabled {
            return self.proxy_mode(ProxyMode::Tun).await;
        }
        self.healthy().await?;
        self.set_tun_state(false).await
    }

    pub async fn proxy_mode(&self, mode: ProxyMode) -> Result<()> {
        if let Some(remote) = &self.remote {
            return remote.request(ControlCommand::Proxy { mode }).await;
        }
        if mode == ProxyMode::Off && !self.running() {
            platform::restore(&self.journal())?;
            return Ok(());
        }
        self.healthy().await?;
        crate::proxy::select(self, mode).await
    }

    async fn set_tun_state(&self, enabled: bool) -> Result<()> {
        #[cfg(target_os = "macos")]
        if !enabled {
            self.mac_protection_dns(false).await?;
        }
        #[cfg(target_os = "linux")]
        if enabled {
            platform::check_tun_environment()?;
            let pid = self
                .child
                .as_ref()
                .and_then(Child::id)
                .context("请先启动内核")?;
            if !platform::core_has_tun_permissions(pid)? {
                bail!("mihomo 缺少 TUN 网络权限，请点击 TUN 模式完成授权");
            }
        }
        self.api
            .patch(json!({"tun":{"enable":enabled}}))
            .await
            .context("TUN 切换失败；请确认内核拥有创建虚拟网卡及修改路由的权限")?;
        let config: Value = self.api.get("configs").await?;
        if config["tun"]["enable"].as_bool() != Some(enabled) {
            bail!("TUN 状态未生效，请查看内核日志和权限");
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    async fn mac_protection_dns(&self, enabled: bool) -> Result<()> {
        tokio::task::spawn_blocking(move || platform::macos::service::protection_dns(enabled))
            .await??;
        Ok(())
    }

    pub fn set_logs(&mut self, enabled: bool) {
        if self.is_remote() {
            return;
        }
        let enabled = enabled
            || self
                .external_logs_until
                .is_some_and(|deadline| deadline > Instant::now());
        if !enabled {
            if let Some(handle) = self.log_pump.take() {
                handle.abort();
            }
            return;
        }
        if !self.running() || self.log_pump.is_some() {
            return;
        }
        let api = self.api.clone();
        let logs = self.logs.clone();
        self.log_pump = Some(tokio::spawn(async move {
            loop {
                if let Ok(response) = api.logs().await {
                    let mut stream = response.bytes_stream();
                    let mut frames = LogFrames::default();
                    while let Some(Ok(chunk)) = stream.next().await {
                        frames.push(&chunk, |frame| {
                            #[derive(serde::Deserialize)]
                            struct Entry<'a> {
                                #[serde(rename = "type", default)]
                                kind: &'a str,
                                #[serde(default, borrow)]
                                payload: std::borrow::Cow<'a, str>,
                            }
                            if let Ok(entry) = serde_json::from_slice::<Entry<'_>>(frame) {
                                append(
                                    &logs,
                                    format!(
                                        "[{}] {}",
                                        if entry.kind.is_empty() {
                                            "info"
                                        } else {
                                            entry.kind
                                        },
                                        entry.payload
                                    ),
                                );
                            }
                        });
                    }
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }));
    }

    pub fn clear_logs(&self) -> Result<()> {
        if let Some(remote) = &self.remote {
            let _: Value = remote.call(ControlCommand::Logs { clear: true })?;
            return Ok(());
        }
        if let Ok(mut logs) = self.logs.lock() {
            logs.clear();
        }
        Ok(())
    }
    pub fn record_failure(&self, error: &str) {
        if let Some(remote) = &self.remote {
            let _: Result<Value> = remote.call(ControlCommand::RecordFailure {
                message: error.into(),
            });
            return;
        }
        append(
            &self.logs,
            format!(
                "[客户端] 操作失败：{}",
                error.replace(&self.settings.secret, "[密钥]")
            ),
        );
    }
    pub fn export_logs(&self) -> Result<PathBuf> {
        if let Some(remote) = &self.remote {
            return remote.call(ControlCommand::ExportLogs);
        }
        let path = self.store.root.join("logs-export.txt");
        let logs = self
            .logs
            .lock()
            .map_err(|_| anyhow::anyhow!("日志缓冲区不可用"))?;
        atomic_write(
            &path,
            logs.iter()
                .map(|line| line.as_ref())
                .collect::<Vec<_>>()
                .join("\n")
                .as_bytes(),
        )?;
        Ok(path)
    }

    async fn current_node(&self, mode: &str, proxies: &Proxies) -> Result<Option<String>> {
        if let Some(protection) = &self.settings.claude_protection {
            return Ok(Some(protection.node.clone()));
        }
        let default = if mode == "rule" {
            self.default_route()?
        } else {
            String::new()
        };
        let mut name = crate::active_route::start(mode, &default).to_owned();
        if !proxies.proxies.is_empty() {
            return Ok(crate::active_route::from_snapshot(&name, proxies));
        }
        let mut visited = std::collections::BTreeSet::new();
        for _ in 0..crate::active_route::MAX_DEPTH {
            if crate::active_route::terminal(&name) {
                return Ok(Some(name));
            }
            if !visited.insert(name.clone()) {
                return Ok(None);
            }
            let Some(proxy) = self.api.proxy_selection(&name).await? else {
                return Ok((visited.len() > 1).then_some(name));
            };
            if proxy.now.is_empty() {
                return Ok(Some(name));
            }
            name = proxy.now;
        }
        Ok(None)
    }

    pub async fn poll(&mut self, scope: Scope) -> Result<Snapshot> {
        self.poll_with_header(scope, false).await
    }

    pub async fn poll_with_header(&mut self, scope: Scope, header: bool) -> Result<Snapshot> {
        if self.is_remote() {
            return self.remote_poll(scope, header).await;
        }
        let mut snapshot = Snapshot::default();
        if let Some(child) = &mut self.child
            && let Some(status) = child.try_wait()?
        {
            self.child = None;
            self.protection_guard = None;
            for reader in self.readers.drain(..) {
                reader.abort();
            }
            self.last_totals = None;
            self.version.clear();
            self.set_logs(false);
            platform::restore(&self.journal())?;
            append(
                &self.logs,
                format!("[客户端] 内核意外退出：{status}，已尝试恢复系统代理"),
            );
        }
        snapshot.running = self.running();
        if scope == Scope::Claude {
            snapshot.claude_guard = self.protection_guard.as_ref().map(|guard| guard.snapshot());
        }
        snapshot.system_proxy = platform::is_owned(&self.journal())?;
        self.set_logs(scope == Scope::Logs);
        if scope == Scope::Logs {
            snapshot.logs = self
                .logs
                .lock()
                .map(|l| l.iter().cloned().collect())
                .unwrap_or_default();
        }
        if !snapshot.running {
            return Ok(snapshot);
        }
        snapshot.version = self.version.clone();
        let config: ConfigStatus = self.api.get("configs").await?;
        snapshot.mode = if config.mode.is_empty() {
            "rule".into()
        } else {
            config.mode
        };
        snapshot.tun = config.tun.enable;
        if matches!(scope, Scope::Home | Scope::Connections) || self.settings.tray_speed {
            if scope != Scope::Connections {
                let stats: ConnectionStats = self.api.get("connections").await?;
                snapshot.connection_count = stats.count;
                snapshot.connections.upload_total = stats.upload_total;
                snapshot.connections.download_total = stats.download_total;
            } else {
                snapshot.connections = self.api.get("connections").await?;
                snapshot.connection_count = snapshot.connections.connections.len();
            }
            let now = Instant::now();
            if scope == Scope::Connections {
                snapshot.connections_sampled_at = Some(now);
            }
            let up = snapshot.connections.upload_total;
            let down = snapshot.connections.download_total;
            if let Some((last, old_up, old_down)) = self.last_totals {
                let seconds = now.duration_since(last).as_secs_f64().max(0.001);
                snapshot.upload_rate = (up.saturating_sub(old_up) as f64 / seconds) as u64;
                snapshot.download_rate = (down.saturating_sub(old_down) as f64 / seconds) as u64;
            }
            self.last_totals = Some((now, up, down));
        }
        if scope == Scope::Proxies {
            snapshot.proxies = self.api.get("proxies").await?;
        }
        if scope == Scope::Rules {
            snapshot.rules = self.api.get("rules").await?;
        }
        if header {
            // Header failures do not erase useful page data or report a core
            // failure. Hidden-window polling does not request any node details.
            snapshot.active_node = tokio::time::timeout(
                Duration::from_millis(900),
                self.current_node(&snapshot.mode, &snapshot.proxies),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
        }
        Ok(snapshot)
    }
}

impl crate::proxy::Backend for Engine {
    async fn tun_enabled(&self) -> Result<bool> {
        let config: Value = self.api.get("configs").await?;
        Ok(config["tun"]["enable"].as_bool().unwrap_or(false))
    }
    fn system_enabled(&self) -> Result<bool> {
        platform::is_owned(&self.journal())
    }
    async fn set_tun(&self, enabled: bool) -> Result<()> {
        self.set_tun_state(enabled).await
    }
    fn set_system(&self, enabled: bool) -> Result<()> {
        if enabled {
            platform::restore(&self.journal())?;
            platform::enable(&self.journal(), self.settings.mixed_port)
        } else {
            platform::restore(&self.journal())?;
            Ok(())
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if self.is_remote() {
            return;
        }
        // Best-effort recovery also covers initialization/runtime errors. A failed
        // restore leaves its journal for the next launch.
        let _ = platform::restore(&self.journal());
        #[cfg(target_os = "macos")]
        let _ = platform::macos::service::protection_dns(false);
        if let Some(handle) = self.log_pump.take() {
            handle.abort();
        }
        for handle in &self.readers {
            handle.abort();
        }
        if let Some(child) = &mut self.child {
            #[cfg(target_os = "macos")]
            if let Some(pid) = child.id() {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = child.start_kill();
        }
    }
}

fn validation_error_detail(error: &str) -> &str {
    let error = error.trim();
    if error.is_empty() {
        return "内核未返回错误详情";
    }
    let start = error.char_indices().rev().nth(2047).map_or(0, |(i, _)| i);
    &error[start..]
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn mac_port_probe_reuses_closed_connections_and_rejects_live_listeners() {
        let listener = mac_port_probe(0).unwrap();
        let address = listener.local_addr().unwrap();
        assert!(mac_port_probe(address.port()).is_err());
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        drop(server);
        assert_eq!(client.read(&mut [0; 1]).await.unwrap(), 0);
        drop(client);
        drop(listener);
        let rebound = mac_port_probe(address.port()).unwrap();
        assert!(mac_port_probe(address.port()).is_err());
        drop(rebound);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn mac_port_wait_rejects_a_real_listener_without_waiting() {
        let listener = mac_port_probe(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            wait_for_mac_port(port, &logs, Duration::ZERO),
        )
        .await
        .expect("an active listener must not trigger the TIME_WAIT delay");
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
        assert!(logs.lock().unwrap().is_empty());
        drop(listener);
        wait_for_mac_port(port, &logs, Duration::ZERO)
            .await
            .unwrap();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn mac_update_port_grace_waits_for_exit_but_remains_bounded() {
        let listener = mac_port_probe(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        let closing = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            drop(listener);
        });
        wait_for_mac_port(port, &logs, Duration::from_secs(1))
            .await
            .unwrap();
        closing.await.unwrap();
        let _listener = mac_port_probe(port).unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            wait_for_mac_port(port, &logs, Duration::from_millis(100)),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
    }

    #[tokio::test]
    async fn validation_output_is_drained_after_capture_limit() {
        use tokio::io::AsyncWriteExt;
        let (reader, mut writer) = tokio::io::duplex(64);
        let producer = tokio::spawn(async move {
            writer
                .write_all(&vec![b'x'; LOG_FRAME_LIMIT * 3])
                .await
                .unwrap();
        });
        let captured = capture_output(reader).await.unwrap();
        producer.await.unwrap();
        assert_eq!(captured.len(), LOG_FRAME_LIMIT);
    }

    #[test]
    fn validation_error_keeps_the_fatal_tail_on_a_utf8_boundary() {
        let message = "启动信息".repeat(2000) + "\n根因：无效代理类型";
        let detail = validation_error_detail(&message);
        assert_eq!(detail.chars().count(), 2048);
        assert!(detail.ends_with("根因：无效代理类型"));
        assert_eq!(validation_error_detail(" \n\r\t"), "内核未返回错误详情");
    }
}
