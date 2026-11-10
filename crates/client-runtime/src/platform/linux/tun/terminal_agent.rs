use anyhow::{Context, Result, bail, ensure};
use std::{
    io::{IsTerminal, Read},
    os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub(super) struct TerminalAgent(Child);

fn subject(status: &str, pid: u32) -> Result<String> {
    let (_, fields) = status
        .rsplit_once(") ")
        .context("无法读取终端认证进程身份")?;
    let started: u64 = fields
        .split_whitespace()
        .nth(19)
        .context("终端认证进程缺少启动时间")?
        .parse()
        .context("终端认证进程启动时间无效")?;
    Ok(format!("{pid},{started}"))
}

impl TerminalAgent {
    pub(super) fn start() -> Result<Option<Self>> {
        if !std::io::stdin().is_terminal() {
            return Ok(None);
        }
        ensure!(
            std::path::Path::new("/usr/bin/pkttyagent").is_file(),
            "缺少 pkttyagent，请安装 polkit 的终端认证代理"
        );
        let mut command = Command::new("/usr/bin/pkttyagent");
        command.args([
            "--process",
            &subject(
                &std::fs::read_to_string("/proc/self/stat")?,
                std::process::id(),
            )?,
            "--notify-fd",
            "1",
        ]);
        Self::spawn(command, Duration::from_secs(10)).map(Some)
    }

    fn spawn(mut command: Command, timeout: Duration) -> Result<Self> {
        let (mut readiness, notification) = UnixStream::pair()?;
        readiness.set_read_timeout(Some(timeout))?;
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::from(OwnedFd::from(notification)))
            .stderr(Stdio::inherit());
        let parent = unsafe { libc::getpid() };
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != parent {
                    return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
                }
                Ok(())
            });
        }
        let mut agent = Self(command.spawn().context("无法启动 polkit 终端认证代理")?);
        drop(command);
        let mut marker = [0];
        match readiness.read(&mut marker) {
            Ok(0) => {}
            Ok(_) => bail!("polkit 终端认证代理返回了无效的就绪信号"),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                bail!("等待 polkit 终端认证代理注册超时，尚未请求 TUN 授权")
            }
            Err(error) => return Err(error).context("无法读取 polkit 终端认证代理状态"),
        }
        ensure!(
            agent.0.try_wait()?.is_none(),
            "polkit 终端认证代理注册失败，请检查终端和系统授权服务"
        );
        Ok(agent)
    }
}

impl Drop for TerminalAgent {
    fn drop(&mut self) {
        match self.0.try_wait() {
            Ok(None) => {}
            Ok(Some(_)) | Err(_) => return,
        }
        unsafe {
            libc::kill(self.0.id() as libc::pid_t, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            match self.0.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
            }
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_agent(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn process_identity_uses_start_time_even_when_the_name_contains_parentheses() {
        let fields = std::iter::repeat_n("0", 18).collect::<Vec<_>>().join(" ");
        assert_eq!(
            subject(&format!("123 (cor ) (test)) S {fields} 456 0"), 123).unwrap(),
            "123,456"
        );
        assert!(subject("123 (cor) S 0", 123).is_err());
        assert!(subject(&format!("123 (cor) S {fields} invalid"), 123).is_err());
    }

    #[test]
    fn registered_agent_lives_until_the_authorization_scope_ends_then_is_reaped() {
        let agent = TerminalAgent::spawn(
            fake_agent("exec 1>&-; exec sleep 30"),
            Duration::from_secs(2),
        )
        .unwrap();
        let pid = agent.0.id() as libc::pid_t;
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
        drop(agent);
        assert_eq!(
            unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }

    #[test]
    fn agent_receives_termination_before_forced_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("terminated");
        let mut command = fake_agent(
            "trap 'printf terminated > \"$COR_TEST_AGENT_RESULT\"; exit 0' TERM; \
             exec 1>&-; while :; do sleep 0.05; done",
        );
        command.env("COR_TEST_AGENT_RESULT", &marker);
        let agent = TerminalAgent::spawn(command, Duration::from_secs(2)).unwrap();
        drop(agent);
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "terminated");
    }

    #[test]
    fn missing_or_unready_agents_fail_before_requesting_authorization() {
        assert!(
            TerminalAgent::spawn(
                Command::new("/missing-cor-test-agent"),
                Duration::from_secs(1)
            )
            .is_err()
        );
        let error = TerminalAgent::spawn(fake_agent("exec sleep 30"), Duration::from_millis(50))
            .err()
            .unwrap();
        assert!(error.to_string().contains("注册超时"));
        let error = TerminalAgent::spawn(
            fake_agent("printf invalid; exec sleep 30"),
            Duration::from_secs(2),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("无效的就绪信号"));
    }
}
