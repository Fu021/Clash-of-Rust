//! Included in core_integration; executed only by the temporary disposable workflow.
use super::*;
use clash_of_rust::{
    claude_check::{self as check, Evidence, Group, Report, Signal},
    engine::ProxyMode,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[cfg(target_os = "linux")]
async fn visible_linux_dialog(title: &str) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let found = tokio::process::Command::new("xdotool")
            .args([
                "search",
                "--onlyvisible",
                "--name",
                &format!("^{}$", regex::escape(title)),
            ])
            .output()
            .await
            .unwrap();
        if found.status.success() {
            let window = String::from_utf8(found.stdout)
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .to_owned();
            let pid = tokio::process::Command::new("xdotool")
                .args(["getwindowpid", &window])
                .output()
                .await
                .unwrap();
            assert!(pid.status.success());
            let pid: u32 = String::from_utf8(pid.stdout)
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
            assert!(
                status.lines().any(|line| line
                    .strip_prefix("PPid:")
                    .is_some_and(|value| value.trim() == std::process::id().to_string())),
                "native warning must belong to this test process"
            );
            return window;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "native Linux warning did not become visible"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn route_probe(
    engine: &Engine,
    target: &str,
    host: &str,
    tls: bool,
) -> (tokio::process::Child, String) {
    let mut command =
        tokio::process::Command::new(std::env::var_os("CLASH_CLAUDE_TEST_PYTHON").unwrap());
    command
        .arg(std::env::var_os("CLASH_CLAUDE_TEST_SCRIPT").unwrap())
        .args([
            "route-probe",
            "--mixed",
            &engine.settings.mixed_port.to_string(),
            "--target",
            target,
            "--host",
            host,
            "--port",
            if tls { "443" } else { "80" },
        ]);
    if tls {
        command.arg("--tls");
    }
    let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut reader = tokio::io::BufReader::new(child.stdout.take().unwrap());
    let mut ready = String::new();
    use tokio::io::AsyncBufReadExt;
    tokio::time::timeout(Duration::from_secs(12), reader.read_line(&mut ready))
        .await
        .expect("cached-IP/SNI route probe timed out")
        .unwrap();
    assert!(
        ready.contains("\"ready\": true"),
        "cached-IP/SNI route probe failed for {target}/{host}"
    );
    let live: serde_json::Value = engine.api.get("connections").await.unwrap();
    let connection = live["connections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|connection| {
            let metadata = &connection["metadata"];
            let identified = if target == "203.0.113.77" {
                metadata["sniffHost"].as_str() == Some(host)
            } else {
                metadata["host"].as_str() == Some(target)
                    && metadata["sniffHost"].as_str().is_none_or(str::is_empty)
            };
            identified
                && connection["chains"].as_array().is_some_and(|chain| {
                    chain.iter().any(|v| v == "FixtureA") && chain.iter().all(|v| v != "DIRECT")
                })
        })
        .unwrap_or_else(|| panic!("cached-IP/SNI request did not use the protection node: {live}"));
    (child, connection["id"].as_str().unwrap().to_owned())
}

struct Servers {
    child: tokio::process::Child,
    http: u16,
    https: u16,
}
impl Servers {
    async fn start() -> Self {
        let mut child =
            tokio::process::Command::new(std::env::var_os("CLASH_CLAUDE_TEST_PYTHON").unwrap())
                .arg(std::env::var_os("CLASH_CLAUDE_TEST_SCRIPT").unwrap())
                .arg("serve")
                .arg("--cert")
                .arg(std::env::var_os("CLASH_CLAUDE_TEST_CERT").unwrap())
                .arg("--key")
                .arg(std::env::var_os("CLASH_CLAUDE_TEST_KEY").unwrap())
                .stdout(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
        let mut line = String::new();
        let mut reader = tokio::io::BufReader::new(child.stdout.take().unwrap());
        use tokio::io::AsyncBufReadExt;
        tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        let ready: serde_json::Value = serde_json::from_str(&line).unwrap();
        Self {
            child,
            http: ready["http"].as_u64().unwrap() as u16,
            https: ready["https"].as_u64().unwrap() as u16,
        }
    }
}
struct Socks {
    task: tokio::task::JoinHandle<()>,
    port: u16,
    doh: Arc<AtomicU64>,
    udp: Arc<AtomicU64>,
}
impl Drop for Socks {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Socks {
    async fn start(http: u16, https: u16) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let doh = Arc::new(AtomicU64::new(0));
        let counter = doh.clone();
        let udp = Arc::new(AtomicU64::new(0));
        let udp_counter = udp.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let counter = counter.clone();
                let udp_counter = udp_counter.clone();
                tokio::spawn(async move {
                    let result = async {
                        let mut hello = [0u8; 2];
                        stream.read_exact(&mut hello).await?;
                        anyhow::ensure!(hello[0] == 5, "SOCKS version");
                        let mut methods = vec![0; hello[1] as usize];
                        stream.read_exact(&mut methods).await?;
                        stream.write_all(&[5, 0]).await?;
                        let mut request = [0; 4];
                        stream.read_exact(&mut request).await?;
                        anyhow::ensure!(matches!(request[1], 1 | 3), "SOCKS command");
                        let host = match request[3] {
                            1 => {
                                let mut ip = [0; 4];
                                stream.read_exact(&mut ip).await?;
                                std::net::Ipv4Addr::from(ip).to_string()
                            }
                            4 => {
                                let mut ip = [0; 16];
                                stream.read_exact(&mut ip).await?;
                                std::net::Ipv6Addr::from(ip).to_string()
                            }
                            3 => {
                                let size = stream.read_u8().await?;
                                let mut name = vec![0; size as usize];
                                stream.read_exact(&mut name).await?;
                                String::from_utf8(name)?
                            }
                            _ => anyhow::bail!("SOCKS address"),
                        };
                        let port = stream.read_u16().await?;
                        if request[1] == 3 {
                            let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
                            let port = socket.local_addr()?.port();
                            let mut reply = vec![5, 0, 0, 1, 127, 0, 0, 1];
                            reply.extend_from_slice(&port.to_be_bytes());
                            stream.write_all(&reply).await?;
                            let mut datagram = [0; 4096];
                            let mut closed = [0; 1];
                            loop {
                                tokio::select! {
                                    _ = stream.read(&mut closed) => break,
                                    packet = socket.recv_from(&mut datagram) => {
                                        let (length, source) = packet?;
                                        udp_counter.fetch_add(1, Ordering::Relaxed);
                                        socket.send_to(&datagram[..length], source).await?;
                                    }
                                }
                            }
                            return Ok(());
                        }
                        if ["1.1.1.1", "8.8.8.8"].contains(&host.as_str()) {
                            counter.fetch_add(1, Ordering::Relaxed);
                        }
                        let mut target = tokio::net::TcpStream::connect((
                            "127.0.0.1",
                            match port {
                                443 => https,
                                80 => http,
                                _ => port,
                            },
                        ))
                        .await?;
                        stream.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await?;
                        tokio::io::copy_bidirectional(&mut stream, &mut target).await?;
                        Ok::<_, anyhow::Error>(())
                    }
                    .await;
                    if let Err(error) = result {
                        eprintln!("controlled relay: {error}");
                    }
                });
            }
        });
        Self {
            task,
            port,
            doh,
            udp,
        }
    }
}
async fn capture(engine: &mut Engine, label: &str) -> Report {
    let client = detector_client(engine.settings.mixed_port);
    let mut report = Report::new(
        check::Environment::Core,
        &format!(
            "临时工作流：{label}；本地受控 HTTPS 响应用于验证程序，不代表真实订阅、出口地区或账号资格"
        ),
    );
    check::network(
        &client,
        &mut report,
        "受控 HTTPS 测试服务；生产检测函数经真实 mihomo 请求",
    )
    .await;
    report
        .signals
        .extend(check::enhanced::configuration(&engine.api, &engine.settings).await);
    check::enhanced::extend(&client, &mut report, None).await;
    if let Ok(routes) = engine
        .api
        .get::<check::ObservedConnections>("connections")
        .await
    {
        report.routes = routes.connections;
    }
    if let Some(guard) = engine.poll(Scope::Claude).await.unwrap().claude_guard {
        let mut signal = Signal::new(
            Group::Desktop,
            "实时保护与异常拦截",
            format!(
                "已关闭 {} 条异常连接 · 提示请求 {} 次",
                guard.blocked, guard.alerts
            ),
            "真实内核连接删除与系统通知调用",
            Evidence::Measured,
            "仅受控测试环境；没有读取账号或 HTTPS 请求内容。正常连接与无关服务同时保留并验证。",
        );
        signal.risk = guard.blocked > 0;
        report.signals.push(signal);
        report.routes.extend(
            guard
                .routes
                .into_iter()
                .filter(|r| r.outcome.starts_with("异常连接")),
        );
        report.routes.truncate(32);
    }
    report.validate().unwrap();
    report
}
fn detector_client(port: u16) -> reqwest::Client {
    let ca = std::fs::read(std::env::var_os("CLASH_CLAUDE_TEST_CA").unwrap()).unwrap();
    reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}")).unwrap())
        .add_root_certificate(reqwest::Certificate::from_pem(&ca).unwrap())
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}
async fn hold(client: &reqwest::Client, host: &str, port: u16) -> reqwest::Response {
    client
        .get(format!("http://{host}:{port}/hold"))
        .send()
        .await
        .unwrap()
}

