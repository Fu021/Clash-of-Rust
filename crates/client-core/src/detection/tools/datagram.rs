use super::*;
use tokio::{io::AsyncWriteExt, net::UdpSocket};

fn question(identifier: u16) -> Vec<u8> {
    let mut bytes = vec![0; 12];
    bytes[..2].copy_from_slice(&identifier.to_be_bytes());
    bytes[2] = 1;
    bytes[5] = 1;
    bytes.extend_from_slice(b"\x07example\x03com\0\0\x01\0\x01");
    bytes
}

fn reply(bytes: &[u8], identifier: u16) -> Result<()> {
    ensure!(
        bytes.len() >= 4 && bytes[..3] == [0, 0, 0],
        "UDP 中继响应格式无效或不支持分片"
    );
    let offset = match bytes[3] {
        1 => 10,
        4 => 22,
        3 => 7 + usize::from(*bytes.get(4).context("UDP 中继地址截断")?),
        _ => bail!("UDP 中继地址类型无效"),
    };
    let dns = bytes.get(offset..).context("UDP 中继响应截断")?;
    ensure!(dns.len() >= 12, "DNS 响应截断");
    ensure!(
        dns[..2] == identifier.to_be_bytes(),
        "DNS 响应不属于本次探针"
    );
    ensure!(dns[2] & 0x80 != 0, "收到的数据不是 DNS 响应");
    ensure!(dns[2] & 2 == 0, "DNS 响应被截断，不将其视为完整响应");
    ensure!(dns[3] & 15 == 0, "DNS 响应错误码 {}", dns[3] & 15);
    Ok(())
}

pub(super) async fn run(
    url: &reqwest::Url,
    target: &str,
    context: &Environment,
    options: &Options,
    report: &mut Report,
) -> Result<()> {
    ensure!(!context.protected, "Claude 保护期间不发送 UDP 探针");
    let host = url
        .host_str()
        .context("缺少 DNS 服务器地址")?
        .trim_matches(['[', ']']);
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(error::failure(
            "invalid_target",
            "UDP 探针需要 DNS 服务器地址，不支持网址路径或查询参数",
        ));
    }
    let port = server_port(target)?;
    let (mut control, relay, _, _) =
        super::transport::socks(proxy_port(context)?, "0.0.0.0", 0, 3).await?;
    let address = if relay.ip().is_unspecified() {
        std::net::SocketAddr::from(([127, 0, 0, 1], relay.port()))
    } else {
        relay
    };
    ensure!(
        address.ip().is_loopback() && address.port() != 0,
        "内核未提供可用的本机 UDP 中继"
    );
    let socket = if address.is_ipv6() {
        UdpSocket::bind("[::1]:0").await?
    } else {
        UdpSocket::bind("127.0.0.1:0").await?
    };
    socket.connect(address).await?;
    let mut packet = vec![0, 0, 0];
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            packet.push(1);
            packet.extend_from_slice(&ip.octets());
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            packet.push(4);
            packet.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            ensure!(host.len() <= 253, "DNS 服务器域名过长");
            packet.extend_from_slice(&[3, u8::try_from(host.len())?]);
            packet.extend_from_slice(host.as_bytes());
        }
    }
    packet.extend_from_slice(&port.to_be_bytes());
    let identifier = u16::from_be_bytes(uuid::Uuid::new_v4().as_bytes()[..2].try_into()?);
    packet.extend_from_slice(&question(identifier));
    let start = Instant::now();
    socket.send(&packet).await?;
    let mut response = [0u8; 4096];
    let received = tokio::time::timeout(
        Duration::from_secs(options.seconds),
        socket.recv(&mut response),
    )
    .await;
    match received {
        Ok(Ok(size)) => {
            if reply(&response[..size], identifier).is_err() {
                report.entries.push(Entry::error(
                    "DNS / UDP",
                    "收到不匹配、截断或错误的 DNS 响应",
                    "udp_invalid_response",
                ));
                return Ok(());
            }
            report.entries.push(Entry::ok(
                "DNS / UDP",
                format!("收到有效响应 · {} ms", start.elapsed().as_millis()),
            ));
        }
        Ok(Err(error)) => report.entries.push(Entry::error(
            "DNS / UDP",
            error.to_string(),
            "udp_io_failed",
        )),
        Err(_) => report.entries.push(Entry::unknown(
            "DNS / UDP",
            "本次探针未获响应；不代表所有 UDP 不可用",
            "udp_no_response",
        )),
    }
    report.entries.push(Entry::unknown(
        "路径",
        "经本机 SOCKS5 UDP 中继，不修改用户 UDP 设置或绕过当前出站规则",
        "information",
    ));
    let _ = control.shutdown().await;
    Ok(())
}

