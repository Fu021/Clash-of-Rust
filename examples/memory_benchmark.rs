//! Standalone memory benchmark: isolated API parsing/refreshes and live GUI/core sampling.
//! `suite` runs each workload in a fresh subprocess; `app` observes an existing GUI.
use anyhow::Context as _;
use clash_of_rust::{
    api::{Api, Connections, Proxies, Rules},
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

impl Count for Proxies {
    fn count(&self) -> usize {
        self.proxies
            .values()
            .filter(|proxy| proxy.all.is_empty())
            .count()
    }
}

async fn measure<T: serde::de::DeserializeOwned + Send + Count + 'static>(
    api: &Api,
    mode: &str,
    path: &str,
    expected: usize,
    payload_size: usize,
    refreshes: usize,
) -> anyhow::Result<()> {
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let started = Instant::now();
    // Keep the previous result alive while receiving its replacement, as the UI does.
    let mut result = None;
    for _ in 0..refreshes {
        let next: T = if mode == "buffered" {
            api.client
                .get(api.url(&[path])?)
                .send()
                .await?
                .json()
                .await?
        } else {
            api.get(path).await?
        };
        anyhow::ensure!(next.count() == expected, "unexpected parsed row count");
        result = Some(next);
    }
    let elapsed = started.elapsed();
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
    let retained = LIVE.load(Ordering::Relaxed).saturating_sub(baseline);
    std::hint::black_box(&result);
    println!(
        "workload={path} mode={mode} rows={expected} refreshes={refreshes} payload_mib={:.2} baseline_heap_mib={:.2} peak_extra_heap_mib={:.2} retained_extra_heap_mib={:.2} elapsed_ms={:.1}",
        payload_size as f64 / 1048576.0,
        baseline as f64 / 1048576.0,
        peak as f64 / 1048576.0,
        retained as f64 / 1048576.0,
        elapsed.as_secs_f64() * 1000.0
    );
    println!(
        "{}",
        serde_json::json!({
            "type": "api", "version": clash_of_rust::VERSION,
            "profile": if cfg!(debug_assertions) { "dev" } else { "release" },
            "workload": path, "mode": mode, "rows": expected, "refreshes": refreshes,
            "payload_bytes": payload_size, "baseline_heap_bytes": baseline,
            "peak_extra_heap_bytes": peak, "retained_extra_heap_bytes": retained,
            "elapsed_ms": elapsed.as_secs_f64() * 1000.0
        })
    );
    Ok(())
}

