use super::security::{Descriptor, Handle, wide};
use anyhow::{Context, Result, bail, ensure};
use serde::{Serialize, de::DeserializeOwned};
use std::time::{Duration, Instant};
use windows_sys::Win32::{
    Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
        ERROR_PIPE_LISTENING, GetLastError, INVALID_HANDLE_VALUE,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_READ_ATTRIBUTES, FILE_READ_DATA,
        FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile,
        WriteFile,
    },
    System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_NOWAIT, PIPE_REJECT_REMOTE_CLIENTS,
        SetNamedPipeHandleState,
    },
};

pub const NAME: &str = r"\\.\pipe\ClashOfRust.Network.v1";
const LIMIT: usize = 64 * 1024;
pub struct Pipe(pub Handle, Vec<u8>);
impl Pipe {
    pub fn server(first: bool) -> Result<Self> {
        // No FILE_CREATE_PIPE_INSTANCE right is granted to interactive users.
        let security = Descriptor::new("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x00120183;;;IU)")?;
        let name = wide(NAME);
        let flags = PIPE_ACCESS_DUPLEX
            | if first {
                FILE_FLAG_FIRST_PIPE_INSTANCE
            } else {
                0
            };
        // SAFETY: The terminated name, attributes temporary and descriptor remain alive
        // through this synchronous call. No overlapped or inheritable handle is created;
        // the returned server handle is transferred once to Handle.
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                flags,
                PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                4,
                65536,
                65536,
                1000,
                &security.attributes(),
            )
        };
        Ok(Self(Handle::new(handle)?, Vec::new()))
    }
    pub fn connected(&self) -> Result<bool> {
        // SAFETY: self owns a non-overlapped server pipe. No OVERLAPPED storage is
        // needed, and the handle remains open until this non-waiting call returns.
        if unsafe { ConnectNamedPipe(self.0.raw(), std::ptr::null_mut()) } != 0 {
            return Ok(true);
        }
        // SAFETY: Read the calling thread's error immediately after the failed call.
        match unsafe { GetLastError() } {
            ERROR_PIPE_CONNECTED => Ok(true),
            ERROR_PIPE_LISTENING | ERROR_NO_DATA => Ok(false),
            code => bail!("网络助手连接失败：{code}"),
        }
    }
    pub fn client() -> Result<Self> {
        let name = wide(NAME);
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            // SAFETY: The terminated pipe name is alive for this synchronous open.
            // Null security/template pointers request defaults; no overlapped or
            // inherited handle is requested. A successful handle is adopted once.
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    FILE_READ_DATA
                        | FILE_WRITE_DATA
                        | FILE_READ_ATTRIBUTES
                        | FILE_WRITE_ATTRIBUTES
                        | 0x00100000,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                let handle = Handle::new(handle)?;
                let mode = PIPE_NOWAIT;
                // SAFETY: The owned client handle and readable mode remain alive
                // through the call; null collection parameters leave them unchanged.
                ensure!(
                    unsafe {
                        SetNamedPipeHandleState(
                            handle.raw(),
                            &mode,
                            std::ptr::null(),
                            std::ptr::null(),
                        )
                    } != 0,
                    "无法初始化助手通信"
                );
                return Ok(Self(handle, Vec::new()));
            }
            // SAFETY: Capture CreateFileW's error before formatting or wrapping it.
            let error = unsafe { GetLastError() };
            ensure!(
                matches!(error, ERROR_PIPE_BUSY | ERROR_FILE_NOT_FOUND)
                    && Instant::now() < deadline,
                "Windows 网络助手未就绪：{error}，请修复安装后重试"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    pub fn write<T: Serialize>(&self, value: &T) -> Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        bytes.push(b'\n');
        ensure!(bytes.len() <= LIMIT, "助手消息超过大小限制");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut offset = 0;
        while offset < bytes.len() {
            let mut written = 0;
            // SAFETY: LIMIT bounds the readable suffix below DWORD::MAX. The pipe
            // is non-overlapped, so bytes and written are borrowed only until return.
            let result = unsafe {
                WriteFile(
                    self.0.raw(),
                    bytes[offset..].as_ptr(),
                    (bytes.len() - offset) as u32,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if result == 0 {
                // SAFETY: Capture this thread's last error before any other OS call.
                let code = unsafe { GetLastError() };
                ensure!(
                    code == ERROR_NO_DATA && Instant::now() < deadline,
                    "助手连接已关闭或写入超时：{code}"
                );
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            offset += written as usize;
            ensure!(Instant::now() < deadline, "助手写入超时");
            if written == 0 {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(())
    }
    pub fn read<T: DeserializeOwned>(&mut self, timeout: Duration) -> Result<T> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(end) = self.1.iter().position(|&b| b == b'\n') {
                let result = serde_json::from_slice(&self.1[..end]).context("助手消息无效");
                self.1.drain(..=end);
                return result;
            }
            ensure!(self.1.len() < LIMIT, "助手消息超过大小限制");
            let mut chunk = [0u8; 4096];
            let mut count = 0;
            // SAFETY: chunk is writable for its advertised DWORD-sized capacity;
            // count and the owned non-overlapped pipe remain alive through the call.
            // Only a successful call's initialized prefix is consumed below.
            if unsafe {
                ReadFile(
                    self.0.raw(),
                    chunk.as_mut_ptr(),
                    chunk.len() as u32,
                    &mut count,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                // SAFETY: Capture this thread's last error before any other OS call.
                let code = unsafe { GetLastError() };
                ensure!(code == ERROR_NO_DATA, "助手连接已关闭：{code}");
            } else if count > 0 {
                self.1.extend_from_slice(&chunk[..count as usize]);
                continue;
            } else {
                bail!("助手连接已关闭");
            }
            ensure!(Instant::now() < deadline, "助手响应超时");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
