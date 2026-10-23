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
    let value: Value = serde_yaml::from_str(raw)?;
    let value = if args.flag("show-secrets") {
        value
    } else {
        output::redact(value)
    };
    let yaml = serde_yaml::to_string(&value)?;
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
    let (port, api, include_prereleases) =
        if let Some(remote) = backend::connect(args, false).await? {
            let meta: Meta = remote.request(Command::Meta).await?;
            (
                meta.running.then_some(meta.settings.mixed_port),
                Some(clash_client_core::api::Api::new(&meta.settings)?),
                meta.settings.update_prereleases,
            )
        } else {
            (
                None,
                None,
                backend::store(args)?.load_settings()?.update_prereleases,
            )
        };
    if args.path == "update install" {
        let prompt = if include_prereleases {
            format!(
                "{}\n按当前通道下载安装，停止当前内核和后台？",
                clash_client_core::update::channel::PRERELEASE_WARNING
            )
        } else {
            "下载并安装最新正式版（预发布版可返回正式版），停止当前内核和后台？".into()
        };
        output::confirm(args, &prompt).await?;
        return output::emit(args, backend::request(args, Command::UpdateInstall).await?);
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