fn server_port(target: &str) -> Result<u16> {
    let authority = target
        .trim()
        .split_once("://")
        .map_or(target.trim(), |(_, rest)| rest)
        .trim_end_matches('/');
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') || host.ends_with(']') => port
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| error::failure("invalid_target", "DNS 服务器端口必须为 1～65535")),
        _ => Ok(53),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datagram_reply_requires_matching_unfragmented_complete_dns_response() {
        let mut bytes = vec![0, 0, 0, 1, 127, 0, 0, 1, 0, 53];
        let mut dns = question(42);
        dns[2] = 0x81;
        bytes.extend(dns);
        assert!(reply(&bytes, 42).is_ok());
        assert!(reply(&bytes, 43).is_err());
        bytes[2] = 1;
        assert!(reply(&bytes, 42).is_err());
        bytes[2] = 0;
        bytes[12] |= 2;
        assert!(reply(&bytes, 42).is_err());
        bytes[12] &= !2;
        bytes[13] = 3;
        assert!(reply(&bytes, 42).is_err());
        bytes[13] = 0;
        bytes.truncate(14);
        assert!(reply(&bytes, 42).is_err());
    }

    #[tokio::test]
    async fn udp_probe_uses_only_the_loopback_relay_and_checks_the_response() {
        use tokio::{io::AsyncReadExt, net::TcpListener};
        for respond in [true, false] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let proxy_port = listener.local_addr().unwrap().port();
            let relay = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let relay_port = relay.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (mut control, _) = listener.accept().await.unwrap();
                let mut greeting = [0; 3];
                control.read_exact(&mut greeting).await.unwrap();
                assert_eq!(greeting, [5, 1, 0]);
                control.write_all(&[5, 0]).await.unwrap();
                let mut request = [0; 10];
                control.read_exact(&mut request).await.unwrap();
                assert_eq!(&request[..4], &[5, 3, 0, 1]);
                let mut response = vec![5, 0, 0, 1, 127, 0, 0, 1];
                response.extend_from_slice(&relay_port.to_be_bytes());
                control.write_all(&response).await.unwrap();
                let mut packet = [0; 4096];
                let (size, peer) = relay.recv_from(&mut packet).await.unwrap();
                assert_eq!(&packet[..10], &[0, 0, 0, 1, 192, 0, 2, 53, 0, 53]);
                if respond {
                    packet[12] |= 0x80;
                    relay.send_to(&packet[..size], peer).await.unwrap();
                }
                let mut closed = [0];
                let _ = control.read(&mut closed).await;
            });
            let result = super::super::run(
                Tool::Udp,
                "192.0.2.53",
                Environment {
                    port: Some(proxy_port),
                    api: None,
                    protected: false,
                    udp_enabled: true,
                },
                Options {
                    seconds: 1,
                    ..Options::default()
                },
            )
            .await
            .unwrap();
            server.await.unwrap();
            assert_eq!(
                result.entries[0].code,
                if respond { "ok" } else { "udp_no_response" }
            );
            assert_eq!(
                result.status,
                if respond { "complete" } else { "unavailable" }
            );
        }
    }

    #[test]
    fn dns_server_ports_preserve_explicit_http_default_ports() {
        for (target, port) in [
            ("1.1.1.1", 53),
            ("https://1.1.1.1:443/", 443),
            ("http://dns.example:80", 80),
            ("[::1]", 53),
            ("[::1]:443", 443),
        ] {
            assert_eq!(server_port(target).unwrap(), port);
        }
        assert!(server_port("1.1.1.1:0").is_err());
    }
}
