//! Ordinary-user, fixed-file bridge. This never starts a service or elevates.
use std::ffi::OsString;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) enum Error {
    Unsupported,
    Unavailable,
    SystemAccount,
    Launch,
    Timeout,
    Output,
}

impl Error {
    pub(super) const fn message(self) -> &'static str {
        match self {
            Self::Unsupported => "WSL 安全网络仅支持 Windows。",
            Self::Unavailable => "受信任的 WSL 管理资源不可用；请修复 fork 安装包。",
            Self::SystemAccount => "禁止使用 SYSTEM 身份操作用户 WSL。",
            Self::Launch => "无法启动 WSL 管理操作；未自动提权。",
            Self::Timeout => "WSL 管理操作超时；实际保护状态未知，请刷新状态后再操作。",
            Self::Output => "WSL 管理输出无效或超过大小限制；保护状态未知。",
        }
    }
}

#[cfg(not(windows))]
pub(super) async fn run(_arguments: Vec<OsString>) -> Result<String, Error> {
    Err(Error::Unsupported)
}

#[cfg(windows)]
pub(super) async fn run(arguments: Vec<OsString>) -> Result<String, Error> {
    use std::{process::Stdio, time::Duration};
    use tokio::io::AsyncReadExt;

    let (powershell, script, _verified) = tokio::task::spawn_blocking(trusted_paths)
        .await
        .map_err(|_| Error::Unavailable)??;
    let directory = script.parent().ok_or(Error::Unavailable)?;
    let mut child = tokio::process::Command::new(powershell)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
        .arg(&script)
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000)
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| Error::Launch)?;
    let stdout = child.stdout.take().ok_or(Error::Output)?;
    let mut stderr = child.stderr.take().ok_or(Error::Output)?;
    // Read both pipes concurrently; never retain stderr, raw errors or command
    // lines, which may include private profile paths or credentials.
    const LIMIT: u64 = 1024 * 1024;
    let operation = async {
        let output = async {
            let mut bytes = Vec::new();
            stdout.take(LIMIT + 1).read_to_end(&mut bytes).await?;
            Ok::<_, std::io::Error>(bytes)
        };
        let discarded = async { tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await };
        let (bytes, _, status) =
            tokio::try_join!(output, discarded, child.wait()).map_err(|_| Error::Output)?;
        if bytes.len() as u64 > LIMIT {
            return Err(Error::Output);
        }
        // A blocked operation may use a non-zero process exit. The strict JSON
        // contract, rather than an exit code, determines its displayed outcome.
        let text = String::from_utf8(bytes)
            .map(|text| text.trim_start_matches('\u{feff}').to_owned())
            .map_err(|_| Error::Output)?;
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| Error::Output)?;
        if !status.success() && !matches!(value["outcome"].as_str(), Some("blocked" | "failed")) {
            return Err(Error::Output);
        }
        Ok(text)
    };
    match tokio::time::timeout(Duration::from_secs(300), operation).await {
        Ok(result) => result,
        Err(_) => {
            let _ = child.kill().await;
            Err(Error::Timeout)
        }
    }
}

#[cfg(windows)]
fn trusted_paths() -> Result<
    (
        std::path::PathBuf,
        std::path::PathBuf,
        super::payload::Verified,
    ),
    Error,
