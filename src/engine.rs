use crate::{
    api::{Api, ConfigStatus, ConnectionStats, Connections, Proxies, Rules},
    assets::{self, GeoManifest},
    config::{Profile, Settings, Store, atomic_write, parse_profile, runtime_config},
    platform,
};
use anyhow::{Context, Result, bail};
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
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader},
    process::{Child, Command},
    task::JoinHandle,
};

pub const LOG_LIMIT: usize = 500;
pub use crate::proxy::ProxyMode;
pub type LogBuffer = Arc<Mutex<VecDeque<Arc<str>>>>;

const LOG_FRAME_LIMIT: usize = 64 * 1024;

#[derive(Default)]
struct LogFrames {
    pending: Vec<u8>,
    discarding: bool,
}

impl LogFrames {
    fn push(&mut self, mut chunk: &[u8], mut emit: impl FnMut(&[u8])) {
        while !chunk.is_empty() {
            let end = chunk.iter().position(|byte| *byte == b'\n');
            let used = end.map_or(chunk.len(), |end| end + 1);
            if !self.discarding {
                if self.pending.len() + used > LOG_FRAME_LIMIT {
                    self.pending.clear();
                    self.discarding = true;
                } else if self.pending.is_empty() && end.is_some() {
                    emit(&chunk[..used]);
                } else {
                    self.pending.extend_from_slice(&chunk[..used]);
                    if end.is_some() {
                        emit(&self.pending);
                        self.pending.clear();
                    }
                }
            }
            if end.is_some() {
                self.discarding = false;
            }
            chunk = &chunk[used..];
        }
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

fn append(logs: &LogBuffer, message: impl Into<String>) {
    if let Ok(mut buffer) = logs.lock() {
        if buffer.len() >= LOG_LIMIT {
            buffer.pop_front();
        }
        let mut message = message.into();
        if let Some((end, _)) = message.char_indices().nth(1000) {
            message.truncate(end);
        }
        buffer.push_back(Arc::from(message));
    }
}

async fn read_core_logs(reader: impl AsyncRead + Unpin, logs: LogBuffer) {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::with_capacity(4096);
    while let Ok(bytes) = reader.fill_buf().await {
        if bytes.is_empty() {
            if !line.is_empty() {
                append(&logs, format!("[内核] {}", String::from_utf8_lossy(&line)));
            }
            break;
        }
        let end = bytes.iter().position(|byte| *byte == b'\n');
        let used = end.map_or(bytes.len(), |end| end + 1);
        line.extend_from_slice(&bytes[..used.min(4096 - line.len())]);
        reader.consume(used);
        if end.is_some() {
            let text = String::from_utf8_lossy(&line);
            append(
                &logs,
                format!("[内核] {}", text.trim_end_matches(['\r', '\n'])),
            );
            line.clear();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Home,
    Proxies,
    Connections,
    Rules,
    Logs,
    Other,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub version: String,
    pub mode: String,
    pub tun: bool,
    pub system_proxy: bool,
    pub proxies: Proxies,
    pub connections: Connections,
    pub connection_count: usize,
    pub rules: Rules,
    pub upload_rate: u64,
    pub download_rate: u64,
    pub logs: Vec<Arc<str>>,
    pub running: bool,
}

impl Snapshot {
    /// Release heap-backed page data, including spare vector capacity.
    pub fn retain_scope(&mut self, scope: Scope) {
        if scope != Scope::Proxies {
            self.proxies = Proxies::default();
        }
        if scope != Scope::Rules {
            self.rules = Rules::default();
        }
        if scope != Scope::Connections {
            self.connections.connections = Vec::new();
        }
        if scope != Scope::Logs {
            self.logs = Vec::new();
        }
    }
}

pub struct Engine {
    pub store: Store,
    pub settings: Settings,
    pub profiles: Vec<Profile>,
    pub api: Api,
    pub geo_manifest: GeoManifest,
    resources: PathBuf,
    child: Option<Child>,
    logs: LogBuffer,
    readers: Vec<JoinHandle<()>>,
    log_pump: Option<JoinHandle<()>>,
    last_totals: Option<(Instant, u64, u64)>,
    version: String,
    // Held for the entire application lifetime; prevents two clients racing on recovery.
    _instance_lock: File,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct RestartState {
    mode: String,
    tun: bool,
    system_proxy: bool,
    selectors: Vec<(String, String)>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ElevationState {
    requested: RestartState,
    previous: RestartState,
}

impl Engine {
    pub fn new(store: Store) -> Result<Self> {
        Self::with_resources(store, assets::discover()?)
    }

    pub fn with_resources(store: Store, resources: PathBuf) -> Result<Self> {
        let instance = store.instance_lock()?;
        let geo_manifest = assets::seed(&resources, &store.runtime())?;
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        match platform::restore(&store.root.join("system-proxy.json")) {
            Ok(true) => append(&logs, "[客户端] 已恢复上次运行遗留的系统代理"),
            Ok(false) => {}
            Err(e) => append(&logs, format!("[客户端] 代理恢复失败：{e}")),
        }
        let mut settings = store.load_settings()?;
        let mut profiles = store.profiles()?;
        if profiles.is_empty() {
            let raw = std::fs::read_to_string(resources.join("default.yaml"))
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
            logs,
            readers: vec![],
            log_pump: None,
            last_totals: None,
            version: String::new(),
            _instance_lock: instance,
        })
    }

    pub fn running(&self) -> bool {
        self.child.is_some()
    }
    fn journal(&self) -> PathBuf {
        self.store.root.join("system-proxy.json")
    }
    fn core(&self) -> Result<PathBuf> {
        assets::core_path(&self.resources)
    }
    fn command(&self) -> Result<Command> {
        let core = self.core()?;
        #[cfg(target_os = "linux")]
        let executable = platform::core_launcher(&core);
        #[cfg(not(target_os = "linux"))]
        let executable = &core;
        let mut command = Command::new(executable);
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
            .command()?
            .arg("-t")
            .arg("-d")
            .arg(directory)
            .arg("-f")
            .arg(&candidate)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().context("无法读取校验输出")?;
        let stderr = child.stderr.take().context("无法读取校验错误输出")?;
        let (status, stdout, stderr) = tokio::time::timeout(Duration::from_secs(30), async {
            tokio::try_join!(child.wait(), capture_output(stdout), capture_output(stderr))
        })
        .await
        .context("内核配置校验超时")??;
        if !status.success() {
            let error =
                String::from_utf8_lossy(&stdout).to_string() + &String::from_utf8_lossy(&stderr);
            append(
                &self.logs,
                format!(
                    "[配置校验] {}",
                    error.replace(&self.settings.secret, "[密钥]")
                ),
            );
            bail!("mihomo 配置校验失败，详情见日志页");
        }
        Ok(())
    }
    async fn healthy(&self) -> Result<String> {
        let version: Value = self.api.get("version").await?;
        Ok(version["version"].as_str().unwrap_or("未知").to_owned())
    }
    pub async fn start(&mut self) -> Result<()> {
        if self.running() {
            bail!("内核已启动");
        }
        for port in [self.settings.controller_port, self.settings.mixed_port] {
            let _listener = std::net::TcpListener::bind(("127.0.0.1", port))
                .with_context(|| format!("端口 {port} 已被占用"))?;
        }
        let id = self
            .settings
            .active_profile
            .as_ref()
            .context("请先导入并选择一个订阅配置")?;
        let raw = std::fs::read_to_string(self.store.profile_path(id)?)?;
        let payload = runtime_config(&raw, &self.settings)?;
        drop(raw);
        self.validate(&payload).await?;
        let path = self.store.runtime().join("config.yaml");
        atomic_write(&path, payload.as_bytes())?;
        drop(payload);
        let mut child = self
            .command()?
            .arg("-d")
            .arg(self.store.runtime())
            .arg("-f")
            .arg(path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
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
        platform::restore(&self.journal()).context("恢复系统代理失败，暂不停止内核以避免断网")?;
        self.stop_process().await
    }

    async fn stop_process(&mut self) -> Result<()> {
        if self.running()
            && let Ok(config) = self.api.get::<Value>("configs").await
            && config["tun"]["enable"].as_bool() == Some(true)
        {
            self.tun(false).await.context("停止内核前关闭 TUN 失败")?;
        }
        self.set_logs(false);
        if let Some(mut child) = self.child.take() {
            if child.try_wait()?.is_none() {
                child.kill().await?;
            }
            child.wait().await?;
        }
        for reader in self.readers.drain(..) {
            reader.abort();
        }
        self.last_totals = None;
        self.version.clear();
        append(&self.logs, "[客户端] 内核已停止");
        Ok(())
    }

    pub async fn save_settings(&mut self, settings: Settings) -> Result<()> {
        settings.validate()?;
        if settings.controller_port == self.settings.controller_port
            && settings.mixed_port == self.settings.mixed_port
        {
            self.store.save_settings(&settings)?;
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
                .map(|(name, p)| (name, p.now))
                .collect(),
        }))
    }

    pub async fn import(
        &mut self,
        name: String,
        source: String,
        existing: Option<String>,
    ) -> Result<()> {
        if source.trim().is_empty() {
            bail!("请输入订阅 URL 或本地 YAML 路径");
        }
        let (raw, usage) = if source.starts_with("https://") || source.starts_with("http://") {
            let url = reqwest::Url::parse(&source).context("订阅 URL 无效")?;
            crate::subscription::download(&url, self.running().then_some(self.settings.mixed_port))
                .await?
        } else {
            let file = File::open(&source).context("本地配置文件无法读取")?;
            if file.metadata()?.len() > 10 * 1024 * 1024 {
                bail!("配置超过 10 MiB 限制");
            }
            let mut raw = String::new();
            use std::io::Read;
            file.take(10 * 1024 * 1024 + 1).read_to_string(&mut raw)?;
            (raw, None)
        };
        let id = existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let path = self.store.profile_path(&id)?;
        let payload = runtime_config(&raw, &self.settings)?;
        self.validate(&payload).await?;
        let active = self.settings.active_profile.as_ref() == Some(&id);
        if active && self.running() {
            self.apply_payload(&payload).await?;
        }
        atomic_write(&path, raw.as_bytes())?;
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
        self.store.save_profiles(&profiles)?;
        self.profiles = profiles;
        if self.settings.active_profile.is_none() {
            self.settings.active_profile = Some(id);
            self.store.save_settings(&self.settings)?;
        }
        append(&self.logs, "[客户端] 配置已导入/更新");
        Ok(())
    }

    async fn apply_payload(&self, payload: &str) -> Result<()> {
        self.validate(payload).await?;
        let path = self.store.runtime().join("config.yaml");
        let old = std::fs::read_to_string(&path)?;
        let current: Value = self.api.get("configs").await?;
        // Reload must preserve the user's current routing and TUN state.
        let mode = json!(current["mode"].as_str().unwrap_or("rule"));
        let tun = json!(current["tun"]["enable"].as_bool().unwrap_or(false));
        let previous = {
            let mut value: Value = serde_yaml::from_str(&old)?;
            value["mode"] = mode.clone();
            value["tun"]["enable"] = tun.clone();
            serde_yaml::to_string(&value)?
        };
        drop(old);
        let next = {
            let mut value: Value = serde_yaml::from_str(payload)?;
            value["mode"] = mode;
            value["tun"]["enable"] = tun;
            serde_yaml::to_string(&value)?
        };
        let result = async {
            self.api.reload(&next).await?;
            self.healthy().await?;
            atomic_write(&path, next.as_bytes())?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            let rollback = self.api.reload(&previous).await;
            if let Err(rollback) = rollback {
                bail!("配置应用失败：{error}；回退也失败：{rollback}");
            }
            return Err(error.context("配置应用失败，已恢复上一份运行配置"));
        }
        Ok(())
    }

    pub async fn activate(&mut self, id: String) -> Result<()> {
        if !self.profiles.iter().any(|p| p.id == id) {
            bail!("配置不存在");
        }
        let raw = std::fs::read_to_string(self.store.profile_path(&id)?)?;
        let payload = runtime_config(&raw, &self.settings)?;
        if self.running() {
            self.apply_payload(&payload).await?;
        }
        self.settings.active_profile = Some(id);
        self.store.save_settings(&self.settings)?;
        Ok(())
    }

    pub async fn update_geo(&mut self) -> Result<()> {
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
        }
        if state.system_proxy && !platform::is_owned(&self.journal())? {
            self.system_proxy(true).await?;
        }
        Ok(())
    }

    pub async fn prepare_elevation(&mut self) -> Result<()> {
        let previous = self.restart_state().await?.context("请先启动内核")?;
        let mut state = previous.clone();
        state.tun = true;
        state.system_proxy = false;
        let path = self.store.root.join("elevation.json");
        atomic_write(
            &path,
            &serde_json::to_vec(&ElevationState {
                requested: state,
                previous,
            })?,
        )?;
        if let Err(error) = platform::elevate(&path) {
            let _ = std::fs::remove_file(path);
            return Err(error);
        }
        Ok(())
    }

    pub async fn finish_elevation(&mut self, path: &std::path::Path) -> Result<()> {
        let state: ElevationState = serde_json::from_slice(&std::fs::read(path)?)?;
        std::fs::remove_file(path)?;
        if let Err(error) = self.resume(&state.requested).await {
            let restored = if self.running() {
                let mode = if state.previous.tun {
                    ProxyMode::Tun
                } else if state.previous.system_proxy {
                    ProxyMode::System
                } else {
                    ProxyMode::Off
                };
                self.proxy_mode(mode).await
            } else {
                self.resume(&state.previous).await
            };
            return match restored {
                Ok(()) => Err(error.context("TUN 提权后切换失败，已恢复原代理模式")),
                Err(rollback) => {
                    Err(error.context(format!("TUN 切换失败，恢复原模式也失败：{rollback:#}")))
                }
            };
        }
        self.remember_modes(&state.requested.mode, ProxyMode::Tun)?;
        Ok(())
    }

    pub async fn delete_profile(&mut self, id: &str) -> Result<()> {
        let profile = self
            .profiles
            .iter()
            .find(|p| p.id == id)
            .context("订阅不存在")?;
        if profile.is_default() {
            bail!("默认直连配置不能删除");
        }
        if self.settings.active_profile.as_deref() == Some(id) {
            let default = self
                .profiles
                .iter()
                .find(|p| p.is_default())
                .context("默认配置缺失")?
                .id
                .clone();
            self.activate(default).await?;
        }
        let profiles: Vec<_> = self
            .profiles
            .iter()
            .filter(|p| p.id != id)
            .cloned()
            .collect();
        self.store.save_profiles(&profiles)?;
        self.profiles = profiles;
        let path = self.store.profile_path(id)?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    pub async fn install_geo(&mut self, staged: &std::path::Path) -> Result<()> {
        let runtime = self.store.runtime();
        let manifest = assets::verify_geo(staged)?;
        // Ask the actual core to read DAT, MMDB and ASN data before replacing anything.
        for mode in [true, false] {
            let raw = format!(
                "proxies: []\ngeodata-mode: {mode}\nrules:\n  - GEOSITE,cn,DIRECT\n  - GEOIP,CN,DIRECT\n  - IP-ASN,13335,DIRECT\n  - MATCH,DIRECT\n"
            );
            let payload = runtime_config(&raw, &self.settings)?;
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
                    .map(|(name, p)| (name, p.now))
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
    fn remember_modes(&mut self, run_mode: &str, proxy_mode: ProxyMode) -> Result<()> {
        let settings = Settings {
            run_mode: run_mode.into(),
            proxy_mode,
            ..self.settings.clone()
        };
        self.store.save_settings(&settings)?;
        self.settings = settings;
        Ok(())
    }

    pub async fn select_mode(&mut self, mode: &str) -> Result<()> {
        self.mode(mode).await?;
        self.remember_modes(mode, self.settings.proxy_mode)
    }

    pub async fn select_proxy_mode(&mut self, mode: ProxyMode) -> Result<()> {
        #[cfg(target_os = "linux")]
        if mode == ProxyMode::Tun {
            self.prepare_linux_tun(true).await?;
        }
        self.proxy_mode(mode).await?;
        self.remember_modes(&self.settings.run_mode.clone(), mode)
    }

    /// Returns true when a successful UAC launch requires the GUI to exit.
    pub async fn restore_proxy_mode(&mut self) -> Result<bool> {
        if cfg!(windows) && self.settings.proxy_mode == ProxyMode::Tun && !platform::is_elevated() {
            self.prepare_elevation().await?;
            return Ok(true);
        }
        #[cfg(target_os = "linux")]
        if self.settings.proxy_mode == ProxyMode::Tun {
            // Background startup must not repeatedly request administrator access.
            let interactive = !std::env::args_os().any(|arg| arg == "--background");
            self.prepare_linux_tun(interactive).await?;
        }
        self.proxy_mode(self.settings.proxy_mode).await?;
        Ok(false)
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
        if !matches!(mode, "rule" | "global" | "direct") {
            bail!("模式无效");
        }
        self.healthy().await?;
        self.api.patch(json!({"mode": mode})).await
    }

    pub async fn system_proxy(&self, enabled: bool) -> Result<()> {
        if enabled {
            self.proxy_mode(ProxyMode::System).await
        } else {
            platform::restore(&self.journal())?;
            Ok(())
        }
    }

    pub async fn tun(&self, enabled: bool) -> Result<()> {
        if enabled {
            return self.proxy_mode(ProxyMode::Tun).await;
        }
        self.healthy().await?;
        self.set_tun_state(false).await
    }

    pub async fn proxy_mode(&self, mode: ProxyMode) -> Result<()> {
        if mode == ProxyMode::Off && !self.running() {
            platform::restore(&self.journal())?;
            return Ok(());
        }
        self.healthy().await?;
        crate::proxy::select(self, mode).await
    }

    async fn set_tun_state(&self, enabled: bool) -> Result<()> {
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

    pub fn set_logs(&mut self, enabled: bool) {
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

    pub fn clear_logs(&self) {
        if let Ok(mut logs) = self.logs.lock() {
            logs.clear();
        }
    }
    pub fn export_logs(&self) -> Result<PathBuf> {
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

    pub async fn poll(&mut self, scope: Scope) -> Result<Snapshot> {
        let mut snapshot = Snapshot::default();
        if let Some(child) = &mut self.child
            && let Some(status) = child.try_wait()?
        {
            self.child = None;
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
        if matches!(scope, Scope::Home | Scope::Connections) {
            if scope == Scope::Home {
                let stats: ConnectionStats = self.api.get("connections").await?;
                snapshot.connection_count = stats.count;
                snapshot.connections.upload_total = stats.upload_total;
                snapshot.connections.download_total = stats.download_total;
            } else {
                snapshot.connections = self.api.get("connections").await?;
                snapshot.connection_count = snapshot.connections.connections.len();
            }
            let now = Instant::now();
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
        // Best-effort recovery also covers initialization/runtime errors. A failed
        // restore leaves its journal for the next launch.
        let _ = platform::restore(&self.journal());
        if let Some(handle) = self.log_pump.take() {
            handle.abort();
        }
        for handle in &self.readers {
            handle.abort();
        }
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    #[test]
    fn log_frames_handle_fragmentation_batches_and_oversized_records() {
        let mut frames = LogFrames::default();
        let mut emitted = Vec::new();
        frames.push(b"first", |line| emitted.push(line.to_vec()));
        frames.push(b"\nsecond\nthird", |line| emitted.push(line.to_vec()));
        frames.push(b"\n", |line| emitted.push(line.to_vec()));
        assert_eq!(
            emitted,
            [
                b"first\n".to_vec(),
                b"second\n".to_vec(),
                b"third\n".to_vec()
            ]
        );
        emitted.clear();
        for _ in 0..100 {
            frames.push(&[b'x'; 2048], |line| emitted.push(line.to_vec()));
            assert!(frames.pending.len() <= LOG_FRAME_LIMIT);
        }
        frames.push(b"tail\nvalid\n", |line| emitted.push(line.to_vec()));
        assert_eq!(emitted, [b"valid\n".to_vec()]);
        // A large network chunk may contain many small valid records.
        emitted.clear();
        frames.push(&b"ok\n".repeat(30_000), |line| emitted.push(line.to_vec()));
        assert_eq!(emitted.len(), 30_000);
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
    fn leaving_a_page_releases_data_and_vector_capacity() {
        let mut snapshot = Snapshot::default();
        snapshot.connections.connections.reserve(10_000);
        snapshot.rules.rules.reserve(10_000);
        snapshot
            .proxies
            .proxies
            .insert("group".into(), Default::default());
        snapshot.logs.push(Arc::from("log"));
        snapshot.retain_scope(Scope::Rules);
        assert_eq!(snapshot.connections.connections.capacity(), 0);
        assert!(snapshot.proxies.proxies.is_empty());
        assert_eq!(snapshot.logs.capacity(), 0);
        assert!(snapshot.rules.rules.capacity() >= 10_000);
        snapshot.retain_scope(Scope::Other);
        assert_eq!(snapshot.rules.rules.capacity(), 0);
    }

    #[tokio::test]
    async fn long_log_lines_are_bounded_and_next_line_survives() {
        use tokio::io::AsyncWriteExt;
        let (reader, mut writer) = tokio::io::duplex(64);
        let producer = tokio::spawn(async move {
            writer.write_all(&vec![b'x'; 100_000]).await.unwrap();
            writer.write_all("\n下一行\n".as_bytes()).await.unwrap();
        });
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        read_core_logs(reader, logs.clone()).await;
        producer.await.unwrap();
        let buffer = logs.lock().unwrap();
        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer[0].chars().count(), 1000);
        assert_eq!(buffer[1].as_ref(), "[内核] 下一行");
    }
    #[test]
    fn log_snapshots_share_text_and_history_is_bounded() {
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        for i in 0..LOG_LIMIT + 10 {
            append(&logs, format!("entry {i}"));
        }
        let buffer = logs.lock().unwrap();
        assert_eq!(buffer.len(), LOG_LIMIT);
        assert_eq!(buffer[0].as_ref(), "entry 10");
        let snapshot: Vec<_> = buffer.iter().cloned().collect();
        assert!(Arc::ptr_eq(&buffer[0], &snapshot[0]));
    }
}