fn suite(arguments: &[String]) -> anyhow::Result<()> {
    use std::io::Write as _;
    anyhow::ensure!(
        arguments.len() <= 3,
        "suite [rows=50000] [repetitions=3] [output.jsonl]"
    );
    let rows: usize = arguments
        .first()
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(50_000);
    let repetitions: usize = arguments
        .get(1)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(3);
    anyhow::ensure!(
        (1..=1_000_000).contains(&rows) && (1..=100).contains(&repetitions),
        "invalid suite size"
    );
    let mut output = arguments
        .get(2)
        .map(std::fs::File::create_new)
        .transpose()?;
    if let Some(file) = &mut output {
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "metadata", "version": clash_of_rust::VERSION,
                "profile": if cfg!(debug_assertions) { "dev" } else { "release" },
                "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
                "rows": rows, "repetitions": repetitions,
                "started_at": chrono::Utc::now().to_rfc3339(),
                "scope": "API Rust heap; fresh process per sample; fixture/runtime excluded from extra heap; refreshes retains previous snapshot"
            })
        )?;
    }
    for workload in ["rules", "connections", "proxies"] {
        for mode in ["streamed", "buffered"] {
            for refreshes in [1, 5] {
                for repetition in 0..repetitions {
                    let result = std::process::Command::new(std::env::current_exe()?)
                        .args([workload, mode, &rows.to_string(), &refreshes.to_string()])
                        .output()?;
                    anyhow::ensure!(
                        result.status.success(),
                        "{workload} failed: {}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                    let text = String::from_utf8(result.stdout)?;
                    let mut record: serde_json::Value = serde_json::from_str(
                        text.lines()
                            .find(|line| line.starts_with('{'))
                            .context("missing benchmark result")?,
                    )?;
                    record["repetition"] = serde_json::json!(repetition + 1);
                    println!("{}", text.lines().next().unwrap_or_default());
                    if let Some(file) = &mut output {
                        writeln!(file, "{record}")?;
                        file.flush()?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn all(arguments: &[String]) -> anyhow::Result<()> {
    anyhow::ensure!(
        (1..=2).contains(&arguments.len()),
        "all <GUI_PID> [output_prefix]"
    );
    let pid: u32 = arguments[0].parse()?;
    anyhow::ensure!(pid != 0 && pid != std::process::id(), "invalid GUI PID");
    let mut api = vec!["50000".into(), "3".into()];
    let mut app = vec![arguments[0].clone(), "30".into(), "250".into()];
    if let Some(prefix) = arguments.get(1) {
        let api_path = format!("{prefix}-api.jsonl");
        let app_path = format!("{prefix}-app.jsonl");
        anyhow::ensure!(
            !std::path::Path::new(&api_path).exists() && !std::path::Path::new(&app_path).exists(),
            "output already exists"
        );
        api.push(api_path);
        app.push(app_path);
    }
    suite(&api)?;
    process_memory::run(&app)
}

#[tokio::main(worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let arguments: Vec<_> = std::env::args().collect();
    if arguments.get(1).is_some_and(|arg| arg == "all") {
        return all(&arguments[2..]);
    }
    if arguments.get(1).is_some_and(|arg| arg == "app") {
        return process_memory::run(&arguments[2..]);
    }
    if arguments
        .get(1)
        .is_some_and(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        println!("memory_benchmark all <GUI_PID> [output_prefix]");
        println!("memory_benchmark suite [rows=50000] [repetitions=3] [output.jsonl]");
        println!(
            "memory_benchmark <rules|connections|proxies> [streamed|buffered] [rows=50000] [refreshes=1]"
        );
        println!("memory_benchmark app <GUI_PID> [seconds=30] [interval_ms=250] [output.jsonl]");
        println!(
            "suite measures single parses and five replacements; app samples GUI/core RSS and Linux PSS/USS while you operate the client."
        );
        return Ok(());
    }
    if arguments.get(1).is_none_or(|arg| arg == "suite") {
        return suite(arguments.get(2..).unwrap_or_default());
    }
    anyhow::ensure!(arguments.len() <= 5, "too many arguments");
    let workload = arguments[1].as_str();
    let mode = arguments.get(2).map(String::as_str).unwrap_or("streamed");
    let count: usize = arguments
        .get(3)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(50_000);
    let refreshes: usize = arguments
        .get(4)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(1);
    anyhow::ensure!(
        matches!(workload, "rules" | "connections" | "proxies"),
        "unknown workload"
    );
    anyhow::ensure!(matches!(mode, "buffered" | "streamed"), "unknown mode");
    anyhow::ensure!(
        (1..=1_000_000).contains(&count) && (1..=1000).contains(&refreshes),
        "invalid workload size"
    );
    let mut body = if workload == "proxies" {
        "{\"proxies\":{".into()
    } else {
        format!("{{\"{workload}\":[")
    };
    for index in 0..count {
        if index > 0 {
            body.push(',');
        }
        if workload == "rules" {
            write!(
                body,
                r#"{{"type":"DomainSuffix","payload":"example-{index}.test","proxy":"Auto"}}"#
            )?;
        } else if workload == "proxies" {
            write!(
                body,
                r#""node-{index}":{{"name":"node-{index}","type":"Shadowsocks","history":["#
            )?;
            for delay in 0..20 {
                if delay > 0 {
                    body.push(',');
                }
                write!(body, r#"{{"delay":{delay}}}"#)?;
            }
            body.push_str("]}");
        } else {
            write!(
                body,
                r#"{{"id":"{index}","metadata":{{"host":"example-{index}.test","destinationIP":"192.0.2.1","destinationPort":"443","network":"tcp","process":"browser.exe"}},"chains":["Auto","node"],"rule":"DomainSuffix","start":"2026-10-06T00:00:00Z","upload":42,"download":73}}"#
            )?;
        }
    }
    if workload == "proxies" {
        for group in 0..50 {
            write!(
                body,
                r#", "group-{group}":{{"name":"group-{group}","type":"Selector","now":"node-0","all":["#
            )?;
            for member in 0..count.min(1000) {
                if member > 0 {
                    body.push(',');
                }
                write!(body, "\"node-{}\"", (group * 1000 + member) % count)?;
            }
            body.push_str("]}");
        }
        body.push_str("}}");
    } else {
        body.push_str("]}");
    }
    let body: Arc<str> = Arc::from(body);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let api = Api::new(&Settings {
        controller_port: listener.local_addr()?.port(),
        ..Settings::default()
    })?;
    let response_body = body.clone();
    let server = tokio::spawn(async move {
        for _ in 0..refreshes {
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
        }
        Ok::<_, std::io::Error>(())
    });
    // Include comparable runtime state in both measurements.
    tokio::task::spawn_blocking(|| ()).await?;
    if workload == "rules" {
        measure::<Rules>(&api, mode, workload, count, body.len(), refreshes).await?;
    } else if workload == "proxies" {
        measure::<Proxies>(&api, mode, workload, count, body.len(), refreshes).await?;
    } else {
        measure::<Connections>(&api, mode, workload, count, body.len(), refreshes).await?;
    }
    server.await??;
    Ok(())
}

mod process_memory {
    // Observe an existing GUI without changing its configuration or proxy mode.
    use anyhow::{Context, Result, ensure};
    use serde::Serialize;
    use std::{
        collections::{HashMap, HashSet},
        fs::File,
        io::{BufWriter, Write},
        time::{Duration, Instant},
    };
    use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

    #[derive(Clone, Debug, Default, Serialize)]
    struct Memory {
        rss_bytes: Option<u64>,
        pss_bytes: Option<u64>,
        uss_bytes: Option<u64>,
    }

    impl Memory {
        fn sum<'a>(values: impl Iterator<Item = &'a Self>) -> Self {
            // Option::sum deliberately rejects incomplete totals. Unknown is not zero.
            let values: Vec<_> = values.collect();
            Self {
                rss_bytes: values.iter().map(|value| value.rss_bytes).sum(),
                pss_bytes: values.iter().map(|value| value.pss_bytes).sum(),
                uss_bytes: values.iter().map(|value| value.uss_bytes).sum(),
            }
        }
    }

    #[derive(Debug, Serialize)]
    struct ProcessMemory {
        pid: u32,
        parent_pid: Option<u32>,
        start_time: u64,
        name: String,
        role: &'static str,
        memory: Memory,
    }

    #[derive(Debug, Serialize)]
    struct Sample {
        elapsed_ms: u64,
        gui: Memory,
        core: Memory,
        other: Memory,
        total: Memory,
        processes: Vec<ProcessMemory>,
    }

    fn descendants(root: u32, parents: &HashMap<u32, Option<u32>>) -> HashSet<u32> {
        let mut selected = HashSet::from([root]);
        loop {
            let before = selected.len();
            for (&pid, parent) in parents {
                if parent.is_some_and(|parent| selected.contains(&parent)) {
                    selected.insert(pid);
                }
            }
            if before == selected.len() {
                return selected;
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn rollup(text: &str) -> Memory {
        fn bytes(text: &str, key: &str) -> Option<u64> {
            let mut parts = text
                .lines()
                .find_map(|line| line.strip_prefix(key))?
                .split_whitespace();
            let value = parts.next()?.parse::<u64>().ok()?;
            (parts.next()? == "kB").then_some(value.checked_mul(1024)?)
        }
        Memory {
            rss_bytes: bytes(text, "Rss:"),
            pss_bytes: bytes(text, "Pss:"),
            uss_bytes: bytes(text, "Private_Clean:")
                .and_then(|clean| clean.checked_add(bytes(text, "Private_Dirty:")?)),
        }
    }

    fn memory(pid: u32, rss: u64) -> Memory {
        let fallback = Memory {
            // Zero may mean access was denied, especially for an elevated core.
            rss_bytes: (rss > 0).then_some(rss),
            ..Memory::default()
        };
        #[cfg(target_os = "linux")]
        if let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
            let measured = rollup(&text);
            return Memory {
                rss_bytes: measured.rss_bytes.or(fallback.rss_bytes),
                ..measured
            };
        }
        #[cfg(not(target_os = "linux"))]
        let _ = pid;
        fallback
    }

    fn refresh(system: &mut System) {
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_memory().without_tasks(),
        );
    }

    fn sample(system: &System, root: u32, elapsed_ms: u64) -> Sample {
        let parents = system
            .processes()
            .iter()
            .map(|(pid, process)| (pid.as_u32(), process.parent().map(Pid::as_u32)))
            .collect();
        let selected = descendants(root, &parents);
        let mut processes: Vec<_> = system
            .processes()
            .iter()
            .filter(|(pid, process)| {
                selected.contains(&pid.as_u32()) && process.status() != ProcessStatus::Zombie
            })
            .map(|(pid, process)| {
                let name = process.name().to_string_lossy().into_owned();
                let role = if pid.as_u32() == root {
                    "gui"
                } else if matches!(name.to_ascii_lowercase().as_str(), "mihomo" | "mihomo.exe") {
                    "core"
                } else {
                    "other"
                };
                ProcessMemory {
                    pid: pid.as_u32(),
                    parent_pid: process.parent().map(Pid::as_u32),
                    start_time: process.start_time(),
                    name,
                    role,
                    memory: memory(pid.as_u32(), process.memory()),
                }
            })
            .collect();
        processes.sort_by_key(|process| process.pid);
        let group = |role| {
            Memory::sum(
                processes
                    .iter()
                    .filter(|process| process.role == role)
                    .map(|process| &process.memory),
            )
        };
        Sample {
            elapsed_ms,
            gui: group("gui"),
            core: group("core"),
            other: group("other"),
            total: Memory::sum(processes.iter().map(|process| &process.memory)),
            processes,
        }
    }

    #[derive(Default)]
    struct Stats {
        count: usize,
        sum: u128,
        peak: u64,
        last: Option<u64>,
    }

    impl Stats {
        fn record(&mut self, value: Option<u64>) {
            self.last = value;
            if let Some(value) = value {
                self.count += 1;
                self.sum += u128::from(value);
                self.peak = self.peak.max(value);
            }
        }
        fn print(&self, label: &str, samples: usize) {
            let average = (self.count > 0).then(|| (self.sum / self.count as u128) as u64);
            let peak = (self.count > 0).then_some(self.peak);
            println!(
                "{label:<12} 平均 {:>9}  峰值 {:>9}  最后 {:>9} MiB  有效 {}/{samples}",
                mib(average),
                mib(peak),
                mib(self.last),
                self.count
            );
        }
    }

    fn mib(bytes: Option<u64>) -> String {
        bytes
            .map(|bytes| format!("{:.2}", bytes as f64 / 1048576.0))
            .unwrap_or_else(|| "N/A".into())
    }

    pub fn run(arguments: &[String]) -> Result<()> {
        ensure!(
            (1..=4).contains(&arguments.len()),
            "用法: memory_benchmark app <GUI_PID> [seconds=30] [interval_ms=250] [output.jsonl]"
        );
        let root: u32 = arguments[0].parse().context("GUI_PID 必须是进程号")?;
        ensure!(
            root != std::process::id() && root != 0,
            "请传入客户端 GUI 进程号"
        );
        let seconds: u64 = arguments
            .get(1)
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(30);
        let interval_ms: u64 = arguments
            .get(2)
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(250);
        ensure!(seconds > 0 && interval_ms > 0, "采样时间与间隔必须大于零");
        let mut system = System::new();
        refresh(&mut system);
        let process = system
            .process(Pid::from_u32(root))
            .context("找不到 GUI 进程，请先打开客户端")?;
        ensure!(
            matches!(
                process
                    .name()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .as_str(),
                "clash-of-rust" | "clash-of-rust.exe"
            ),
            "指定进程不是 Clash of Rust GUI，请检查进程号"
        );
        let identity = process.start_time();
        // Never overwrite an earlier measurement by mistake.
        let mut output = arguments
            .get(3)
            .map(|path| File::create_new(path).map(BufWriter::new))
            .transpose()?;
        if let Some(output) = &mut output {
            serde_json::to_writer(
                &mut *output,
                &serde_json::json!({
                    "type": "metadata", "tool_version": env!("CARGO_PKG_VERSION"),
                    "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
                    "gui_pid": root, "gui_start_time": identity,
                    "requested_seconds": seconds, "interval_ms": interval_ms,
                    "started_at": chrono::Utc::now().to_rfc3339(),
                    "scope": "GUI and its current descendants; observer excluded",
                    "rss_note": "sum may double-count shared pages; PSS/USS are Linux-only"
                }),
            )?;
            writeln!(output)?;
        }
        println!("整程序内存：GUI PID={root}，采样 {seconds}s / {interval_ms}ms；单位 MiB");
        println!(
            "时间(s)      GUI RSS    内核 RSS    其他 RSS    合计 RSS    合计 PSS    合计 USS    内核数"
        );
        let start = Instant::now();
        let duration = Duration::from_secs(seconds);
        let interval = Duration::from_millis(interval_ms);
        let mut stats: [Stats; 6] = std::array::from_fn(|_| Stats::default());
        let mut samples = 0;
        let mut core_seen = false;
        loop {
            let round_started = Instant::now();
            refresh(&mut system);
            if !system.process(Pid::from_u32(root)).is_some_and(|process| {
                process.start_time() == identity && process.status() != ProcessStatus::Zombie
            }) {
                println!("GUI 已退出或 PID 已变化，结束采样。");
                break;
            }
            let measured = sample(&system, root, start.elapsed().as_millis() as u64);
            let cores = measured
                .processes
                .iter()
                .filter(|process| process.role == "core")
                .count();
            core_seen |= cores > 0;
            println!(
                "{:>7.2}  {:>11} {:>11} {:>11} {:>11} {:>11} {:>11} {:>9}",
                measured.elapsed_ms as f64 / 1000.0,
                mib(measured.gui.rss_bytes),
                mib(measured.core.rss_bytes),
                mib(measured.other.rss_bytes),
                mib(measured.total.rss_bytes),
                mib(measured.total.pss_bytes),
                mib(measured.total.uss_bytes),
                cores
            );
            for (stat, value) in stats.iter_mut().zip([
                measured.gui.rss_bytes,
                measured.core.rss_bytes,
                measured.other.rss_bytes,
                measured.total.rss_bytes,
                measured.total.pss_bytes,
                measured.total.uss_bytes,
            ]) {
                stat.record(value);
            }
            samples += 1;
            if let Some(output) = &mut output {
                serde_json::to_writer(&mut *output, &measured)?;
                writeln!(output)?;
                output.flush()?;
            }
            let remaining = duration.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                break;
            }
            std::thread::sleep(
                interval
                    .saturating_sub(round_started.elapsed())
                    .min(remaining),
            );
            if start.elapsed() >= duration {
                break;
            }
        }
        ensure!(samples > 0, "GUI 在首次采样前退出，没有生成内存数据");
        println!(
            "\n观测汇总（{samples} 次采样，实际 {:.2}s）：",
            start.elapsed().as_secs_f64()
        );
        for (stat, label) in stats.iter().zip([
            "GUI RSS",
            "内核 RSS",
            "其他 RSS",
            "合计 RSS",
            "合计 PSS",
            "合计 USS",
        ]) {
            stat.print(label, samples);
        }
        if !core_seen {
            println!("未观察到 mihomo 子进程；本次合计不含运行中的内核。");
        }
        println!(
            "RSS 含共享页面，直接相加可能重复计算；Linux PSS 按共享比例分摊，USS 为私有驻留页。"
        );
        println!("N/A 表示平台不支持或权限不足；部分有效时均值与峰值仅覆盖有效样本。");
        println!(
            "峰值是采样期间观察到的值；不含独立显存、驱动内存、进程外的系统缓存或本基准进程。"
        );
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn nested_children_are_counted_once_and_unrelated_cores_are_excluded() {
            let parents = HashMap::from([
                (10, Some(1)),
                (11, Some(10)),
                (12, Some(11)),
                (20, Some(1)),
                (21, Some(20)),
                (30, Some(31)),
                (31, Some(30)),
            ]);
            assert_eq!(descendants(10, &parents), HashSet::from([10, 11, 12]));
        }

        #[test]
        fn missing_metrics_never_become_a_partial_total_or_zero() {
            let values = [
                Memory {
                    rss_bytes: Some(10),
                    pss_bytes: Some(5),
                    uss_bytes: Some(3),
                },
                Memory {
                    rss_bytes: Some(20),
                    ..Memory::default()
                },
            ];
            let total = Memory::sum(values.iter());
            assert_eq!(total.rss_bytes, Some(30));
            assert_eq!(total.pss_bytes, None);
            assert_eq!(total.uss_bytes, None);
            let empty = Memory::sum(std::iter::empty());
            assert_eq!(empty.rss_bytes, Some(0));
            let mut stats = Stats::default();
            for value in [Some(10), None, Some(30), None] {
                stats.record(value);
            }
            assert_eq!(
                (stats.count, stats.sum, stats.peak, stats.last),
                (2, 40, 30, None)
            );
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn rollup_uses_exact_fields_and_rejects_incomplete_private_memory() {
            let measured = rollup(
                "Rss: 100 kB\nPss: 70 kB\nPss_Anon: 60 kB\nPrivate_Clean: 20 kB\nPrivate_Dirty: 30 kB\n",
            );
            assert_eq!(measured.rss_bytes, Some(102400));
            assert_eq!(measured.pss_bytes, Some(71680));
            assert_eq!(measured.uss_bytes, Some(51200));
            assert_eq!(rollup("Rss: 1 kB\nPss: 0 kB\n").pss_bytes, Some(0));
            assert_eq!(rollup("Private_Clean: 20 kB\n").uss_bytes, None);
            assert_eq!(rollup("Pss: 99 MB\n").pss_bytes, None);
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn real_child_memory_is_included_and_core_restart_replaces_its_pid() {
            let directory = tempfile::tempdir().unwrap();
            let core = directory.path().join("mihomo");
            std::fs::copy("/bin/sleep", &core).unwrap();
            let mut system = System::new();
            let root = std::process::id();
            let mut previous_pid = None;
            for _ in 0..2 {
                let mut child = std::process::Command::new(&core).arg("10").spawn().unwrap();
                let pid = child.id();
                // Wait for exec to publish the fixture's process name and pages.
                let measured = (0..100).find_map(|_| {
                    refresh(&mut system);
                    let measured = sample(&system, root, 0);
                    if measured.processes.iter().any(|process| {
                        process.pid == pid
                            && process.role == "core"
                            && process.memory.rss_bytes.is_some()
                    }) {
                        Some(measured)
                    } else {
                        std::thread::sleep(Duration::from_millis(10));
                        None
                    }
                });
                child.kill().unwrap();
                // A terminated child can remain in /proc until wait() reaps it.
                for _ in 0..100 {
                    refresh(&mut system);
                    if system
                        .process(Pid::from_u32(pid))
                        .is_some_and(|process| process.status() == ProcessStatus::Zombie)
                    {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(
                    !sample(&system, root, 0)
                        .processes
                        .iter()
                        .any(|process| process.pid == pid)
                );
                child.wait().unwrap();
                let measured = measured.expect("Core child was not observed");
                assert!(measured.core.rss_bytes.unwrap() > 0);
                assert!(
                    measured.total.rss_bytes.unwrap()
                        >= measured.gui.rss_bytes.unwrap() + measured.core.rss_bytes.unwrap()
                );
                if let Some(old) = previous_pid {
                    assert!(!measured.processes.iter().any(|process| process.pid == old));
                }
                previous_pid = Some(pid);
                refresh(&mut system);
                assert!(
                    !sample(&system, root, 0)
                        .processes
                        .iter()
                        .any(|process| process.pid == pid)
                );
            }
        }
    }
}
