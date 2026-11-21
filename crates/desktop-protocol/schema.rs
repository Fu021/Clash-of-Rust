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
        "int" => "i32".into(),
        "bool" => "bool".into(),
        "float" | "length" => "f32".into(),
        v if v.starts_with('[') => format!("List<{}>", wire_type(&v[1..v.len() - 1])),
        v if v.chars().next().is_some_and(char::is_uppercase) => v.into(),
        _ => panic!("Unsupported desktop contract type: {value}"),
    }
}
fn block<'a>(source: &'a str, declaration: &str) -> &'a str {
    let body = source.split_once(declaration).unwrap().1;
    let mut chars = body.char_indices().peekable();
    let mut depth = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;
    while let Some((index, character)) = chars.next() {
        if line_comment {
            line_comment = character != '\n';
        } else if block_comment {
            if character == '*' && chars.peek().is_some_and(|(_, next)| *next == '/') {
                chars.next();
                block_comment = false;
            }
        } else if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
        } else {
            match character {
                '"' => quoted = true,
                '/' if chars.peek().is_some_and(|(_, next)| *next == '/') => {
                    chars.next();
                    line_comment = true;
                }
                '/' if chars.peek().is_some_and(|(_, next)| *next == '*') => {
                    chars.next();
                    block_comment = true;
                }
                '{' => depth += 1,
                '}' if depth == 0 => return &body[..index],
                '}' => depth -= 1,
                _ => {}
            }
        }
    }
    panic!("Unclosed desktop contract declaration: {declaration}");
}

pub fn read(state_path: &str, models_path: &str) -> Schema {
    let state = fs::read_to_string(state_path).unwrap();
    let models = fs::read_to_string(models_path).unwrap();
    parse(&state, &models)
}

pub fn parse(source: &str, models: &str) -> Schema {
    let body = block(source, "export enum Intent {");
    let intents = body
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let mut structs = Vec::new();
    for part in [source, models]
        .into_iter()
        .flat_map(|declarations| declarations.split("export struct ").skip(1))
    {
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
    let body = block(source, "export global UiData {");
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
