//! The windowless desktop executable delegates login startup to the console.
//! No window, desktop lock or elevated service is initialized on this path.
#[cfg(windows)]
pub fn helper_main() -> Option<anyhow::Result<()>> {
    use anyhow::{Context, ensure};
    use std::ffi::OsStr;
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new("--cli-background")) {
        return None;
    }
    Some((|| {
        let mut forwarded = Vec::new();
        let mut data = None;
        let mut resources = false;
        while let Some(key) = args.next() {
            ensure!(
                key == "--data-dir" || key == "--resources",
                "后台启动参数无效"
            );
            let value = args.next().context("缺少后台路径")?;
            if key == "--data-dir" {
                ensure!(data.is_none(), "重复的数据目录");
                data = Some(std::path::PathBuf::from(&value));
            } else {
                ensure!(!resources, "重复的资源目录");
                resources = true;
            }
            forwarded.extend([key, value]);
        }
        let store = crate::config::Store::at(data.context("缺少后台数据目录")?)?;
        let log = std::fs::File::options()
            .create(true)
            .append(true)
            .open(store.root.join("daemon.log"))?;
        let console = std::env::current_exe()?.with_file_name("cor.exe");
        let mut arguments = vec!["start".into(), "--quiet".into()];
        arguments.extend(forwarded);
        crate::platform::spawn_background_console(&console, &arguments, &log)?;
        Ok(())
    })())
}
