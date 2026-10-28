//! Native resolver coexistence in disposable macOS CI, without a tailnet login.
use clash_of_rust::{
    engine::{Engine, ProxyMode},
    platform::macos::{dns, service},
};
use std::{
    io::Write,
    net::IpAddr,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn run_scutil(script: &str) -> std::io::Result<std::process::Output> {
    let mut child = Command::new("/usr/bin/sudo")
        .args(["-n", "/usr/sbin/scutil"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(script.as_bytes())?;
    child.wait_with_output()
}

fn scutil(script: &str) -> std::process::Output {
    let output = run_scutil(script).unwrap();
    assert!(output.status.success(), "scutil: {output:?}");
    output
}

fn has_resolver(text: &str, domain: &str, server: &str) -> bool {
    text.split("resolver #").any(|resolver| {
        let mut domain_matches = false;
        let mut server_matches = false;
        for line in resolver.lines() {
            if let Some((key, value)) = line.trim().split_once(" : ") {
                let value = value.trim().trim_end_matches('.');
                domain_matches |= key.trim() == "domain" && value == domain;
                server_matches |= key.trim().starts_with("nameserver[") && value == server;
            }
        }
        domain_matches && server_matches
    })
}

struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Err(error) = service::protection_dns(false) {
            eprintln!("Could not restore test protection DNS: {error:#}");
        }
        match run_scutil(&format!("remove {}\nquit\n", self.0)) {
            Ok(output) if output.status.success() => {}
            result => eprintln!("Could not remove test DNS resolver: {result:?}"),
        }
    }
}

struct SavedSetup {
    key: String,
    backup: Option<String>,
}
impl SavedSetup {
    fn new(key: String, suffix: &str) -> Self {
        let shown = scutil(&format!("show {key}\nquit\n"));
        let backup = String::from_utf8_lossy(&shown.stdout)
            .contains("<dictionary>")
            .then(|| {
                let backup = format!("State:/ClashOfRust/Test/{suffix}/OriginalDNS");
                scutil(&format!("get {key}\nset {backup}\nquit\n"));
                backup
            });
        Self { key, backup }
    }
}
impl Drop for SavedSetup {
    fn drop(&mut self) {
        let _ = service::protection_dns(false);
        let script = match &self.backup {
            Some(backup) => format!("get {backup}\nset {}\nremove {backup}\nquit\n", self.key),
            None => format!("remove {}\nquit\n", self.key),
        };
        if let Err(error) = run_scutil(&script) {
            eprintln!("Could not restore test Setup DNS: {error}");
        }
    }
}

struct LoopbackAlias;
impl LoopbackAlias {
    fn install() -> Self {
        let current = Command::new("/sbin/ifconfig").arg("lo0").output().unwrap();
        assert!(current.status.success());
        assert!(
            !String::from_utf8_lossy(&current.stdout).contains("100.100.100.100"),
            "The test must not replace an existing Tailscale DNS address"
        );
        let output = Command::new("/usr/bin/sudo")
            .args([
                "-n",
                "/sbin/ifconfig",
                "lo0",
                "alias",
                "100.100.100.100",
                "255.255.255.255",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "ifconfig alias: {output:?}");
        Self
    }
}
impl Drop for LoopbackAlias {
    fn drop(&mut self) {
        match Command::new("/usr/bin/sudo")
            .args(["-n", "/sbin/ifconfig", "lo0", "-alias", "100.100.100.100"])
            .output()
        {
            Ok(output) if output.status.success() => {}
            result => eprintln!("Could not remove test DNS alias: {result:?}"),
        }
    }
}

async fn stub(
    bind_address: &str,
    address: [u8; 4],
    drop_responses: Arc<AtomicBool>,
) -> (u16, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    let mut sockets = None;
    for _ in 0..32 {
        let socket = tokio::net::UdpSocket::bind((bind_address, 0))
            .await
            .unwrap();
        let port = socket.local_addr().unwrap().port();
        if let Ok(listener) = tokio::net::TcpListener::bind((bind_address, port)).await {
            sockets = Some((socket, listener));
            break;
        }
    }
    let (socket, listener) = sockets.expect("No shared TCP/UDP fixture port was available");
    let port = socket.local_addr().unwrap().port();
    let queries = Arc::new(Mutex::new(vec![]));
    let captured = queries.clone();
    let job = tokio::spawn(async move {
        let mut buffer = [0; 4096];
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                packet = socket.recv_from(&mut buffer) => {
                    let (count, peer) = packet.unwrap();
                    if let Some(reply) = stub_response(&buffer[..count], address, &captured, &drop_responses) {
                        socket.send_to(&reply, peer).await.unwrap();
                    }
                }
                connection = listener.accept() => {
                    let (stream, _) = connection.unwrap();
                    let captured = captured.clone();
                    let drop_responses = drop_responses.clone();
                    connections.spawn(async move {
                        let _ = stub_tcp(stream, address, captured, drop_responses).await;
                    });
                }
                Some(result) = connections.join_next(), if !connections.is_empty() => result.unwrap(),
            }
        }
    });
    (port, queries, job)
}

