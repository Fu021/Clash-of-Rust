use crate::args::Args;
use anyhow::{Context, Result, bail, ensure};
use clash_client_runtime::{
    config::Store,
    control::{self, Command, Meta, Owner, Remote},
    engine::{Engine, Scope},
};
use serde::de::DeserializeOwned;
#[cfg(not(windows))]
use std::process::Stdio;
use std::{path::PathBuf, process::Command as Process, sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct Unavailable(pub String);
impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Unavailable {}

pub fn store(args: &Args) -> Result<Store> {
    match args.get("data-dir") {
        Some(path) => Store::at(absolute(path)?),
        None => Store::discover(),
    }
}
pub fn absolute(path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path);
    Ok(if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    })
}
pub fn timeout(args: &Args) -> Result<Duration> {
    let value = args.number("timeout", 300, 3600)?;
    ensure!(value > 0, "--timeout 必须大于 0");
    Ok(Duration::from_secs(value as u64))
}

pub async fn connect(args: &Args, spawn: bool) -> Result<Option<Remote>> {
    let store = store(args)?;
    if let Some(remote) = Remote::discover(&store, timeout(args)?)? {
        match remote.request::<Meta>(Command::Meta).await {
            Ok(_) => return Ok(Some(remote)),
            Err(error) if store.instance_lock().is_err() => {
                return Err(error.context(Unavailable("存活的后台暂时不可达，请稍后重试".into())));
            }
            Err(_) => {}
        }
    }
    if !spawn {
        return Ok(None);
    }
    let log = store.root.join("daemon.log");
    if std::fs::metadata(&log).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
        std::fs::remove_file(&log)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    let mut process = Process::new(std::env::current_exe()?);
    process
        .args(["run", "--no-core", "--quiet", "--data-dir"])
        .arg(&store.root);
    if let Some(path) = args.get("resources") {
        process.arg("--resources").arg(absolute(path)?);
    }
    #[cfg(not(windows))]
    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(file);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        process.process_group(0);
    }
    #[cfg(windows)]
    let mut child = clash_client_runtime::platform::spawn_background_console(
        std::path::Path::new(process.get_program()),
        &process
            .get_args()
            .map(std::ffi::OsStr::to_owned)
            .collect::<Vec<_>>(),
        &file,
    )?;
    #[cfg(not(windows))]
    let mut child = process.spawn().context("启动无界面后台失败")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(remote) = Remote::discover(&store, timeout(args)?)?
            && remote.request::<Meta>(Command::Meta).await.is_ok()
        {
            return Ok(Some(remote));
        }
        if let Some(status) = child.try_wait()?
            && store.instance_lock().is_ok()
        {
            bail!(Unavailable(format!(
                "后台启动失败（{status}）；请查看 {}",
                log.display()
            )));
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(Unavailable(format!(
                "等待后台启动超时；请查看 {}",
                log.display()
            )));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
pub async fn remote(args: &Args) -> Result<Remote> {
    connect(args, true).await?.context("后台不可用")
}
pub async fn request<T: DeserializeOwned + Send + 'static>(
    args: &Args,
    operation: Command,
) -> Result<T> {
    remote(args).await?.request(operation).await
}
pub async fn meta(args: &Args) -> Result<Meta> {
    request(args, Command::Meta).await
}
pub async fn live(args: &Args) -> Result<Meta> {
    prepare_start(args).await?;
    let _: serde_json::Value = request(args, Command::Start).await?;
    meta(args).await
}
pub async fn prepare_start(args: &Args) -> Result<()> {
    let state = meta(args).await?;
    if !state.running
        && (state.settings.proxy_mode == clash_client_runtime::engine::ProxyMode::Tun
            || state.settings.claude_protection.is_some())
    {
        authorize_tun(args).await?;
    }
    Ok(())
}
pub async fn authorize_tun(args: &Args) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let resources = match args.get("resources") {
            Some(path) => absolute(path)?,
            None => clash_client_runtime::assets::discover()?,
        };
        let core = clash_client_runtime::assets::core_path(&resources)?;
        tokio::task::spawn_blocking(move || {
            clash_client_runtime::platform::authorize_tun_terminal(&core)
        })
        .await??;
    }
    #[cfg(not(target_os = "linux"))]
    let _ = args;
    Ok(())
}

