//! A bounded loopback-only bridge for native and resolv.conf DNS clients.
use anyhow::{Context, Result, ensure};
use std::{
    net::{Ipv4Addr, SocketAddr, TcpListener, UdpSocket},
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net,
    sync::{Semaphore, oneshot},
};

pub(crate) struct Relay {
    port: u16,
    probes: super::native::Probes,
    stop: Option<oneshot::Sender<()>>,
    worker: Option<JoinHandle<std::io::Result<()>>>,
}
impl Relay {
    pub fn start(port: u16) -> Result<Self> {
        ensure!(port >= 1024, "保护 DNS 内核端口无效");
        let listen = SocketAddr::from((Ipv4Addr::LOCALHOST, super::PORT));
        let udp =
            UdpSocket::bind(listen).context("本机 UDP DNS 端口 53 已被占用，未修改系统 DNS")?;
        let tcp =
            TcpListener::bind(listen).context("本机 TCP DNS 端口 53 已被占用，未修改系统 DNS")?;
        udp.set_nonblocking(true)?;
        tcp.set_nonblocking(true)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (tcp, udp) = {
            let _entered = runtime.enter();
            (
                net::TcpListener::from_std(tcp)?,
                Arc::new(net::UdpSocket::from_std(udp)?),
            )
        };
        let (stop, mut stopped) = oneshot::channel();
        let probes = Arc::new(Mutex::new(None));
        let active = probes.clone();
        let target = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let worker = std::thread::Builder::new().name("cor-local-dns".into()).spawn(move || {
            runtime.block_on(async move {
                let budget = Arc::new(Semaphore::new(64));
                let mut packet = vec![0; 65535];
                loop {
                    tokio::select! {
                        _ = &mut stopped => return Ok(()),
                        request = udp.recv_from(&mut packet) => {
                            let (count, peer) = request?;
                            if !peer.ip().is_loopback() || count < 12 { continue; }
                            if let Some(reply) = super::native::response(&packet[..count], &active) {
                                let _ = udp.send_to(&reply, peer).await;
                                continue;
                            }
                            let Ok(permit) = budget.clone().try_acquire_owned() else { continue; };
                            let packet = packet[..count].to_vec();
                            let listener = udp.clone();
                            let _forward = tokio::spawn(async move {
                                let _permit = permit;
                                let _ = tokio::time::timeout(Duration::from_secs(5), async {
                                    let upstream = net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?;
                                    upstream.connect(target).await?;
                                    upstream.send(&packet).await?;
                                    let mut response = vec![0; 65535];
                                    let count = upstream.recv(&mut response).await?;
                                    if count >= 12 && response[..2] == packet[..2] {
                                        listener.send_to(&response[..count], peer).await?;
                                    }
                                    Ok::<_, std::io::Error>(())
                                }).await;
                            });
                        }
                        connection = tcp.accept() => {
                            let (mut client, peer) = connection?;
                            if !peer.ip().is_loopback() { continue; }
                            let Ok(permit) = budget.clone().try_acquire_owned() else { continue; };
                            let active = active.clone();
                            let _forward = tokio::spawn(async move {
                                let _permit = permit;
                                let _ = tokio::time::timeout(Duration::from_secs(15), async {
                                    let count = client.read_u16().await?;
                                    if count < 12 { return Ok(()); }
                                    let mut packet = vec![0; usize::from(count)];
                                    client.read_exact(&mut packet).await?;
                                    if let Some(reply) = super::native::response(&packet, &active) {
                                        client.write_u16(reply.len() as u16).await?;
                                        client.write_all(&reply).await?;
                                        return Ok(());
                                    }
                                    let mut upstream = net::TcpStream::connect(target).await?;
                                    upstream.write_u16(count).await?;
                                    upstream.write_all(&packet).await?;
                                    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
                                    Ok::<_, std::io::Error>(())
                                }).await;
                            });
                        }
                    }
                }
            })
        })?;
        Ok(Self {
            port,
            probes,
            stop: Some(stop),
            worker: Some(worker),
        })
    }

    pub fn serving(&self, port: u16) -> bool {
        self.port == port
            && self
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
    }

    pub fn verify_native(&self) -> Result<()> {
        super::native::check(&self.probes, Duration::from_secs(2))
    }
}
impl Drop for Relay {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            match worker.join() {
                Ok(Ok(())) => {}
                result => eprintln!("保护 DNS 本机转发已停止：{result:?}"),
            }
        }
    }
}
