use super::*;
use anyhow::Context;
use clash_client_core::config;
use std::io::Read;

pub(super) async fn execute(args: &Args) -> Result<()> {
    let meta = backend::meta(args).await?;
    match args.path.as_str() {
        "profile list" => {
            let filter = args.get("filter").unwrap_or("").to_lowercase();
            output::emit(args,json!(meta.profiles.iter().filter(|p|p.name.to_lowercase().contains(&filter)||p.id.contains(&filter))
                .map(|p|json!({"id":p.id,"name":p.name,"active":meta.settings.active_profile.as_deref()==Some(&p.id),"source":p.source,"updated":p.updated,"usage":p.usage})).collect::<Vec<_>>()))
        }
        "profile directory" => {
            output::emit(args, json!(backend::store(args)?.root.join("profiles")))
        }
        "profile add" | "profile import" => {
            let (name, source) = if args.path == "profile add" {
                (&*args.positional[0], &*args.positional[1])
            } else {
                (
                    args.get("name").context("导入文件时请提供 --name")?,
                    &*args.positional[0],
                )
            };
            let source = source_path(args, source)?;
            let _: Value = backend::request(
                args,
                Command::Import {
                    name: name.into(),
                    source,
                    id: None,
                },
            )
            .await?;
            output::emit(args, json!({"imported":name}))
        }
        "profile update" => {
            ensure!(
                !(args.flag("all") && !args.positional.is_empty()),
                "PROFILE 与 --all 不能同时指定"
            );
            let profiles = if args.flag("all") {
                meta.profiles
                    .iter()
                    .filter(|p| !p.is_default())
                    .collect::<Vec<_>>()
            } else {
                vec![super::profile(
                    &meta,
                    args.positional.first().map(String::as_str),
                )?]
            };
            let mut results = Vec::new();
            let mut failed = false;
            for p in profiles {
                if p.is_default() {
                    continue;
                }
                let result = backend::request::<Value>(
                    args,
                    Command::Import {
                        name: p.name.clone(),
                        source: p.source.clone(),
                        id: Some(p.id.clone()),
                    },
                )
                .await;
                let error = result.err().map(|e| format!("{e:#}"));
                failed |= error.is_some();
                results
                    .push(json!({"id":p.id,"name":p.name,"updated":error.is_none(),"error":error}));
            }
            output::emit(args, json!(results))?;
            ensure!(!failed, "部分订阅更新失败");
            Ok(())
        }
        "profile use" => {
            let p = super::profile(&meta, Some(&args.positional[0]))?;
            let _: Value = backend::request(args, Command::Activate { id: p.id.clone() }).await?;
            output::emit(args, json!({"active":p.id,"name":p.name}))
        }
        "profile edit" => {
            ensure!(
                args.get("name").is_some() || args.get("source").is_some(),
                "请提供 --name 或 --source"
            );
            let p = super::profile(&meta, Some(&args.positional[0]))?;
            let source = args
                .get("source")
                .map(|s| source_path(args, s))
                .transpose()?
                .unwrap_or_else(|| p.source.clone());
            let name = args.get("name").unwrap_or(&p.name).into();
            let _: Value = backend::request(
                args,
                Command::EditProfile {
                    id: p.id.clone(),
                    name,
                    source,
                },
            )
            .await?;
            output::emit(args, json!({"edited":p.id}))
        }
        "profile remove" => {
            let p = super::profile(&meta, Some(&args.positional[0]))?;
            output::confirm(args, &format!("删除订阅“{}”？", p.name)).await?;
            let _: Value =
                backend::request(args, Command::DeleteProfile { id: p.id.clone() }).await?;
            output::emit(args, json!({"removed":p.id}))
        }
        "profile export" => {
            let p = super::profile(&meta, Some(&args.positional[0]))?;
            let raw = config::read_profile(&backend::store(args)?.profile_path(&p.id)?)?;
            super::maintenance::yaml(args, &raw)
        }
        _ => unreachable!(),
    }
}

fn source_path(args: &Args, source: &str) -> Result<String> {
    if source == "-" {
        let mut raw = String::new();
        std::io::stdin()
            .take(config::PROFILE_LIMIT as u64 + 1)
            .read_to_string(&mut raw)?;
        ensure!(raw.len() <= config::PROFILE_LIMIT, "订阅超过大小限制");
        config::parse_profile(&raw)?;
        // Keep the imported source so later updates remain valid; it is never in /tmp.
        let root = backend::store(args)?.root.join("imports");
        std::fs::create_dir_all(&root)?;
        let path = root.join(format!("{}.yaml", uuid::Uuid::new_v4()));
        config::atomic_write(&path, raw.as_bytes())?;
        return Ok(path.to_string_lossy().into_owned());
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        return Ok(source.into());
    }
    Ok(backend::absolute(source)?
        .canonicalize()
        .context("找不到订阅文件")?
        .to_string_lossy()
        .into_owned())
}
