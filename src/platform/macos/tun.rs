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
    #[serde(default)]
    ports: Vec<u16>,
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
        let launcher = std::env::current_exe()?
            .parent()
            .context("无法定位 TUN 助手")?
            .join("clash-tun-launcher");
        Self::authorize_with(&launcher, super::administrator)
    }
    fn authorize_with(launcher: &Path, authorize: impl FnOnce(&str) -> Result<()>) -> Result<Self> {
        let directory = tempfile::Builder::new().prefix("clash-tun-").tempdir()?;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        let token = uuid::Uuid::new_v4().to_string();
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
        #[cfg(feature = "ui-preview")]
        let script = if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && std::env::var("CLASH_CI_SHORT_IDLE").as_deref() == Ok("true")
        {
            format!("{script} --ci-short-idle")
        } else {
            script
        };
        authorize(&script)?;
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
                ports: Vec::new(),
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
    pub fn is_available(&self) -> bool {
        // Connect without a ping: an active core owns the service until its
        // lease ends, so waiting for a response would block behind that core.
        UnixStream::connect(self.directory.path().join("core.sock"))
            .is_ok_and(|stream| peer_uid(&stream).is_ok_and(|uid| uid == 0))
    }
    pub fn command(&self, controller: u16, mixed: u16) -> Result<tokio::process::Command> {
        let launcher = std::env::current_exe()?
            .parent()
            .context("无法定位 TUN 助手")?
            .join("clash-tun-launcher");
        let mut command = tokio::process::Command::new(launcher);
        command.args([
            "--macos-bridge",
            &self.directory.path().join("core.sock").to_string_lossy(),
            &self.token,
            "--probe-ports",
            &controller.to_string(),
            &mixed.to_string(),
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
pub(crate) fn probe_port(port: u16) -> std::io::Result<std::net::TcpListener> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let listener = unsafe { std::net::TcpListener::from_raw_fd(fd) };
    if unsafe { libc::fcntl(listener.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let reuse: libc::c_int = 1;
    let address = libc::sockaddr_in {
        sin_len: std::mem::size_of::<libc::sockaddr_in>() as u8,
        sin_family: libc::AF_INET as u8,
        sin_port: port.to_be(),
        sin_addr: libc::in_addr {
            s_addr: u32::from_ne_bytes([127, 0, 0, 1]),
        },
        sin_zero: [0; 8],
    };
    if unsafe {
        libc::setsockopt(
            listener.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            (&reuse as *const libc::c_int).cast(),
            std::mem::size_of_val(&reuse) as libc::socklen_t,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe {
        libc::bind(
            listener.as_raw_fd(),
            (&address as *const libc::sockaddr_in).cast(),
            address.sin_len as libc::socklen_t,
        )
    } != 0
        || unsafe { libc::listen(listener.as_raw_fd(), 1) } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(listener)
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
fn restore_runtime_owner(directory: &std::fs::File, uid: u32) -> Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd};
    fn visit(
        directory: &std::fs::File,
        uid: u32,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<()> {
        ensure!(depth <= 32, "TUN 缓存目录层级过深");
        // Enumerate an already-open directory. Never traverse a path supplied
        // by a cache entry, follow links, or change a multiply-linked file.
        let fd = unsafe { libc::dup(directory.as_raw_fd()) };
        ensure!(fd >= 0, "无法读取 TUN 缓存目录");
        let entries = unsafe { libc::fdopendir(fd) };
        if entries.is_null() {
            unsafe { libc::close(fd) };
            bail!("无法列出 TUN 缓存目录");
        }
        struct Entries(*mut libc::DIR);
        impl Drop for Entries {
            fn drop(&mut self) {
                unsafe { libc::closedir(self.0) };
            }
        }
        let entries = Entries(entries);
        let device = directory.metadata()?.dev();
        loop {
            unsafe { *libc::__error() = 0 };
            let entry = unsafe { libc::readdir(entries.0) };
            if entry.is_null() {
                ensure!(
                    std::io::Error::last_os_error().raw_os_error() == Some(0),
                    "读取 TUN 缓存目录失败：{}",
                    std::io::Error::last_os_error()
                );
                break;
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            ensure!(*remaining > 0, "TUN 缓存文件数量过多");
            *remaining -= 1;
            let flags = libc::O_NOFOLLOW | libc::O_CLOEXEC;
            let mut fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    flags | libc::O_DIRECTORY,
                )
            };
            if fd < 0 {
                // Metadata-only opening avoids device/FIFO I/O side effects.
                fd = unsafe {
                    libc::openat(
                        directory.as_raw_fd(),
                        name.as_ptr(),
                        flags | libc::O_EVTONLY,
                    )
                };
            }
            if fd < 0 {
                continue;
            }
            let file = unsafe { std::fs::File::from_raw_fd(fd) };
            let metadata = file.metadata()?;
            if metadata.dev() != device || (metadata.uid() != 0 && metadata.uid() != uid) {
                continue;
            }
            if metadata.is_dir() {
                visit(&file, uid, depth + 1, remaining)?;
            } else if !metadata.is_file() || metadata.nlink() != 1 {
                continue;
            }
            if metadata.uid() == 0 {
                ensure!(
                    unsafe { libc::fchown(file.as_raw_fd(), uid, u32::MAX) } == 0,
                    "无法恢复 TUN 缓存所有者：{}",
                    std::io::Error::last_os_error()
                );
            }
        }
        Ok(())
    }
    visit(directory, uid, 0, &mut 10_000)
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
    let runtime = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&directory)?;
    ensure!(
        runtime.metadata()?.uid() == uid,
        "TUN 配置目录所有者发生变化"
    );
    ensure!(request.ports.len() <= 2, "TUN 端口参数无效");
    for port in request.ports {
        ensure!(port != 0, "TUN 端口参数无效");
        // Probe as the same identity that will bind the actual listeners.
        // Darwin prevents a user probe from reusing root-owned TIME_WAIT state.
        let _listener = probe_port(port)
            .with_context(|| format!("无法绑定端口 {port}，请检查占用或系统端口限制"))?;
    }
    let monitor = stream.try_clone()?;
    monitor.set_read_timeout(None)?;
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
    restore_runtime_owner(&runtime, uid)?;
    result
}
fn serve(socket: &Path, uid: u32, token: &str, idle_limit: Duration) -> Result<()> {
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
    while idle.elapsed() < idle_limit && socket.exists() {
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
        // A running lease is activity, regardless of how long its core lived.
        // Begin the reusable service's idle grace period after that lease ends.
        idle = Instant::now();
    }
    let _ = std::fs::remove_file(socket);
    Ok(())
}
fn bridge(socket: &Path, token: &str, arguments: &[String]) -> Result<i32> {
    let (ports, arguments) = if arguments.first().map(String::as_str) == Some("--probe-ports") {
        ensure!(arguments.len() == 7, "TUN 端口参数无效");
        (
            vec![arguments[1].parse::<u16>()?, arguments[2].parse::<u16>()?],
            &arguments[3..],
        )
    } else {
        (Vec::new(), arguments)
    };
    ensure!(
        arguments.len() == 4 && arguments[0] == "-d" && arguments[2] == "-f",
        "TUN 启动参数无效"
    );
    // Darwin's /var and /tmp aliases (and custom data directories) may differ
    // from their canonical paths. Send one canonical directory to the service
    // while retaining its no-symlink check on the config file itself.
    let directory = Path::new(&arguments[1]);
    ensure!(
        Path::new(&arguments[3]) == directory.join("config.yaml"),
        "TUN 配置路径无效"
    );
    let directory = std::fs::canonicalize(directory)?;
    let config = directory.join("config.yaml");
    let mut stream =
        UnixStream::connect(socket).context("TUN 授权会话已结束，请重新点击 TUN 模式")?;
    ensure!(peer_uid(&stream)? == 0, "TUN 服务不是管理员进程");
    serde_json::to_writer(
        &mut stream,
        &Request {
            token: token.into(),
            operation: "start".into(),
            directory: Some(directory),
            config: Some(config),
            ports,
        },
    )?;
    stream.write_all(b"\n")?;
    // A GUI crash does not automatically kill its launcher child on macOS.
    // Close the privileged lease when the GUI parent dies or sends SIGTERM.
    unsafe {
        libc::signal(libc::SIGTERM, user_stop as *const () as usize);
    }
    let parent = unsafe { libc::getppid() };
    ensure!(parent > 1, "客户端已退出，已取消 TUN 启动");
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watch_done = done.clone();
    let monitor = stream.try_clone()?;
    std::thread::spawn(move || {
        while !watch_done.load(std::sync::atomic::Ordering::Relaxed) {
            if unsafe { libc::getppid() } != parent
                || USER_STOP.load(std::sync::atomic::Ordering::Relaxed)
            {
                let _ = monitor.shutdown(std::net::Shutdown::Both);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    let result = (|| {
        for line in BufReader::new(stream).lines() {
            let event: Event = serde_json::from_str(&line?)?;
            if let Some(line) = event.line {
                println!("{line}");
            }
            if let Some(code) = event.exit {
                return Ok(code);
            }
        }
        if USER_STOP.load(std::sync::atomic::Ordering::Relaxed)
            || unsafe { libc::getppid() } != parent
        {
            return Ok(0);
        }
        bail!("TUN 服务意外退出")
    })();
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    result
}

static USER_STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
extern "C" fn user_stop(_: i32) {
    USER_STOP.store(true, std::sync::atomic::Ordering::Relaxed);
}
fn session_idle_limit(args: &[String]) -> Result<Duration> {
    if args.len() == 4 {
        return Ok(Duration::from_secs(300));
    }
    #[cfg(feature = "ui-preview")]
    if args.len() == 5 && args[4] == "--ci-short-idle" {
        return Ok(Duration::from_secs(5));
    }
    bail!("TUN 会话参数无效")
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
        Some("--macos-authorize") if args.len() >= 4 => {
            session_idle_limit(&args)?;
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
        Some("--macos-serve") if args.len() >= 4 => {
            serve(
                Path::new(&args[1]),
                args[2].parse()?,
                &args[3],
                session_idle_limit(&args)?,
            )?;
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
        let outside = tempfile::tempdir().unwrap();
        let external_link = outside.path().join("symlink-target");
        let external_hardlink = outside.path().join("hardlink-target");
        for path in [&external_link, &external_hardlink] {
            std::fs::write(path, b"external fixture").unwrap();
        }
        assert!(
            Command::new("/usr/bin/sudo")
                .args(["-n", "chown", "0"])
                .arg(&external_link)
                .arg(&external_hardlink)
                .status()
                .unwrap()
                .success()
        );
        std::os::unix::fs::symlink(outside.path(), directory_path.join("linked-directory"))
            .unwrap();
        std::fs::hard_link(&external_hardlink, directory_path.join("linked-file")).unwrap();
        let runtime_alias = outside.path().join("runtime alias");
        std::os::unix::fs::symlink(&directory_path, &runtime_alias).unwrap();
        // A real HTTP provider makes mihomo create nested root-owned cache
        // files. The ordinary user must be able to update them after TUN stops.
        let provider = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let provider_port = provider.local_addr().unwrap().port();
        provider.set_nonblocking(true).unwrap();
        let provider_server = std::thread::spawn(move || {
            use std::io::Read;
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline {
                if let Ok((mut stream, _)) = provider.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0; 4096];
                    let _ = stream.read(&mut request);
                    let body =
                        "proxies:\n  - {name: fixture, type: http, server: 127.0.0.1, port: 1}\n";
                    return write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .is_ok();
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            false
        });
        let provider_cache = directory_path.join("providers/native.yaml");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let config = directory_path.join("config.yaml");
        crate::config::atomic_write(&config, format!("mixed-port: {port}\nmode: direct\nlog-level: debug\nproxy-providers:\n  native:\n    type: http\n    url: http://127.0.0.1:{provider_port}/nodes.yaml\n    path: ./providers/native.yaml\n    interval: 3600\nproxy-groups:\n  - {{name: fixture, type: select, use: [native]}}\ntun:\n  enable: true\n  stack: system\n  device: utun233\n  auto-route: false\n  auto-detect-interface: true\nrules: ['MATCH,DIRECT']\n").as_bytes()).unwrap();
        let authorized = Session::authorize_with(&launcher, |script| {
            super::super::authorization::without_dialog(&format!(
                "/usr/bin/sudo -n /bin/sh -c {}",
                super::super::shell_quote(script)
            ))
        })
        .unwrap();
        let socket = authorized.directory.path().join("core.sock");
        let token = authorized.token.clone();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !socket.exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(50));
        }
        // The root-side probe must reject a real listener, without starting a
        // core or consuming the reusable authorization session.
        let rejected = Command::new(&launcher)
            .args([
                "--macos-bridge",
                &socket.to_string_lossy(),
                &token,
                "--probe-ports",
                &provider_port.to_string(),
                &port.to_string(),
                "-d",
                &runtime_alias.to_string_lossy(),
                "-f",
                &runtime_alias.join("config.yaml").to_string_lossy(),
            ])
            .output()
            .unwrap();
        assert!(!rejected.status.success());
        assert!(
            String::from_utf8_lossy(&rejected.stdout)
                .contains(&format!("无法绑定端口 {provider_port}"))
        );
        authorized.wait_idle().unwrap();
        let script = format!(
            "{} --macos-bridge {} {} --probe-ports {port} {port} -d {} -f {} & wait",
            super::super::shell_quote(&launcher.to_string_lossy()),
            super::super::shell_quote(&socket.to_string_lossy()),
            super::super::shell_quote(&token),
            super::super::shell_quote(&runtime_alias.to_string_lossy()),
            super::super::shell_quote(&runtime_alias.join("config.yaml").to_string_lossy())
        );
        let mut bridge = Command::new("/bin/sh")
            .args(["-c", &script])
            .spawn()
            .unwrap();
        let ready = (|| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline {
                if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
                    && provider_cache
                        .metadata()
                        .is_ok_and(|metadata| metadata.uid() == 0)
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
        let session = authorized;
        session.wait_idle().unwrap();
        assert!(
            provider_server.join().unwrap(),
            "provider was not downloaded"
        );
        assert_eq!(provider_cache.metadata().unwrap().uid(), unsafe {
            libc::getuid()
        });
        std::fs::write(&provider_cache, b"ordinary user cache update").unwrap();
        assert_eq!(
            external_link.metadata().unwrap().uid(),
            0,
            "followed an external symlink"
        );
        assert_eq!(
            external_hardlink.metadata().unwrap().uid(),
            0,
            "changed an external hardlink"
        );
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
        assert!(session.is_available());
        session.request("stop").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.is_available() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            !session.is_available(),
            "stopped service was still reusable"
        );
        assert!(ready, "authorized root core did not start its TUN/listener");
    }
}
