//! Local signals and a headless diagnostic entry point for the real Code shell.
use super::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const ENV_KEYS: [&str; 10] = [
    "https_proxy",
    "HTTPS_PROXY",
    "http_proxy",
    "HTTP_PROXY",
    "no_proxy",
    "NO_PROXY",
    "TZ",
    "LANG",
    "LC_ALL",
    "ANTHROPIC_BASE_URL",
];

pub use clash_client_core::claude_check::preferences::{
    bounded, preference, proxy_bypass_display, redact_url, region_tag, timezone_country,
};

#[cfg(any(windows, target_os = "macos"))]
async fn command(program: &str, args: &[&str]) -> Option<String> {
    use tokio::io::AsyncReadExt;
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        let mut bytes = Vec::new();
        (&mut stdout)
            .take(4097)
            .read_to_end(&mut bytes)
            .await
            .ok()?;
        if bytes.len() > 4096 {
            return None;
        }
        if !child.wait().await.ok()?.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&bytes).trim().to_owned())
    })
    .await
    .ok()
    .flatten();
    if result.is_none() {
        let _ = child.kill().await;
    }
    result
}

pub async fn local(environment: Environment) -> Report {
    let group = if environment == Environment::Code {
        Group::Code
    } else {
        Group::Desktop
    };
    let source = if environment == Environment::Code {
        "运行检测命令的终端进程"
    } else {
        "本机系统；不是 Desktop 内部读取结果"
    };
    let mut report = Report::new(environment, source);
    let mut zone = (environment == Environment::Code)
        .then(|| std::env::var("TZ").ok().filter(|s| !s.is_empty()))
        .flatten();
    #[cfg(unix)]
    if zone.is_none() {
        zone = std::fs::read_link("/etc/localtime")
            .ok()
            .and_then(|p| p.to_str()?.split("zoneinfo/").nth(1).map(str::to_owned));
        #[cfg(target_os = "linux")]
        if zone.is_none() {
            zone = std::fs::read_to_string("/etc/timezone")
                .ok()
                .map(|s| s.trim().to_owned());
        }
    }
    #[cfg(windows)]
    let native = command("powershell.exe", &["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); @{zone=(Get-TimeZone).Id;language=(Get-Culture).Name}|ConvertTo-Json -Compress"]).await
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    #[cfg(windows)]
    if zone.is_none() {
        zone = native
            .as_ref()
            .and_then(|v| v["zone"].as_str())
            .map(str::to_owned);
    }
    let offset = chrono::Local::now().offset().local_minus_utc() / 60;
    report.signals.push(match zone {
        Some(zone) => preference(
            group,
            "时区",
            &format!("{} · UTC{:+.1}", bounded(&zone), offset as f64 / 60.0),
            source,
            timezone_country(&zone),
        ),
        None => Signal::new(
            group,
            "UTC 偏移",
            format!("UTC{:+.1}；时区名称未确认", offset as f64 / 60.0),
            source,
            Evidence::Measured,
            "偏移相同的地区很多，未按偏移推断国家。",
        ),
    });
    #[allow(unused_mut)]
    let mut language = std::env::var("LC_ALL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("LANG").ok());
    #[cfg(windows)]
    if environment != Environment::Code || language.is_none() {
        language = native
            .as_ref()
            .and_then(|v| v["language"].as_str())
            .map(str::to_owned);
    }
    #[cfg(target_os = "macos")]
    if environment != Environment::Code || language.is_none() {
        language = command("/usr/bin/defaults", &["read", "-g", "AppleLocale"]).await;
    }
    if let Some(language) = language {
        report.signals.push(preference(
            group,
            "语言与地区格式",
            &language,
            source,
            region_tag(&language).as_deref(),
        ));
    } else {
        report.signals.push(Signal::unknown(
            group,
            "语言与地区格式",
            source,
            "未读取到设置值。",
        ));
    }
    report.signals.push(Signal::new(
        group,
        "操作系统",
        format!("{} · {}", std::env::consts::OS, std::env::consts::ARCH),
        source,
        Evidence::Measured,
        "记录运行环境；操作系统本身不能确定地区。",
    ));
    if environment == Environment::Desktop {
        let proxy =
            tokio::task::spawn_blocking(|| crate::platform::configured_proxy("https")).await;
        report.signals.push(match proxy {
            Ok(Ok(Some(proxy))) => Signal::new(
                group,
                "系统 HTTPS 代理",
                redact_url(&proxy),
                source,
                Evidence::Measured,
                "这是系统代理配置；Desktop 是否遵循该配置需要结合请求观测确认。",
            ),
            Ok(Ok(None)) => Signal::new(
                group,
                "系统 HTTPS 代理",
                "未配置；TUN 仍可能接管流量",
                source,
                Evidence::Measured,
                "未配置系统代理不能证明客户端直连。",
            ),
            _ => Signal::unknown(group, "系统 HTTPS 代理", source, "系统代理读取失败。"),
        });
        report.signals.push(Signal::unknown(group, "Desktop 内部地区环境", "客户端内部", "系统语言和时区已记录，但不能当作 Desktop 内部 JavaScript、定位权限或网络出口的实测结果。可开启 Claude 请求实时检测，再操作 Desktop。"));
        report.signals.push(Signal::unknown(
            group,
            "定位权限",
            "系统应用权限",
            "请在系统的应用定位权限中核对 Claude Desktop；本工具不会请求或修改 Desktop 的权限。",
        ));
    }
    report
}

fn environment() -> BTreeMap<String, String> {
    ENV_KEYS
        .iter()
        .filter_map(|key| std::env::var(key).ok().map(|value| ((*key).into(), value)))
        .collect()
}

fn settings_env(path: &Path) -> Result<BTreeMap<String, String>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(REPORT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > REPORT_LIMIT {
        bail!("配置文件超过检测大小限制");
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let Some(env) = value.get("env").and_then(|value| value.as_object()) else {
        return Ok(BTreeMap::new());
    };
    Ok(ENV_KEYS
        .iter()
        .filter_map(|key| {
            env.get(*key)
                .and_then(|v| v.as_str())
                .map(|value| ((*key).into(), value.into()))
        })
        .collect())
}

pub fn code_settings(home: &Path, cwd: &Path) -> Vec<PathBuf> {
    let project = cwd
        .ancestors()
        .find(|path| path.join(".git").exists() || path.join(".claude").is_dir())
        .unwrap_or(cwd);
    let mut files = vec![
        home.join(".claude/settings.json"),
        project.join(".claude/settings.json"),
        project.join(".claude/settings.local.json"),
    ];
    #[cfg(target_os = "macos")]
    files.push(PathBuf::from(
        "/Library/Application Support/ClaudeCode/managed-settings.json",
    ));
    #[cfg(target_os = "linux")]
    files.push(PathBuf::from("/etc/claude-code/managed-settings.json"));
    #[cfg(windows)]
    if let Some(directory) = std::env::var_os("ProgramFiles") {
        files.push(PathBuf::from(directory).join("ClaudeCode/managed-settings.json"));
    }
    files
}

fn apply_settings(env: &mut BTreeMap<String, String>, report: &mut Report, files: &[PathBuf]) {
    for file in files {
        if !file.is_file() {
            continue;
        }
        let title = file
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("settings.json");
        match settings_env(file) {
            Ok(values) => {
                for (key, value) in &values {
                    let display = if key.ends_with("PROXY")
                        || key.ends_with("proxy")
                        || key == "ANTHROPIC_BASE_URL"
                    {
                        if key.eq_ignore_ascii_case("no_proxy") {
                            proxy_bypass_display(value)
                        } else {
                            redact_url(value)
                        }
                    } else {
                        bounded(value)
                    };
                    report.signals.push(Signal::new(Group::Code, &format!("配置 {key}"), display, &format!("{title}（按用户、项目、本地、设备托管顺序检查）"), Evidence::Measured, "只读取地区与代理相关的白名单字段；配置是否生效还取决于项目信任、启动参数和其他托管来源。"));
                }
                env.extend(values);
            }
            Err(_) => report.signals.push(Signal::unknown(
                Group::Code,
                title,
                "Code 配置文件",
                "配置未能解析或超过大小限制；相关覆盖值未确认。",
            )),
        }
    }
}

fn code_client(env: &BTreeMap<String, String>) -> Result<Client> {
    let mut builder = Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(value) = ["https_proxy", "HTTPS_PROXY", "http_proxy", "HTTP_PROXY"]
        .iter()
        .find_map(|key| env.get(*key))
    {
        let url = reqwest::Url::parse(value)?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("Claude Code 不支持此代理协议");
        }
        let no_proxy = env
            .get("no_proxy")
            .or_else(|| env.get("NO_PROXY"))
            .map(|s| {
                s.split(|c: char| c == ',' || c.is_ascii_whitespace())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(",")
            });
        builder = builder.proxy(
            reqwest::Proxy::all(value)?
                .no_proxy(no_proxy.as_deref().and_then(reqwest::NoProxy::from_string)),
        );
    }
    Ok(builder.build()?)
}

pub async fn code(offline: bool) -> Result<Report> {
    let mut report = local(Environment::Code).await;
    let mut env = environment();
    // Display shell values independently of candidate config overrides.
    for key in ENV_KEYS {
        if let Some(value) = env.get(key) {
            let display = if key.eq_ignore_ascii_case("no_proxy") {
                proxy_bypass_display(value)
            } else if key.to_ascii_lowercase().ends_with("proxy") || key == "ANTHROPIC_BASE_URL" {
                redact_url(value)
            } else {
                bounded(value)
            };
            report.signals.push(Signal::new(
                Group::Code,
                key,
                display,
                "启动检测命令的终端",
                Evidence::Measured,
                "此值来自检测命令的启动环境；已经运行的 Code 会话可能仍使用旧值。",
            ));
        }
    }
    if let Some(dirs) = directories::BaseDirs::new() {
        let cwd = std::env::current_dir()?;
        apply_settings(&mut env, &mut report, &code_settings(dirs.home_dir(), &cwd));
    }
    report.signals.push(Signal::unknown(Group::Code, "运行中 Code 会话", "Claude Code 内部", "终端采样和可读取配置用于复现候选启动环境；不读取会话凭据。项目是否受信任、--settings 参数、MDM、云端托管及 Desktop 嵌入会话仍需在 Code 的 /status 中确认。"));
    if let Some(base) = env.get("ANTHROPIC_BASE_URL") {
        report.signals.push(Signal::new(Group::Code, "API 地址覆盖", redact_url(base), "终端与候选配置", Evidence::Measured, "若使用第三方中转服务，Claude 可能看到中转端的出口，本地无法识别该远端出口。本页官方域名 trace 仅表示本机探测，未代替中转或运行中 Code 请求。"));
    }
    if offline {
        report.signals.push(Signal::unknown(
            Group::Exit,
            "网络与 DNS",
            "离线检测",
            "未发出网络请求。",
        ));
    } else {
        match code_client(&env) {
            Ok(client) => {
                network(
                    &client,
                    &mut report,
                    "终端环境与可读取配置复现；不是运行中 Code 请求",
                )
                .await;
                report.signals.extend(
                    system_dns(&client, "运行检测命令的系统解析器；不是 HTTP 代理远端 DNS").await,
                );
            }
            Err(_) => report.signals.push(Signal::unknown(
                Group::Exit,
                "Code 网络出口",
                "终端代理设置",
                "代理协议或地址不受支持；未回退到直连。请检查 HTTPS_PROXY/HTTP_PROXY。",
            )),
        }
    }
    report.validate()?;
    Ok(report)
}

/// Runs before any tray, window, store, privilege or core initialization.
pub fn helper_main() -> Option<Result<()>> {
    let args: Vec<_> = std::env::args_os().collect();
    if !args.iter().any(|arg| arg == "--claude-diagnose") {
        return None;
    }
    Some((|| {
        let mut output = None;
        let mut submit = None;
        let mut offline = false;
        let mut args = args.into_iter().skip(1);
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--claude-diagnose") => {}
                Some("--output") => {
                    output = Some(PathBuf::from(args.next().context("缺少报告路径")?))
                }
                Some("--submit") => {
                    submit = Some(
                        args.next()
                            .context("缺少回传地址")?
                            .into_string()
                            .map_err(|_| anyhow::anyhow!("回传地址无效"))?,
                    )
                }
                Some("--offline") => offline = true,
                _ => bail!("不支持的终端检测参数"),
            }
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let report = code(offline).await?;
            let bytes = serde_json::to_vec_pretty(&report)?;
            if let Some(url) = submit {
                browser::validate_submit_url(&url)?;
                let response = Client::builder()
                    .no_proxy()
                    .timeout(Duration::from_secs(8))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()?
                    .post(url)
                    .header("X-Clash-Diagnostic", "1")
                    .header("Content-Type", "application/json")
                    .body(bytes.clone())
                    .send()
                    .await?;
                response
                    .error_for_status_ref()
                    .context("回传失败；请保持检测页面开启并重新复制命令")?;
            } else if output.is_none() {
                output = Some(std::env::current_dir()?.join("claude-code-report.json"));
            }
            if let Some(path) = output {
                let path = if path.is_absolute() {
                    path
                } else {
                    std::env::current_dir()?.join(path)
                };
                crate::config::atomic_write(&path, &bytes)?;
            }
            Ok(())
        })
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_and_endpoint_reports_never_include_credentials_or_query_tokens() {
        assert_eq!(
            redact_url("http://alice:secret@127.0.0.1:7890/private?token=secret#secret"),
            "http://127.0.0.1:7890/"
        );
        assert!(!redact_url("invalid secret").contains("secret"));
        assert!(
            !proxy_bypass_display("example.com http://alice:secret@proxy.test?key=secret")
                .contains("secret")
        );
    }
    #[test]
    fn settings_only_read_whitelisted_fields_in_scope_order() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a.json");
        let b = root.path().join("b.json");
        std::fs::write(&a, r#"{"env":{"HTTPS_PROXY":"http://alice:secret@proxy.test:7890","ANTHROPIC_API_KEY":"never-export"},"other":"never-export"}"#).unwrap();
        std::fs::write(&b, r#"{"env":{"HTTPS_PROXY":"http://next.test:7891"}}"#).unwrap();
        let mut env = BTreeMap::new();
        let mut report = Report::new(Environment::Code, "fixture");
        apply_settings(&mut env, &mut report, &[a, b]);
        assert_eq!(env.get("HTTPS_PROXY").unwrap(), "http://next.test:7891");
        let json = serde_json::to_string(&report).unwrap();
        assert!(
            !json.contains("secret") && !json.contains("never-export") && !json.contains("alice")
        );
    }
    #[test]
    fn preferences_do_not_infer_country_from_language_or_offset_alone() {
        assert_eq!(region_tag("zh-CN.UTF-8"), Some("CN".into()));
        assert_eq!(region_tag("zh-Hant-TW"), Some("TW".into()));
        assert_eq!(region_tag("zh"), None);
        assert_eq!(timezone_country("UTC+8"), None);
        assert!(
            code_client(&BTreeMap::from([(
                "HTTPS_PROXY".into(),
                "socks5://localhost:7890".into()
            )]))
            .is_err()
        );
    }
}
