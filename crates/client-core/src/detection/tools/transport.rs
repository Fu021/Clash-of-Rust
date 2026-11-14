use super::*;
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::{TlsConnector, rustls};

pub(super) async fn socks(
    proxy: u16,
    target: &str,
    port: u16,
    command: u8,
) -> Result<(TcpStream, SocketAddr, u128, u128)> {
    let start = Instant::now();
    let mut stream = TcpStream::connect(("127.0.0.1", proxy)).await?;
    let connected = start.elapsed().as_millis();
    stream.write_all(&[5, 1, 0]).await?;
    let mut authentication = [0; 2];
    stream.read_exact(&mut authentication).await?;
    if authentication != [5, 0] {
        return Err(error::failure(
            "proxy_authentication",
            "本机代理拒绝无认证探针，未尝试其他出口",
        ));
    }
    let mut request = vec![5, command, 0];
    match target.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            request.push(1);
            request.extend_from_slice(&address.octets());
        }
        Ok(IpAddr::V6(address)) => {
            request.push(4);
            request.extend_from_slice(&address.octets());
        }
        Err(_) => {
            ensure!(
                !target.is_empty() && target.len() <= 253,
                "目标域名长度无效"
            );
            request.extend_from_slice(&[3, u8::try_from(target.len())?]);
            request.extend_from_slice(target.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    let target_start = Instant::now();
    stream.write_all(&request).await?;
    let mut header = [0; 4];
    stream.read_exact(&mut header).await?;
    if header[0] != 5 || header[2] != 0 {
        return Err(error::failure(
            "proxy_response_invalid",
            "SOCKS5 响应格式无效",
        ));
    }
    if header[1] != 0 {
        return Err(error::failure(
            match header[1] {
                2 => "proxy_rule_denied",
                3 | 4 => "proxy_target_unreachable",
                5 => "proxy_target_refused",
                7 => "proxy_command_unsupported",
                8 => "proxy_family_unsupported",
                _ => "proxy_target_failed",
            },
            "本机代理未能连接目标，请查看内核日志",
        ));
    }
    let address = match header[3] {
        1 => {
            let mut bytes = [0; 4];
            stream.read_exact(&mut bytes).await?;
            IpAddr::from(bytes)
        }
        4 => {
            let mut bytes = [0; 16];
            stream.read_exact(&mut bytes).await?;
            IpAddr::from(bytes)
        }
        3 => {
            let size = stream.read_u8().await?;
            let mut bytes = vec![0; usize::from(size)];
            stream.read_exact(&mut bytes).await?;
            std::str::from_utf8(&bytes)?
                .parse()
                .context("SOCKS5 中继未返回 IP 地址")?
        }
        _ => {
            return Err(error::failure(
                "proxy_response_invalid",
                "SOCKS5 响应地址类型无效",
            ));
        }
    };
    let bound_port = stream.read_u16().await?;
    Ok((
        stream,
        SocketAddr::new(address, bound_port),
        connected,
        target_start.elapsed().as_millis(),
    ))
}

fn tls() -> Result<TlsConnector> {
    let mut roots = rustls::RootCertStore::empty();
    for certificate in webpki_root_certs::TLS_SERVER_ROOT_CERTS {
        roots.add(certificate.clone())?;
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let configuration = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(configuration)))
}

