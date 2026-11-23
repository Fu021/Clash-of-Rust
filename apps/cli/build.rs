#[path = "../../translations/compile_catalog.rs"]
mod compile_catalog;

fn main() {
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    compile_catalog::compile(
        std::path::Path::new("../../translations/cli"),
        &output.join("messages.rs"),
    );
}
