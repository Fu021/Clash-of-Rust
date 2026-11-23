mod args;
mod backend;
mod commands;
mod diagnostics;
mod host_network;
mod messages {
    use clash_client_core::config::Language;
    include!(concat!(env!("OUT_DIR"), "/messages.rs"));
}
mod output;
mod service;
mod tui;
mod users;

fn main() {
    #[cfg(target_os = "linux")]
    if std::env::args_os()
        .skip(1)
        .eq([std::ffi::OsString::from("__authorize-tun-for-service")])
    {
        let result = clash_client_runtime::platform::authorize_tun_terminal(std::path::Path::new(
            "/opt/clash-of-rust/resources/mihomo",
        ));
        if let Err(error) = result {
            eprintln!("cor: {error:#}");
            std::process::exit(1);
        }
        return;
    }
    let mut completion = std::env::args_os().skip(1);
    if completion.next().as_deref() == Some(std::ffi::OsStr::new("__complete")) {
        use std::io::Write;
        let words = completion
            .map(|word| word.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let _ = std::io::stdout()
            .lock()
            .write_all(args::complete(&words).as_bytes());
        return;
    }
    if clash_client_runtime::update::helper_main() {
        return;
    }
    if let Some(result) = clash_client_runtime::claude_check::system::helper_main() {
        if let Err(error) = result {
            eprintln!("{error:#}");
            diagnostics::failure_hint(None);
            std::process::exit(1);
        }
        return;
    }
    #[cfg(windows)]
    if clash_client_runtime::platform::autostart_helper_main() {
        return;
    }
    let input: Vec<String> = std::env::args().skip(1).collect();
    let language = args::language_from_input(&input);
    let result = args::parse(input).and_then(|args| {
        if args.help {
            use std::io::Write;
            std::io::stdout()
                .lock()
                .write_all(args::localized_help(&args.path, args.language()).as_bytes())?;
            return Ok(());
        }
        users::prepare(&args)?;
        #[cfg(windows)]
        if let Err(error) = clash_client_runtime::platform::notification::prepare_process() {
            eprintln!(
                "{}{error:#}",
                args.language()
                    .text("通知初始化失败：", "Notification initialization failed: ")
            );
            diagnostics::failure_hint(Some(&args));
        }
        anyhow::ensure!(
            !(args.flag("json") && args.flag("jsonl")),
            "{}",
            language.text(
                "--json 与 --jsonl 不能同时指定",
                "--json and --jsonl cannot be combined"
            )
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
                result=backend::interrupted()=>{ result?; Err(Cancelled(language).into()) },
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
            eprintln!("cor: {}", output::localized_error(&error, language));
            if code != 130 {
                diagnostics::failure_hint_in(None, language);
            }
        }
        std::process::exit(code);
    }
}
#[derive(Debug)]
struct Cancelled(clash_client_core::config::Language);
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.text(
            "已取消；后台已提交的操作可通过 cor status 查看",
            "Cancelled; use cor status to check operations already submitted to the backend",
        ))
    }
}
impl std::error::Error for Cancelled {}
