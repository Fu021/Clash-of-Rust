//! A per-user, root-owned launchd broker for fixed proxy and TUN operations.
//! Installation is the only interactive operation. No caller-supplied executable.
use crate::platform::ProxyState;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const PROTOCOL: u32 = 8;
const PREFIX: &str = "org.clashofrust.network";
const LIMIT: u64 = 256 * 1024;
static STOP: AtomicBool = AtomicBool::new(false);

#[derive(Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Status {},
    Dns {
        enabled: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
    },
    DnsCheck {
        port: u16,
    },
    Adopt {
        original: ProxyState,
        owned: ProxyState,
    },
    Proxy {
        state: ProxyState,
        restore: bool,
    },
    Tun {
        socket: PathBuf,
        token: String,
        short_idle: bool,
    },
}

#[derive(Deserialize, Serialize)]
struct Response {
    protocol: u32,
    core_sha256: String,
    error: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct ProxyLease {
    original: ProxyState,
    owned: ProxyState,
    owner: u32,
}

fn adopt_lease(
    lease: &mut Option<ProxyLease>,
    original: ProxyState,
    owned: ProxyState,
    owner: u32,
) {
    if let Some(current) = lease {
        current.owned = owned;
        current.owner = owner;
    } else {
        *lease = Some(ProxyLease {
            original,
            owned,
            owner,
        });
    }
}

fn peer_pid(stream: &UnixStream) -> Result<u32> {
    use std::os::fd::AsRawFd;
    let mut pid: libc::pid_t = 0;
    let mut length = std::mem::size_of_val(&pid) as libc::socklen_t;
    ensure!(
        unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_LOCAL,
                libc::LOCAL_PEERPID,
                (&mut pid as *mut libc::pid_t).cast(),
                &mut length,
            )
        } == 0
            && pid > 1,
        "无法验证网络助手调用进程"
    );
    Ok(pid as u32)
}

fn save_lease(path: &Path, lease: &ProxyLease) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("代理恢复路径无效")?)?;
    serde_json::to_writer(&mut file, lease)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn restore_lease(path: &Path, lease: &mut Option<ProxyLease>) -> Result<()> {
    if let Some(owned) = lease {
        super::native_proxy::restore(&owned.original, &owned.owned)?;
        fs::remove_file(path)?;
        *lease = None;
    }
    Ok(())
}

fn accepts_protocol(request: &Request, protocol: u32) -> bool {
    protocol == PROTOCOL
        || (matches!(protocol, 2..=7)
            && matches!(
                request,
                Request::Proxy { restore: true, .. } | Request::Dns { enabled: false, .. }
            ))
}

fn paths(uid: u32) -> (String, PathBuf, PathBuf) {
    let label = format!("{PREFIX}.{uid}");
    let directory = PathBuf::from(format!("/Library/PrivilegedHelperTools/{label}"));
    let plist = PathBuf::from(format!("/Library/LaunchDaemons/{label}.plist"));
    (label, directory, plist)
}

fn socket_path(uid: u32) -> PathBuf {
    paths(uid).1.join("network.sock")
}

fn peer_uid(stream: &UnixStream) -> Result<u32> {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (0, 0);
    ensure!(
        unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0,
        "无法验证网络助手身份"
    );
    Ok(uid)
}

fn exchange(request: &Request) -> Result<Response> {
    let uid = unsafe { libc::getuid() };
    let mut stream = UnixStream::connect(socket_path(uid)).context("网络助手尚未启动")?;
    ensure!(peer_uid(&stream)? == 0, "网络助手不是管理员进程");
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    serde_json::to_writer(&mut stream, request)?;
    stream.write_all(b"\n")?;
    let mut bytes = Vec::new();
    BufReader::new(stream)
        .take(LIMIT)
        .read_until(b'\n', &mut bytes)?;
    ensure!(bytes.last() == Some(&b'\n'), "网络助手响应无效");
    let response: Response = serde_json::from_slice(&bytes)?;
    if let Some(error) = &response.error {
        bail!("{error}");
    }
    ensure!(
        accepts_protocol(request, response.protocol),
        "网络助手版本不兼容，请重新授权安装"
    );
    Ok(response)
}

fn bundled_core() -> Result<(PathBuf, String)> {
    super::tun::core_path()
}

