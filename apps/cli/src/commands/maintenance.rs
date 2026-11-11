use super::*;
use anyhow::Context;
use clash_client_core::config::atomic_write;

pub(super) async fn execute(args: &Args) -> Result<()> {
    match args.path.as_str() {
        "config get" => {
            let mut value = serde_json::to_value(backend::meta(args).await?.settings)?;
            value
                .as_object_mut()
                .expect("settings object")
                .remove("controller_port");
            let value = if let Some(key) = args.positional.first() {
                ensure!(key != "controller_port", "控制端口由后台自动管理");
                ensure!(value.get(key).is_some(), "未知设置项：{key}");
                json!({(key):value[key].clone()})
            } else {
                value
            };
            output::emit(args, value)
        }
        "config set" => {
            let raw = &args.positional[1];
            let value = serde_json::from_str(raw).unwrap_or_else(|_| json!(raw));
            let _: Value = backend::request(
                args,
                Command::ConfigSet {
                    key: args.positional[0].clone(),
                    value,
                },
            )
            .await?;
            output::emit(args, json!({"saved":true,"key":args.positional[0]}))
        }
        "config check" => output::emit(args, backend::request(args, Command::CheckConfig).await?),
        "config effective" => {
            let value: Value = backend::request(args, Command::EffectiveConfig).await?;
            yaml(args, value["yaml"].as_str().context("配置内容缺失")?)
        }
        "env" => environment(args).await,
        "geo status" => output::emit(args, serde_json::to_value(backend::meta(args).await?.geo)?),
        "geo update" => {
            let _: Value = backend::request(args, Command::GeoUpdate).await?;
            output::emit(args, serde_json::to_value(backend::meta(args).await?.geo)?)
        }
        "autostart" => {
            let enabled = args
                .positional
                .first()
                .map(|v| output::on_off(v))
                .transpose()?;
            output::emit(
                args,
                backend::request(args, Command::Autostart { enabled }).await?,
            )
        }
        "notify" => {
            let _: Value = backend::request(
                args,
                Command::Notify {
                    title: "Clash of Rust".into(),
                    message: args
                        .positional
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "喵!".into()),
                },
            )
            .await?;
            output::emit(args, json!({"sent":true}))
        }
        path if path.starts_with("service ") => {
            if path != "service status" {
                backend::store(args)?.load_settings()?.require_mutable()?;
                output::confirm(
                    args,
                    if path == "service install" {
                        "设置当前用户的 CLI 后台登录自启？"
                    } else {
                        "移除 CLI 后台登录自启？"
                    },
                )
                .await?;
            }
            crate::service::execute(args)
        }
        path if path.starts_with("update ") => update(args).await,
        _ => bail!("未知命令：{}", args.path),
    }
}

pub(super) fn yaml(args: &Args, raw: &str) -> Result<()> {
    let value: Value = yaml_serde::from_str(raw)?;
    let value = if args.flag("show-secrets") {
        value
    } else {
        output::redact(value)
    };
    let yaml = yaml_serde::to_string(&value)?;
    if let Some(path) = args.get("output") {
        atomic_write(&backend::absolute(path)?, yaml.as_bytes())?;
        if !args.flag("quiet") {
            eprintln!("已保存：{path}");
        }
        Ok(())
    } else if args.flag("json") || args.flag("jsonl") {
        output::emit(args, value)
    } else {
        use std::io::Write;
        std::io::stdout().lock().write_all(yaml.as_bytes())?;
        Ok(())
    }
}

async fn environment(args: &Args) -> Result<()> {
    let settings = backend::meta(args).await?.settings;
    let port = settings.mixed_port;
    let shell = args
        .get("shell")
        .unwrap_or(if cfg!(windows) { "powershell" } else { "bash" });
    let mut lines = Vec::new();
    let mut values = serde_json::Map::new();
    for (key, value) in [
        ("http_proxy", format!("http://127.0.0.1:{port}")),
        ("https_proxy", format!("http://127.0.0.1:{port}")),
        ("all_proxy", format!("socks5h://127.0.0.1:{port}")),
        ("no_proxy", "localhost,127.0.0.1,::1".into()),
    ] {
        for key in [key.to_owned(), key.to_ascii_uppercase()] {
            values.insert(
                key.clone(),
                if args.flag("unset") {
                    Value::Null
                } else {
                    json!(value)
                },
            );
            let line = match (shell, args.flag("unset")) {
                ("bash" | "zsh", true) => format!("unset {key}"),
                ("bash" | "zsh", false) => format!("export {key}='{value}'"),
                ("fish", true) => format!("set -e {key}"),
                ("fish", false) => format!("set -gx {key} '{value}'"),
                ("powershell", true) => {
                    format!("Remove-Item Env:{key} -ErrorAction SilentlyContinue")
                }
                ("powershell", false) => format!("$env:{key} = '{value}'"),
                ("cmd", true) => format!("set \"{key}=\""),
                ("cmd", false) => format!("set \"{key}={value}\""),
                _ => bail!("不支持的 shell"),
            };
            lines.push(line);
        }
    }
    if args.flag("json") || args.flag("jsonl") {
        output::emit(args, Value::Object(values))
    } else {
        use std::io::Write;
        writeln!(std::io::stdout().lock(), "{}", lines.join("\n"))?;
        Ok(())
    }
}