/// One owner per data directory; the GUI can attach to this owner later.
pub async fn run(args: &Args) -> Result<()> {
    let store = store(args)?;
    if connect(args, false).await?.is_some() {
        bail!("这个数据目录已有 GUI 或后台；请使用 cor status/start");
    }
    let resources = args.get("resources").map(absolute).transpose()?;
    let engine = tokio::task::spawn_blocking(move || match resources {
        Some(path) => Engine::with_resources(store, path),
        None => Engine::new(store),
    })
    .await??;
    ensure!(
        !engine.is_remote(),
        "已有其他进程先启动后台，请使用 cor status"
    );
    let engine = Arc::new(Mutex::new(engine));
    control::serve(&engine, Owner::Daemon).await?;
    let mut shutdown = engine
        .lock()
        .await
        .shutdown_signal()
        .context("后台未创建控制端点")?;
    if !args.flag("no-core") {
        let state = engine.lock().await.settings.clone();
        if state.proxy_mode == clash_client_runtime::engine::ProxyMode::Tun
            || state.claude_protection.is_some()
        {
            authorize_tun(args).await?;
        }
        let mut owner = engine.lock().await;
        owner.start().await?;
        if let Err(error) = owner.restore_proxy_mode().await {
            let _ = owner.stop().await;
            return Err(error);
        }
    }
    if !args.flag("quiet") {
        eprintln!("Clash of Rust 后台已运行；Ctrl+C 恢复网络后退出");
    }
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut delay_at = tokio::time::Instant::now();
    let mut delay_job: Option<tokio::task::JoinHandle<()>> = None;
    loop {
        tokio::select! {
            result = interrupted() => { result?; break; },
            result = shutdown.changed() => { if result.is_err() || *shutdown.borrow() { break; } },
            _ = tick.tick() => {
                let mut owner = engine.lock().await;
                if let Err(error) = owner.poll(Scope::Other).await { owner.record_failure(&format!("{error:#}")); }
                let minutes = owner.settings.delay_interval_minutes;
                if (!owner.running() || minutes == 0 || owner.settings.claude_protection.is_some())
                    && let Some(job) = delay_job.take() {
                    job.abort();
                }
                if delay_job.as_ref().is_some_and(|job| job.is_finished()) {
                    delay_job = None;
                }
                if owner.running() && minutes > 0 && owner.settings.claude_protection.is_none()
                    && delay_job.is_none()
                    && delay_at.elapsed() >= Duration::from_secs(u64::from(minutes)*60) {
                    // The owning process schedules this once. Network probes must
                    // not hold the engine lock and block an attached GUI or stop.
                    let api = owner.api.clone();
                    let shared = engine.clone();
                    delay_job = Some(tokio::spawn(async move {
                        match api.delay_all_nodes().await {
                            Ok(_) => shared.lock().await.invalidate_views(),
                            Err(error) => shared.lock().await.record_failure(&format!("定时测速失败：{error:#}")),
                        }
                    }));
                    delay_at = tokio::time::Instant::now();
                }
            }
        }
    }
    if let Some(job) = delay_job {
        job.abort();
    }
    let mut owner = engine.lock().await;
    let closing = control::request_gui_exit(&owner.store);
    owner.stop().await?;
    closing?;
    Ok(())
}
pub async fn interrupted() -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! { result=tokio::signal::ctrl_c()=>{result?;}, _=terminate.recv()=>{} }
    }
    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            return Err(error.into());
        }
        // Windowless Windows login processes have no console control source.
        // Their graceful shutdown comes from the authenticated control endpoint.
        std::future::pending::<()>().await;
    }
    Ok(())
}
