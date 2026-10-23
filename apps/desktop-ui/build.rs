#[path = "../../crates/desktop-protocol/schema.rs"]
mod schema;
use schema::{rust_name, variant};
use std::{env, fs, path::PathBuf};
fn mapped_rows(kind: &str) -> String {
    match kind {
        "int" | "bool" | "float" | "length" => "rows.to_vec()".into(),
        "string" => "rows.iter().map(|v|v.as_str().into()).collect::<Vec<_>>()".into(),
        "image" => {
            "rows.iter().map(|v|crate::flags::label(v).1.unwrap_or_default()).collect::<Vec<_>>()"
                .into()
        }
        kind => format!(
            "rows.iter().map(convert_{}).collect::<Vec<_>>()",
            rust_name(kind)
        ),
    }
}
fn convert(kind: &str, value: &str) -> String {
    match kind {
        "string" => format!("{value}.as_str().into()"),
        "image" => format!("crate::flags::label(&{value}).1.unwrap_or_default()"),
        "int" | "bool" | "float" | "length" => value.into(),
        kind if kind.starts_with('[') => {
            let inner = &kind[1..kind.len() - 1];
            let rows = mapped_rows(inner);
            format!("{value}.with_rows(|rows|slint::ModelRc::new(slint::VecModel::from({rows})))")
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
                code += "frame.groups.with_rows(|rows|models.set_groups(rows.iter().map(|g|{let row=convert_GroupItem(g);(row,g.nodes.with_rows(|rows|rows.iter().map(convert_NodePair).collect()))}).collect()));";
            } else {
                let row_type = &kind[1..kind.len() - 1];
                code += &format!(
                    "frame.{field}.with_rows(|rows|ui::models::reconcile(&models.{field},rows.iter().map(convert_{}).collect()));",
                    rust_name(row_type)
                );
            }
        } else if kind.starts_with('[') {
            let inner = &kind[1..kind.len() - 1];
            let rows = mapped_rows(inner);
            code += &format!(
                "frame.{field}.with_rows(|rows|data.set_{field}(list(data.get_{field}(),{rows})));\n"
            );
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
        code.push('\n');
    }
    code += "}\n";
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bridge.rs"),
        code,
    )
    .unwrap();
}
