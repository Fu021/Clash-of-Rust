//! GPL-3.0-only bridge to the separately licensed AGPL-3.0 detector.
//! The bundled script receives only a catalog identifier and a loopback proxy.
use crate::ip_check::{CheckResult, Service, State, decorate_country};
use crate::probe::country_name;
use anyhow::{Context, Result, bail};
use std::{path::Path, process::Stdio};
use tokio::io::AsyncReadExt;

pub async fn check(service: &Service, port: u16) -> Result<CheckResult> {
    let folder = crate::assets::discover()?.join("ip-check");
    let work = tempfile::tempdir()?;
    run(service, port, &folder, work.path()).await
}

fn script_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

async fn run(service: &Service, port: u16, folder: &Path, work: &Path) -> Result<CheckResult> {
    let timing = work.join("timings.txt");
    let bash = if cfg!(windows) {
        folder.join("runtime/usr/bin/bash.exe")
    } else {
        std::path::PathBuf::from("/bin/bash")
    };
    let mut paths = vec![folder.join("runtime/usr/bin")];
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    let mut command = tokio::process::Command::new(bash);
    command
        .args(["--noprofile", "--norc"])
        .arg(script_path(&folder.join("check-adapted.sh")))
        .arg(&service.id)
        .arg(format!("http://127.0.0.1:{port}"))
        .arg(script_path(&timing))
        .env("COR_WORK", script_path(work))
        .env("COR_HELPER", script_path(&std::env::current_exe()?))
        .env("PATH", std::env::join_paths(paths)?)
        .env("TMP", work)
        .env("TEMP", work)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .current_dir(work)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .context("无法启动安装包内的 IP 检测工具，请重新安装")?;
    let tree = ProcessTree::attach(&child)?;
    let stdout = child.stdout.take().context("无法读取检测输出")?;
    let stderr = child.stderr.take().context("无法读取检测错误输出")?;
    // Drain pipes concurrently with a hard output limit, avoiding deadlocks and unbounded buffers.
    let output = async {
        let mut text = Vec::new();
        stdout.take(65537).read_to_end(&mut text).await?;
        anyhow::ensure!(text.len() <= 65536, "检测输出超过限制");
        Ok::<_, anyhow::Error>(text)
    };
    let errors = async {
        let mut text = Vec::new();
        stderr.take(65537).read_to_end(&mut text).await?;
        anyhow::ensure!(text.len() <= 65536, "检测错误输出超过限制");
        Ok::<_, anyhow::Error>(text)
    };
    let (output, errors, status) = tokio::try_join!(output, errors, async {
        child.wait().await.map_err(anyhow::Error::from)
    })?;
    drop(tree);
    if !status.success() {
        bail!("检测工具未正常完成（{}）", status);
    }
    let errors = !errors.is_empty();
    let mut result = parse_output(&String::from_utf8_lossy(&output));
    let challenged = tokio::fs::read_to_string(work.join("challenges.txt"))
        .await
        .unwrap_or_default()
        .lines()
        .any(|line| line == "challenge");
    let failed = tokio::fs::read_to_string(work.join("failures.txt"))
        .await
        .unwrap_or_default()
        .lines()
        .any(|line| line == "transport");
    let denied = service.id == "AIUnlockTest_Claude"
        && tokio::fs::read_to_string(work.join("statuses.txt"))
            .await
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.parse::<u16>().ok())
            .any(|status| status >= 400);
    if challenged {
        result.state = State::Unknown;
        result.summary = "验证拦截，未确认".into();
        result
            .detail
            .push_str("；响应包含 Cloudflare 浏览器验证，不能据此判定地区不支持");
    } else if (failed || denied) && result.state == State::Confirmed {
        // Some upstream functions treat the absence of a denial string as Yes,
        // even when the underlying request failed (notably Claude).
        result.state = State::Unknown;
        result.summary = "请求失败，未确认".into();
    } else if result.state == State::Restricted
        && result.summary == "不可用或受限"
        && matches!(
            service.id.as_str(),
            "MediaUnlockTest_ChatGPT" | "MediaUnlockTest_Sora"
        )
    {
        // These functions infer No solely from an absent Location header.
        // A changed landing page or bot response is insufficient proof.
        result.state = State::Unknown;
        result.summary = "未确认可用性".into();
    }
    let timings = tokio::fs::read_to_string(timing).await.unwrap_or_default();
    result.millis = timings
        .lines()
        .filter_map(|line| line.trim().parse::<f64>().ok())
        .find(|value| value.is_finite() && *value > 0.0)
        .map(|seconds| (seconds * 1000.0).round().max(1.0) as u128)
        .unwrap_or(0);
    if result.millis == 0 && result.state != State::Unknown {
        result.state = State::Unknown;
        result.summary = "未获得有效响应".into();
        result.detail.push_str("；没有记录到有效的 HTTP 响应延迟");
    }
    if errors && result.state == State::Unknown {
        // Do not expose raw stderr: upstream commands can contain authentication values.
        result
            .detail
            .push_str("；检测工具报告错误，可能是接口变化或运行依赖问题");
    }
    Ok(result)
}