fn udp_packet(host: &str, port: u16) -> Vec<u8> {
    let mut packet = vec![0, 0, 0, 3, host.len() as u8];
    packet.extend_from_slice(host.as_bytes());
    packet.extend_from_slice(&port.to_be_bytes());
    packet.extend_from_slice(b"claude-udp-regression");
    packet
}

async fn udp_routing(
    engine: &mut Engine,
    saved: &str,
    tcp_port: u16,
    relay: &Socks,
) -> serde_json::Value {
    const HOSTS: [&str; 3] = ["claude.com", "claude.ai", "s-cdn.anthropic.com"];
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = socket.local_addr().unwrap();
    let delivered = Arc::new(AtomicU64::new(0));
    let counter = delivered.clone();
    let sentinel = tokio::spawn(async move {
        let mut packet = [0; 4096];
        while let Ok((length, source)) = socket.recv_from(&mut packet).await {
            counter.fetch_add(1, Ordering::Relaxed);
            socket.send_to(&packet[..length], source).await.unwrap();
        }
    });
    let mut candidate: serde_yaml::Value = serde_yaml::from_str(saved).unwrap();
    candidate["proxies"][0]["udp"] = false.into();
    for host in HOSTS {
        candidate["hosts"][host] = "127.0.0.1".into();
    }
    engine
        .api
        .reload(&scoped_payload(&serde_yaml::to_string(&candidate).unwrap()))
        .await
        .unwrap();
    engine.poll(Scope::Logs).await.unwrap();
    // The fixture sees traffic only if the core actually emits a DIRECT
    // datagram. Checking guard counters alone would miss a brief DNS leak.
    let mut sockets = Vec::new();
    for index in 0..28 {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        socket
            .send_to(
                &udp_packet(HOSTS[index % HOSTS.len()], target.port()),
                ("127.0.0.1", engine.settings.mixed_port),
            )
            .await
            .unwrap();
        sockets.push(socket);
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        delivered.load(Ordering::Relaxed),
        0,
        "unsupported protected UDP reached DIRECT before the guard could close it"
    );
    let logs = engine.poll(Scope::Logs).await.unwrap().logs;
    for host in HOSTS {
        assert!(
            logs.iter().any(|line| line.contains("[UDP]")
                && line.contains(host)
                && line.contains("REJECT")),
            "missing native UDP rejection for {host}: {:#?}",
            logs.iter()
                .rev()
                .filter(|line| line.contains("[UDP]") || line.contains("TUN"))
                .take(16)
                .collect::<Vec<_>>()
        );
    }
    // An unrelated subscription flow still works, proving the sentinel and
    // UDP ingress are live and that this isn't a blanket UDP shutdown.
    let unrelated = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    unrelated
        .send_to(
            &udp_packet("example.org", target.port()),
            ("127.0.0.1", engine.settings.mixed_port),
        )
        .await
        .unwrap();
    let mut packet = [0; 4096];
    let (length, _) =
        tokio::time::timeout(Duration::from_secs(5), unrelated.recv_from(&mut packet))
            .await
            .unwrap()
            .unwrap();
    assert!(packet[..length].ends_with(b"claude-udp-regression"));
    assert_eq!(delivered.load(Ordering::Relaxed), 1);
    let client = detector_client(engine.settings.mixed_port);
    let mut tcp = hold(&client, "claude.ai", tcp_port).await;
    assert!(
        tcp.chunk().await.unwrap().is_some(),
        "TCP fallback must remain usable"
    );

    // Exercise the same unsupported-node policy through native TUN, using
    // fake DNS addresses inside the disposable fake-IP subnet only.
    let dns_port = free_port();
    candidate["dns"]["listen"] = format!("127.0.0.1:{dns_port}").into();
    // Keep the default fake-IP pool: changing it also changes the derived
    // TUN interface address, which cannot safely be hot-swapped on Darwin.
    for host in HOSTS {
        candidate["hosts"]
            .as_mapping_mut()
            .unwrap()
            .remove(serde_yaml::Value::from(host));
    }
    engine
        .api
        .reload(&scoped_payload(&serde_yaml::to_string(&candidate).unwrap()))
        .await
        .unwrap();
    for host in HOSTS {
        let dns = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut query = vec![0x49, 0x52, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        for label in host.split('.') {
            query.push(label.len() as u8);
            query.extend_from_slice(label.as_bytes());
        }
        query.extend_from_slice(&[0, 0, 1, 0, 1]);
        dns.send_to(&query, ("127.0.0.1", dns_port)).await.unwrap();
        let (length, _) = tokio::time::timeout(Duration::from_secs(5), dns.recv_from(&mut packet))
            .await
            .unwrap()
            .unwrap();
        assert!(
            length > 16 && packet[..2] == query[..2] && packet[length - 6..length - 4] == [0, 4]
        );
        let address = std::net::Ipv4Addr::new(
            packet[length - 4],
            packet[length - 3],
            packet[length - 2],
            packet[length - 1],
        );
        assert_eq!(
            &address.octets()[..2],
            &[198, 18],
            "DNS did not return a scoped fake IP"
        );
        let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await.unwrap();
        socket
            .send_to(b"claude-native-tun-udp", (address, target.port()))
            .await
            .unwrap();
        sockets.push(socket);
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let logs = engine.poll(Scope::Logs).await.unwrap().logs;
    for host in HOSTS {
        assert!(
            logs.iter().any(|line| line.contains("[UDP]")
                && line.contains(host)
                && line.contains("REJECT")
                && !line.contains("127.0.0.1:")),
            "native TUN did not reject unsupported UDP for {host}: {:#?}",
            logs.iter()
                .rev()
                .filter(|line| line.contains("[UDP]") || line.contains("TUN"))
                .take(16)
                .collect::<Vec<_>>()
        );
    }
    // A UDP-capable node must keep UDP working through that very node.
    candidate = serde_yaml::from_str(saved).unwrap();
    for host in HOSTS {
        candidate["hosts"][host] = "127.0.0.1".into();
    }
    engine
        .api
        .reload(&scoped_payload(&serde_yaml::to_string(&candidate).unwrap()))
        .await
        .unwrap();
    let before = relay.udp.load(Ordering::Relaxed);
    for host in HOSTS {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        socket
            .send_to(
                &udp_packet(host, target.port()),
                ("127.0.0.1", engine.settings.mixed_port),
            )
            .await
            .unwrap();
        let (length, _) =
            tokio::time::timeout(Duration::from_secs(5), socket.recv_from(&mut packet))
                .await
                .unwrap()
                .unwrap();
        assert!(packet[..length].ends_with(b"claude-udp-regression"));
        sockets.push(socket);
    }
    assert_eq!(
        relay.udp.load(Ordering::Relaxed) - before,
        3,
        "supported UDP bypassed the pinned node"
    );
    assert_eq!(
        delivered.load(Ordering::Relaxed),
        1,
        "protected datagrams leaked to the DIRECT sentinel"
    );
    // Reproduce the reported AnyTLS case, including its real TLS + UoT data
    // path. Merely using a UDP-capable SOCKS fixture would miss this default.
    let anytls_directory = tempfile::tempdir().unwrap();
    let anytls_port = free_port();
    let config = serde_json::json!({
        "mode":"rule", "log-level":"silent", "dns":{"enable":false}, "ipv6":true,
        "listeners":[{"name":"anytls-regression", "type":"anytls", "listen":"127.0.0.1", "port":anytls_port,
            "users":{"fixture":"temporary-regression-password"},
            "certificate":std::fs::read_to_string(std::env::var_os("CLASH_CLAUDE_TEST_CERT").unwrap()).unwrap(),
            "private-key":std::fs::read_to_string(std::env::var_os("CLASH_CLAUDE_TEST_KEY").unwrap()).unwrap()}],
        "proxies":[], "rules":["MATCH,DIRECT"]
    });
    let path = anytls_directory.path().join("config.yaml");
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let mut server = tokio::process::Command::new(test_resources().join(if cfg!(windows) {
        "mihomo.exe"
    } else {
        "mihomo"
    }))
    .arg("-d")
    .arg(anytls_directory.path())
    .arg("-f")
    .arg(path)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true)
    .spawn()
    .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", anytls_port))
            .await
            .is_ok()
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline && server.try_wait().unwrap().is_none(),
            "AnyTLS loopback fixture failed to start"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    candidate["proxies"][0] =
        serde_yaml::to_value(serde_json::json!({"name":"FixtureA","type":"anytls",
        "server":"127.0.0.1","port":anytls_port,"password":"temporary-regression-password"}))
        .unwrap();
    for enabled in [false, true] {
        if enabled {
            candidate["proxies"][0]["udp"] = true.into();
        }
        engine
            .api
            .reload(&scoped_payload(&serde_yaml::to_string(&candidate).unwrap()))
            .await
            .unwrap();
        let proxies: serde_json::Value = engine.api.get("proxies").await.unwrap();
        let proxy = &proxies["proxies"]["FixtureA"];
        assert_eq!(proxy["type"], "AnyTLS");
        assert_eq!(
            proxy["udp"], enabled,
            "AnyTLS missing udp must not be treated as enabled"
        );
        for host in HOSTS {
            let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            socket
                .send_to(
                    &udp_packet(host, target.port()),
                    ("127.0.0.1", engine.settings.mixed_port),
                )
                .await
                .unwrap();
            if enabled {
                let (length, _) =
                    tokio::time::timeout(Duration::from_secs(8), socket.recv_from(&mut packet))
                        .await
                        .unwrap()
                        .unwrap();
                assert!(
                    packet[..length].ends_with(b"claude-udp-regression"),
                    "AnyTLS UoT response mismatch"
                );
            }
            sockets.push(socket);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(
            delivered.load(Ordering::Relaxed),
            if enabled { 4 } else { 1 },
            "AnyTLS UDP reached the sentinel through the wrong path"
        );
        if enabled {
            let live: serde_json::Value = engine.api.get("connections").await.unwrap();
            for host in HOSTS {
                assert!(
                    live["connections"].as_array().unwrap().iter().any(
                        |row| row["metadata"]["host"] == host
                            && row["metadata"]["network"] == "udp"
                            && row["chains"]
                                .as_array()
                                .is_some_and(|chain| chain.iter().any(|s| s == "FixtureA")
                                    && chain.iter().all(|s| s != "DIRECT"))
                    ),
                    "AnyTLS UDP did not use the pinned node: {host}: {live}"
                );
            }
        }
    }
    server.kill().await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let guard = engine
        .poll(Scope::Claude)
        .await
        .unwrap()
        .claude_guard
        .unwrap();
    assert_eq!(
        guard.blocked, 0,
        "normal/rejected UDP must not rely on reactive blocking: {guard:?}"
    );
    assert_eq!(
        guard.alerts, 0,
        "explicit core rejections must not cause OS warnings: {guard:?}"
    );
    sentinel.abort();
    drop((sockets, unrelated, tcp));
    engine.api.reload(&scoped_payload(saved)).await.unwrap();
    serde_json::json!({"unsupported_socks_requests":28,"unsupported_native_tun_requests":3,
        "protected_direct_datagrams":0,"supported_pinned_udp_responses":3,
        "anytls_missing_udp_rejected":3,"anytls_uot_responses":3,
        "unrelated_udp_preserved":true,"tcp_preserved":true,"guard_blocked":guard.blocked,"guard_alerts":guard.alerts})
}

fn scoped_payload(payload: &str) -> String {
    // The production overlay is separately asserted to cover all routes.
    // Limit the disposable test's live routes so the VM control agent does
    // not lose its network to our deliberately synthetic DNS/node fixture.
    let mut value: serde_yaml::Value = serde_yaml::from_str(payload).unwrap();
    value["tun"]["route-address"] = serde_yaml::to_value([
        "203.0.113.0/24",
        "198.18.0.0/16",
        "8.8.8.8/32",
        "2001:db8::/64",
    ])
    .unwrap();
    serde_yaml::to_string(&value).unwrap()
}
async fn scope_runner(engine: &Engine) {
    let payload = std::fs::read_to_string(engine.store.runtime().join("config.yaml")).unwrap();
    let mut value: serde_yaml::Value = serde_yaml::from_str(&payload).unwrap();
    assert!(
        value["tun"]["route-address"].is_null(),
        "production protection must cover full routing"
    );
    let live: serde_json::Value = engine.api.get("configs").await.unwrap();
    assert_eq!(
        live["tun"]["enable"], true,
        "protection must enable TUN before test scoping"
    );
    value["tun"]["enable"] = true.into();
    engine
        .api
        .reload(&scoped_payload(&serde_yaml::to_string(&value).unwrap()))
        .await
        .unwrap();
}

pub async fn run() {
    assert_eq!(std::env::var("CLASH_PROXY_SMOKE").as_deref(), Ok("1"));
    let mut servers = Servers::start().await;
    let first = Socks::start(servers.http, servers.https).await;
    let other = Socks::start(servers.http, servers.https).await;
    let temp = tempfile::tempdir().unwrap();
    let store = Store::at(temp.path().join("data")).unwrap();
    let controller_port = free_port();
    let mut mixed_port = free_port();
    while mixed_port == controller_port {
        mixed_port = free_port();
    }
    store
        .save_settings(&Settings {
            controller_port,
            mixed_port,
            dns_override: false,
            ipv6: false,
            udp: false,
            ..Default::default()
        })
        .unwrap();
    let mut engine = Engine::with_resources(store.clone(), test_resources()).unwrap();
    let profile = temp.path().join("subscription.yaml");
    let device = if cfg!(target_os = "macos") {
        "utun233"
    } else {
        "ClashRustTest"
    };
    let raw = format!(
        "proxies:\n  - {{name: FixtureA, type: socks5, server: 127.0.0.1, port: {}, udp: true}}\n  - {{name: FixtureB, type: socks5, server: 127.0.0.1, port: {}, udp: true}}\nproxy-groups: [{{name: Main, type: select, proxies: [FixtureA, FixtureB]}}]\nhosts: {{claude.ai: 127.0.0.1, api.anthropic.com: 127.0.0.1, shared.sentry.io: 127.0.0.1, example.org: 127.0.0.1}}\ndns: {{enable: true, enhanced-mode: redir-host, nameserver: ['https://127.0.0.1:{}/dns-query'], fake-ip-filter: ['dns-check.example']}}\ntun: {{device: {device}, route-address: [203.0.113.0/24], dns-hijack: []}}\nrules: ['DOMAIN-SUFFIX,claude.ai,DIRECT', 'DOMAIN,shared.sentry.io,DIRECT', 'IP-CIDR6,2001:db8::/64,Main,no-resolve', 'MATCH,Main']\n",
        first.port, other.port, servers.https
    );
    let raw = format!(
        "{raw}\nsniffer:\n  enable: true\n  force-dns-mapping: true\n  parse-pure-ip: false\n  override-destination: false\n  force-domain: ['+.claude.ai', '+.anthropic.com']\n  sniff:\n    HTTP: {{ports: [80]}}\n    TLS: {{ports: [443]}}\n"
    );
    std::fs::write(&profile, &raw).unwrap();
    engine
        .import("受控测试订阅".into(), profile.display().to_string(), None)
        .await
        .unwrap();
    let id = engine.profiles.last().unwrap().id.clone();
    engine.activate(id.clone()).await.unwrap();
    engine.start().await.unwrap();
    for udp in [true, false] {
        engine
            .save_settings(Settings {
                udp,
                ..engine.settings.clone()
            })
            .await
            .unwrap();
        let proxies: serde_json::Value = engine.api.get("proxies").await.unwrap();
        assert_eq!(
            proxies["proxies"]["FixtureA"]["udp"], udp,
            "UDP setting must apply to the running core"
        );
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::http(format!("http://127.0.0.1:{mixed_port}")).unwrap())
        .timeout(Duration::from_secs(180))
        .build()
        .unwrap();
    let mut before_hold = hold(&client, "claude.ai", servers.http).await;
    assert!(before_hold.chunk().await.unwrap().is_some());
    let before = capture(&mut engine, "开启前").await;
    // Dropping an unread streaming response need not remove its core tracker
    // immediately on Linux. End the deliberately DIRECT before-sample at the
    // controller, so it cannot contaminate the new-request/alert assertions.
    let live: serde_json::Value = engine.api.get("connections").await.unwrap();
    for connection in live["connections"].as_array().unwrap() {
        if connection["metadata"]["host"] == "claude.ai"
            && connection["chains"]
                .as_array()
                .is_some_and(|chain| chain.iter().any(|v| v == "DIRECT"))
        {
            engine
                .api
                .close_connection(connection["id"].as_str().unwrap())
                .await
                .unwrap();
        }
    }
    drop(before_hold);
    let original_settings = serde_json::to_value(&engine.settings).unwrap();
    let original_runtime = std::fs::read(store.runtime().join("config.yaml")).unwrap();
    let original_profile = std::fs::read(store.profile_path(&id).unwrap()).unwrap();
    assert!(!engine.claude_protection(true).await.unwrap());
    scope_runner(&engine).await;
    let protection = engine.settings.claude_protection.clone().unwrap();
    let snapshot = engine.poll(Scope::Claude).await.unwrap();
    assert!(snapshot.tun && !snapshot.system_proxy);
    assert_eq!(snapshot.mode, "rule");
    let proxies: serde_json::Value = engine.api.get("proxies").await.unwrap();
    assert_eq!(
        proxies["proxies"]["FixtureA"]["udp"], true,
        "protection must force UDP while leaving the saved preference off"
    );
    assert!(
        engine
            .save_settings(Settings {
                udp: true,
                ..engine.settings.clone()
            })
            .await
            .is_err(),
        "protection must prevent overwriting the saved UDP preference"
    );
    assert!(
        !engine.settings.dns_override && !engine.settings.ipv6 && !engine.settings.udp,
        "temporary overlay must not rewrite preferences"
    );
    assert_eq!(
        std::fs::read(store.profile_path(&id).unwrap()).unwrap(),
        original_profile
    );
    let saved = std::fs::read_to_string(store.runtime().join("config.yaml")).unwrap();
    let mut cache_probe: serde_yaml::Value = serde_yaml::from_str(&saved).unwrap();
    let probe_rules = cache_probe["rules"].as_sequence_mut().unwrap();
    let last = probe_rules.len() - 1;
    probe_rules.insert(last, "IP-CIDR,203.0.113.0/24,DIRECT,no-resolve".into());
    probe_rules.insert(0, "DOMAIN,outside.ech.test,DIRECT".into());
    engine
        .api
        .reload(&scoped_payload(
            &serde_yaml::to_string(&cache_probe).unwrap(),
        ))
        .await
        .unwrap();
    let mut probes = Vec::new();
    for (target, host, tls) in [
        ("203.0.113.77", "claude.ai", false),
        ("203.0.113.77", "s-cdn.anthropic.com", true),
        ("claude.ai", "outside.ech.test", true),
    ] {
        probes.push(route_probe(&engine, target, host, tls).await);
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let live: serde_json::Value = engine.api.get("connections").await.unwrap();
    for (_, id) in &probes {
        assert!(
            live["connections"]
                .as_array()
                .unwrap()
                .iter()
                .any(|connection| connection["id"].as_str() == Some(id.as_str())
                    && connection["chains"]
                        .as_array()
                        .is_some_and(|chain| chain.iter().any(|v| v == "FixtureA"))),
            "normal cached-IP/SNI connection was closed during monitoring: {id}"
        );
    }
    let state = engine
        .poll(Scope::Claude)
        .await
        .unwrap()
        .claude_guard
        .unwrap();
    assert_eq!(
        state.blocked, 0,
        "protected cached-IP/SNI requests must not fall back to reactive blocking: {state:?}"
    );
    for (mut child, _) in probes {
        child.kill().await.unwrap();
    }
    engine.api.reload(&scoped_payload(&saved)).await.unwrap();
    let udp_routing = udp_routing(&mut engine, &saved, servers.http, &first).await;
    let mut normal = hold(&client, "api.anthropic.com", servers.http).await;
    assert!(normal.chunk().await.unwrap().is_some());
    let mut protected = capture(&mut engine, "保护开启后").await;
    let detector = detector_client(mixed_port);
    for kind in [
        check::additional::Kind::Registration,
        check::additional::Kind::GlobalPing,
        check::additional::Kind::Speed,
    ] {
        let results = check::additional::run(&detector, kind, "1.2.3.4")
            .await
            .unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|s| s.country.is_empty() && !s.risk));
        protected.signals.extend(results);
    }
    protected.validate().unwrap();
    // Force a non-fake DNS lookup through the actual controller and assert
    // the TLS DNS request traversed the pinned SOCKS node.
    let url = engine.api.url(&["dns", "query"]).unwrap();
    let response = engine
        .api
        .request(reqwest::Method::GET, url)
        .query(&[("name", "dns-check.example"), ("type", "A")])
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(
        status.is_success(),
        "controlled DNS query failed: {status}: {text}; core logs: {:?}",
        engine.poll(Scope::Logs).await.unwrap().logs
    );
    let body: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        body["Answer"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v["data"] == "203.0.113.2")),
        "DNS reply: {body}"
    );
    assert!(
        first.doh.load(Ordering::Relaxed) > 0,
        "DoH did not pass through the protection node"
    );
    let mut query = vec![0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in ["dns-check", "example"] {
        query.push(label.len() as u8);
        query.extend_from_slice(label.as_bytes());
    }
    query.extend_from_slice(&[0, 0, 1, 0, 1]);
    let udp = tokio::net::UdpSocket::bind("0.0.0.0:0").await.unwrap();
    udp.send_to(&query, ("8.8.8.8", 53)).await.unwrap();
    let mut reply = [0; 4096];
    let (length, _) = tokio::time::timeout(Duration::from_secs(5), udp.recv_from(&mut reply))
        .await
        .expect("TUN UDP DNS was not intercepted")
        .unwrap();
    assert!(
        length > 12 && reply[..2] == query[..2] && reply.windows(4).any(|w| w == [203, 0, 113, 2])
    );
    let mut tcp = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect(("8.8.8.8", 53)),
    )
    .await
    .expect("TUN TCP DNS was not intercepted")
    .unwrap();
    tcp.write_u16(query.len() as u16).await.unwrap();
    tcp.write_all(&query).await.unwrap();
    let size = tokio::time::timeout(Duration::from_secs(5), tcp.read_u16())
        .await
        .unwrap()
        .unwrap();
    let mut reply = vec![0; size as usize];
    tcp.read_exact(&mut reply).await.unwrap();
    assert!(reply[..2] == query[..2] && reply.windows(4).any(|w| w == [203, 0, 113, 2]));
    drop(tcp);
    let v6 = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let mut response = v6
        .get(format!("http://[2001:db8::2]:{}/hold", servers.http))
        .send()
        .await
        .expect("native IPv6 TUN routing failed");
    assert!(response.chunk().await.unwrap().is_some());
    drop(response);
    let live_before: serde_json::Value = engine.api.get("connections").await.unwrap();
    assert!(
        live_before["connections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["metadata"]["host"] == "api.anthropic.com")
    );
    // Deliberately corrupt only the live core policy; persisted subscription
    // and settings remain untouched. This exercises the real guard backup.
    let saved = std::fs::read_to_string(store.runtime().join("config.yaml")).unwrap();
    let mut attacked: serde_yaml::Value = serde_yaml::from_str(&saved).unwrap();
    let rules = attacked["rules"].as_sequence_mut().unwrap();
    rules.insert(0, serde_yaml::Value::from("DOMAIN,claude.ai,DIRECT"));
    rules.insert(
        0,
        serde_yaml::Value::from("DOMAIN,shared.sentry.io,FixtureB"),
    );
    engine
        .api
        .reload(&scoped_payload(&serde_yaml::to_string(&attacked).unwrap()))
        .await
        .unwrap();
    let mut unrelated = hold(&client, "example.org", servers.http).await;
    let mut bad_direct = hold(&client, "claude.ai", servers.http).await;
    let mut bad_node = hold(&client, "shared.sentry.io", servers.http).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    let guard = loop {
        let state = engine
            .poll(Scope::Claude)
            .await
            .unwrap()
            .claude_guard
            .unwrap();
        if state.blocked >= 2 && state.alerts >= 1 {
            break state;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "guard did not close both unsafe connections: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    for response in [&mut bad_direct, &mut bad_node] {
        tokio::time::timeout(Duration::from_secs(3), async {
            while let Ok(Some(_)) = response.chunk().await {}
        })
        .await
        .expect("unsafe connection remained open");
    }
    assert!(
        unrelated.chunk().await.unwrap().is_some(),
        "unrelated traffic was killed"
    );
    assert!(
        normal.chunk().await.unwrap().is_some(),
        "normal Claude traffic was killed"
    );
    assert_eq!(
        guard.alerts, 1,
        "two related failures should produce one notification request"
    );
    #[cfg(target_os = "linux")]
    {
        let window = visible_linux_dialog("Clash of Rust · Claude 保护").await;
        let output =
            std::path::PathBuf::from(std::env::var_os("CLASH_CLAUDE_TEST_REPORT").unwrap());
        let status = tokio::process::Command::new("/usr/bin/import")
            .args(["-window", &window])
            .arg(output.with_file_name("linux-native-warning.png"))
            .status()
            .await
            .unwrap();
        assert!(status.success(), "native Linux warning screenshot failed");
        let status = tokio::process::Command::new("xdotool")
            .args(["key", "--window", &window, "Return"])
            .status()
            .await
            .unwrap();
        assert!(status.success());
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            FindWindowW, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
        };
        let title: Vec<u16> = "Clash of Rust · Claude 保护"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let window = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
            let mut pid = 0;
            if !window.is_null() {
                unsafe { GetWindowThreadProcessId(window, &mut pid) };
            }
            if pid == std::process::id() {
                unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "native Windows alert did not become visible"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    loop {
        let state = engine
            .poll(Scope::Claude)
            .await
            .unwrap()
            .claude_guard
            .unwrap();
        assert!(
            state.error.is_empty(),
            "native enforcement/notification failed: {}",
            state.error
        );
        if state.notifications_completed >= 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "native notification did not complete: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let blocked = capture(&mut engine, "异常直连与错误节点拦截后").await;
    for host in ["claude.ai", "shared.sentry.io"] {
        assert!(
            blocked
                .routes
                .iter()
                .any(|route| route.host == host && route.outcome == "异常连接已关闭"),
            "normal diagnostic traffic evicted the abnormal connection details for {host}"
        );
    }
    let notification = engine
        .poll(Scope::Claude)
        .await
        .unwrap()
        .claude_guard
        .unwrap();
    assert!(
        !notification.error.contains("系统通知失败"),
        "native notification failed: {}",
        notification.error
    );
    let guard = notification;
    #[cfg(target_os = "linux")]
    {
        let alert = tokio::spawn(clash_of_rust::platform::notification::show(
            "Clash of Rust · Claude 保护（取消测试）",
            "验证关闭保护时不会残留系统提示框。",
        ));
        let window = visible_linux_dialog("Clash of Rust · Claude 保护（取消测试）").await;
        alert.abort();
        assert!(alert.await.unwrap_err().is_cancelled());
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let found = tokio::process::Command::new("xdotool")
                .args(["getwindowname", &window])
                .output()
                .await
                .unwrap();
            if !found.status.success() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "cancelled native warning remained visible"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    drop((normal, unrelated, bad_direct, bad_node));
    // Recreate the engine as a genuine application restart. The persisted
    // restoration record must survive unchanged, including its pinned node.
    engine.stop().await.unwrap();
    drop(engine);
    let mut engine = Engine::with_resources(store.clone(), test_resources()).unwrap();
    assert_eq!(
        engine.settings.claude_protection.as_ref(),
        Some(&protection)
    );
    engine.start().await.unwrap();
    assert!(!engine.restore_proxy_mode().await.unwrap());
    let restarted = engine.poll(Scope::Claude).await.unwrap();
    assert!(restarted.tun && restarted.claude_guard.is_some());
    scope_runner(&engine).await;
    assert_eq!(
        engine.settings.claude_protection.as_ref(),
        Some(&protection)
    );
    assert!(!engine.claude_protection(false).await.unwrap());
    let state = engine.poll(Scope::Claude).await.unwrap();
    assert!(!state.tun && !state.system_proxy && state.claude_guard.is_none());
    assert_eq!(
        serde_json::to_value(&engine.settings).unwrap(),
        original_settings
    );
    let restored_runtime: serde_yaml::Value =
        serde_yaml::from_slice(&std::fs::read(store.runtime().join("config.yaml")).unwrap())
            .unwrap();
    let original_runtime: serde_yaml::Value = serde_yaml::from_slice(&original_runtime).unwrap();
    assert_eq!(
        restored_runtime, original_runtime,
        "all DNS/rule/TUN settings must be restored"
    );
    let restored = capture(&mut engine, "重启后关闭保护").await;
    // Repeat from system-proxy and already-TUN states. Closing must restore
    // whichever state existed before this particular activation.
    for mode in [ProxyMode::System, ProxyMode::Tun] {
        engine.select_proxy_mode(mode).await.unwrap();
        let saved_settings = serde_json::to_value(&engine.settings).unwrap();
        engine.claude_protection(true).await.unwrap();
        scope_runner(&engine).await;
        assert_eq!(
            engine
                .settings
                .claude_protection
                .as_ref()
                .unwrap()
                .previous
                .proxy_mode,
            mode
        );
        engine.claude_protection(false).await.unwrap();
        let state = engine.poll(Scope::Claude).await.unwrap();
        assert_eq!(state.tun, mode == ProxyMode::Tun);
        assert_eq!(state.system_proxy, mode == ProxyMode::System);
        assert_eq!(
            serde_json::to_value(&engine.settings).unwrap(),
            saved_settings
        );
    }
    engine.select_mode("direct").await.unwrap();
    engine.claude_protection(true).await.unwrap();
    scope_runner(&engine).await;
    engine.claude_protection(false).await.unwrap();
    assert_eq!(engine.poll(Scope::Claude).await.unwrap().mode, "direct");
    engine.select_mode("global").await.unwrap();
    engine.api.select("GLOBAL", "DIRECT").await.unwrap();
    let before_rejected = serde_json::to_value(&engine.settings).unwrap();
    assert!(engine.claude_protection(true).await.is_err());
    assert_eq!(
        serde_json::to_value(&engine.settings).unwrap(),
        before_rejected,
        "rejected activation changed preferences"
    );
    engine.select_mode("rule").await.unwrap();
    let provider_path = store.runtime().join("providers/fixture.yaml");
    clash_of_rust::config::atomic_write(&provider_path,format!("proxies: [{{name: FixtureA, type: socks5, server: 127.0.0.1, port: {}}}, {{name: FixtureB, type: socks5, server: 127.0.0.1, port: {}}}]\n",first.port,other.port).as_bytes()).unwrap();
    let mut provider_profile: serde_yaml::Value = serde_yaml::from_str(&raw).unwrap();
    provider_profile
        .as_mapping_mut()
        .unwrap()
        .remove(serde_yaml::Value::from("proxies"));
    provider_profile["proxy-providers"] = serde_yaml::to_value(
        serde_json::json!({"Feed":{"type":"file","path":"providers/fixture.yaml"}}),
    )
    .unwrap();
    provider_profile["proxy-groups"] =
        serde_yaml::to_value(serde_json::json!([{"name":"Main","type":"select","use":["Feed"]}]))
            .unwrap();
    let provider_source = temp.path().join("provider-profile.yaml");
    std::fs::write(
        &provider_source,
        serde_yaml::to_string(&provider_profile).unwrap(),
    )
    .unwrap();
    engine
        .import(
            "集合订阅".into(),
            provider_source.display().to_string(),
            None,
        )
        .await
        .unwrap();
    let provider_id = engine.profiles.last().unwrap().id.clone();
    engine.activate(provider_id).await.unwrap();
    assert_eq!(
        engine
            .poll_with_header(Scope::Claude, true)
            .await
            .unwrap()
            .active_node
            .as_deref(),
        Some("FixtureA")
    );
    let provider_settings = serde_json::to_value(&engine.settings).unwrap();
    engine.claude_protection(true).await.unwrap();
    assert_eq!(
        engine
            .settings
            .claude_protection
            .as_ref()
            .unwrap()
            .provider
            .as_deref(),
        Some("Feed")
    );
    let proxies: serde_json::Value = engine.api.get("proxies").await.unwrap();
    let route = &engine.settings.claude_protection.as_ref().unwrap().route;
    assert_eq!(
        proxies["proxies"][route]["udp"], true,
        "provider-backed protection must force UDP"
    );
    scope_runner(&engine).await;
    let result = detector_client(mixed_port)
        .get("https://api.anthropic.com/cdn-cgi/trace")
        .send()
        .await
        .unwrap();
    assert!(
        result.status().is_success(),
        "provider-backed protection did not route the request"
    );
    engine.claude_protection(false).await.unwrap();
    assert_eq!(
        serde_json::to_value(&engine.settings).unwrap(),
        provider_settings
    );
    engine.select_proxy_mode(ProxyMode::Off).await.unwrap();
    engine.stop().await.unwrap();
    assert!(!store.root.join("profile-transaction.json").exists());
    assert!(!store.root.join("system-proxy.json").exists());
    let report = serde_json::json!({"schema":1,"environment":"controlled disposable native runner","version":clash_of_rust::VERSION,
        "before":before,"protected":protected,"blocked":blocked,"restored":restored,"protection":protection,
        "routing_scope":"Production full-route configuration asserted; native fixture routes limited to documentation IPv4/IPv6, the fake-IP test pool and DNS test target to preserve runner control traffic",
        "tests":["native TUN and system proxy","native IPv6 TUN traffic","public DoH via pinned node","real UDP DNS hijack","real TCP DNS hijack","normal Claude connection preserved","unrelated connection preserved","DIRECT connection closed","wrong-node connection closed","native notification request deduplicated and completed","restart preserves original snapshot","off/system/TUN restored exactly","rejected activation leaves preferences unchanged","subscription source unchanged","provider-backed concrete node and restore","cached-IP HTTP hostname routed","cached-IP TLS SNI routed","known hostname preserved despite alternate TLS SNI","unsupported UDP rejected before DIRECT","unsupported UDP over native TUN rejected","supported UDP preserved through pinned node","AnyTLS without udp fails closed","AnyTLS real UoT round trip"],
        "linux_visible_warning_and_cancellation":cfg!(target_os = "linux"),
        "udp_routing":udp_routing,
        "guard":guard});
    let output = std::path::PathBuf::from(std::env::var_os("CLASH_CLAUDE_TEST_REPORT").unwrap());
    clash_of_rust::config::atomic_write(&output, &serde_json::to_vec_pretty(&report).unwrap())
        .unwrap();
    servers.child.kill().await.unwrap();
    println!(
        "PASS: reversible protection, native TUN/DNS, restart, connection enforcement and native notification request"
    );
}
