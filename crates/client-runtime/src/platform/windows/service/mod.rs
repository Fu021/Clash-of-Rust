//! A demand-start Windows service owns only a fixed, verified mihomo core.
//! The GUI and its bridge retain the user's ordinary token across every launch.
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    System::{
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
        Threading::WaitForSingleObject,
    },
    UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
};
mod pipe;
mod runtime;
mod scm;
mod security;
const NAME: &str = "ClashOfRust.Network";
static STOP: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol: u8,
    directory: PathBuf,
    config: PathBuf,
}
#[derive(Default, Serialize, Deserialize)]
struct Event {
    line: Option<String>,
    exit: Option<i32>,
    error: Option<String>,
}

pub fn installed() -> bool {
    scm::exists()
}
pub fn installed_directory() -> Result<PathBuf> {
    security::installed_directory()
}

/// Prepare the existing machine service without elevating the GUI. The
/// exceptional install/repair path elevates only the fixed helper executable.
pub fn ensure_available(source: &Path, interactive: bool) -> Result<()> {
    let installed = installed_directory()?.join("clash-tun-launcher.exe");
    let current = scm::exists()
        && installed.is_file()
        && crate::file_hash::hash_file(source).is_ok_and(|source| {
            crate::file_hash::hash_file(&installed).is_ok_and(|installed| source == installed)
        });
    if !current {
        ensure!(
            interactive,
            "Windows TUN 助手需要安装或修复，请打开客户端并点击 TUN 模式授权一次"
        );
        let file = security::wide(source);
        let verb = security::wide("runas");
        let parameters = security::wide("--windows-service-install");
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of_val(&info) as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = parameters.as_ptr();
        info.nShow = 0;
        ensure!(
            unsafe { ShellExecuteExW(&mut info) } != 0,
            "网络助手授权未完成：{}",
            std::io::Error::last_os_error()
        );
        let process = security::Handle::new(info.hProcess)?;
        ensure!(
            unsafe { WaitForSingleObject(process.raw(), 60_000) } == 0,
            "网络助手安装未完成，请稍后重试"
        );
        let mut code = 1;
        unsafe {
            windows_sys::Win32::System::Threading::GetExitCodeProcess(process.raw(), &mut code);
        }
        ensure!(
            code == 0,
            "网络助手安装失败，请运行安装程序修复（错误 {code}）"
        );
    }
    scm::start()
}

pub fn command() -> Result<tokio::process::Command> {
    let mut command =
        tokio::process::Command::new(installed_directory()?.join("clash-tun-launcher.exe"));
    command
        .arg("--windows-bridge")
        .kill_on_drop(true)
        .creation_flags(0x08000000);
    Ok(command)
}

fn lines(reader: impl Read, sender: std::sync::mpsc::SyncSender<Event>) {
    let mut reader = std::io::BufReader::new(reader);
    let mut bytes = Vec::with_capacity(4096);
    loop {
        bytes.clear();
        let Ok(count) = reader.by_ref().take(8192).read_until(b'\n', &mut bytes) else {
            break;
        };
        if count == 0 {
            break;
        }
        if bytes.last() != Some(&b'\n') && count == 8192 {
            loop {
                let Ok(chunk) = reader.fill_buf() else {
                    return;
                };
                if chunk.is_empty() {
                    break;
                }
                let end = chunk.iter().position(|&b| b == b'\n');
                let used = end.map_or(chunk.len(), |p| p + 1);
                reader.consume(used);
                if end.is_some() {
                    break;
                }
            }
        }
        let line: String = String::from_utf8_lossy(&bytes)
            .trim_end_matches(['\r', '\n'])
            .chars()
            .take(4096)
            .collect();
        if sender
            .send(Event {
                line: Some(line),
                ..Default::default()
            })
            .is_err()
        {
            break;
        }
    }
}

