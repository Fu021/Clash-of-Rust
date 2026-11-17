fn main() {
    let config = slint_build::CompilerConfiguration::new().with_edition((1, 18));
    slint_build::compile_with_config("ui/app.slint", config).expect("Compile the desktop UI");
}
