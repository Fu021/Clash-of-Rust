use crate::{
    api::{Api, Connections, Proxies, Rules},
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
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    task::JoinHandle,
};

pub const LOG_LIMIT: usize = 500;
pub use crate::proxy::ProxyMode;
pub type LogBuffer = Arc<Mutex<VecDeque<String>>>;

fn append(logs: &LogBuffer, message: impl Into<String>) {
    if let Ok(mut buffer) = logs.lock() {
        if buffer.len() >= LOG_LIMIT {
            buffer.pop_front();
        }
        buffer.push_back(message.into().chars().take(1000).collect());
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
    pub rules: Rules,
    pub upload_rate: u64,
    pub download_rate: u64,
    pub logs: Vec<String>,
    pub running: bool,
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
        let output = tokio::time::timeout(
            Duration::from_secs(30),
            self.command()?
                .arg("-t")
                .arg("-d")
                .arg(directory)
                .arg("-f")
                .arg(&candidate)
                .output(),
        )
        .await
        .context("内核配置校验超时")??;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stdout).to_string()
                + &String::from_utf8_lossy(&output.stderr);
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
        self.validate(&payload).await?;
        let path = self.store.runtime().join("config.yaml");
        atomic_write(&path, payload.as_bytes())?;
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
            self.readers.push(tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    append(&logs, format!("[内核] {line}"));
                }
            }));
        }
        if let Some(stderr) = child.stderr.take() {
            let logs = self.logs.clone();
            self.readers.push(tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    append(&logs, format!("[内核] {line}"));
                }
            }));
        }
        self.child = Some(child);
        for _ in 0..30 {
            if let Some(status) = self.child.as_mut().unwrap().try_wait()? {
                self.child = None;
                bail!("内核启动后退出：{status}，请查看日志");
            }
            if tokio::time::timeout(Duration::from_millis(400), self.healthy())
                .await
                .is_ok_and(|r| r.is_ok())
            {
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
        self.import_with_route(name, source, existing, false).await
    }

    pub async fn import_via_proxy(
        &mut self,
        name: String,
        source: String,
        existing: Option<String>,
    ) -> Result<()> {
        self.import_with_route(name, source, existing, true).await
    }

    async fn import_with_route(
        &mut self,
        name: String,
        source: String,
        existing: Option<String>,
        via_proxy: bool,
    ) -> Result<()> {
        if source.trim().is_empty() {
            bail!("请输入订阅 URL 或本地 YAML 路径");
        }
        let (raw, usage) = if source.starts_with("https://") || source.starts_with("http://") {
            let url = reqwest::Url::parse(&source).context("订阅 URL 无效")?;
            let mut builder = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(30))
                .user_agent(concat!("clash-of-rust/", env!("CARGO_PKG_VERSION")));
            if via_proxy {
                let tun_enabled = if self.running() {
                    let config: Value = self.api.get("configs").await?;
                    config["tun"]["enable"].as_bool() == Some(true)
                } else {
                    false
                };
                if tun_enabled {
                    // Ordinary sockets follow the active TUN routes.
                } else if let Some(proxy) = platform::configured_proxy(url.scheme())? {
                    builder = builder.proxy(reqwest::Proxy::all(proxy)?);
                } else if self.running() {
                    builder = builder.proxy(reqwest::Proxy::all(format!(
                        "http://127.0.0.1:{}",
                        self.settings.mixed_port
                    ))?);
                } else {
                    bail!("请先启动内核或启用系统代理");
                }
            }
            let client = builder.build()?;
            let mut response = client
                .get(url)
                .send()
                .await
                .context("订阅下载失败")?
                .error_for_status()
                .context("订阅服务器返回错误状态")?;
            let usage = response
                .headers()
                .get("subscription-userinfo")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if bytes.len() + chunk.len() > 10 * 1024 * 1024 {
                    bail!("订阅超过 10 MiB 限制");
                }
                bytes.extend_from_slice(&chunk);
            }
            (
                String::from_utf8(bytes).context("订阅不是 UTF-8 YAML 文本")?,
                usage,
            )
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
        parse_profile(&raw)?;
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
        let mut previous: Value = serde_yaml::from_str(&old)?;
        let mut next: Value = serde_yaml::from_str(payload)?;
        // Reload must preserve the user's current routing and TUN state.
        let mode = json!(current["mode"].as_str().unwrap_or("rule"));
        let tun = json!(current["tun"]["enable"].as_bool().unwrap_or(false));
        next["mode"] = mode.clone();
        next["tun"]["enable"] = tun.clone();
        previous["mode"] = mode;
        previous["tun"]["enable"] = tun;
        let previous = serde_yaml::to_string(&previous)?;
        let next = serde_yaml::to_string(&next)?;
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
                    let mut pending = Vec::new();
                    while let Some(Ok(chunk)) = stream.next().await {
                        if pending.len() + chunk.len() > 64 * 1024 {
                            pending.clear();
                            continue;
                        }
                        pending.extend_from_slice(&chunk);
                        while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                            let line: Vec<_> = pending.drain(..=end).collect();
                            if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                                append(
                                    &logs,
                                    format!(
                                        "[{}] {}",
                                        value["type"].as_str().unwrap_or("info"),
                                        value["payload"].as_str().unwrap_or("")
                                    ),
                                );
                            }
                        }
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
                .cloned()
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
        snapshot.logs = self
            .logs
            .lock()
            .map(|l| l.iter().cloned().collect())
            .unwrap_or_default();
        if !snapshot.running {
            return Ok(snapshot);
        }
        snapshot.version = self.healthy().await?;
        let config: Value = self.api.get("configs").await?;
        snapshot.mode = config["mode"].as_str().unwrap_or("rule").to_owned();
        snapshot.tun = config["tun"]["enable"].as_bool().unwrap_or(false);
        if matches!(scope, Scope::Home | Scope::Connections) {
            snapshot.connections = self.api.get("connections").await?;
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
