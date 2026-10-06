use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=resources/icons/app.ico");
    println!("cargo:rerun-if-env-changed=RC");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let numeric = format!("{},0", version.replace('.', ","));
    let icon = root
        .join("resources/icons/app.ico")
        .display()
        .to_string()
        .replace('\\', "/");
    let source = out.join("app.rc");
    fs::write(
        &source,
        format!(
            r#"
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 1
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904B0"
  BEGIN
   VALUE "CompanyName", "Clash of Rust contributors\0"
   VALUE "FileDescription", "Clash of Rust\0"
   VALUE "FileVersion", "{version}\0"
   VALUE "ProductName", "Clash of Rust\0"
   VALUE "ProductVersion", "{version}\0"
   VALUE "OriginalFilename", "clash-of-rust.exe\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x0409, 1200
 END
END
"#
        ),
    )
    .unwrap();
    let resource = out.join("app.res");
    let compiler = env::var_os("RC").map(PathBuf::from).unwrap_or_else(find_rc);
    let status = Command::new(compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource)
        .arg(&source)
        .status()
        .expect(
            "Windows SDK resource compiler unavailable; install Windows SDK with C++ Build Tools",
        );
    assert!(status.success(), "Windows icon resource compilation failed");
    println!(
        "cargo:rustc-link-arg-bin=clash-of-rust={}",
        resource.display()
    );
}

fn find_rc() -> PathBuf {
    if Command::new("rc.exe").arg("/?").output().is_ok() {
        return "rc.exe".into();
    }
    let sdk = env::var_os("WindowsSdkDir")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                env::var_os("ProgramFiles(x86)").expect("Windows SDK location unavailable"),
            )
            .join("Windows Kits/10")
        });
    let mut versions: Vec<_> = fs::read_dir(sdk.join("bin"))
        .expect("Windows SDK is required")
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("x64/rc.exe"))
        .filter(|path| path.is_file())
        .collect();
    versions.sort();
    versions.pop().expect("Windows SDK rc.exe is required")
}
