//! Fixed macOS administrator prompt and argument quoting. No GUI dependencies.
use anyhow::{Context, Result, bail, ensure};
use std::{
    io::Read,
    process::{Command, Output, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
/// Quote separately for the shell and the AppleScript string literal.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
pub fn administrator(script: &str) -> Result<()> {
    execute_script(script, true)
}

fn execute_script(script: &str, elevated: bool) -> Result<()> {
    let literal = serde_json::to_string(script)?;
    let expression = if script == "error number -128" && !elevated {
        script.to_owned()
    } else {
        format!(
            "do shell script {literal}{}",
            if elevated {
                " with administrator privileges"
            } else {
                ""
            }
        )
    };
    let mut command = Command::new("/usr/bin/osascript");
    command.args(["-e", &expression]);
    let output = run(&mut command, "系统授权", Duration::from_secs(180))?;
    let error = String::from_utf8_lossy(&output.stderr);
    if error.contains("(-128)") {
        bail!("已取消系统授权，原设置未改变");
    }
    ensure!(output.status.success(), "管理员授权失败：{}", error.trim());
    Ok(())
}

/// Drain bounded diagnostics concurrently and bound both process and pipe waits.
pub fn run(command: &mut Command, operation: &str, timeout: Duration) -> Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("无法启动{operation}"))?;
    fn capture(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            while let Ok(count) = pipe.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                let keep = count.min(16 * 1024 - bytes.len());
                bytes.extend_from_slice(&chunk[..keep]);
            }
            let _ = send.send(bytes);
        });
        receive
    }
    let stdout = capture(child.stdout.take().context("无法读取操作输出")?);
    let stderr = capture(child.stderr.take().context("无法读取操作错误")?);
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{operation}超时，已停止本次操作，请重试");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    Ok(Output {
        status,
        stdout: stdout
            .recv_timeout(Duration::from_secs(1))
            .unwrap_or_default(),
        stderr: stderr
            .recv_timeout(Duration::from_secs(1))
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subprocess_timeout_does_not_wait_for_an_unresponsive_child() {
        let start = Instant::now();
        let error = run(
            Command::new("/bin/sleep").arg("5"),
            "测试操作",
            Duration::from_millis(50),
        )
        .unwrap_err();
        assert!(error.to_string().contains("超时"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