fn strip_ansi(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut output = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for part in chars.by_ref() {
                if ('@'..='~').contains(&part) {
                    break;
                }
            }
        } else if ch == '\r' || ch == '\t' {
            output.push(' ');
        } else if !ch.is_control() || ch == '\n' {
            output.push(ch);
        }
    }
    output
}

fn parse_output(text: &str) -> CheckResult {
    let text = strip_ansi(text);
    let line = text
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("");
    // Platform labels can contain a colon (J:COM). Separator is followed by whitespace.
    let value = line
        .char_indices()
        .find_map(|(at, ch)| {
            (ch == ':' && line[at + 1..].starts_with(char::is_whitespace))
                .then(|| line[at + 1..].trim())
        })
        .unwrap_or(line);
    let lower = value.to_ascii_lowercase();
    let (state, summary) = if crate::flags::country_code(value.trim()).is_some() {
        (State::Confirmed, "已识别地区")
    } else if lower.starts_with("failed") || lower.contains("unexpected") || lower.is_empty() {
        (State::Unknown, "检测失败或结果未确认")
    } else if lower.starts_with("web reachable") {
        (State::Reachable, "网页可达")
    } else if lower.starts_with("web only") {
        (State::Reachable, "仅网页可用")
    } else if lower.starts_with("originals only") || lower.starts_with("original only") {
        (State::Reachable, "仅自制内容")
    } else if lower.starts_with("yes") {
        (State::Confirmed, "可用")
    } else if lower.starts_with("no") && lower.contains("unsupported region") {
        (State::Restricted, "地区不支持")
    } else if lower.starts_with("no") && lower.contains("disallowed isp") {
        (State::Restricted, "出口网络受限")
    } else if lower.starts_with("no") && lower.contains("blocked") {
        (State::Unknown, "请求被拦截，未确认")
    } else if lower.starts_with("no") || lower.starts_with("blocked") {
        (State::Restricted, "不可用或受限")
    } else {
        (State::Unknown, "已返回检测信息")
    };
    let country = value
        .split_once("Region:")
        .and_then(|(_, tail)| tail.split(')').next().map(str::trim))
        .and_then(|name| {
            crate::flags::country_code(name)
                .or_else(|| crate::flags::country_code(&name.to_ascii_uppercase()))
        })
        .or_else(|| crate::flags::country_code(value.trim()))
        .and_then(country_name)
        .map(decorate_country)
        .unwrap_or_else(|| "未提供".into());
    CheckResult {
        state,
        summary: summary.into(),
        country,
        millis: 0,
        detail: format!("RegionRestrictionCheck 检测结果：{value}"),
    }
}

