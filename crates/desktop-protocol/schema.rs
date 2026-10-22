//! Both sides derive their contract from the production Slint declaration.
//! Unsupported schema changes fail the build instead of silently losing a UI binding.
use std::fs;

pub struct Schema {
    pub intents: Vec<String>,
    pub structs: Vec<(String, Vec<(String, String)>)>,
    pub properties: Vec<(String, String, String)>,
}
pub fn rust_name(value: &str) -> String {
    value.replace('-', "_")
}
pub fn variant(value: &str) -> String {
    value
        .split('-')
        .map(|s| {
            let mut chars = s.chars();
            chars.next().unwrap().to_uppercase().to_string() + chars.as_str()
        })
        .collect()
}
pub fn wire_type(value: &str) -> String {
    match value {
        "string" => "Text".into(),
        "image" => "String".into(),
        "int" => "i32".into(),
        "bool" => "bool".into(),
        "float" | "length" => "f32".into(),
        v if v.starts_with('[') => format!("List<{}>", wire_type(&v[1..v.len() - 1])),
        v if v.chars().next().is_some_and(char::is_uppercase) => v.into(),
        _ => panic!("Unsupported desktop contract type: {value}"),
    }
}
pub fn read(path: &str) -> Schema {
    let source = fs::read_to_string(path).unwrap();
    let body = source
        .split_once("export enum Intent {")
        .unwrap()
        .1
        .split_once('}')
        .unwrap()
        .0;
    let intents = body
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let mut structs = Vec::new();
    for part in source.split("export struct ").skip(1) {
        let (name, body) = part.split_once('{').unwrap();
        let fields = body
            .split_once('}')
            .unwrap()
            .0
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                let (name, kind) = s.split_once(':').unwrap();
                (name.trim().into(), kind.trim().into())
            })
            .collect();
        structs.push((name.trim().into(), fields));
    }
    let body = source
        .split_once("export global UiData {")
        .unwrap()
        .1
        .split_once('}')
        .unwrap()
        .0;
    let properties = body
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("in-out property <")?;
            let (kind, rest) = rest.split_once('>').unwrap();
            let (name, default) = rest
                .trim()
                .trim_end_matches(';')
                .split_once(':')
                .unwrap_or((rest.trim().trim_end_matches(';'), ""));
            Some((name.trim().into(), kind.into(), default.trim().into()))
        })
        .collect();
    Schema {
        intents,
        structs,
        properties,
    }
}
