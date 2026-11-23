use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::Path,
};

macro_rules! supported_languages {
    ($($variant:ident => ($tag:literal, $name:literal, $slint:literal, $aliases:expr, $excluded:expr)),+ $(,)?) => {
        const LANGUAGE_TAGS: &[&str] = &[$($tag,)+];
    };
}

include!("languages.rs");

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn parameters(mut text: &str) -> Option<BTreeSet<&str>> {
    let mut result = BTreeSet::new();
    while let Some(offset) = text.find(['{', '}']) {
        text = &text[offset..];
        if text.starts_with("{{") || text.starts_with("}}") {
            text = &text[2..];
        } else {
            let (parameter, rest) = text.strip_prefix('{')?.split_once('}')?;
            if !valid_identifier(parameter) {
                return None;
            }
            result.insert(parameter);
            text = rest;
        }
    }
    Some(result)
}

pub fn compile(root: &Path, catalog: &str, output: &Path) {
    let directory = root.join(catalog);
    println!("cargo:rerun-if-changed={}", directory.display());
    println!(
        "cargo:rerun-if-changed={}",
        root.join("languages.rs").display()
    );
    let mut catalogs = BTreeMap::<String, BTreeMap<String, String>>::new();
    for entry in fs::read_dir(&directory).expect("Read translation directory") {
        let path = entry.expect("Read translation entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let tag = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("Translation locale");
        let source = fs::read_to_string(&path).expect("Read translation catalog");
        let catalog = serde_json::from_str(&source)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        catalogs.insert(tag.to_owned(), catalog);
    }
    let found: BTreeSet<_> = catalogs.keys().map(String::as_str).collect();
    assert_eq!(
        found,
        LANGUAGE_TAGS.iter().copied().collect(),
        "Translation catalogs in {} must match the language registry",
        directory.display()
    );
    let fallback = catalogs.remove("en").expect("English fallback catalog");
    for (tag, catalog) in &catalogs {
        assert!(
            catalog.keys().eq(fallback.keys()),
            "Translation keys differ for {tag}"
        );
    }
    let mut generated = String::new();
    for (key, english) in &fallback {
        assert!(valid_identifier(key), "Invalid translation key: {key}");
        let expected =
            parameters(english).unwrap_or_else(|| panic!("Invalid English template: {key}"));
        write!(generated, "pub fn {key}(language: Language").unwrap();
        for parameter in &expected {
            write!(generated, ", arg_{parameter}: impl std::fmt::Display").unwrap();
        }
        let return_type = if expected.is_empty() {
            "&'static str"
        } else {
            "String"
        };
        writeln!(generated, ") -> {return_type} {{ match language.tag() {{").unwrap();
        for (tag, catalog) in &catalogs {
            let localized = &catalog[key];
            assert_eq!(
                parameters(localized).as_ref(),
                Some(&expected),
                "Translation parameters differ for {tag}: {key}"
            );
            write!(generated, "{tag:?} => ").unwrap();
            emit_message(&mut generated, localized, &expected);
        }
        generated.push_str("_ => ");
        emit_message(&mut generated, english, &expected);
        generated.push_str("} }\n");
    }
    fs::write(output, generated).expect("Write compiled translation catalog");
}

fn emit_message(generated: &mut String, text: &str, parameters: &BTreeSet<&str>) {
    if parameters.is_empty() {
        writeln!(
            generated,
            "{:?},",
            text.replace("{{", "{").replace("}}", "}")
        )
        .unwrap();
    } else {
        write!(generated, "format!({text:?}").unwrap();
        for parameter in parameters {
            write!(generated, ", {parameter}=arg_{parameter}").unwrap();
        }
        generated.push_str("),\n");
    }
}
