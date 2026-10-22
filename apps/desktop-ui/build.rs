#[path = "../../crates/desktop-protocol/schema.rs"]
mod schema;
use schema::{rust_name, variant};
use std::{env, fs, path::PathBuf};
fn convert(kind: &str, value: &str) -> String {
    match kind {
        "string" => format!("{value}.as_str().into()"),
        "image" => format!("crate::flags::label(&{value}).1.unwrap_or_default()"),
        "int" | "bool" | "float" | "length" => value.into(),
        kind if kind.starts_with('[') => {
            let inner = &kind[1..kind.len() - 1];
            let rows = if matches!(inner, "int" | "bool" | "float" | "length") {
                format!("{value}.rows()")
            } else {
                format!(
                    "{value}.rows().into_iter().map(|v|{}).collect::<Vec<_>>()",
                    convert(inner, "v")
                )
            };
            format!("slint::ModelRc::new(slint::VecModel::from({rows}))")
        }
        kind => format!("convert_{}(&{value})", rust_name(kind)),
    }
}
fn main() {
    let path = "../../crates/client-ui/ui/state.slint";
    println!("cargo:rerun-if-changed={path}");
    let schema = schema::read(path);
    let mut code = String::new();
    for (struct_name, fields) in &schema.structs {
        code += &format!(
            "#[allow(non_snake_case)]fn convert_{}(value:&wire::{struct_name})->ui::{struct_name} {{ui::{struct_name}{{",
            rust_name(struct_name)
        );
        for (name, kind) in fields {
            let field = rust_name(name);
            let expression = format!("value.{field}");
            let converted = if struct_name == "GroupItem" && field == "nodes" {
                "Default::default()".into()
            } else {
                convert(kind, &expression)
            };
            code += &format!("{field}:{converted},");
        }
        code += "}}\n";
    }
    code += "pub fn intent(value:ui::Intent)->wire::Intent {match value{";
    for name in &schema.intents {
        let name = variant(name);
        code += &format!("ui::Intent::{name}=>wire::Intent::{name},");
    }
    code += "}}\npub fn apply(frame:&wire::Frame,data:&ui::UiData<'_>,models:&mut ui::models::ViewModels){";
    let roots = [
        "groups",
        "profiles",
        "connections",
        "rules",
        "logs",
        "categories",
        "site-results",
        "claude-results",
    ];
    for (name, kind, _) in &schema.properties {
        let _validated = schema::wire_type(kind);
        if name.starts_with("scroll-") {
            continue;
        }
        let field = rust_name(name);
        if roots.contains(&name.as_str()) {
            if name == "groups" {
                code += "models.set_groups(frame.groups.rows().iter().map(|g|{let row=convert_GroupItem(g);(row,g.nodes.rows().iter().map(convert_NodePair).collect())}).collect());";
            } else {
                let row_type = &kind[1..kind.len() - 1];
                code += &format!(
                    "ui::models::reconcile(&models.{field},frame.{field}.rows().iter().map(convert_{}).collect());",
                    rust_name(row_type)
                );
            }
        } else if kind.starts_with('[') {
            let inner = &kind[1..kind.len() - 1];
            let rows = if matches!(inner, "int" | "bool" | "float" | "length") {
                format!("frame.{field}.rows()")
            } else {
                let expression = convert(inner, "v");
                format!("frame.{field}.rows().into_iter().map(|v|{expression}).collect()")
            };
            code += &format!("data.set_{field}(list(data.get_{field}(),{rows}));");
        } else if kind == "string" {
            code += &format!(
                "if data.get_{field}().as_str()!=frame.{field}.as_str(){{data.set_{field}(frame.{field}.as_str().into());}}"
            );
        } else if kind == "image" {
            code += &format!(
                "data.set_{field}({});",
                convert(kind, &format!("frame.{field}"))
            );
        } else {
            code += &format!(
                "if data.get_{field}()!=frame.{field}{{data.set_{field}(frame.{field});}}"
            );
        }
    }
    code += "}\n";
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bridge.rs"),
        code,
    )
    .unwrap();
}
