//! Measure live Rust heap while Api::get parses synthetic HTTP responses.
//! `app <GUI_PID>` samples the real desktop process tree, including mihomo.
//! Run each synthetic parsing workload in a fresh process.
#[path = "memory_benchmark/process_memory.rs"]
mod process_memory;
use clash_of_rust::{
    api::{Api, Connections, Rules},
    config::Settings,
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    fmt::Write,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct CountingAllocator;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn allocated(size: usize) {
    let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe {
            System.dealloc(pointer, layout);
        }
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if !pointer.is_null() {
            if size >= layout.size() {
                allocated(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        pointer
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

trait Count {
    fn count(&self) -> usize;
}
impl Count for Rules {
    fn count(&self) -> usize {
        self.rules.len()
    }
}
impl Count for Connections {
    fn count(&self) -> usize {
        self.connections.len()
    }
}

async fn measure<T: serde::de::DeserializeOwned + Send + Count + 'static>(
    api: &Api,
    mode: &str,
    path: &str,
    expected: usize,
    payload_size: usize,
) -> anyhow::Result<()> {
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let started = Instant::now();
    let result: T = if mode == "buffered" {
        api.client
            .get(api.url(&[path])?)
            .send()
            .await?
            .json()
            .await?
    } else {
        api.get(path).await?
    };
    let elapsed = started.elapsed();
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
    let retained = LIVE.load(Ordering::Relaxed).saturating_sub(baseline);
    assert_eq!(result.count(), expected);
    println!(
        "workload={path} mode={mode} rows={expected} payload_mib={:.2} baseline_heap_mib={:.2} peak_extra_heap_mib={:.2} retained_extra_heap_mib={:.2} elapsed_ms={:.1}",
        payload_size as f64 / 1048576.0,
        baseline as f64 / 1048576.0,
        peak as f64 / 1048576.0,
        retained as f64 / 1048576.0,
        elapsed.as_secs_f64() * 1000.0
    );
    Ok(())
}

#[tokio::main(worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let arguments: Vec<_> = std::env::args().collect();
    if arguments.get(1).is_some_and(|arg| arg == "app") {
        return process_memory::run(&arguments[2..]);
    }
    if arguments
        .get(1)
        .is_some_and(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        println!(
            "解析堆内存: memory_benchmark <rules|connections> [streamed|buffered] [rows=50000]"
        );
        println!(
            "GUI + 内核: memory_benchmark app <GUI_PID> [seconds=30] [interval_ms=250] [output.jsonl]"
        );
        println!("先正常打开客户端，再传入 GUI 的进程号；采样期间可以切换页面或操作客户端。");
        return Ok(());
    }
    let workload = arguments.get(1).map(String::as_str).unwrap_or("rules");
    let mode = arguments.get(2).map(String::as_str).unwrap_or("streamed");
    let count: usize = arguments
        .get(3)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(50_000);
    anyhow::ensure!(
        matches!(workload, "rules" | "connections"),
        "workload must be rules or connections"
    );
    anyhow::ensure!(
        matches!(mode, "buffered" | "streamed"),
        "mode must be buffered or streamed"
    );
    let mut body = format!("{{\"{workload}\":[");
    for index in 0..count {
        if index > 0 {
            body.push(',');
        }
        if workload == "rules" {
            write!(
                body,
                r#"{{"type":"DomainSuffix","payload":"example-{index}.test","proxy":"Auto"}}"#
            )?;
        } else {
            write!(
                body,
                r#"{{"id":"{index}","metadata":{{"host":"example-{index}.test","destinationIP":"192.0.2.1","destinationPort":"443","network":"tcp","process":"browser.exe"}},"chains":["Auto","node"],"rule":"DomainSuffix","start":"2026-10-06T00:00:00Z","upload":42,"download":73}}"#
            )?;
        }
    }
    body.push_str("]}");
    let body: Arc<str> = Arc::from(body);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let api = Api::new(&Settings {
        controller_port: listener.local_addr()?.port(),
        ..Settings::default()
    })?;
    let response_body = body.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        assert!(socket.read(&mut [0; 4096]).await? > 0);
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response_body.len()
                )
                .as_bytes(),
            )
            .await?;
        socket.write_all(response_body.as_bytes()).await?;
        Ok::<_, std::io::Error>(())
    });
    // Include comparable runtime state in both measurements.
    tokio::task::spawn_blocking(|| ()).await?;
    if workload == "rules" {
        measure::<Rules>(&api, mode, workload, count, body.len()).await?;
    } else {
        measure::<Connections>(&api, mode, workload, count, body.len()).await?;
    }
    server.await??;
    Ok(())
}
