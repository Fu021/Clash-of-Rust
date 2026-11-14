use super::*;
use std::{net::IpAddr, process::Stdio};
use tokio::{io::AsyncReadExt, process::Command};

fn command(tool: Tool, address: IpAddr, options: &Options) -> Command {
    let ipv6 = address.is_ipv6();
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new(if tool == Tool::Ping {
            "ping.exe"
        } else {
            "tracert.exe"
        });
        command.creation_flags(0x08000000);
        command.arg(if ipv6 { "-6" } else { "-4" });
        if tool == Tool::Ping {
            command.args(["-n", &options.count.to_string(), "-w", "1000"]);
        } else {
            command.args(["-d", "-h", &options.max_hops.to_string(), "-w", "1000"]);
        }
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        #[cfg(target_os = "macos")]
        let name = match (tool, ipv6) {
            (Tool::Ping, true) => "/sbin/ping6",
            (Tool::Ping, false) => "/sbin/ping",
            (_, true) => "/usr/sbin/traceroute6",
            (_, false) => "/usr/sbin/traceroute",
        };
        #[cfg(target_os = "android")]
        let name = if tool == Tool::Ping {
            "/system/bin/ping"
        } else {
            "/system/bin/traceroute"
        };
        #[cfg(not(any(target_os = "macos", target_os = "android")))]
        let name = if tool == Tool::Ping {
            "ping"
        } else {
            "traceroute"
        };
        let mut command = Command::new(name);
        #[cfg(not(target_os = "macos"))]
        command.arg(if ipv6 { "-6" } else { "-4" });
        command.arg("-n");
        if tool == Tool::Ping {
            command.args(["-c", &options.count.to_string()]);
            if !(cfg!(target_os = "macos") && ipv6) {
                command.args([
                    "-W",
                    if cfg!(target_os = "macos") {
                        "1000"
                    } else {
                        "1"
                    },
                ]);
            }
        } else {
            command.args(["-m", &options.max_hops.to_string(), "-q", "1", "-w", "1"]);
        }
        command
    };
    command.arg(address.to_string());
    command
}

async fn capture(
    mut stream: impl tokio::io::AsyncRead + Unpin,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut kept = Vec::with_capacity(4096);
    let mut buffer = [0; 2048];
    let mut truncated = false;
    loop {
        let size = stream.read(&mut buffer).await?;
        if size == 0 {
            break;
        }
        let remaining = 8192 - kept.len();
        kept.extend_from_slice(&buffer[..size.min(remaining)]);
        truncated |= size > remaining;
    }
    Ok((kept, truncated))
}

async fn execute(mut command: Command, budget: Duration, report: &mut Report) -> Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(failure) => {
            report
                .entries
                .push(error::entry("系统探针", &failure.into()));
            return Ok(());
        }
    };
    let stdout = child.stdout.take().context("无法读取探针输出")?;
    let stderr = child.stderr.take().context("无法读取探针错误")?;
    let collect = async {
        let (status, output, errors) = tokio::join!(child.wait(), capture(stdout), capture(stderr));
        Ok::<_, anyhow::Error>((status?, output?, errors?))
    };
    match tokio::time::timeout(budget, collect).await {
        Ok(Ok((status, output, errors))) => {
            for (label, (bytes, truncated)) in [("系统探针结果", output), ("系统探针说明", errors)]
            {
                if !bytes.is_empty() {
                    report.entries.push(Entry::unknown(
                        label,
                        bounded(&String::from_utf8_lossy(&bytes), 2048),
                        if truncated {
                            "output_truncated"
                        } else {
                            "process_output"
                        },
                    ));
                }
            }
            report.entries.push(if status.success() {
                Entry::ok("执行结果", "系统探针已完成；逐跳中的未响应不单独视为断网")
            } else {
                Entry::error(
                    "执行结果",
                    format!("探针退出状态 {status}；可能为未响应或系统权限限制，请查看探针说明"),
                    "process_failed",
                )
            });
        }
        Ok(Err(failure)) => report.entries.push(error::entry("系统探针", &failure)),
        Err(_) => {
            child.kill().await?;
            report.entries.push(Entry::error(
                "执行结果",
                "达到时限，已停止系统探针",
                "probe_timeout",
            ));
        }
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
    ensure!(!context.protected, "Claude 保护期间不发送 ICMP 或逐跳探针");
    let addresses = resolve(url, context).await?;
    let address = *addresses.first().context("目标没有可用 IP")?;
    let budget = options.budget(tool).saturating_sub(Duration::from_secs(6));
    execute(command(tool, address, options), budget, report).await?;
    report.entries.push(Entry::unknown("路径", "系统 ICMP / 逐跳探针；系统代理不承载此流量，TUN 是否接管取决于系统路由。这不是代理节点延迟", "information"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        Report {
            schema: 1,
            tool: Tool::Ping,
            target: "127.0.0.1".into(),
            elapsed_ms: 0,
            status: "unavailable",
            entries: Vec::new(),
        }
    }

    #[tokio::test]
    async fn missing_component_is_an_explicit_failure() {
        let directory = tempfile::tempdir().unwrap();
        let mut result = report();
        execute(
            Command::new(directory.path().join("missing-probe")),
            Duration::from_secs(1),
            &mut result,
        )
        .await
        .unwrap();
        assert_eq!(result.entries[0].code, "component_unavailable");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_unexecutable_probe_reports_permission_denied() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("probe");
        std::fs::write(&executable, b"not an executable").unwrap();
        let mut result = report();
        execute(
            Command::new(executable),
            Duration::from_secs(1),
            &mut result,
        )
        .await
        .unwrap();
        assert_eq!(result.entries[0].code, "permission_denied");
    }

    #[tokio::test]
    async fn process_output_is_drained_but_retained_with_a_limit() {
        use tokio::io::AsyncWriteExt;
        let (mut writer, reader) = tokio::io::duplex(128);
        let writing = async move {
            writer.write_all(&vec![b'x'; 20_000]).await.unwrap();
        };
        let ((), output) = tokio::join!(writing, capture(reader));
        let (bytes, truncated) = output.unwrap();
        assert_eq!(bytes.len(), 8192);
        assert!(truncated);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_kills_the_owned_probe_and_timeout_is_reported() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("pid");
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "printf '%s' $$ > \"$1\"; exec sleep 30", "probe"])
            .arg(&pid_file);
        let task = tokio::spawn(async move {
            let mut result = report();
            execute(command, Duration::from_secs(40), &mut result).await
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let pid: i32 = loop {
            if let Ok(text) = tokio::fs::read_to_string(&pid_file).await
                && let Ok(pid) = text.parse()
            {
                break pid;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        task.abort();
        let _ = task.await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "cancelled probe is still alive"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec sleep 30"]);
        let mut result = report();
        execute(command, Duration::from_millis(30), &mut result)
            .await
            .unwrap();
        assert_eq!(result.entries[0].code, "probe_timeout");
    }
}
