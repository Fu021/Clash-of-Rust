use super::*;
use crate::{
    config::{Store, atomic_write},
    engine::Engine,
};
use anyhow::{Context, Result, bail, ensure};
use serde::de::DeserializeOwned;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Mutex, Semaphore, watch},
    task::JoinHandle,
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub protocol: u32,
    pub port: u16,
    pub pid: u32,
    pub owner: Owner,
    token: String,
}
impl Endpoint {
    fn path(store: &Store) -> PathBuf {
        store.root.join("control.json")
    }
    pub fn read(store: &Store) -> Result<Option<Self>> {
        let path = Self::path(store);
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let file = match options.open(&path) {
            Ok(value) => value,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.len() <= 4096,
            "本机控制端点文件无效"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
                "本机控制端点必须归当前用户所有且仅当前用户可读写"
            );
        }
        let mut bytes = Vec::new();
        file.take(4097).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 4096, "本机控制端点文件过大");
        let endpoint: Self = serde_json::from_slice(&bytes)?;
        ensure!(
            endpoint.protocol == PROTOCOL
                && endpoint.port != 0
                && endpoint.token.len() == 64
                && endpoint.token.bytes().all(|c| c.is_ascii_hexdigit()),
            "本机控制协议不兼容，请升级客户端或退出旧版 GUI／cor 后台后重试"
        );
        Ok(Some(endpoint))
    }
}

