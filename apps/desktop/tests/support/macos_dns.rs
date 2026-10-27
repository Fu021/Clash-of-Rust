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

async fn stub(
    address: [u8; 4],
    drop_responses: Arc<AtomicBool>,
) -> (u16, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let queries = Arc::new(Mutex::new(vec![]));
    let captured = queries.clone();
    let job = tokio::spawn(async move {
        let mut buffer = [0; 4096];
        loop {
            let (count, peer) = socket.recv_from(&mut buffer).await.unwrap();
            if count < 17 || buffer[4..6] != [0, 1] {
                continue;
            }
            let mut end = 12;
            let mut labels = vec![];
            while end < count && buffer[end] != 0 {
                let length = usize::from(buffer[end]);
                if length > 63 || end + length + 1 >= count {
                    break;
                }
                labels
                    .push(String::from_utf8_lossy(&buffer[end + 1..end + 1 + length]).into_owned());
                end += length + 1;
            }
            if end + 5 > count {
                continue;
            }
            captured.lock().unwrap().push(labels.join("."));
            if drop_responses.load(Ordering::Relaxed) {
                continue;
            }
            let answer = buffer[end + 1..end + 3] == [0, 1];
            let mut reply = buffer[..end + 5].to_vec();
            reply[2..4].copy_from_slice(&[0x81, 0x80]);
            reply[6..12].copy_from_slice(&[0, u8::from(answer), 0, 0, 0, 0]);
            if answer {
                reply.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 1, 0, 4]);
                reply.extend_from_slice(&address);
            }
            socket.send_to(&reply, peer).await.unwrap();
        }
    });
    (port, queries, job)
}

async fn lookup_name(host: &str, expected: &str) {
    let addresses =
        tokio::time::timeout(Duration::from_secs(12), tokio::net::lookup_host((host, 80)))
            .await
            .expect("native DNS timed out")
            .unwrap()
            .map(|address| address.ip())
            .collect::<Vec<_>>();
    assert!(
        addresses.contains(&expected.parse::<IpAddr>().unwrap()),
        "Unexpected native DNS answer for {host}: {addresses:?}"
    );
}

async fn lookup(host: &str, expected: &str) {
    lookup_name(&format!("{host}."), expected).await;
}

pub async fn run(engine: &mut Engine, profile: &Path) {
    engine.select_proxy_mode(ProxyMode::Off).await.unwrap();
    let configured = dns::configured().unwrap();
    let (vpn_port, vpn_queries, vpn_job) =
        stub([100, 100, 12, 34], Arc::new(AtomicBool::new(false))).await;
    let drop_core_dns = Arc::new(AtomicBool::new(false));
    let (core_port, core_queries, core_job) = stub([203, 0, 113, 20], drop_core_dns.clone()).await;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let tail = format!("tail-{suffix}.ts.net");
    let corp = format!("{suffix}.corp.test");
    let key = format!("State:/Network/Service/cor-dns-fixture-{suffix}/DNS");
    let _cleanup = Cleanup(key.clone());
    let install = |search: &str| {
        scutil(&format!(
            "d.init\nd.add ServerAddresses * 127.0.0.1\nd.add ServerPort # {vpn_port}\n\
             d.add SearchOrder # 100600\nd.add SupplementalMatchDomains * \"\" {tail} {corp} claude.ai bash.ws\n\
             d.add SupplementalMatchDomainsNoSearch # 1\nd.add SearchDomains * {search}\nset {key}\nquit\n"
        ));
    };
    install(&tail);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while dns::system_servers().unwrap() != ["127.0.0.1"] {
        assert!(
            tokio::time::Instant::now() < deadline,
            "VPN resolver not selected"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    std::fs::write(
        profile,
        format!(
            "proxies: []\nrules: ['MATCH,DIRECT']\n\
             dns: {{enable: true, enhanced-mode: redir-host, nameserver: ['127.0.0.1:{core_port}']}}\n\
             tun: {{device: utun233, route-address: ['1.1.1.1/32', '8.8.8.8/32'], dns-hijack: ['any:53', 'tcp://any:53']}}\n"
        ),
    )
    .unwrap();
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
    for attempt in 0..2 {
        let mut expected_vpn = scutil(&format!("show {key}\nquit\n")).stdout;
        tokio::task::spawn_blocking(|| service::protection_dns(true))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dns::system_servers().unwrap(), ["1.1.1.1", "8.8.8.8"]);
        assert_eq!(dns::configured().unwrap(), configured);
        assert_eq!(scutil(&format!("show {key}\nquit\n")).stdout, expected_vpn);
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
        if attempt == 0 {
            lookup_name(&format!("short-{suffix}"), "100.100.12.34").await;
        }
        lookup(&format!("peer-{attempt}.{corp}"), "100.100.12.34").await;
        if attempt == 0 {
            // An unreachable protected upstream must never make macOS retry a
            // protected query on the VPN resolver with the same match suffix.
            let failed_host = format!("failed-{suffix}.claude.ai");
            drop_core_dns.store(true, Ordering::Relaxed);
            let _result = tokio::time::timeout(
                Duration::from_secs(25),
                tokio::net::lookup_host((format!("{failed_host}."), 80)),
            )
            .await;
            drop_core_dns.store(false, Ordering::Relaxed);
            assert!(
                core_queries
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|host| host == &failed_host),
                "Protected failure fixture did not reach the core DNS: {failed_host}"
            );
            assert!(
                !vpn_queries
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|host| host == &failed_host),
                "Protected DNS fell back to the VPN resolver: {failed_host}"
            );
            // Simulate a VPN changing its own configuration while the
            // protection resolver is installed. Its settings remain its own.
            install(&format!("{suffix}.new-search.test"));
            expected_vpn = scutil(&format!("show {key}\nquit\n")).stdout;
            lookup(&format!("reconnected-{attempt}.{tail}"), "100.100.12.34").await;
        }
        let captured = vpn_queries.lock().unwrap().clone();
        assert!(
            captured
                .iter()
                .all(|host| host.ends_with(&tail) || host.ends_with(&corp)),
            "Public DNS reached the VPN resolver: {captured:?}"
        );
        assert!(!core_queries.lock().unwrap().is_empty());
        tokio::task::spawn_blocking(|| service::protection_dns(false))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dns::system_servers().unwrap(), ["127.0.0.1"]);
        assert_eq!(dns::configured().unwrap(), configured);
        assert_eq!(scutil(&format!("show {key}\nquit\n")).stdout, expected_vpn);
        lookup(&format!("restored-{attempt}.{tail}"), "100.100.12.34").await;
    }
    engine.select_proxy_mode(ProxyMode::Off).await.unwrap();
    vpn_job.abort();
    core_job.abort();
    println!(
        "PASS: macOS supplemental default and protected suffixes use TUN DNS; VPN private DNS, search settings and persistent preferences survive enable/disable and reconnect"
    );
}
