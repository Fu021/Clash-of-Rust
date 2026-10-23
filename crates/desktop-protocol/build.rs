mod schema;
use schema::{rust_name, variant, wire_type};
use std::{env, fs, path::PathBuf};
fn main() {
    let path = "../client-ui/ui/state.slint";
    println!("cargo:rerun-if-changed={path}");
    let schema = schema::read(path);
    let derive = "#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]\n";
    let mut code = format!("{derive}#[derive(Copy)] pub enum Intent {{");
    for intent in schema.intents {
        code += &format!("{},", variant(&intent));
    }
    code += "}\n";
    for (name, fields) in schema.structs {
        code += &format!("{derive}#[derive(Default)] pub struct {name} {{");
        for (name, kind) in fields {
            code += &format!("pub {}: {},", rust_name(&name), wire_type(&kind));
        }
        code += "}\n";
    }
    code += &format!("{derive}pub struct Frame {{");
    for (name, kind, _) in &schema.properties {
        code += &format!("pub {}: {},", rust_name(name), wire_type(kind));
    }
    code += "}\nimpl Default for Frame {fn default()->Self{Self{";
    for (name, kind, default) in &schema.properties {
        let value = if default.is_empty() {
            "Default::default()".into()
        } else if kind == "string" {
            format!("{default}.into()")
        } else if matches!(kind.as_str(), "length" | "float") {
            format!("{default}f32")
        } else {
            default.clone()
        };
        code += &format!("{}:{value},", rust_name(name));
    }
    code += "}}}\n#[derive(Default)]pub struct UiData {data:std::cell::RefCell<Frame>}\nimpl UiData {pub fn snapshot(&self)->Frame {self.data.borrow().clone()}";
    for (name, kind, _) in schema.properties {
        let name = rust_name(&name);
        let copy = matches!(kind.as_str(), "int" | "bool" | "float" | "length");
        let kind = wire_type(&kind);
        let value = if copy {
            format!("self.data.borrow().{name}")
        } else {
            format!("self.data.borrow().{name}.clone()")
        };
        code += &format!(
            "pub fn get_{name}(&self)->{kind}{{{value}}}pub fn set_{name}(&self,value:{kind}){{self.data.borrow_mut().{name}=value;}}"
        );
    }
    code += "}\n";
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("contract.rs"),
        code,
    )
    .unwrap();
}