async fn exchange(
    url: &reqwest::Url,
    destination: &str,
    context: &Environment,
    report: &mut Report,
) -> Result<()> {
    let host = url.host_str().context("缺少域名")?.trim_matches(['[', ']']);
    let port = url.port_or_known_default().context("目标端口未知")?;
    let (mut stream, _, local, remote) = socks(proxy_port(context)?, destination, port, 1).await?;
    report
        .entries
        .push(Entry::ok("本机代理 TCP", format!("{local} ms")));
    report.entries.push(Entry::ok(
        "目标 TCP",
        format!("{remote} ms（含代理侧 DNS；沿用当前线路）"),
    ));
    let mut path = url.path().to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    let request = format!(
        "HEAD {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        url.authority()
    );
    if url.scheme() == "https" {
        let start = Instant::now();
        let name = rustls::pki_types::ServerName::try_from(host.to_owned())?;
        let mut secure = tls()?
            .connect(name, stream)
            .await
            .map_err(|_| error::failure("tls_failed", "TLS 握手或证书验证失败，未绕过验证"))?;
        report.entries.push(Entry::ok(
            "TLS 握手",
            format!("{} ms", start.elapsed().as_millis()),
        ));
        let session = secure.get_ref().1;
        report.entries.push(Entry::ok(
            "TLS 协议",
            format!("{:?}", session.protocol_version().context("TLS 协议未知")?),
        ));
        if let Some(chain) = session.peer_certificates() {
            report.entries.push(Entry::ok(
                "证书验证",
                format!(
                    "域名、有效期和信任链验证通过 · 服务端提供 {} 张证书",
                    chain.len()
                ),
            ));
            if let Some(certificate) = chain.first()
                && certificate_entries(certificate.as_ref(), &mut report.entries).is_err()
            {
                report.entries.push(Entry::unknown(
                    "证书详情",
                    "握手已验证证书，但无法展示证书字段",
                    "certificate_display_invalid",
                ));
            }
        }
        let start = Instant::now();
        secure.write_all(request.as_bytes()).await?;
        secure.flush().await?;
        let mut byte = [0];
        if secure.read(&mut byte).await? != 1 {
            return Err(error::failure("empty_response", "目标在响应前关闭连接"));
        }
        report.entries.push(Entry::ok(
            "等待首字节",
            format!("{} ms（请求写入后）", start.elapsed().as_millis()),
        ));
    } else {
        let start = Instant::now();
        stream.write_all(request.as_bytes()).await?;
        stream.flush().await?;
        let mut byte = [0];
        if stream.read(&mut byte).await? != 1 {
            return Err(error::failure("empty_response", "目标在响应前关闭连接"));
        }
        report.entries.push(Entry::ok(
            "等待首字节",
            format!("{} ms（请求写入后）", start.elapsed().as_millis()),
        ));
        report.entries.push(Entry::unknown(
            "TLS",
            "HTTP 目标没有 TLS 握手",
            "not_applicable",
        ));
    }
    Ok(())
}

pub(super) async fn run(
    tool: Tool,
    url: &reqwest::Url,
    context: &Environment,
    options: &Options,
    report: &mut Report,
) -> Result<()> {
    proxy_port(context)?;
    if tool == Tool::Family {
        let addresses = resolve(url, context).await?;
        for (label, v6) in [("IPv4", false), ("IPv6", true)] {
            let Some(address) = addresses.iter().find(|address| address.is_ipv6() == v6) else {
                report.entries.push(Entry::unknown(
                    label,
                    "未返回此地址族的目标地址，未发起连接",
                    "family_no_address",
                ));
                continue;
            };
            let mut sample = Report {
                entries: Vec::new(),
                ..report.clone()
            };
            let start = Instant::now();
            match tokio::time::timeout(
                Duration::from_secs(options.seconds),
                exchange(url, &address.to_string(), context, &mut sample),
            )
            .await
            {
                Ok(Ok(())) => report.entries.push(Entry::ok(
                    label,
                    format!("{address} · {} ms", start.elapsed().as_millis()),
                )),
                Ok(Err(error)) => report.entries.push(error::entry(label, &error)),
                Err(_) => report.entries.push(Entry::error(
                    label,
                    format!("{address} · 连接超时"),
                    "family_timeout",
                )),
            }
        }
        report.entries.push(Entry::unknown("检查路径", "强制目标 IP，经本机 SOCKS5 代理；IP 规则可能与普通域名请求不同。没有 IPv6 地址不代表 TUN 绕行", "information"));
        return Ok(());
    }
    let mut url = url.clone();
    if tool == Tool::Tls {
        url.set_scheme("https")
            .map_err(|_| anyhow::anyhow!("无法检查 TLS 地址"))?;
    }
    let host = url
        .host_str()
        .context("缺少域名")?
        .trim_matches(['[', ']'])
        .to_owned();
    if tool == Tool::Phase {
        let start = Instant::now();
        match resolve(&url, context).await {
            Ok(addresses) => report.entries.push(Entry::ok(
                "内核 DNS（独立查询）",
                format!(
                    "{} ms · {} 个地址；不计入目标 TCP 耗时",
                    start.elapsed().as_millis(),
                    addresses.len()
                ),
            )),
            Err(error) => report.entries.push(Entry::unknown(
                "内核 DNS（独立查询）",
                format!("{error:#}"),
                "dns_not_observed",
            )),
        }
    }
    tokio::time::timeout(
        Duration::from_secs(options.seconds),
        exchange(&url, &host, context, report),
    )
    .await
    .map_err(|_| error::failure("connection_timeout", "本次连接达到时限，已停止检查"))??;
    report.entries.push(Entry::unknown(
        "路径",
        "经本机 SOCKS5 代理，不切换模式、节点或 DNS；分阶段值与独立的 DNS 查询不直接相加",
        "information",
    ));
    Ok(())
}