pub fn ensure_available(interactive: bool) -> Result<()> {
    let (_, expected) = bundled_core()?;
    if exchange(&Request::Status {}).is_ok_and(|status| status.core_sha256 == expected) {
        return Ok(());
    }
    // launchd may still be restoring the installed job during login.
    if paths(unsafe { libc::getuid() }).1.is_dir() {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if exchange(&Request::Status {}).is_ok_and(|status| status.core_sha256 == expected) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    ensure!(
        interactive,
        "网络助手需要首次授权或修复，请打开客户端并重新选择系统代理或 TUN 模式"
    );
    let launcher = std::env::current_exe()?
        .parent()
        .context("无法定位网络助手")?
        .join("clash-tun-launcher");
    ensure!(
        launcher.is_file(),
        "网络助手缺失，请使用完整的 macOS 安装包"
    );
    super::administrator(&format!(
        "{} --macos-install-service {}",
        super::shell_quote(&launcher.to_string_lossy()),
        unsafe { libc::getuid() }
    ))?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let status = exchange(&Request::Status {});
        let reason = match status {
            Ok(status) if status.core_sha256 == expected => return Ok(()),
            Ok(_) => "助手内核快照与当前应用不一致".into(),
            Err(error) => format!("{error:#}"),
        };
        ensure!(
            Instant::now() < deadline,
            "网络助手安装后未就绪：{reason}；请检查系统日志"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn write_proxy(state: &ProxyState) -> Result<()> {
    let interactive = !std::env::args_os().any(|argument| argument == "--background");
    ensure_available(interactive)?;
    exchange(&Request::Proxy {
        state: state.clone(),
        restore: false,
    })?;
    Ok(())
}

pub fn restore_proxy(state: &ProxyState) -> Result<()> {
    // Cleanup must never install a service or open a dialog, including logout.
    exchange(&Request::Proxy {
        state: state.clone(),
        restore: true,
    })?;
    Ok(())
}

pub fn adopt_proxy(original: &ProxyState, owned: &ProxyState) -> Result<()> {
    exchange(&Request::Adopt {
        original: original.clone(),
        owned: owned.clone(),
    })?;
    Ok(())
}

pub fn open_tun(socket: &Path, token: &str) -> Result<()> {
    ensure_available(true)?;
    exchange(&Request::Tun {
        socket: socket.into(),
        token: token.into(),
        // Preserve the existing helper protocol without accepting the removed
        // diagnostic idle override.
        short_idle: false,
    })?;
    Ok(())
}

pub fn protection_dns(enabled: bool) -> Result<()> {
    ensure!(!enabled, "开启保护 DNS 必须指定本机监听端口");
    protection_dns_at(None)
}

pub fn protection_dns_at(port: Option<u16>) -> Result<()> {
    // No prompting or helper installation while cleaning up (including old
    // protocol clients which never took ownership of DNS in the first place).
    if port.is_none()
        && !paths(unsafe { libc::getuid() })
            .1
            .join("dns-lease.json")
            .exists()
    {
        return Ok(());
    }
    exchange(&Request::Dns {
        enabled: port.is_some(),
        port,
    })?;
    Ok(())
}

pub fn check_protection_dns(port: u16) -> Result<()> {
    exchange(&Request::DnsCheck { port })?;
    Ok(())
}

fn root_owned(path: &Path, directory: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        !metadata.file_type().is_symlink()
            && metadata.uid() == 0
            && metadata.mode() & 0o022 == 0
            && if directory {
                metadata.is_dir()
            } else {
                metadata.is_file() && metadata.nlink() == 1
            },
        "网络助手安装路径必须由 root 持有且不可由普通用户修改：{}",
        path.display()
    );
    Ok(())
}

fn checked(command: &mut Command, action: &str) -> Result<()> {
    let output = super::run_command(command, action, Duration::from_secs(20))?;
    ensure!(
        output.status.success(),
        "{action}失败：{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

fn copy_contents(source: &Path, destination: &Path) -> Result<()> {
    // Darwin copyfile/STAT can retain a user's uid. Create a new root-owned
    // inode and copy bytes only; never inherit source ownership or ACLs.
    let mut source = fs::File::open(source)?;
    let mut destination = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)?;
    std::io::copy(&mut source, &mut destination)?;
    destination.sync_all()?;
    Ok(())
}

fn plist(label: &str, launcher: &Path, uid: u32) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>Label</key><string>{label}</string><key>ProgramArguments</key><array><string>{}</string><string>--macos-network-service</string><string>{uid}</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><true/><key>ThrottleInterval</key><integer>3</integer><key>ExitTimeOut</key><integer>10</integer><key>Umask</key><integer>18</integer><key>StandardErrorPath</key><string>{}/network-service.log</string></dict></plist>\n",
        super::cf::escape(&launcher.to_string_lossy()),
        super::cf::escape(
            &launcher
                .parent()
                .expect("installed helper directory")
                .to_string_lossy()
        )
    )
}

pub fn install(uid: u32) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0 && uid != 0,
        "网络助手安装需要管理员权限及普通用户身份"
    );
    let (label, destination, agent) = paths(uid);
    for parent in [
        Path::new("/Library"),
        Path::new("/Library/PrivilegedHelperTools"),
        Path::new("/Library/LaunchDaemons"),
    ] {
        if !parent.exists() {
            fs::create_dir(parent)?;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o755))?;
        }
        root_owned(parent, true)?;
    }
    if destination.exists() {
        root_owned(&destination, true)?;
    }
    if agent.exists() {
        root_owned(&agent, false)?;
    }
    let executable = std::env::current_exe()?;
    let (core, expected) = bundled_core()?;
    let candidate = tempfile::Builder::new()
        .prefix(&format!(".{label}-"))
        .tempdir_in("/Library/PrivilegedHelperTools")?;
    fs::set_permissions(candidate.path(), fs::Permissions::from_mode(0o755))?;
    let launcher = candidate.path().join("clash-tun-launcher");
    let resources = candidate.path().join("resources");
    fs::create_dir(&resources)?;
    fs::set_permissions(&resources, fs::Permissions::from_mode(0o755))?;
    for (source, target) in [
        (&executable, &launcher),
        (&core, &resources.join("mihomo")),
        (
            &core.parent().context("内核目录无效")?.join("core.json"),
            &resources.join("core.json"),
        ),
    ] {
        copy_contents(source, target)?;
    }
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755))?;
    fs::set_permissions(resources.join("mihomo"), fs::Permissions::from_mode(0o755))?;
    fs::set_permissions(
        resources.join("core.json"),
        fs::Permissions::from_mode(0o644),
    )?;
    for path in [
        &launcher,
        &resources.join("mihomo"),
        &resources.join("core.json"),
    ] {
        root_owned(path, false)?;
    }
    ensure!(
        crate::assets::hash_file(&resources.join("mihomo"))?.0 == expected,
        "安装网络助手期间内核发生变化"
    );
    ensure!(
        crate::assets::hash_file(&launcher)?.0 == crate::assets::hash_file(&executable)?.0,
        "安装网络助手期间程序发生变化"
    );
    checked(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict"])
            .arg(&launcher),
        "校验网络助手签名",
    )?;
    let _ = super::run_command(
        Command::new("/bin/launchctl").args(["bootout", &format!("system/{label}")]),
        "停止旧网络助手",
        Duration::from_secs(15),
    );
    if destination.exists() {
        fs::remove_dir_all(&destination)?;
    }
    fs::rename(candidate.path(), &destination)?;
    let contents = plist(&label, &destination.join("clash-tun-launcher"), uid);
    let staged = tempfile::NamedTempFile::new_in("/Library/LaunchDaemons")?;
    fs::write(staged.path(), contents)?;
    fs::set_permissions(staged.path(), fs::Permissions::from_mode(0o644))?;
    staged.persist(&agent).map_err(|error| error.error)?;
    checked(
        Command::new("/bin/launchctl")
            .args(["bootstrap", "system"])
            .arg(&agent),
        "注册网络助手",
    )
}

