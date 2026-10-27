//! Confirm that mDNSResponder has consumed the overlay, using a local-only reply.
use anyhow::{Context, Result, ensure};
use std::{
    ffi::{CString, c_char, c_void},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

type Service = *mut c_void;
type QueryCallback = unsafe extern "C" fn(
    Service,
    u32,
    u32,
    i32,
    *const c_char,
    u16,
    u16,
    u16,
    *const c_void,
    u32,
    *mut c_void,
);

// macOS exports DNS-SD through the implicitly linked libSystem.
unsafe extern "C" {
    fn DNSServiceQueryRecord(
        service: *mut Service,
        flags: u32,
        interface: u32,
        name: *const c_char,
        record_type: u16,
        class: u16,
        callback: QueryCallback,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceRefSockFD(service: Service) -> i32;
    fn DNSServiceProcessResult(service: Service) -> i32;
    fn DNSServiceRefDeallocate(service: Service);
}

pub(super) struct Probe {
    question: Vec<u8>,
    received: AtomicBool,
}
pub(super) type Probes = Arc<Mutex<Option<Arc<Probe>>>>;

struct Registration {
    probes: Probes,
    probe: Arc<Probe>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut current) = self.probes.lock()
            && current
                .as_ref()
                .is_some_and(|probe| Arc::ptr_eq(probe, &self.probe))
        {
            *current = None;
        }
    }
}
struct Query(Service);
impl Drop for Query {
    fn drop(&mut self) {
        unsafe { DNSServiceRefDeallocate(self.0) };
    }
}
unsafe extern "C" fn answered(
    _: Service,
    _: u32,
    _: u32,
    _: i32,
    _: *const c_char,
    _: u16,
    _: u16,
    _: u16,
    _: *const c_void,
    _: u32,
    _: *mut c_void,
) {
}

impl Probe {
    fn new(name: &str) -> Self {
        let mut question = Vec::new();
        for label in name.trim_end_matches('.').split('.') {
            question.push(label.len() as u8);
            question.extend_from_slice(label.as_bytes());
        }
        question.push(0);
        Self {
            question,
            received: AtomicBool::new(false),
        }
    }

    fn response(&self, packet: &[u8]) -> Option<Vec<u8>> {
        let end = 12 + self.question.len();
        if packet.len() < end + 4
            || packet[2] & 0xf8 != 0
            || packet[4..6] != [0, 1]
            || !packet[12..end].eq_ignore_ascii_case(&self.question)
            || !matches!(&packet[end..end + 2], [0, 1] | [0, 28])
            || packet[end + 2..end + 4] != [0, 1]
        {
            return None;
        }
        // Terminate the active probe here instead of forwarding it to mihomo.
        // Negative replies leave no positive synthetic address in the cache.
        let mut reply = packet[..end + 4].to_vec();
        reply[2..4].copy_from_slice(&[0x81, 0x83]);
        reply[6..12].fill(0);
        self.received.store(true, Ordering::Release);
        Some(reply)
    }
}

pub(super) fn response(packet: &[u8], probes: &Probes) -> Option<Vec<u8>> {
    probes.lock().ok()?.as_ref()?.response(packet)
}

pub(super) fn check(probes: &Probes, timeout: Duration) -> Result<()> {
    // A random final label cannot match a configured private suffix (including
    // common testing suffixes). It contains no user's lookup or network data.
    let name = format!("probe.cor-dns-{}.", uuid::Uuid::new_v4().simple());
    let probe = Arc::new(Probe::new(&name));
    {
        let mut current = probes
            .lock()
            .map_err(|_| anyhow::anyhow!("DNS 探针状态不可用"))?;
        ensure!(current.is_none(), "DNS 探针正在运行");
        *current = Some(probe.clone());
    }
    let _registration = Registration {
        probes: probes.clone(),
        probe: probe.clone(),
    };
    let name = CString::new(name)?;
    let mut service = ptr::null_mut();
    let status = unsafe {
        DNSServiceQueryRecord(
            &mut service,
            0,
            0,
            name.as_ptr(),
            1,
            1,
            answered,
            ptr::null_mut(),
        )
    };
    ensure!(
        status == 0 && !service.is_null(),
        "无法检查系统原生 DNS：{status}"
    );
    let query = Query(service);
    let fd = unsafe { DNSServiceRefSockFD(query.0) };
    ensure!(fd >= 0, "系统原生 DNS 检查连接无效");
    let deadline = Instant::now() + timeout;
    while !probe.received.load(Ordering::Acquire) {
        ensure!(Instant::now() < deadline, "系统原生 DNS 尚未进入保护入口");
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, 20) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error).context("无法等待系统原生 DNS 检查");
        }
        if ready > 0 {
            ensure!(
                descriptor.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) == 0,
                "系统原生 DNS 检查连接已断开"
            );
            ensure!(
                unsafe { DNSServiceProcessResult(query.0) } == 0,
                "系统原生 DNS 检查失败"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(probe: &Probe, kind: u16) -> Vec<u8> {
        let mut packet = vec![0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        packet.extend_from_slice(&probe.question);
        packet.extend_from_slice(&kind.to_be_bytes());
        packet.extend_from_slice(&[0, 1]);
        packet
    }

    #[test]
    fn only_the_active_native_nonce_confirms_the_resolver_path() {
        let probe = Arc::new(Probe::new("cor-dns-first.test."));
        let probes = Arc::new(Mutex::new(Some(probe.clone())));
        let other = Probe::new("cor-dns-other.test.");
        assert!(response(&packet(&other, 1), &probes).is_none());
        assert!(response(&packet(&probe, 16), &probes).is_none());
        assert!(!probe.received.load(Ordering::Acquire));
        let mut request = packet(&probe, 1);
        request[12..].make_ascii_uppercase();
        let reply = response(&request, &probes).unwrap();
        assert!(probe.received.load(Ordering::Acquire));
        assert_eq!(&reply[..2], &[0x12, 0x34]);
        assert_eq!(&reply[2..12], &[0x81, 0x83, 0, 1, 0, 0, 0, 0, 0, 0]);
        let registration = Registration {
            probes: probes.clone(),
            probe,
        };
        drop(registration);
        assert!(response(&request, &probes).is_none());
    }
}
