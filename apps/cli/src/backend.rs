use crate::args::Args;
use anyhow::{Context, Result, bail, ensure};
use clash_client_runtime::{
    config::Store,
    control::{self, Command, Meta, Owner, Remote},
    engine::{Engine, Scope},
};
use serde::de::DeserializeOwned;
use std::{
    path::PathBuf,
    process::{Command as Process, Stdio},
    sync::Arc,
    time::Duration,
};
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
    if let Some(remote) = Remote::discover(&store, timeout(args)?)?
        && remote.request::<Meta>(Command::Meta).await.is_ok()
    {
        return Ok(Some(remote));
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
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x08000000);
    }
    let mut child = process.spawn().context("启动无界面后台失败")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(remote) = Remote::discover(&store, timeout(args)?)?
            && remote.request::<Meta>(Command::Meta).await.is_ok()
        {
            return Ok(Some(remote));
        }
        if let Some(status) = child.try_wait()? {
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
    let _: serde_json::Value = request(args, Command::Start).await?;
    meta(args).await
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
    loop {
        tokio::select! {
            result = interrupted() => { result?; break; },
            result = shutdown.changed() => { if result.is_err() || *shutdown.borrow() { break; } },
            _ = tick.tick() => {
                let mut owner = engine.lock().await;
                if let Err(error) = owner.poll(Scope::Other).await { owner.record_failure(&format!("{error:#}")); }
                let minutes = owner.settings.delay_interval_minutes;
                if owner.running() && minutes > 0 && owner.settings.claude_protection.is_none()
                    && delay_at.elapsed() >= Duration::from_secs(u64::from(minutes)*60) {
                    // Reuse the core's bounded group probing and automatic selection.
                    if let Err(error) = owner.api.delay_all_nodes().await { owner.record_failure(&format!("定时测速失败：{error:#}")); }
                    delay_at = tokio::time::Instant::now();
                }
            }
        }
    }
    engine.lock().await.stop().await?;
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
    tokio::signal::ctrl_c().await?;
    Ok(())
}