fn stub_response(
    packet: &[u8],
    address: [u8; 4],
    captured: &Mutex<Vec<String>>,
    drop_responses: &AtomicBool,
) -> Option<Vec<u8>> {
    if packet.len() < 17 || packet[4..6] != [0, 1] {
        return None;
    }
    let mut end = 12;
    let mut labels = vec![];
    while end < packet.len() && packet[end] != 0 {
        let length = usize::from(packet[end]);
        if length > 63 || end + length + 1 >= packet.len() {
            return None;
        }
        labels.push(String::from_utf8_lossy(&packet[end + 1..end + 1 + length]).into_owned());
        end += length + 1;
    }
    if end + 5 > packet.len() {
        return None;
    }
    captured.lock().unwrap().push(labels.join("."));
    if drop_responses.load(Ordering::Relaxed) {
        return None;
    }
    let answer = packet[end + 1..end + 3] == [0, 1];
    let mut reply = packet[..end + 5].to_vec();
    reply[2..4].copy_from_slice(&[0x81, 0x80]);
    reply[6..12].copy_from_slice(&[0, u8::from(answer), 0, 0, 0, 0]);
    if answer {
        reply.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 1, 0, 4]);
        reply.extend_from_slice(&address);
    }
    Some(reply)
}

async fn stub_tcp(
    mut stream: tokio::net::TcpStream,
    address: [u8; 4],
    captured: Arc<Mutex<Vec<String>>>,
    drop_responses: Arc<AtomicBool>,
) -> std::io::Result<()> {
    loop {
        let count = usize::from(stream.read_u16().await?);
        if !(17..=4096).contains(&count) {
            return Ok(());
        }
        let mut packet = vec![0; count];
        stream.read_exact(&mut packet).await?;
        let Some(reply) = stub_response(&packet, address, &captured, &drop_responses) else {
            return Ok(());
        };
        stream.write_u16(reply.len() as u16).await?;
        stream.write_all(&reply).await?;
    }
}

async fn lookup_name(host: &str, expected: &str) {
    let addresses =
        tokio::time::timeout(Duration::from_secs(12), tokio::net::lookup_host((host, 80)))
            .await
            .expect("native DNS timed out")
            .unwrap()
            .map(|address| address.ip())
            .collect::<Vec<_>>();
    let expected = expected.parse::<IpAddr>().unwrap();
    if !addresses.contains(&expected) {
        let shown = Command::new("/usr/sbin/scutil")
            .arg("--dns")
            .output()
            .unwrap();
        eprintln!(
            "Native DNS before fixture cleanup:\n{}",
            String::from_utf8_lossy(&shown.stdout)
        );
    }
    assert!(
        addresses.contains(&expected),
        "Unexpected native DNS answer for {host}: {addresses:?}"
    );
}

async fn lookup(host: &str, expected: &str) {
    lookup_name(&format!("{host}."), expected).await;
}

