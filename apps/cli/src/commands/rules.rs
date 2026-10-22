use super::*;
use clash_client_runtime::{
    control::RuleChange,
    rule_manager::{Document, ProfileRules},
};
const SHORTCUTS: [&str; 5] = ["ads", "private-domain", "private-ip", "cn-domain", "cn-ip"];

pub(super) async fn execute(args: &Args) -> Result<()> {
    let meta = backend::meta(args).await?;
    let p = super::profile(&meta, args.get("profile"))?;
    let id = p.id.clone();
    if args.path == "rule list" || (args.path == "rule shortcut" && args.positional.len() < 2) {
        let doc: Document =
            backend::request(args, Command::RuleDocument { id: id.clone() }).await?;
        let draft = ProfileRules::for_profile(&meta.settings, &id, doc.builtin);
        if args.path == "rule shortcut" {
            let selected = args.positional.first();
            if let Some(name) = selected {
                ensure!(SHORTCUTS.contains(&name.as_str()), "未知快捷规则");
            }
            return output::emit(args,json!(SHORTCUTS.iter().enumerate().filter(|(_,name)|selected.is_none_or(|v|v==*name))
                .map(|(i,name)|json!({"name":name,"enabled":draft.enabled[i],"already_in_subscription":doc.included[i],"rule":clash_client_core::config::SUBSCRIPTION_RULES[i]})).collect::<Vec<_>>()));
        }
        let (rows, warnings) = doc.rows(&draft);
        for warning in warnings {
            if !args.flag("quiet") {
                eprintln!("{warning}");
            }
        }
        let filter = args.get("filter").unwrap_or("").to_lowercase();
        let rows=rows.iter().enumerate().filter(|(_,r)|r.raw.to_lowercase().contains(&filter)||r.id.contains(&filter))
            .map(|(i,r)|json!({"position":i+1,"id":r.id,"rule":r.raw,"source":r.source,"enabled":r.enabled,"duplicate":r.duplicate})).collect();
        return output::emit(args, output::page(args, rows)?);
    }
    let first = &args.positional[0];
    let change = match args.path.as_str() {
        "rule add" => RuleChange::Add {
            raw: first.clone(),
            position: args.number("position", 1, usize::MAX)?,
            enabled: !args.flag("disabled"),
        },
        "rule edit" => RuleChange::Edit {
            rule: first.clone(),
            raw: args.positional[1].clone(),
        },
        "rule remove" => {
            output::confirm(args, "删除这条自定义规则？").await?;
            RuleChange::Remove {
                rule: first.clone(),
            }
        }
        "rule move" => RuleChange::Move {
            rule: first.clone(),
            position: args.positional[1].parse()?,
        },
        "rule enable" => RuleChange::Enable {
            rule: first.clone(),
            enabled: output::on_off(&args.positional[1])?,
        },
        "rule shortcut" => RuleChange::Shortcut {
            index: SHORTCUTS
                .iter()
                .position(|s| s == first)
                .ok_or_else(|| anyhow::anyhow!("未知快捷规则"))?,
            enabled: output::on_off(&args.positional[1])?,
        },
        _ => unreachable!(),
    };
    let _: Value = backend::request(
        args,
        Command::RuleChange {
            id: id.clone(),
            change,
        },
    )
    .await?;
    output::emit(args, json!({"saved":true,"profile":id}))
}