#[derive(Clone)]
pub struct Remote {
    endpoint: Endpoint,
    timeout: Duration,
}
impl Remote {
    pub fn new(endpoint: Endpoint, timeout: Duration) -> Self {
        Self { endpoint, timeout }
    }
    pub fn owner(&self) -> Owner {
        self.endpoint.owner
    }
    /// A shutdown acknowledgement precedes owner cleanup. Do not let the next
    /// CLI invocation attach to an owner that is already on its way out.
    pub async fn wait_closed(&self, store: &Store) -> Result<()> {
        tokio::time::timeout(self.timeout, async {
            loop {
                match Endpoint::read(store) {
                    Ok(current)
                        if current
                            .as_ref()
                            .is_none_or(|current| current.token != self.endpoint.token) =>
                    {
                        return Ok::<(), anyhow::Error>(());
                    }
                    Ok(_) => {}
                    // Windows rejects opening a file marked for deletion until
                    // its last reader closes it. Only an acknowledged shutdown
                    // uses this bounded wait; discovery keeps strict checks.
                    #[cfg(windows)]
                    Err(error)
                        if error
                            .downcast_ref::<std::io::Error>()
                            .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied) => {}
                    Err(error) => return Err(error).context("等待后台退出时读取控制端点失败"),
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .context("等待后台退出超时")?
    }
    pub fn discover(store: &Store, timeout: Duration) -> Result<Option<Self>> {
        Ok(Endpoint::read(store)?.map(|endpoint| Self::new(endpoint, timeout)))
    }
    pub fn call<T: DeserializeOwned>(&self, operation: Command) -> Result<T> {
        let address = SocketAddr::from(([127, 0, 0, 1], self.endpoint.port));
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
            .context("无法连接本机客户端后台")?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        let request = serde_json::to_vec(&Request {
            protocol: PROTOCOL,
            token: self.endpoint.token.clone(),
            operation,
        })?;
        ensure!(request.len() <= REQUEST_LIMIT, "命令超过 8 MiB 限制");
        stream.write_all(&(request.len() as u32).to_be_bytes())?;
        stream.write_all(&request)?;
        let mut length = [0; 4];
        stream.read_exact(&mut length)?;
        let length = u32::from_be_bytes(length) as usize;
        ensure!(length <= RESPONSE_LIMIT, "后台响应超过 32 MiB 限制");
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes)?;
        let reply: Reply = serde_json::from_slice(&bytes)?;
        if let Some(error) = reply.error {
            bail!("{error}");
        }
        Ok(serde_json::from_value(reply.result)?)
    }
    pub async fn request<T: DeserializeOwned + Send + 'static>(
        &self,
        operation: Command,
    ) -> Result<T> {
        let exchange = async {
            let mut stream = tokio::time::timeout(
                Duration::from_secs(2),
                tokio::net::TcpStream::connect(("127.0.0.1", self.endpoint.port)),
            )
            .await??;
            let request = serde_json::to_vec(&Request {
                protocol: PROTOCOL,
                token: self.endpoint.token.clone(),
                operation,
            })?;
            ensure!(request.len() <= REQUEST_LIMIT, "命令超过 8 MiB 限制");
            stream.write_u32(request.len() as u32).await?;
            stream.write_all(&request).await?;
            let size = stream.read_u32().await? as usize;
            ensure!(size <= RESPONSE_LIMIT, "后台响应超过 32 MiB 限制");
            let mut bytes = vec![0; size];
            stream.read_exact(&mut bytes).await?;
            let reply: Reply = serde_json::from_slice(&bytes)?;
            if let Some(error) = reply.error {
                bail!("{error}");
            }
            Ok::<T, anyhow::Error>(serde_json::from_value(reply.result)?)
        };
        tokio::time::timeout(self.timeout, exchange).await?
    }
}

pub struct Server {
    task: JoinHandle<()>,
    endpoint: Endpoint,
    path: PathBuf,
    shutdown: watch::Receiver<bool>,
}
impl Server {
    pub fn shutdown(&self) -> watch::Receiver<bool> {
        self.shutdown.clone()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
        if std::fs::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Endpoint>(&b).ok())
            .is_some_and(|endpoint| endpoint.token == self.endpoint.token)
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn authorized(request: &Request, endpoint: &Endpoint) -> bool {
    request.protocol == PROTOCOL
        && request.token.len() == endpoint.token.len()
        && request
            .token
            .bytes()
            .zip(endpoint.token.bytes())
            .fold(0, |sum, (a, b)| sum | (a ^ b))
            == 0
}

pub async fn serve(engine: &Arc<Mutex<Engine>>, owner: Owner) -> Result<()> {
    let mut locked = engine.lock().await;
    if locked.is_remote() || locked.control_server.is_some() {
        return Ok(());
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let endpoint = Endpoint {
        protocol: PROTOCOL,
        port: listener.local_addr()?.port(),
        pid: std::process::id(),
        owner,
        token: format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ),
    };
    let path = Endpoint::path(&locked.store);
    atomic_write(&path, &serde_json::to_vec(&endpoint)?)?;
    let weak = Arc::downgrade(engine);
    let captured = endpoint.clone();
    let (shutdown_tx, shutdown) = watch::channel(false);
    let task = tokio::spawn(async move {
        let permits = Arc::new(Semaphore::new(4));
        loop {
            let Ok((mut stream, address)) = listener.accept().await else {
                break;
            };
            if !address.ip().is_loopback() {
                continue;
            }
            let Ok(permit) = permits.clone().try_acquire_owned() else {
                continue;
            };
            let weak = weak.clone();
            let endpoint = captured.clone();
            let shutdown = shutdown_tx.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let incoming = async {
                    let length = stream.read_u32().await? as usize;
                    ensure!(length <= REQUEST_LIMIT, "命令过大");
                    let mut bytes = vec![0; length];
                    stream.read_exact(&mut bytes).await?;
                    Ok::<Request, anyhow::Error>(serde_json::from_slice(&bytes)?)
                };
                let operation = match tokio::time::timeout(Duration::from_secs(10), incoming).await
                {
                    Ok(Ok(request)) if authorized(&request, &endpoint) => request.operation,
                    _ => return,
                };
                let Some(engine) = weak.upgrade() else {
                    return;
                };
                let result = dispatch::execute(&engine, operation, owner).await;
                let shutting_down = matches!(&result, Ok(value) if value.get("shutdown").and_then(Value::as_bool)==Some(true));
                let reply = match result {
                    Ok(result) => Reply {
                        result,
                        error: None,
                    },
                    Err(error) => {
                        let secret = engine.lock().await.settings.secret.clone();
                        Reply {
                            result: Value::Null,
                            error: Some(format!("{error:#}").replace(&secret, "[密钥]")),
                        }
                    }
                };
                if let Ok(mut bytes) = serde_json::to_vec(&reply) {
                    if bytes.len() > RESPONSE_LIMIT {
                        bytes = serde_json::to_vec(&Reply {
                            result: Value::Null,
                            error: Some("结果超过 32 MiB，请缩小查询范围".into()),
                        })
                        .expect("static reply");
                    }
                    let _ = stream.write_u32(bytes.len() as u32).await;
                    let _ = stream.write_all(&bytes).await;
                    let _ = stream.shutdown().await;
                }
                if shutting_down {
                    let _ = shutdown.send(true);
                }
            });
        }
    });
    locked.control_server = Some(Server {
        task,
        endpoint,
        path,
        shutdown,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[tokio::test]
    async fn shutdown_waits_for_a_delete_pending_endpoint_reader() {
        use std::os::windows::ffi::OsStrExt;
        let root = tempfile::tempdir().unwrap();
        let store = Store::at(root.path().to_owned()).unwrap();
        let endpoint = Endpoint {
            protocol: PROTOCOL,
            port: 1,
            pid: std::process::id(),
            owner: Owner::Daemon,
            token: "a".repeat(64),
        };
        let path = Endpoint::path(&store);
        atomic_write(&path, &serde_json::to_vec(&endpoint).unwrap()).unwrap();
        let reader = std::fs::File::open(&path).unwrap();
        let wide: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
        assert_ne!(
            unsafe { windows_sys::Win32::Storage::FileSystem::DeleteFileW(wide.as_ptr()) },
            0
        );
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            drop(reader);
        });
        Remote::new(endpoint, Duration::from_secs(5))
            .wait_closed(&store)
            .await
            .unwrap();
        release.await.unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn protocol_and_secret_are_required_even_on_loopback() {
        let endpoint = Endpoint {
            protocol: PROTOCOL,
            port: 1,
            pid: 1,
            owner: Owner::Daemon,
            token: "a".repeat(64),
        };
        let mut request = Request {
            protocol: PROTOCOL,
            token: "a".repeat(64),
            operation: Command::Stop,
        };
        assert!(authorized(&request, &endpoint));
        request.token.replace_range(63.., "b");
        assert!(!authorized(&request, &endpoint));
        request.token = endpoint.token.clone();
        request.protocol += 1;
        assert!(!authorized(&request, &endpoint));
    }
}
