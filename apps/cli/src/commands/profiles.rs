use super::*;
use anyhow::Context;
use clash_client_core::config;
use std::io::Read;

pub(super) async fn execute(args: &Args) -> Result<()> {
    let meta = backend::meta(args).await?;
    match args.path.as_str() {
        "profile list" => {
            let filter = args.get("filter").unwrap_or("").to_lowercase();
            let rows: Vec<_> = meta
                .profiles
                .iter()
                .filter(|profile| output::profile_list_matches(args, profile, &filter))
                .map(|profile| {
                    let mut row = json!({
                        "id": profile.id,
                        "name": output::profile_list_name(args, profile),
                        "active": meta.settings.active_profile.as_deref() == Some(&profile.id),
                        "source": profile.source,
                        "updated": profile.updated,
                        "usage": profile.usage,
                    });
                    if !output::writes_json(args) && !profile.is_default() && !profile.is_local() {
                        row["show_usage"] = Value::Bool(true);
                    }
                    row
                })
                .collect();
            output::emit(args, json!(rows))
        }
        "profile directory" => {
            output::emit(args, json!(backend::store(args)?.root.join("profiles")))
        }
        "profile add" | "profile import" => {
            let (name, source) = if args.path == "profile add" {
                (&*args.positional[0], &*args.positional[1])
            } else {
                (
                    args.get("name").context(args.language().text(
                        "导入文件时请提供 --name",
                        "Pass --name when importing a file",
                    ))?,
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
                    expect_current: false,
                },
            )
            .await?;
            output::emit(args, json!({"imported":name}))
        }
        "profile update" => {
            ensure!(
                !(args.flag("all") && !args.positional.is_empty()),
                "{}",
                args.language().text(
                    "PROFILE 与 --all 不能同时指定",
                    "PROFILE and --all cannot be used together"
                )
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
                    args.language(),
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
                        expect_current: true,
                    },
                )
                .await;
                let error = result.err().map(|e| format!("{e:#}"));
                failed |= error.is_some();
                results
                    .push(json!({"id":p.id,"name":p.name,"updated":error.is_none(),"error":error}));
            }
            output::emit(args, json!(results))?;
            ensure!(
                !failed,
                "{}",
                args.language().text(
                    "部分订阅更新失败",
                    "Some subscriptions could not be updated"
                )
            );
            Ok(())
        }
        "profile use" => {
            let p = super::profile(&meta, Some(&args.positional[0]), args.language())?;
            let _: Value = backend::request(args, Command::Activate { id: p.id.clone() }).await?;
            output::emit(args, json!({"active":p.id,"name":p.name}))
        }
        "profile edit" => {
            ensure!(
                args.get("name").is_some() || args.get("source").is_some(),
                "{}",
                args.language()
                    .text("请提供 --name 或 --source", "Pass --name or --source")
            );
            let p = super::profile(&meta, Some(&args.positional[0]), args.language())?;
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
            let p = super::profile(&meta, Some(&args.positional[0]), args.language())?;
            let prompt = match args.language() {
                clash_client_core::config::Language::SimplifiedChinese => {
                    format!("删除订阅“{}”？", p.name)
                }
                clash_client_core::config::Language::English => {
                    format!("Delete subscription “{}”?", p.name)
                }
            };
            output::confirm(args, &prompt).await?;
            let _: Value =
                backend::request(args, Command::DeleteProfile { id: p.id.clone() }).await?;
            output::emit(args, json!({"removed":p.id}))
        }
        "profile export" => {
            let p = super::profile(&meta, Some(&args.positional[0]), args.language())?;
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
        ensure!(
            raw.len() <= config::PROFILE_LIMIT,
            "{}",
            args.language()
                .text("订阅超过大小限制", "Subscription exceeds the size limit")
        );
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
        .context(
            args.language()
                .text("找不到订阅文件", "Subscription file not found"),
        )?
        .to_string_lossy()
        .into_owned())
}