#[cfg(windows)]
struct ProcessTree(usize);
#[cfg(windows)]
impl ProcessTree {
    fn attach(child: &tokio::process::Child) -> Result<Self> {
        use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
        let child_handle = child.raw_handle().context("检测进程句柄不可用")?;
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        anyhow::ensure!(!handle.is_null(), "无法创建检测进程组");
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let assigned = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            ) != 0
                && AssignProcessToJobObject(handle, child_handle) != 0
        };
        if !assigned {
            unsafe {
                CloseHandle(handle);
            }
            bail!("无法管理检测子进程");
        }
        Ok(Self(handle as usize))
    }
}
#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}
#[cfg(unix)]
struct ProcessTree(i32);
#[cfg(unix)]
impl ProcessTree {
    fn attach(child: &tokio::process::Child) -> Result<Self> {
        Ok(Self(child.id().context("检测进程不存在")? as i32))
    }
}
#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("detector");
        let bin = folder.join("runtime/usr/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(folder.join("runtime/tmp")).unwrap();
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("bundle/resources/ip-check");
        for entry in std::fs::read_dir(bundled.join("runtime/usr/bin")).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name().unwrap() != "curl.exe" {
                std::fs::copy(&path, bin.join(path.file_name().unwrap())).unwrap();
            }
        }
        for name in ["check-adapted.sh", "cookies", "IATACode.txt"] {
            std::fs::copy(bundled.join(name), folder.join(name)).unwrap();
        }
        (temp, folder)
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "Requires prepared bundled detector runtime; executes mock HTTP tools only"]
    async fn bundled_detector_preserves_real_upstream_decisions() {
        let (temp, folder) = fixture();
        std::fs::write(folder.join("runtime/usr/bin/curl"), r##"#!/usr/bin/bash
[[ "$*" == *"--proxy http://127.0.0.1:7897 --noproxy"* ]] || exit 64
printf '0.123\n' >> "$COR_TIMINGS"
case "$*" in
    *philo.com/geo*) printf '{"status":"SUCCESS"}' ;;
    *chatgpt.com/cdn-cgi/trace*) printf 'h=chatgpt.com\nloc=JP\n' ;;
    *ios.chat.openai.com*) printf '{"cf_details":null}' ;;
    *chatgpt.com*) printf 'HTTP/1.1 302\nLocation: /home\n' ;;
    *netflix.com*) printf '<meta property="og:video"/>\nnetflix.reactContext={"models":{"geo":{"data":{"requestCountry":{"id":"HK"}}}}};\n' ;;
    *cwtv.com*) [[ "$*" == *"%{http_code}%output"* ]] || exit 65; printf '200' ;;
    *) exit 66 ;;