fn lease(pipe: &mut pipe::Pipe) -> Result<()> {
    let mut pid = 0;
    ensure!(
        unsafe { GetNamedPipeClientProcessId(pipe.0.raw(), &mut pid) } != 0,
        "无法验证调用进程"
    );
    let process = security::process(pid)?;
    ensure!(
        security::image(&process)?.canonicalize()?
            == installed_directory()?
                .join("clash-tun-launcher.exe")
                .canonicalize()?,
        "只有已安装的桥接程序可以启动内核"
    );
    let owner = security::parent(pid)?;
    ensure!(
        security::user(owner.raw())? == security::user(process.raw())?,
        "桥接与客户端账户不一致"
    );
    let request: Request = pipe.read(Duration::from_secs(5))?;
    ensure!(request.protocol == 1, "网络助手协议不兼容，请修复安装");
    let snapshot = runtime::snapshot(pipe.0.raw(), &request)?;
    let mut core = runtime::Core::start(&snapshot)?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(8);
    let out = core.stdout.take().unwrap();
    let error = core.stderr.take().unwrap();
    let output_sender = sender.clone();
    let output = std::thread::spawn(move || lines(out, output_sender));
    let errors = std::thread::spawn(move || lines(error, sender));
    let result = (|| -> Result<()> {
        loop {
            if STOP.load(Ordering::SeqCst)
                || unsafe { WaitForSingleObject(owner.raw(), 0) } == 0
                || unsafe { WaitForSingleObject(process.raw(), 0) } == 0
            {
                break;
            }
            if let Some(code) = core.exit() {
                pipe.write(&Event {
                    exit: Some(code),
                    ..Default::default()
                })?;
                break;
            }
            let event = receiver
                .recv_timeout(Duration::from_millis(200))
                .unwrap_or_default();
            pipe.write(&event)?;
        }
        Ok(())
    })();
    core.stop();
    drop(receiver);
    let _ = output.join();
    let _ = errors.join();
    // Restore provider caches as the original user, even after bridge loss.
    let cache = snapshot.save_cache();
    result.and(cache)
}

fn serve() -> Result<()> {
    let active = Arc::new(AtomicUsize::new(0));
    let core = Arc::new(Mutex::new(()));
    let mut workers = Vec::<std::thread::JoinHandle<()>>::new();
    let mut pipe = pipe::Pipe::server(true)?;
    let mut idle = Instant::now();
    while !STOP.load(Ordering::SeqCst) {
        workers.retain(|worker| !worker.is_finished());
        if active.load(Ordering::SeqCst) > 0 {
            idle = Instant::now();
        }
        if idle.elapsed() > Duration::from_secs(15) {
            break;
        }
        if active.load(Ordering::SeqCst) < 3 && pipe.connected()? {
            let next = pipe::Pipe::server(false)?;
            let mut connected = std::mem::replace(&mut pipe, next);
            let active = active.clone();
            let core = core.clone();
            active.fetch_add(1, Ordering::SeqCst);
            workers.push(std::thread::spawn(move || {
                if let Ok(_lease) = core.lock()
                    && let Err(error) = lease(&mut connected)
                {
                    let _ = connected.write(&Event {
                        error: Some(format!("{error:#}")),
                        ..Default::default()
                    });
                }
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    STOP.store(true, Ordering::SeqCst);
    for worker in workers {
        let _ = worker.join();
    }
    Ok(())
}

fn bridge(args: &[std::ffi::OsString]) -> Result<i32> {
    ensure!(
        args.len() == 4 && args[0] == "-d" && args[2] == "-f",
        "内核桥接参数无效"
    );
    // The service can be exiting its idle grace period. Retry a fresh SCM
    // start after a failed connection instead of requiring another UAC grant.
    scm::start()?;
    let mut pipe = pipe::Pipe::client()?;
    let mut server_pid = 0;
    ensure!(
        unsafe { GetNamedPipeServerProcessId(pipe.0.raw(), &mut server_pid) } != 0
            && server_pid == scm::server_pid()?,
        "网络助手身份验证失败"
    );
    pipe.write(&Request {
        protocol: 1,
        directory: PathBuf::from(&args[1]),
        config: PathBuf::from(&args[3]),
    })?;
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    loop {
        let event: Event = pipe.read(Duration::from_secs(30))?;
        if let Some(error) = event.error {
            bail!("{error}");
        }
        if let Some(line) = event.line {
            writeln!(stdout, "{line}")?;
            stdout.flush()?;
        }
        if let Some(code) = event.exit {
            return Ok(code);
        }
    }
}

pub fn helper_main() -> Result<i32> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.first().and_then(|s| s.to_str()) {
        Some("--windows-service-install") if args.len() == 1 => scm::install()?,
        Some("--windows-service-uninstall") if args.len() == 1 => scm::uninstall()?,
        Some("--windows-service-stop") if args.len() == 1 => {
            ensure!(security::administrator()?, "停止服务需要管理员权限");
            scm::stop()?;
        }
        Some("--windows-service") if args.len() == 1 => scm::dispatch()?,
        Some("--windows-bridge") => return bridge(&args[1..]),
        _ => bail!("Windows 网络助手参数无效"),
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_has_no_executable_or_shell_command_parameters() {
        assert!(serde_json::from_str::<Request>(r#"{"protocol":1,"directory":"C:\\data","config":"C:\\data\\config.yaml","executable":"cmd.exe"}"#).is_err());
    }
}