pub(super) async fn authorize_tun(args: &Args) -> Result<()> {
    backend::authorize_tun(args).await
}

async fn update(args: &Args) -> Result<()> {
    if args.path == "update status" {
        let status = clash_client_runtime::update::installation_status(&backend::data_root(args)?)?;
        return output::emit(
            args,
            match status {
                Some(status) => json!({
                    "state":status.state, "version":status.version,
                    "message":status.message, "error":status.error, "updated_at":status.updated_at,
                }),
                None => json!({"state":"none","message":"没有安装记录"}),
            },
        );
    }
    let (port, api, include_prereleases) =
        if let Some(remote) = backend::connect(args, false).await? {
            let meta: Meta = remote.request(Command::Meta).await?;
            meta.settings.require_mutable()?;
            (
                meta.running.then_some(meta.settings.download_ports()),
                Some(clash_client_core::api::Api::new(&meta.settings)?),
                meta.settings.update_prereleases,
            )
        } else {
            let settings = backend::store(args)?.load_settings()?;
            settings.require_mutable()?;
            (None, None, settings.update_prereleases)
        };
    if args.path == "update install" {
        let available =
            clash_client_runtime::update::check_with_prereleases(port, include_prereleases)
                .await?
                .context("当前更新通道没有可安装的版本")?;
        output::confirm(args, &update_install_prompt(&available)).await?;
        if !args.flag("quiet") && !args.flag("json") && !args.flag("jsonl") {
            eprintln!("正在下载安装包并校验 SHA256…");
        }
        let terminal = cfg!(target_os = "linux")
            && clash_client_runtime::control::gui_pid(&backend::store(args)?)?.is_none();
        let operation = Command::UpdateSelected {
            version: available.version,
            preview: available.preview,
            terminal,
        };
        #[cfg(target_os = "linux")]
        if terminal {
            let prepared: Value = backend::request(args, operation)
                .await
                .context("终端安装准备失败；旧版后台请先退出后重试")?;
            let directory = std::path::PathBuf::from(
                prepared["installer"]
                    .as_str()
                    .context("后台没有返回终端安装目录")?,
            );
            let result = tokio::task::spawn_blocking(move || {
                clash_client_runtime::update::run_terminal_install(&directory)
            })
            .await??;
            if result.error {
                bail!("{}", result.message);
            }
            return output::emit(
                args,
                json!({"state":result.state,"version":result.version,"message":result.message,"error":false}),
            );
        }
        return output::emit(args, backend::request(args, operation).await?);
    }
    let available =
        clash_client_runtime::update::check_with_prereleases(port, include_prereleases).await?;
    if args.path == "update check" {
        return output::emit(
            args,
            match available {
                Some(a) => {
                    json!({"available":true,"version":a.version,"preview":a.preview,"url":a.url,"package":a.package.map(|p|p.asset.name)})
                }
                None => json!({"available":false}),
            },
        );
    }
    let output_path = backend::absolute(args.get("output").context("请提供 --output FILE")?)?;
    let available = available.context("当前更新通道没有可安装的版本")?;
    let mut last_report: Option<std::time::Instant> = None;
    let downloaded = clash_client_runtime::update::download(available, port, api, |value| {
        if !args.flag("quiet")
            && value.received > 0
            && (value.received == value.total
                || last_report
                    .is_none_or(|last| last.elapsed() >= std::time::Duration::from_secs(1)))
        {
            eprintln!("{}", output::download_progress(&value));
            last_report = Some(std::time::Instant::now());
        }
    })
    .await?;
    let parent = output_path.parent().context("导出路径无效")?;
    std::fs::create_dir_all(parent)?;
    clash_client_runtime::assets::atomic_copy(downloaded.package_path(), &output_path)?;
    // Do not make output::emit interpret --output a second time.
    let value = json!({"path":output_path,"sha256":downloaded.sha256()});
    if args.flag("json") || args.flag("jsonl") {
        use std::io::Write;
        writeln!(
            std::io::stdout().lock(),
            "{}",
            json!({"schema":1,"command":args.path,"data":value})
        )?;
    } else if !args.flag("quiet") {
        eprintln!(
            "已保存：{}\nSHA256：{}",
            output_path.display(),
            downloaded.sha256()
        );
    }
    Ok(())
}

fn update_install_prompt(available: &clash_client_runtime::update::Available) -> String {
    let channel = if available.preview {
        "预发布版"
    } else {
        "正式版"
    };
    let prompt = format!(
        "下载并安装{channel} {}，停止当前内核和后台？",
        available.version
    );
    if available.preview {
        format!(
            "{}\n{prompt}",
            clash_client_core::update::channel::PRERELEASE_WARNING
        )
    } else {
        prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_install_warning_follows_the_selected_release_not_the_channel() {
        for (version, preview) in [
            ("0.7.1", false),
            ("0.7.1-fix.1.example", true),
            ("0.7.1", true),
        ] {
            let available = clash_client_runtime::update::Available {
                version: version.into(),
                preview,
                url: clash_client_runtime::update::RELEASES_URL.into(),
                package: None,
            };
            let prompt = update_install_prompt(&available);
            assert!(prompt.contains(version));
            assert_eq!(
                prompt.contains(clash_client_core::update::channel::PRERELEASE_WARNING),
                preview
            );
            assert_eq!(prompt.starts_with("下载并安装正式版"), !preview);
        }
    }
}