esac
"##).unwrap();
        for (id, expected_country) in [
            ("MediaUnlockTest_Philo", "未提供"),
            ("MediaUnlockTest_ChatGPT", "🇯🇵 日本"),
            ("MediaUnlockTest_Netflix", "香港"),
            ("MediaUnlockTest_CWTV", "未提供"),
        ] {
            let service = crate::ip_check::services()
                .iter()
                .find(|item| item.id == id)
                .unwrap();
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(8),
                run(service, 7897, &folder, temp.path()),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(result.state, State::Confirmed, "{}: {}", id, result.detail);
            assert_eq!(result.country, expected_country);
            assert_eq!(result.millis, 123);
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "Requires prepared detector runtime; mock HTTP responses only"]
    async fn challenge_and_transport_failures_are_not_unlock_verdicts() {
        let (temp, folder) = fixture();
        std::fs::write(
            folder.join("runtime/usr/bin/curl"),
            r##"#!/usr/bin/bash
args="$*"
[[ "$args" != *"--max-time 12"* ]] || exit 64
while (( $# )); do
    if [[ "$1" == "--dump-header" ]]; then headers="$2"; shift 2; else shift; fi
done
printf '0.123\n' >> "$COR_TIMINGS"
if [[ "$args" == *"claude.ai"* ]]; then exit 7; fi
if [[ "$args" == *"ios.chat.openai.com"* ]]; then printf '{"cf_details":null}'; exit 0; fi
if [[ "$args" == *"chatgpt.com/cdn-cgi/trace"* ]]; then printf 'h=chatgpt.com\nloc=US\n'; exit 0; fi
if [[ -f "$COR_WORK/homepage" ]]; then
    printf 'HTTP/1.1 200 Connection established\r\n\r\nHTTP/1.1 200 OK\r\n'
    exit 0
fi
printf 'HTTP/1.1 403 Forbidden\r\nCf-Mitigated: challenge\r\n' > "$headers"
printf 'HTTP/1.1 403 Forbidden\r\nCf-Mitigated: challenge\r\n'
"##,
        )
        .unwrap();
        let service = crate::ip_check::services()
            .iter()
            .find(|item| item.id == "MediaUnlockTest_ChatGPT")
            .unwrap();
        let result = run(service, 7897, &folder, temp.path()).await.unwrap();
        assert_eq!(result.state, State::Unknown);
        assert_eq!(result.summary, "验证拦截，未确认");
        assert_eq!(result.country, "🇺🇸 美国");
        std::fs::remove_file(temp.path().join("challenges.txt")).unwrap();
        let service = crate::ip_check::services()
            .iter()
            .find(|item| item.id == "AIUnlockTest_Claude")
            .unwrap();
        let result = run(service, 7897, &folder, temp.path()).await.unwrap();
        assert_eq!(result.state, State::Unknown);
        assert_eq!(result.summary, "请求失败，未确认");
        std::fs::remove_file(temp.path().join("failures.txt")).unwrap();
        std::fs::write(temp.path().join("homepage"), "").unwrap();
        let service = crate::ip_check::services()
            .iter()
            .find(|item| item.id == "MediaUnlockTest_ChatGPT")
            .unwrap();
        let result = run(service, 7897, &folder, temp.path()).await.unwrap();
        assert_eq!(result.state, State::Reachable);
        assert_eq!(result.summary, "网页可达");
        assert_eq!(result.country, "🇺🇸 美国");
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "Requires prepared detector runtime; verifies cancellation kills descendant processes"]
    async fn bundled_detector_cancellation_kills_descendants() {
        use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::*};
        let (temp, folder) = fixture();
        std::fs::write(
            folder.join("runtime/usr/bin/curl"),
            r##"#!/usr/bin/bash
sleep 60 &
cat "/proc/$!/winpid" > "$COR_WORK/sleep.pid"
wait
"##,
        )
        .unwrap();
        let work = temp.path().to_owned();
        let task = tokio::spawn(async move {
            let service = crate::ip_check::services()
                .iter()
                .find(|item| item.id == "MediaUnlockTest_Philo")
                .unwrap();
            run(service, 7897, &folder, &work).await
        });
        let pid = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(text) = tokio::fs::read_to_string(temp.path().join("sleep.pid")).await
                    && let Ok(pid) = text.trim().parse::<u32>()
                {
                    break pid;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        assert!(!handle.is_null());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let mut status = 0;
                assert_ne!(unsafe { GetExitCodeProcess(handle, &mut status) }, 0);
                if status != 259 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        unsafe {
            CloseHandle(handle);
        }
    }

    #[test]
    fn parses_status_before_color_and_preserves_limited_access() {
        assert_eq!(
            parse_output("\r Philo:\t\x1b[32mFailed\x1b[0m\n").state,
            State::Unknown
        );
        let success = parse_output("ChatGPT:\t\x1b[32mYes (Region: JP)\x1b[0m");
        assert_eq!(success.state, State::Confirmed);
        assert_eq!(success.country, "🇯🇵 日本");
        assert_eq!(
            parse_output("Netflix: Originals Only (Region: HK)").country,
            "香港"
        );
        assert_eq!(
            parse_output("ChatGPT: Web Only (Disallowed ISP[1])").summary,
            "仅网页可用"
        );
        assert_eq!(parse_output("J:COM On Demand: No").state, State::Restricted);
        assert_eq!(parse_output("Gemini: null").state, State::Unknown);
        assert_eq!(
            parse_output("Google Gemini Location: United States").country,
            "🇺🇸 美国"
        );
        assert_eq!(
            parse_output("Google Gemini Location: United States").state,
            State::Confirmed
        );
        assert_eq!(parse_output("ChatGPT: No").country, "未提供");
        assert_eq!(parse_output("ChatGPT: No (Blocked)").state, State::Unknown);
        assert_eq!(
            parse_output("ChatGPT: No (Unsupported Region)").summary,
            "地区不支持"
        );
    }
}
