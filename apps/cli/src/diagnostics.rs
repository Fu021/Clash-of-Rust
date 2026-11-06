use crate::{args::Args, backend};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::PathBuf;

pub(crate) use clash_client_core::support::has_failure;

pub(crate) fn failure_hint(args: Option<&Args>) {
    static SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if SHOWN.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if args.is_some_and(|args| args.path == "diagnose" && args.get("output").is_some()) {
        eprintln!("诊断含失败项目。提交 issue 时附上已保存的报告、操作步骤和发生时间。");
    } else {
        eprintln!("诊断报告：cor diagnose --output cor-diagnose.json");
        eprintln!("后台不可达时：cor diagnose --offline --output cor-diagnose.json");
        eprintln!(
            "沿用原用户及 --user / --data-dir / --resources 参数。提交 issue 时附上报告、操作步骤和发生时间。"
        );
    }
    eprintln!("Issue：https://github.com/Fu021/Clash-of-Rust/issues；分享前检查报告中的敏感信息。");
}

pub(crate) fn core_logs(lines: Vec<String>, meta: &clash_client_runtime::control::Meta) -> Value {
    match serde_json::to_value(meta) {
        Ok(credentials) => clash_client_core::support::core_logs(lines, &credentials),
        Err(_) => json!({"state":"withheld","reason":"credential_redaction_unavailable"}),
    }
}

pub(crate) fn validate_output(args: &Args) -> Result<()> {
    if let Some(path) = args.get("output") {
        let output_path = backend::absolute(path)?;
        let parent = output_path
            .parent()
            .context("无效报告路径")?
            .canonicalize()
            .context("诊断报告的父目录必须已经存在")?;
        let root = backend::data_root(args)?;
        if let Ok(root) = root.canonicalize() {
            ensure!(
                !parent.starts_with(root),
                "诊断报告须保存到应用数据目录之外，避免覆盖排障现场"
            );
        }
    }
    Ok(())
}

pub(crate) async fn collect(args: &Args) -> Result<Value> {
    let root = backend::data_root(args)?;
    let explicit_resources = args.get("resources").map(backend::absolute).transpose()?;
    tokio::task::spawn_blocking(move || {
        let resources = explicit_resources
            .map(Ok)
            .unwrap_or_else(clash_client_runtime::assets::discover);
        let mut report = clash_client_core::support::collect(&root);
        report["resources"] = json!(
            resources
                .map(|path: PathBuf| path.to_string_lossy().into_owned())
                .map_err(|error| format!("{error:#}"))
        );
        report
    })
    .await
    .context("本地诊断采集任务失败")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_cannot_replace_files_in_the_diagnosed_data_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let runtime = temporary.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let target = runtime.join("geo-transaction.json");
        std::fs::write(&target, b"preserve-this-evidence").unwrap();
        let args = crate::args::parse(vec![
            "diagnose".into(),
            "--offline".into(),
            "--data-dir".into(),
            temporary.path().to_string_lossy().into_owned(),
            "--output".into(),
            target.to_string_lossy().into_owned(),
        ])
        .unwrap();
        assert!(validate_output(&args).is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"preserve-this-evidence");
    }
}
