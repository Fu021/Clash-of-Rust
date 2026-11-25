use crate::{args::Args, output};
use anyhow::Result;
use clash_client_core::config::Language;
use std::io::{self, Write};

macro_rules! supported_languages {
    ($($variant:ident => ($tag:literal, $name:literal, $slint:literal, $aliases:expr, $excluded:expr)),+ $(,)?) => {
        fn markdown(language: Language) -> &'static str {
            match language {
                $(Language::$variant => include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"), "/../../translations/cli/agent-guide/", $tag, ".md"
                )),)+
            }
        }
    };
}

include!("../../../translations/languages.rs");

pub fn print(args: &Args) -> Result<()> {
    let guide = markdown(args.language());
    if output::writes_json(args) {
        return output::emit(args, serde_json::json!({ "markdown": guide }));
    }
    io::stdout().lock().write_all(guide.as_bytes())?;
    Ok(())
}
