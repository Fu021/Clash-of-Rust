mod args;
mod backend;
mod commands;
mod output;
mod service;

fn main() {
    if clash_client_runtime::update::helper_main() {
        return;
    }
    if let Some(result) = clash_client_runtime::claude_check::system::helper_main() {
        if let Err(error) = result {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(windows)]
    if clash_client_runtime::platform::autostart_helper_main() {
        return;
    }
    let result = args::parse(std::env::args().skip(1)).and_then(|args| {
        if args.help {
            use std::io::Write;
            std::io::stdout()
                .lock()
                .write_all(args::help(&args.path).as_bytes())?;
            return Ok(());
        }
        #[cfg(windows)]
        if let Err(error) = clash_client_runtime::platform::notification::prepare_process() {
            eprintln!("通知初始化失败：{error:#}");
        }
        anyhow::ensure!(
            !(args.flag("json") && args.flag("jsonl")),
            "--json 与 --jsonl 不能同时指定"
        );
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        runtime.block_on(async {
            if args.path == "run" {
                return commands::execute(&args).await;
            }
            tokio::select! {
                result=commands::execute(&args)=>result,
                result=backend::interrupted()=>{ result?; Err(Cancelled.into()) },
            }
        })
    });
    if let Err(error) = result {
        let code = if error.is::<args::Usage>() {
            2
        } else if error.is::<Cancelled>() {
            130
        } else if error.is::<backend::Unavailable>() {
            3
        } else if error.is::<tokio::time::error::Elapsed>()
            || error.chain().any(|e| {
                e.downcast_ref::<std::io::Error>().is_some_and(|e| {
                    matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    )
                })
            })
        {
            124
        } else if error.chain().any(|e| {
            e.downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::BrokenPipe)
        }) {
            0
        } else {
            1
        };
        if code != 0 {
            eprintln!("cor: {error:#}");
        }
        std::process::exit(code);
    }
}
#[derive(Debug)]
struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("已取消；后台已提交的操作可通过 cor status 查看")
    }
}
impl std::error::Error for Cancelled {}
