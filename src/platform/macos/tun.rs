//! A short-lived, authorized core service. No persistent root daemon or setuid files.
//! The private socket authenticates the caller's uid and a fresh session token.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
struct Request {
    token: String,
    operation: String,
    directory: Option<PathBuf>,
    config: Option<PathBuf>,
}
#[derive(Serialize, Deserialize)]
struct Event {
    line: Option<String>,
    exit: Option<i32>,
}
pub struct Session {
    directory: tempfile::TempDir,
    token: String,
}
impl Session {
    pub fn authorize() -> Result<Self> {
        let directory = tempfile::Builder::new().prefix("clash-tun-").tempdir()?;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        let token = uuid::Uuid::new_v4().to_string();
        let launcher = std::env::current_exe()?
            .parent()
            .context("无法定位 TUN 助手")?
            .join("clash-tun-launcher");
        ensure!(
            launcher.is_file(),
            "TUN 助手缺失，请使用完整的 macOS 安装包"
        );
        let socket = directory.path().join("core.sock");
        let script = format!(
            "{} --macos-authorize {} {} {}",
            super::shell_quote(&launcher.to_string_lossy()),
            super::shell_quote(&socket.to_string_lossy()),
            unsafe { libc::getuid() },
            super::shell_quote(&token)
        );
        super::administrator(&script)?;
        let session = Self { directory, token };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if session.request("ping").is_ok() {
                return Ok(session);
            }
            ensure!(
                Instant::now() < deadline,
                "TUN 授权助手未就绪，请检查安装包与系统日志"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    fn request(&self, operation: &str) -> Result<()> {
        let mut stream = UnixStream::connect(self.directory.path().join("core.sock"))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        serde_json::to_writer(
            &mut stream,
            &Request {
                token: self.token.clone(),
                operation: operation.into(),
                directory: None,
                config: None,
            },
        )?;
        stream.write_all(b"\n")?;
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response)?;
        ensure!(response.trim() == "ready", "TUN 助手响应无效");
        Ok(())
    }
    pub fn wait_idle(&self) -> Result<()> {
        self.request("ping")
    }
    pub fn command(&self) -> Result<tokio::process::Command> {
        let launcher = std::env::current_exe()?
            .parent()
            .context("无法定位 TUN 助手")?
            .join("clash-tun-launcher");
        let mut command = tokio::process::Command::new(launcher);
        command.args([
            "--macos-bridge",
            &self.directory.path().join("core.sock").to_string_lossy(),
            &self.token,
        ]);
        Ok(command)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.request("stop");
    }
}
fn peer_uid(stream: &UnixStream) -> Result<u32> {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (0, 0);
    ensure!(
        unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0,
        "无法确认 TUN 会话身份"
    );
    Ok(uid)
}
fn request(stream: &UnixStream) -> Result<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream);
    let mut bytes = Vec::new();
    // Bound the request before deserializing user-controlled socket input.
    use std::io::Read;
    reader
        .by_ref()
        .take(16 * 1024)
        .read_until(b'\n', &mut bytes)?;
    ensure!(bytes.last() == Some(&b'\n'), "TUN 请求过大或未完成");
    Ok(serde_json::from_slice(&bytes)?)
}
fn check_user_file(path: &Path, uid: u32, directory: bool) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        !metadata.file_type().is_symlink()
            && metadata.uid() == uid
            && metadata.mode() & 0o022 == 0
            && if directory {
                metadata.is_dir()
            } else {
                metadata.is_file() && metadata.nlink() == 1
            },
        "TUN 配置必须位于当前用户的私有目录"
    );
    Ok(())
}
fn core_path() -> Result<(PathBuf, String)> {
    let exe = std::env::current_exe()?;
    let parent = exe.parent().context("TUN 助手路径无效")?;
    let sibling = parent.join("resources");
    let resources = if sibling.is_dir() {
        sibling
    } else {
        parent
            .parent()
            .context("安装包路径无效")?
            .join("Resources/resources")
    };
    let core = std::fs::canonicalize(resources.join("mihomo"))?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(resources.join("core.json"))?)?;
    ensure!(
        crate::assets::hash_file(&core)?.0
            == manifest["exe_sha256"]
                .as_str()
                .context("内核校验信息缺失")?,
        "TUN 内核 SHA256 校验失败"
    );
    let expected = manifest["exe_sha256"]
        .as_str()
        .context("内核校验信息缺失")?
        .to_owned();
    Ok((core, expected))
}
fn terminate(child: &mut std::process::Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    let _ = child.wait();
}
fn restore_cache_owner(directory: &Path, uid: u32) {
    // Only the known core cache, never arbitrary paths/symlinks/hardlinks.
    use std::os::fd::AsRawFd;
    for name in ["cache.db", "cache.db-shm", "cache.db-wal"] {
        if let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join(name))
            && let Ok(metadata) = file.metadata()
            && metadata.is_file()
            && metadata.uid() == 0
            && metadata.nlink() == 1
        {
            unsafe {
                libc::fchown(file.as_raw_fd(), uid, u32::MAX);
            }
        }
    }
}
fn run_core(mut stream: UnixStream, request: Request, uid: u32, core: &Path) -> Result<()> {
    let directory = request.directory.context("缺少 TUN 配置目录")?;
    let config = request.config.context("缺少 TUN 配置")?;
    ensure!(
        directory.is_absolute()
            && std::fs::canonicalize(&directory)? == directory
            && config == directory.join("config.yaml"),
        "TUN 配置路径无效"
    );
    check_user_file(&directory, uid, true)?;
    check_user_file(&config, uid, false)?;
    // Use only the installed core and a fixed argument set; no executable/shell
    // supplied by the socket caller. The lease dies when its bridge disconnects.
    let mut child = Command::new(core)
        .args(["-d"])
        .arg(&directory)
        .arg("-f")
        .arg(&config)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let (send, receive) = mpsc::sync_channel::<Option<String>>(64);
    for pipe in [
        Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ] {
        let send = send.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines() {
                if send.send(Some(line.unwrap_or_default())).is_err() {
                    break;
                }
            }
        });
    }
    let monitor = stream.try_clone()?;
    monitor.set_read_timeout(None)?;
    std::thread::spawn(move || {
        use std::io::Read;
        let mut monitor = monitor;
        let mut byte = [0];
        let _ = monitor.read(&mut byte);
        let _ = send.send(None);
    });
    let result = (|| {
        loop {
            match receive.recv_timeout(Duration::from_millis(100)) {
                Ok(Some(line)) => {
                    serde_json::to_writer(
                        &mut stream,
                        &Event {
                            line: Some(line),
                            exit: None,
                        },
                    )?;
                    stream.write_all(b"\n")?;
                }
                Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(status) = child.try_wait()? {
                serde_json::to_writer(
                    &mut stream,
                    &Event {
                        line: None,
                        exit: Some(status.code().unwrap_or(1)),
                    },
                )?;
                stream.write_all(b"\n")?;
                break;
            }
        }
        Ok::<_, anyhow::Error>(())
    })();
    terminate(&mut child);
    let _ = stream.shutdown(std::net::Shutdown::Both);
    restore_cache_owner(&directory, uid);
    result
}
fn serve(socket: &Path, uid: u32, token: &str) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0 && uid != 0 && uuid::Uuid::parse_str(token).is_ok(),
        "TUN 服务身份无效"
    );
    check_user_file(socket.parent().context("TUN 会话路径无效")?, uid, true)?;
    let (source, expected) = core_path()?;
    // Freeze the authorized executable in a root-private directory. A later
    // user-space app replacement must never change what this lease executes.
    let frozen = tempfile::Builder::new()
        .prefix("clash-core-root-")
        .tempdir()?;
    let core = frozen.path().join("mihomo");
    std::fs::copy(&source, &core)?;
    ensure!(
        crate::assets::hash_file(&core)?.0 == expected,
        "授权过程中内核文件发生变化"
    );
    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    let cpath = std::ffi::CString::new(socket.as_os_str().as_encoded_bytes())?;
    ensure!(
        unsafe { libc::chown(cpath.as_ptr(), uid, u32::MAX) } == 0,
        "无法设置 TUN 会话权限"
    );
    listener.set_nonblocking(true)?;
    let mut idle = Instant::now();
    while idle.elapsed() < Duration::from_secs(300) && socket.exists() {
        let (mut stream, _) = match listener.accept() {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        // Darwin accept inherits the listener's nonblocking flag. The request
        // and lease monitor require blocking reads; EAGAIN is not a disconnect.
        stream.set_nonblocking(false)?;
        if peer_uid(&stream)? != uid {
            continue;
        }
        let request = match request(&stream) {
            Ok(value) if value.token == token => value,
            _ => continue,
        };
        idle = Instant::now();
        match request.operation.as_str() {
            "ping" => {
                stream.write_all(b"ready\n")?;
            }
            "stop" => {
                stream.write_all(b"ready\n")?;
                break;
            }
            "start" => {
                if let Err(error) = run_core(stream.try_clone()?, request, uid, &core) {
                    let _ = serde_json::to_writer(
                        &mut stream,
                        &Event {
                            line: Some(format!("TUN 内核启动失败：{error:#}")),
                            exit: Some(1),
                        },
                    );
                    let _ = stream.write_all(b"\n");
                }
            }
            _ => {}
        }
    }
    let _ = std::fs::remove_file(socket);
    Ok(())
}
fn bridge(socket: &Path, token: &str, arguments: &[String]) -> Result<i32> {
    ensure!(
        arguments.len() == 4 && arguments[0] == "-d" && arguments[2] == "-f",
        "TUN 启动参数无效"
    );
    let mut stream =
        UnixStream::connect(socket).context("TUN 授权会话已结束，请重新点击 TUN 模式")?;
    ensure!(peer_uid(&stream)? == 0, "TUN 服务不是管理员进程");
    serde_json::to_writer(
        &mut stream,
        &Request {
            token: token.into(),
            operation: "start".into(),
            directory: Some(arguments[1].clone().into()),
            config: Some(arguments[3].clone().into()),
        },
    )?;
    stream.write_all(b"\n")?;
    for line in BufReader::new(stream).lines() {
        let event: Event = serde_json::from_str(&line?)?;
        if let Some(line) = event.line {
            println!("{line}");
        }
        if let Some(code) = event.exit {
            return Ok(code);
        }
    }
    bail!("TUN 服务意外退出")
}
static USER_STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
extern "C" fn user_stop(_: i32) {
    USER_STOP.store(true, std::sync::atomic::Ordering::Relaxed);
}
pub fn launcher_main() -> Result<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--macos-user-core") if args.len() >= 2 => {
            ensure!(
                unsafe { libc::geteuid() } != 0,
                "普通内核启动器不能以 root 运行"
            );
            unsafe {
                libc::signal(libc::SIGTERM, user_stop as *const () as usize);
            }
            let parent = unsafe { libc::getppid() };
            use std::os::unix::process::CommandExt;
            let mut command = Command::new(&args[1]);
            command.args(&args[2..]);
            unsafe {
                command.pre_exec(|| {
                    libc::signal(libc::SIGTERM, libc::SIG_DFL);
                    Ok(())
                });
            }
            let mut child = command.spawn()?;
            loop {
                if let Some(status) = child.try_wait()? {
                    return Ok(status.code().unwrap_or(1));
                }
                if unsafe { libc::getppid() } != parent
                    || USER_STOP.load(std::sync::atomic::Ordering::Relaxed)
                {
                    terminate(&mut child);
                    return Ok(0);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        Some("--macos-authorize") if args.len() == 4 => {
            ensure!(
                unsafe { libc::geteuid() } == 0,
                "TUN 授权助手需要管理员权限"
            );
            core_path()?;
            let mut service = Command::new(std::env::current_exe()?)
                .arg("--macos-serve")
                .args(&args[1..])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let deadline = Instant::now() + Duration::from_secs(10);
            while !Path::new(&args[1]).exists() {
                ensure!(
                    service.try_wait()?.is_none(),
                    "TUN 服务无法启动，请检查安装文件与会话目录权限"
                );
                if Instant::now() >= deadline {
                    let _ = service.kill();
                    bail!("TUN 服务启动超时");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(0)
        }
        Some("--macos-serve") if args.len() == 4 => {
            serve(Path::new(&args[1]), args[2].parse()?, &args[3])?;
            Ok(0)
        }
        Some("--macos-bridge") if args.len() >= 3 => {
            bridge(Path::new(&args[1]), &args[2], &args[3..])
        }
        _ => bail!("TUN 助手参数无效"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "starts a disposable root core lease on macOS Actions only"]
    fn native_tun_lease() {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
        let resources = PathBuf::from(std::env::var_os("MIHOMO_TEST_RESOURCES").unwrap());
        let launcher = resources.parent().unwrap().join("clash-tun-launcher");
        let directory = tempfile::tempdir().unwrap();
        let directory_path = std::fs::canonicalize(directory.path()).unwrap();
        let socket = directory_path.join("core.sock");
        let token = uuid::Uuid::new_v4().to_string();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let config = directory_path.join("config.yaml");
        crate::config::atomic_write(&config, format!("mixed-port: {port}\nmode: direct\nlog-level: debug\ntun:\n  enable: true\n  stack: system\n  device: utun233\n  auto-route: false\n  auto-detect-interface: true\nrules: ['MATCH,DIRECT']\n").as_bytes()).unwrap();
        let mut server = Command::new("/usr/bin/sudo")
            .args(["-n"])
            .arg(&launcher)
            .arg("--macos-serve")
            .arg(&socket)
            .arg(unsafe { libc::getuid() }.to_string())
            .arg(&token)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !socket.exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut bridge = Command::new(&launcher)
            .arg("--macos-bridge")
            .arg(&socket)
            .arg(&token)
            .arg("-d")
            .arg(&directory_path)
            .arg("-f")
            .arg(&config)
            .spawn()
            .unwrap();
        let ready = (|| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline {
                if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
                    && Command::new("/sbin/ifconfig")
                        .arg("utun233")
                        .output()
                        .is_ok_and(|output| output.status.success())
                {
                    return true;
                }
                if bridge.try_wait().unwrap().is_some() {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            false
        })();
        let _ = bridge.kill();
        let _ = bridge.wait();
        let session = Session { directory, token };
        session.wait_idle().unwrap();
        assert!(
            std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
            "root core survived bridge death"
        );
        assert!(
            !Command::new("/sbin/ifconfig")
                .arg("utun233")
                .output()
                .unwrap()
                .status
                .success(),
            "TUN interface survived bridge death"
        );
        drop(session);
        server.wait().unwrap();
        assert!(ready, "authorized root core did not start its TUN/listener");
    }
}