extern "C" fn stop(_: i32) {
    STOP.store(true, Ordering::Relaxed);
}

pub fn serve(uid: u32) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0 && uid != 0,
        "网络助手服务身份无效"
    );
    let (_, directory, _) = paths(uid);
    root_owned(&directory, true)?;
    root_owned(&directory.join("resources"), true)?;
    let executable = std::env::current_exe()?;
    eprintln!("网络助手启动：{}", executable.display());
    ensure!(
        executable.canonicalize()? == directory.join("clash-tun-launcher").canonicalize()?,
        "请使用已安装的网络助手"
    );
    for path in [
        &executable,
        &directory.join("resources/mihomo"),
        &directory.join("resources/core.json"),
    ] {
        root_owned(path, false)?;
    }
    let (_, core_sha256) = bundled_core()?;
    let journal = directory.join("proxy-lease.json");
    let dns_journal = directory.join("dns-lease.json");
    if dns_journal.exists() {
        root_owned(&dns_journal, false)?;
    }
    let mut dns_lease = super::dns::load(&dns_journal)?;
    let mut dns_changes = super::dns::Changes::new()?;
    if dns_lease
        .as_ref()
        .is_some_and(super::dns::Lease::owner_exited)
    {
        super::dns::restore(&dns_journal, &mut dns_lease)?;
    }
    let mut lease: Option<ProxyLease> = if journal.exists() {
        root_owned(&journal, false)?;
        let mut bytes = Vec::new();
        fs::File::open(&journal)?
            .take(LIMIT)
            .read_to_end(&mut bytes)?;
        Some(serde_json::from_slice(&bytes)?)
    } else {
        None
    };
    // Preserve a live GUI's lease when launchd restarts only the helper.
    // Recover settings when the owning GUI has already exited.
    if lease
        .as_ref()
        .is_some_and(|owned| unsafe { libc::kill(owned.owner as i32, 0) } != 0)
    {
        restore_lease(&journal, &mut lease)?;
    }
    let socket = socket_path(uid);
    if socket.exists() {
        fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    let path = std::ffi::CString::new(socket.as_os_str().as_encoded_bytes())?;
    ensure!(
        unsafe { libc::chown(path.as_ptr(), uid, u32::MAX) } == 0,
        "无法设置网络助手通信权限"
    );
    listener.set_nonblocking(true)?;
    unsafe {
        libc::signal(libc::SIGTERM, stop as *const () as usize);
    }
    let mut children: Vec<Child> = Vec::new();
    let mut dns_error: Option<String> = None;
    let mut dns_native_ready = false;
    let mut dns_relay = match dns_lease.as_ref().filter(|lease| lease.port() >= 1024) {
        Some(lease) => match super::dns::Relay::start(lease.port()) {
            Ok(relay) => Some(relay),
            Err(error) => {
                dns_error = Some(format!("保护 DNS 转发恢复失败：{error:#}"));
                None
            }
        },
        None => None,
    };
    let mut dns_refresh = Instant::now();
    let result = (|| -> Result<()> {
        while !STOP.load(Ordering::Relaxed) {
            if dns_lease
                .as_ref()
                .is_some_and(super::dns::Lease::owner_exited)
            {
                let _ = super::dns::restore(&dns_journal, &mut dns_lease);
            }
            if dns_lease.is_none() {
                dns_relay = None;
                dns_native_ready = false;
            }
            let relay_failed = dns_lease.as_ref().is_some_and(|lease| {
                lease.port() >= 1024
                    && !dns_relay
                        .as_ref()
                        .is_some_and(|relay| relay.serving(lease.port()))
            });
            if dns_lease.is_some()
                && (dns_changes.changed()
                    || ((dns_error.is_some() || relay_failed)
                        && dns_refresh.elapsed() >= Duration::from_secs(5)))
            {
                dns_refresh = Instant::now();
                dns_native_ready = false;
                // The same helper thread serializes changes, journal writes
                // and user requests. No competing DNS recovery worker exists.
                let refreshed = (|| -> Result<()> {
                    let port = dns_lease.as_ref().unwrap().port();
                    if port >= 1024 && !dns_relay.as_ref().is_some_and(|relay| relay.serving(port))
                    {
                        dns_relay = None;
                        dns_relay = Some(super::dns::Relay::start(port)?);
                    }
                    let previous_scopes = super::dns::protected_scopes(&dns_lease);
                    let refreshed = super::dns::refresh(&dns_journal, &mut dns_lease);
                    let mut scopes = previous_scopes?;
                    scopes.extend(super::dns::protected_scopes(&dns_lease)?);
                    scopes.sort_unstable();
                    scopes.dedup();
                    // Dynamic-store publication can precede mDNSResponder's
                    // actual switch; a default-path probe alone says nothing
                    // about a conflicting or recently removed split resolver.
                    dns_relay
                        .as_ref()
                        .context("保护 DNS 本机转发不可用")?
                        .verify_native_scopes(&scopes)?;
                    refreshed?;
                    super::dns::verify(&dns_lease, port)?;
                    dns_native_ready = true;
                    Ok(())
                })();
                match refreshed {
                    Ok(()) => dns_error = None,
                    Err(error) => {
                        let message = format!("保护 DNS 自动更新失败：{error:#}");
                        if dns_error.as_ref() != Some(&message) {
                            eprintln!("{message}");
                        }
                        // Retain the local DNS overlay on failure. Restoring the
                        // VPN default here would silently disable protection.
                        dns_error = Some(message);
                    }
                }
            }
            if lease
                .as_ref()
                .is_some_and(|owned| unsafe { libc::kill(owned.owner as i32, 0) } != 0)
            {
                let _ = restore_lease(&journal, &mut lease);
            }
            children.retain_mut(|child| child.try_wait().ok().flatten().is_none());
            let (mut stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            stream.set_nonblocking(false)?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            stream.set_write_timeout(Some(Duration::from_secs(5)))?;
            if peer_uid(&stream)? != uid {
                continue;
            }
            let result = (|| -> Result<()> {
                let mut bytes = Vec::new();
                BufReader::new(&stream)
                    .take(LIMIT)
                    .read_until(b'\n', &mut bytes)?;
                ensure!(bytes.last() == Some(&b'\n'), "网络助手请求过大或未完成");
                match serde_json::from_slice::<Request>(&bytes)? {
                    Request::Status {} => {}
                    Request::Dns { enabled, port } => {
                        let caller = peer_pid(&stream)?;
                        if enabled {
                            let port = port.context("保护 DNS 本机端口缺失")?;
                            ensure!(port >= 1024, "保护 DNS 本机端口无效");
                            if !dns_relay.as_ref().is_some_and(|relay| relay.serving(port)) {
                                ensure!(
                                    dns_relay.is_none(),
                                    "保护 DNS 转发异常，请先关闭保护后重试"
                                );
                                dns_relay = Some(super::dns::Relay::start(port)?);
                            }
                            dns_native_ready = false;
                            let enabled = super::dns::enable(
                                &dns_journal,
                                &mut dns_lease,
                                caller,
                                uid,
                                port,
                                dns_relay.as_ref().unwrap(),
                            );
                            if dns_lease.is_none() {
                                dns_relay = None;
                            }
                            enabled?;
                            dns_native_ready = true;
                        } else {
                            ensure!(
                                dns_lease
                                    .as_ref()
                                    .is_none_or(|lease| lease.owned_by(caller)),
                                "DNS 正由其他客户端持有"
                            );
                            super::dns::restore(&dns_journal, &mut dns_lease)?;
                            dns_relay = None;
                            dns_native_ready = false;
                        }
                        dns_error = None;
                    }
                    Request::DnsCheck { port } => {
                        ensure!(
                            dns_relay.as_ref().is_some_and(|relay| relay.serving(port)),
                            "保护 DNS 本机转发不可用"
                        );
                        if let Some(error) = &dns_error {
                            bail!("{error}");
                        }
                        ensure!(dns_native_ready, "系统原生 DNS 尚未进入保护入口");
                        super::dns::verify(&dns_lease, port)?;
                    }
                    Request::Adopt { original, owned } => {
                        ensure!(
                            super::proxy_preferences::matches_managed(
                                &super::native_proxy::read()?,
                                &owned
                            )?,
                            "系统代理已被其他程序修改，请重新选择代理模式"
                        );
                        adopt_lease(&mut lease, original, owned, peer_pid(&stream)?);
                        save_lease(&journal, lease.as_ref().unwrap())?;
                    }
                    Request::Proxy { state, restore } => {
                        if restore {
                            if lease.is_some() {
                                restore_lease(&journal, &mut lease)?;
                            } else if !state.is_empty() {
                                super::native_proxy::write(&state)?;
                            }
                            return Ok(());
                        }
                        if lease.is_none() {
                            lease = Some(ProxyLease {
                                original: super::native_proxy::read()?,
                                owned: state.clone(),
                                owner: peer_pid(&stream)?,
                            });
                        }
                        if let Some(owned) = &mut lease {
                            owned.owned = state.clone();
                            owned.owner = peer_pid(&stream)?;
                            save_lease(&journal, owned)?;
                        }
                        super::native_proxy::write(&state)?;
                        if lease.as_ref().is_some_and(|owned| owned.original == state) {
                            fs::remove_file(&journal).or_else(|error| {
                                if error.kind() == std::io::ErrorKind::NotFound {
                                    Ok(())
                                } else {
                                    Err(error)
                                }
                            })?;
                            lease = None;
                        }
                    }
                    Request::Tun {
                        socket,
                        token,
                        short_idle,
                    } => {
                        ensure!(
                            children.len() < 16 && uuid::Uuid::parse_str(&token).is_ok(),
                            "TUN 会话参数无效"
                        );
                        super::tun::validate_session_path(&socket, uid)?;
                        let mut command = Command::new(&executable);
                        command
                            .arg("--macos-serve")
                            .arg(&socket)
                            .arg(uid.to_string())
                            .arg(token)
                            .env_clear()
                            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null());
                        ensure!(!short_idle, "TUN 会话参数无效");
                        children.push(command.spawn()?);
                    }
                }
                Ok(())
            })();
            let _ = serde_json::to_writer(
                &mut stream,
                &Response {
                    protocol: PROTOCOL,
                    core_sha256: core_sha256.clone(),
                    error: result.err().map(|error| format!("{error:#}")),
                },
            );
            let _ = stream.write_all(b"\n");
        }
        Ok(())
    })();
    let dns_restoration = super::dns::restore(&dns_journal, &mut dns_lease);
    let restoration = restore_lease(&journal, &mut lease);
    for mut child in children {
        super::tun::terminate(&mut child);
    }
    let _ = fs::remove_file(socket);
    result.and(dns_restoration).and(restoration)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn older_helpers_can_clean_up_but_must_be_updated_before_new_operations() {
        assert_eq!(
            serde_json::to_value(Request::Dns {
                enabled: false,
                port: None
            })
            .unwrap(),
            serde_json::json!({"operation":"dns","enabled":false}),
            "Old helpers reject unknown fields even during cleanup",
        );
        assert!(accepts_protocol(&Request::Status {}, PROTOCOL));
        for protocol in [2, 3, 4, 5, 6, 7] {
            assert!(!accepts_protocol(&Request::Status {}, protocol));
            assert!(!accepts_protocol(
                &Request::Dns {
                    enabled: true,
                    port: Some(25353)
                },
                protocol
            ));
            assert!(accepts_protocol(
                &Request::Dns {
                    enabled: false,
                    port: None
                },
                protocol
            ));
            assert!(accepts_protocol(
                &Request::Proxy {
                    state: ProxyState::new(),
                    restore: true,
                },
                protocol
            ));
            assert!(!accepts_protocol(
                &Request::Proxy {
                    state: ProxyState::new(),
                    restore: false,
                },
                protocol
            ));
        }
        assert!(!accepts_protocol(
            &Request::Dns {
                enabled: false,
                port: None
            },
            1
        ));
    }
    #[test]
    fn copying_helper_contents_creates_a_private_inode_without_source_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let destination = directory.path().join("destination");
        fs::write(&source, b"helper bytes").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o777)).unwrap();
        copy_contents(&source, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"helper bytes");
        assert_eq!(destination.metadata().unwrap().uid(), unsafe {
            libc::geteuid()
        });
        assert_eq!(destination.metadata().unwrap().mode() & 0o777, 0o600);
        assert_ne!(
            source.metadata().unwrap().ino(),
            destination.metadata().unwrap().ino()
        );
        assert!(
            copy_contents(&source, &destination).is_err(),
            "overwrote an existing helper target"
        );
    }

    #[test]
    fn proxy_adoption_moves_ownership_without_losing_the_original_settings() {
        use crate::platform::ProxyValue;
        let original = ProxyState::from([("original".into(), None)]);
        let owned = ProxyState::from([("owned".into(), Some(ProxyValue::Text("first".into())))]);
        let mut lease = None;
        adopt_lease(&mut lease, original.clone(), owned, 100);
        let next = ProxyState::from([("owned".into(), Some(ProxyValue::Text("second".into())))]);
        adopt_lease(&mut lease, ProxyState::new(), next.clone(), 200);
        let lease = lease.unwrap();
        assert_eq!(lease.original, original);
        assert_eq!(lease.owned, next);
        assert_eq!(lease.owner, 200);
    }
    #[test]
    fn service_protocol_does_not_accept_executables_or_shell_commands() {
        for data in [
            r#"{"operation":"execute","command":"id"}"#,
            r#"{"operation":"status","executable":"/tmp/untrusted"}"#,
        ] {
            assert!(serde_json::from_str::<Request>(data).is_err());
        }
        assert!(serde_json::from_str::<Request>(r#"{"operation":"status"}"#).is_ok());
    }
    #[test]
    fn installed_service_has_fixed_root_paths_and_no_user_loader_environment() {
        let (label, directory, agent) = paths(501);
        assert_eq!(label, "org.clashofrust.network.501");
        assert!(agent.starts_with("/Library/LaunchDaemons"));
        let text = plist(&label, &directory.join("clash-tun-launcher"), 501);
        assert!(text.contains("--macos-network-service"));
        assert!(!text.contains("EnvironmentVariables"));
        assert!(!text.contains("/Applications"));
    }
}
