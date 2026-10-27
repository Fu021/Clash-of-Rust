//! A bounded loopback-only bridge for native and resolv.conf DNS clients.
use anyhow::{Context, Result, ensure};
use std::{
    net::{Ipv4Addr, SocketAddr, TcpListener, UdpSocket},
    sync::Arc,
    thread::JoinHandle,
    time::Duration,
};
use tokio::{
    net,
    sync::{Semaphore, oneshot},
};

pub(crate) struct Relay {
    port: u16,
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
                            let _forward = tokio::spawn(async move {
                                let _permit = permit;
                                let _ = tokio::time::timeout(Duration::from_secs(15), async {
                                    let mut upstream = net::TcpStream::connect(target).await?;
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