async fn lookup_resolv_conf(host: &str, tcp: bool) {
    let output = tokio::time::timeout(
        Duration::from_secs(8),
        tokio::process::Command::new("/usr/bin/dig")
            .args([
                "+short",
                "+time=2",
                "+tries=1",
                if tcp { "+tcp" } else { "+notcp" },
            ])
            .arg(format!("{host}."))
            .arg("A")
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(output.status.success(), "dig: {output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line == "203.0.113.20"),
        "resolv.conf DNS did not reach the local core (TCP={tcp}): {output:?}"
    );
}

pub async fn run(engine: &mut Engine, profile: &Path) {
    engine.select_proxy_mode(ProxyMode::Off).await.unwrap();
    let configured = dns::configured().unwrap();
    let _loopback_alias = LoopbackAlias::install();
    let (vpn_port, vpn_queries, vpn_job) = stub(
        "100.100.100.100",
        [100, 100, 12, 34],
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    let drop_core_dns = Arc::new(AtomicBool::new(false));
    let (core_port, core_queries, core_job) =
        stub("127.0.0.1", [203, 0, 113, 20], drop_core_dns.clone()).await;
    let (fallback_port, fallback_queries, fallback_job) = stub(
        "127.0.0.1",
        [198, 51, 100, 30],
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    let listen_port = dns::available_port().unwrap();
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let global = scutil("show State:/Network/Global/IPv4\nquit\n");
    let primary = String::from_utf8_lossy(&global.stdout)
        .lines()
        .find_map(|line| line.trim().strip_prefix("PrimaryService : "))
        .expect("The disposable runner must have an active IPv4 service")
        .to_owned();
    let setup_key = format!("Setup:/Network/Service/{primary}/DNS");
    let _setup_cleanup = SavedSetup::new(setup_key.clone(), &suffix);
    // Setup cache overrides State servers in configd. It must be covered too,
    // while the SCPreferences disk value remains untouched.
    scutil(&format!(
        "d.init\nd.add ServerAddresses * 127.0.0.1\nd.add ServerPort # {fallback_port}\nset {setup_key}\nquit\n"
    ));
    let expected_setup = scutil(&format!("show {setup_key}\nquit\n")).stdout;
    let tail = format!("tail-{suffix}.ts.net");
    let corp = format!("{suffix}.corp.test");
    let new_search = format!("{suffix}.new-search.test");
    let key = format!("State:/Network/Service/cor-dns-fixture-{suffix}/DNS");
    let search_key = format!("State:/Network/Service/cor-dns-search-{suffix}/DNS");
    let fallback_key = format!("State:/Network/Service/cor-dns-fallback-{suffix}/DNS");
    let _cleanup = Cleanup(key.clone());
    let _cleanup_search = Cleanup(search_key.clone());
    let _cleanup_fallback = Cleanup(fallback_key.clone());
    let install = |search: &str, conflicts: bool| {
        let protected = if conflicts { " claude.ai bash.ws" } else { "" };
        // Render the original dictionaries before publishing them. Reading the
        // store afterwards can already observe the helper's refreshed overlay.
        let rendered = scutil(&format!(
            "d.init\nd.add ServerAddresses * 100.100.100.100\nd.add ServerPort # {vpn_port}\n\
             d.add InterfaceName lo0\nd.add SearchOrder # 100600\n\
             d.add SupplementalMatchDomains * \"\" {tail} {corp}{protected}\n\
             d.add SupplementalMatchDomainsNoSearch # 1\nd.add SearchDomains * {search}\nd.show\nset {key}\n\
             d.init\nd.add ServerAddresses * 100.100.100.100\nd.add ServerPort # {vpn_port}\n\
             d.add InterfaceName lo0\nd.add SearchOrder # 100599\n\
             d.add SupplementalMatchDomains * {search}\nd.show\nset {search_key}\nquit\n"
        )).stdout;
        let marker = b"<dictionary>";
        let second = rendered
            .windows(marker.len())
            .enumerate()
            .filter(|(_, bytes)| *bytes == marker)
            .nth(1)
            .unwrap()
            .0;
        (rendered[..second].to_vec(), rendered[second..].to_vec())
    };
    let original_conflict = install(&tail, true);
    wait_resolver("claude.ai", "100.100.100.100", true).await;
    std::fs::write(profile, format!(
        "proxies: []\nrules: ['MATCH,DIRECT']\n\
         dns: {{enable: true, listen: '127.0.0.1:{listen_port}', enhanced-mode: redir-host, nameserver: ['127.0.0.1:{core_port}']}}\n\
         tun: {{device: utun233, route-address: ['1.1.1.1/32', '8.8.8.8/32'], dns-hijack: ['any:53', 'tcp://any:53']}}\n"
    )).unwrap();
    engine
        .import(
            "macOS split DNS fixture".into(),
            profile.display().to_string(),
            None,
        )
        .await
        .unwrap();
    engine
        .activate(engine.profiles.last().unwrap().id.clone())
        .await
        .unwrap();
    engine.select_proxy_mode(ProxyMode::Tun).await.unwrap();
    let enable = move || service::protection_dns_at(Some(listen_port));
    let error = tokio::task::spawn_blocking(enable)
        .await
        .unwrap()
        .expect_err("Overlapping Tailscale domains must be rejected");
    assert!(
        format!("{error:#}").contains("分域 DNS 与保护范围重叠"),
        "{error:#}"
    );
    assert_eq!(
        scutil(&format!("show {key}\nquit\n")).stdout,
        original_conflict.0
    );
    assert_eq!(
        scutil(&format!("show {search_key}\nquit\n")).stdout,
        original_conflict.1
    );
    let (mut expected_vpn, mut expected_search) = install(&tail, false);
    wait_resolver("claude.ai", "100.100.100.100", false).await;
    // Non-Quad100 split resolvers are subject to the same safety check.
    let non_quad_key = format!("State:/Network/Service/cor-dns-nonquad-{suffix}/DNS");
    let _cleanup_non_quad = Cleanup(non_quad_key.clone());
    scutil(&format!(
        "d.init\nd.add ServerAddresses * 127.0.0.1\nd.add ServerPort # {fallback_port}\n\
         d.add SupplementalMatchDomains * claude.ai\nset {non_quad_key}\nquit\n"
    ));
    wait_resolver("claude.ai", "127.0.0.1", true).await;
    let error = tokio::task::spawn_blocking(enable)
        .await
        .unwrap()
        .expect_err("Overlapping non-Quad100 split DNS must be rejected");
    assert!(
        format!("{error:#}").contains("分域 DNS 与保护范围重叠"),
        "{error:#}"
    );
    scutil(&format!("remove {non_quad_key}\nquit\n"));
    wait_resolver("claude.ai", "127.0.0.1", false).await;
    scutil(&format!(
        "d.init\nd.add ServerAddresses * 127.0.0.1\nd.add ServerPort # {fallback_port}\n\
         d.add SearchOrder # 200000\nd.add SupplementalMatchDomains * \"\"\nset {fallback_key}\nquit\n"
    ));
    let expected_fallback = scutil(&format!("show {fallback_key}\nquit\n")).stdout;
    vpn_queries.lock().unwrap().clear();
    core_queries.lock().unwrap().clear();
    fallback_queries.lock().unwrap().clear();
    for attempt in 0..2 {
        tokio::task::spawn_blocking(enable).await.unwrap().unwrap();
        assert_eq!(dns::system_servers().unwrap(), ["127.0.0.1"]);
        assert_eq!(dns::configured().unwrap(), configured);
        lookup(&format!("public-{attempt}.{suffix}.test"), "203.0.113.20").await;
        lookup(
            &format!("protected-{attempt}-{suffix}.claude.ai"),
            "203.0.113.20",
        )
        .await;
        lookup(
            &format!("diagnostic-{attempt}-{suffix}.bash.ws"),
            "203.0.113.20",
        )
        .await;
        lookup(&format!("peer-{attempt}.{tail}"), "100.100.12.34").await;
        lookup(&format!("peer-{attempt}.{corp}"), "100.100.12.34").await;
        for tcp in [false, true] {
            lookup_resolv_conf(&format!("file-{attempt}-{tcp}-{suffix}.claude.ai"), tcp).await;
        }
        if attempt == 0 {
            lookup_name(&format!("short-{suffix}"), "100.100.12.34").await;
            drop_core_dns.store(true, Ordering::Relaxed);
            for host in [
                format!("failed-{suffix}.claude.ai"),
                format!("failed-public-{suffix}.test"),
            ] {
                let _ = tokio::time::timeout(
                    Duration::from_secs(25),
                    tokio::net::lookup_host((format!("{host}."), 80)),
                )
                .await;
                assert!(
                    core_queries.lock().unwrap().contains(&host),
                    "Failure fixture did not reach protected DNS: {host}"
                );
                assert!(
                    !vpn_queries.lock().unwrap().contains(&host),
                    "Public DNS fell back to VPN: {host}"
                );
                assert!(
                    !fallback_queries.lock().unwrap().contains(&host),
                    "Public DNS fell back to physical DNS: {host}"
                );
            }
            drop_core_dns.store(false, Ordering::Relaxed);
            let _ = install(&new_search, false);
            // Wait for native change notifications, without requesting enable
            // again: reconnect recovery belongs to the background helper.
            wait_resolver("claude.ai", "127.0.0.1", true).await;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                let shown = scutil(&format!("show {key}\nquit\n"));
                if String::from_utf8_lossy(&shown.stdout).contains("127.0.0.1") {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "VPN reconnect was not recaptured"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            wait_protection_ready(listen_port).await;
            lookup(&format!("reconnected.{tail}"), "100.100.12.34").await;
            lookup(&format!("reconnected.{new_search}"), "100.100.12.34").await;
            lookup_name(&format!("new-short-{suffix}"), "100.100.12.34").await;
            lookup(&format!("public-reconnected-{suffix}.test"), "203.0.113.20").await;
            lookup(
                &format!("protected-reconnected-{suffix}.claude.ai"),
                "203.0.113.20",
            )
            .await;
            assert_eq!(dns::system_servers().unwrap(), ["127.0.0.1"]);
            // Reconnection can also introduce an unsafe split. The helper must
            // quarantine it and report failure, then recover without a toggle.
            let _ = install(&new_search, true);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
            loop {
                let state =
                    tokio::task::spawn_blocking(move || service::check_protection_dns(listen_port))
                        .await
                        .unwrap();
                // A transient global-DNS mismatch can precede the helper's
                // refresh. Wait for its completed conflict quarantine.
                if state
                    .as_ref()
                    .err()
                    .is_some_and(|error| format!("{error:#}").contains("分域 DNS 与保护范围重叠"))
                {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "Conflicting reconnect was not quarantined: {state:?}"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            wait_resolver("claude.ai", "100.100.100.100", false).await;
            lookup(&format!("quarantined-{suffix}.claude.ai"), "203.0.113.20").await;
            (expected_vpn, expected_search) = install(&new_search, false);
            wait_protection_ready(listen_port).await;
            wait_split_recovery(&new_search, &vpn_queries).await;
            lookup(&format!("public-recovered-{suffix}.test"), "203.0.113.20").await;
        }
        // System daemons may query while protection is off between cycles.
        // Attribute only this test's unique names to the path under test.
        let captured = vpn_queries
            .lock()
            .unwrap()
            .iter()
            .filter(|host| host.contains(&suffix))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            captured.iter().all(|host| host.ends_with(&tail)
                || host.ends_with(&corp)
                || host.ends_with(&new_search)),
            "Public DNS reached VPN: {captured:?}"
        );
        assert!(
            !fallback_queries
                .lock()
                .unwrap()
                .iter()
                .any(|host| host.contains(&suffix)),
            "Public DNS reached physical fallback"
        );
        let leaked_probes = core_queries
            .lock()
            .unwrap()
            .iter()
            .filter(|host| host.starts_with("probe.cor-dns-"))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            leaked_probes.is_empty(),
            "Native DNS readiness probes must terminate at the local relay: {leaked_probes:?}"
        );
        tokio::task::spawn_blocking(|| service::protection_dns(false))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dns::configured().unwrap(), configured);
        assert_eq!(
            scutil(&format!("show {setup_key}\nquit\n")).stdout,
            expected_setup
        );
        assert_eq!(scutil(&format!("show {key}\nquit\n")).stdout, expected_vpn);
        assert_eq!(
            scutil(&format!("show {search_key}\nquit\n")).stdout,
            expected_search
        );
        assert_eq!(
            scutil(&format!("show {fallback_key}\nquit\n")).stdout,
            expected_fallback
        );
        lookup(&format!("restored-{attempt}.{tail}"), "100.100.12.34").await;
    }
    engine.select_proxy_mode(ProxyMode::Off).await.unwrap();
    vpn_job.abort();
    core_job.abort();
    fallback_job.abort();
    println!(
        "PASS: macOS public/protected DNS uses a local core listener without VPN/physical fallback; native split DNS, short names, reconnect and restoration survive"
    );
}

async fn wait_split_recovery(domain: &str, captured: &Mutex<Vec<String>>) {
    // The dynamic store can be ready before mDNSResponder switches its
    // supplemental resolver. Use fresh names to avoid caching a stale answer.
    wait_resolver(domain, "100.100.100.100", true).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    let expected = "100.100.12.34".parse::<IpAddr>().unwrap();
    let mut attempt = 0;
    loop {
        let host = format!("recovered-{attempt}.{domain}");
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            tokio::net::lookup_host((format!("{host}."), 80)),
        )
        .await;
        let observation = match result {
            Ok(Ok(addresses)) => {
                let addresses = addresses.map(|address| address.ip()).collect::<Vec<_>>();
                if addresses.contains(&expected) && captured.lock().unwrap().contains(&host) {
                    return;
                }
                format!("{addresses:?}")
            }
            Ok(Err(error)) => error.to_string(),
            Err(_) => "lookup timed out".into(),
        };
        assert!(
            tokio::time::Instant::now() < deadline,
            "Native split DNS did not recover for {domain}: {observation}"
        );
        attempt += 1;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_protection_ready(port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    loop {
        let state = tokio::task::spawn_blocking(move || service::check_protection_dns(port))
            .await
            .unwrap();
        if state.is_ok() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Native protection DNS was not ready after reconnect: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_resolver(domain: &str, server: &str, present: bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let shown = Command::new("/usr/sbin/scutil")
            .arg("--dns")
            .output()
            .unwrap();
        assert!(shown.status.success());
        if has_resolver(&String::from_utf8_lossy(&shown.stdout), domain, server) == present {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Native DNS did not publish expected resolver for {domain} -> {server} (present={present})"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