> {
    use std::{os::windows::fs::MetadataExt, path::PathBuf};
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        Security::{
            GetTokenInformation, IsWellKnownSid, TOKEN_QUERY, TOKEN_USER, TokenUser,
            WinLocalServiceSid, WinLocalSystemSid, WinNetworkServiceSid,
        },
        System::{
            SystemInformation::GetSystemDirectoryW,
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    };

    // A service must never enumerate or launch a user's distributions. Do not
    // infer identity from environment variables such as USERNAME.
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(Error::SystemAccount);
    }
    let mut needed = 0;
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
    let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    let success = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    unsafe { CloseHandle(token) };
    if success == 0 || buffer.is_empty() {
        return Err(Error::SystemAccount);
    }
    let sid = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() }.User.Sid;
    if [WinLocalSystemSid, WinLocalServiceSid, WinNetworkServiceSid]
        .into_iter()
        .any(|kind| unsafe { IsWellKnownSid(sid, kind) } != 0)
    {
        return Err(Error::SystemAccount);
    }

    let mut directory = vec![0u16; 32768];
    let length =
        unsafe { GetSystemDirectoryW(directory.as_mut_ptr(), directory.len() as u32) } as usize;
    if length == 0 || length >= directory.len() {
        return Err(Error::Unavailable);
    }
    let powershell =
        PathBuf::from(String::from_utf16(&directory[..length]).map_err(|_| Error::Unavailable)?)
            .join("WindowsPowerShell/v1.0/powershell.exe");
    if !powershell.is_file() {
        return Err(Error::Unavailable);
    }

    let executable = std::env::current_exe().map_err(|_| Error::Unavailable)?;
    let installed = executable.parent().ok_or(Error::Unavailable)?;
    let resources = installed.join("resources");
    let package = resources.join("wsl-secure-egress");
    let script = package.join("Manage-WslSecureEgress.ps1");
    // Reject junctions/reparse points in the resource chain. There is no debug
    // fallback to cwd, a development checkout, PATH or a user-selected script.
    for path in [&resources, &package, &script] {
        let metadata = std::fs::symlink_metadata(path).map_err(|_| Error::Unavailable)?;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(Error::Unavailable);
        }
    }
    let verified = super::payload::verify(&package).map_err(|_| Error::Unavailable)?;
    Ok((powershell, script, verified))
}

/// Display-only instructions. No shell evaluates this string in the client.
#[cfg(windows)]
pub(super) async fn administrator_command(arguments: Vec<OsString>) -> String {
    let Ok(Ok((powershell, script, _verified))) = tokio::task::spawn_blocking(trusted_paths).await
    else {
        return "# 未找到校验通过的安装资源；请先修复定制版安装包。".into();
    };
    if !protected_installation(&script) {
        return "# 管理员操作仅支持 Program Files 内的正式安装资源；请先修复定制版安装位置。"
            .into();
    }
    let quote =
        |text: &std::ffi::OsStr| format!("'{}'", text.to_string_lossy().replace('\'', "''"));
    // The elevated process runs under a different token. Ask the same Windows
    // user to start an elevated PowerShell explicitly; an administrator can
    // then execute the reviewed command after reading it.
    let mut parts = vec![
        "Start-Process".into(),
        "-FilePath".into(),
        quote(powershell.as_os_str()),
        "-Verb".into(),
        "RunAs".into(),
        "-ArgumentList".into(),
    ];
    let mut inner = vec![
        "-NoLogo".to_owned(),
        "-NoProfile".to_owned(),
        "-NonInteractive".to_owned(),
        "-File".to_owned(),
        quote(script.as_os_str()),
    ];
    inner.extend(arguments.iter().map(|argument| quote(argument)));
    // Start-Process joins ArgumentList with spaces. Escape embedded PowerShell
    // quotes once for that join, then escape again for the outer single-quoted
    // literal. This keeps the displayed command from breaking on paths.
    let argument_list = inner.join(" ").replace('"', "\\\"");
    parts.push(format!("'{}'", argument_list.replace('\'', "''")));
    format!(
        "# 复制到当前用户的 PowerShell 后手动执行；会显示 UAC，不会静默提权：\n{}",
        parts.join(" ")
    )
}

#[cfg(windows)]
fn protected_installation(script: &std::path::Path) -> bool {
    use windows_sys::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath},
    };
    let mut raw = std::ptr::null_mut();
    if unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramFiles, 0, std::ptr::null_mut(), &mut raw) }
        < 0
    {
        return false;
    }
    let mut length = 0;
    unsafe {
        while *raw.add(length) != 0 {
            length += 1;
        }
    }
    let folder = unsafe { String::from_utf16(std::slice::from_raw_parts(raw, length)) };
    unsafe { CoTaskMemFree(raw.cast()) };
    let Ok(folder) = folder else {
        return false;
    };
    let (Ok(script), Ok(folder)) = (
        script.canonicalize(),
        std::path::Path::new(&folder).canonicalize(),
    ) else {
        return false;
    };
    script.starts_with(folder)
}

#[cfg(not(windows))]
pub(super) async fn administrator_command(_arguments: Vec<OsString>) -> String {
    "WSL 安全网络仅支持 Windows。".into()
}
