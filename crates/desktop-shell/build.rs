use std::path::{Path, PathBuf};

#[path = "../../translations/compile_catalog.rs"]
mod compile_catalog;

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    compile_catalog::compile(
        Path::new("../../translations"),
        "desktop/native",
        &out.join("messages.rs"),
    );
}
