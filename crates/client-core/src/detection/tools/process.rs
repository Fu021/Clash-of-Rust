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

async fn execute(
    mut command: Command,
    budget: Duration,
    report: &mut Report,
) -> Result<Option<String>> {
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
            return Ok(None);
        }
    };
    let stdout = child.stdout.take().context("无法读取探针输出")?;
    let stderr = child.stderr.take().context("无法读取探针错误")?;
    let collect = async {
        let (status, output, errors) = tokio::join!(child.wait(), capture(stdout), capture(stderr));
        Ok::<_, anyhow::Error>((status?, output?, errors?))
    };
    let mut trace_output = (report.tool == Tool::Trace).then(String::new);
    match tokio::time::timeout(budget, collect).await {
        Ok(Ok((status, output, errors))) => {
            for (label, (bytes, truncated)) in [("系统探针结果", output), ("系统探针说明", errors)]
            {
                if !bytes.is_empty() {
                    if report.tool == Tool::Trace && label == "系统探针结果" {
                        trace_output = Some(String::from_utf8_lossy(&bytes).into_owned());
                        if !truncated {
                            continue;
                        }
                    }
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
                Entry::ok("执行结果", "系统命令已结束；目标是否到达请看探针结果")
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
    Ok(trace_output)
}

fn trace_entries(output: &str, target: IpAddr, max_hops: u8, report: &mut Report) {
    let mut observed = 0;
    let mut reached = false;
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        let Some(hop) = fields
            .next()
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|hop| (1..=max_hops).contains(hop))
        else {
            continue;
        };
        let details: Vec<_> = fields.collect();
        let addresses: Vec<IpAddr> = details
            .iter()
            .filter_map(|value| value.trim_matches(['(', ')', '[', ']']).parse().ok())
            .collect();
        let unanswered = details.iter().filter(|value| **value == "*").count();
        if addresses.is_empty() && unanswered == 0 {
            continue;
        }
        observed += 1;
        reached |= addresses.contains(&target);
        report.entries.push(if addresses.is_empty() {
            Entry::unknown(
                &format!("第 {hop} 跳"),
                "未收到该跳响应（*）；可能未回包、被过滤或超过等待时间，不能据此判定断网",
                "information",
            )
        } else {
            Entry::ok(&format!("第 {hop} 跳"), bounded(&details.join(" "), 256))
        });
    }
    report.entries.push(if reached {
        Entry::ok("追踪结果", "已观察到目标地址响应；中间跳的 * 不影响此结论")
    } else {
        Entry::unknown(
            "追踪结果",
            if observed == 0 {
                "未识别到逐跳结果；请查看系统探针说明，无法确认到达目标"
            } else {
                "本次未确认到达目标，可能达到跳数上限或后续不回包；不等于网站或代理不可用"
            },
            "trace_destination_unconfirmed",
        )
    });
    if observed == 0 && !output.is_empty() {
        report.entries.push(Entry::unknown(
            "系统探针原文",
            bounded(output, 2048),
            "process_output",
        ));
    }
    report.entries.push(Entry::unknown(
        "探针设置",
        format!("最多 {max_hops} 跳，每个响应等待 1 秒；星号表示等待期间未收到响应。可调整跳数后重试，网络设备也可能限制逐跳响应"),
        "information",
    ));
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
    if let Some(output) = execute(command(tool, address, options), budget, report).await?
        && tool == Tool::Trace
    {
        trace_entries(&output, address, options.max_hops, report);
    }
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

    #[test]
    fn trace_stars_do_not_hide_destination_or_claim_an_outage() {
        let target: IpAddr = "203.0.113.42".parse().unwrap();
        for output in [
            " 1 192.0.2.1 1.1 ms\n 2 *\n 3 203.0.113.42 23.0 ms\n",
            " 1 1 ms <1 ms 1 ms 192.0.2.1\n 2 * * * Request timed out.\n 3 20 ms 21 ms 22 ms 203.0.113.42\n",
        ] {
            let mut result = report();
            result.tool = Tool::Trace;
            trace_entries(output, target, 8, &mut result);
            result.finish(Instant::now());
            assert_eq!(result.status, "complete", "{result:?}");
            assert!(
                result
                    .entries
                    .iter()
                    .any(|entry| entry.label == "第 2 跳" && entry.success.is_none())
            );
            assert!(
                result
                    .entries
                    .iter()
                    .any(|entry| entry.label == "追踪结果" && entry.success == Some(true))
            );
        }
        let mut result = report();
        result.tool = Tool::Trace;
        trace_entries(" 1 192.0.2.1 1 ms\n 2 *\n", target, 2, &mut result);
        result.finish(Instant::now());
        assert_eq!(result.status, "partial");
        assert!(
            result
                .entries
                .iter()
                .any(|entry| entry.code == "trace_destination_unconfirmed")
        );
        let mut result = report();
        result.tool = Tool::Trace;
        result.entries.push(Entry::ok("执行结果", "系统命令已结束"));
        trace_entries("", target, 8, &mut result);
        result.finish(Instant::now());
        assert_eq!(result.status, "partial");
    }

    #[tokio::test]
    async fn missing_component_is_an_explicit_failure() {
        let directory = tempfile::tempdir().unwrap();
        let mut result = report();
        assert!(
            execute(
                Command::new(directory.path().join("missing-probe")),
                Duration::from_secs(1),
                &mut result,
            )
            .await
            .unwrap()
            .is_none()
        );
        assert_eq!(result.entries[0].code, "component_unavailable");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_unexecutable_probe_reports_permission_denied() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("probe");
        std::fs::write(&executable, b"not an executable").unwrap();
        let mut result = report();
        assert!(
            execute(
                Command::new(executable),
                Duration::from_secs(1),
                &mut result,
            )
            .await
            .unwrap()
            .is_none()
        );
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
        assert!(
            execute(command, Duration::from_millis(30), &mut result)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(result.entries[0].code, "probe_timeout");
    }
}