fn field<'a>(data: &mut &'a [u8]) -> Result<(u8, &'a [u8])> {
    ensure!(data.len() >= 2, "证书字段截断");
    let tag = data[0];
    let first = data[1];
    let (header, size) = if first & 0x80 == 0 {
        (2, usize::from(first))
    } else {
        let count = usize::from(first & 0x7f);
        ensure!(
            (1..=4).contains(&count) && data.len() >= count + 2,
            "证书字段长度无效"
        );
        let mut size = 0usize;
        for byte in &data[2..2 + count] {
            size = size
                .checked_mul(256)
                .and_then(|size| size.checked_add(usize::from(*byte)))
                .context("证书字段长度溢出")?;
        }
        (count + 2, size)
    };
    let end = header.checked_add(size).context("证书字段长度溢出")?;
    ensure!(end <= data.len(), "证书字段截断");
    let value = &data[header..end];
    *data = &data[end..];
    Ok((tag, value))
}

fn common_name(mut sequence: &[u8]) -> Result<String> {
    for _ in 0..32 {
        if sequence.is_empty() {
            break;
        }
        let (_, mut set) = field(&mut sequence)?;
        for _ in 0..16 {
            if set.is_empty() {
                break;
            }
            let (_, mut pair) = field(&mut set)?;
            let (tag, identifier) = field(&mut pair)?;
            if tag == 6 && identifier == [0x55, 4, 3] {
                let (tag, value) = field(&mut pair)?;
                if matches!(tag, 12 | 19 | 20 | 22) {
                    return Ok(bounded(
                        std::str::from_utf8(value).unwrap_or("无法解码的名称"),
                        160,
                    ));
                }
            }
        }
    }
    Ok("证书未提供通用名称".into())
}

pub(super) fn certificate_entries(der: &[u8], entries: &mut Vec<Entry>) -> Result<()> {
    use sha2::{Digest, Sha256};
    ensure!(der.len() <= 128 * 1024, "证书数据过大");
    let mut remaining = der;
    let (tag, mut certificate) = field(&mut remaining)?;
    ensure!(tag == 0x30 && remaining.is_empty(), "证书外层格式无效");
    let (_, mut body) = field(&mut certificate)?;
    let mut serial = field(&mut body)?;
    if serial.0 == 0xa0 {
        serial = field(&mut body)?;
    }
    ensure!(serial.0 == 2, "证书序列号格式无效");
    let _ = field(&mut body)?;
    let (_, issuer) = field(&mut body)?;
    let (_, mut validity) = field(&mut body)?;
    let (_, before) = field(&mut validity)?;
    let (_, after) = field(&mut validity)?;
    let (_, subject) = field(&mut body)?;
    entries.push(Entry::ok("证书主体", common_name(subject)?));
    entries.push(Entry::ok("签发者", common_name(issuer)?));
    entries.push(Entry::ok(
        "有效期（UTC）",
        format!(
            "{} ～ {}",
            bounded(std::str::from_utf8(before)?, 32),
            bounded(std::str::from_utf8(after)?, 32)
        ),
    ));
    entries.push(Entry::ok(
        "证书 SHA256",
        crate::file_hash::hex(Sha256::digest(der)),
    ));
    Ok(())
}
