mod input;
mod model;
mod render;
mod terminal;
#[cfg(test)]
mod tests;

use crate::{args::Args, backend};
use anyhow::{Context, Result, ensure};
use clash_client_runtime::{
    control::{Command, Poll},
    engine::Scope,
};
use input::Key;
use model::{Action, Page, State, Target};
use std::{io::IsTerminal, time::Duration};
use tokio::task::JoinHandle;

enum Reply {
    Snapshot(Option<Box<Poll>>),
    Network(Box<Result<clash_client_core::probe::NetworkReport, String>>),
}

struct Request(JoinHandle<Result<Reply>>);

impl Drop for Request {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn read(args: &Args, page: Page) -> Result<Option<Poll>> {
    tokio::time::timeout(Duration::from_secs(4), async {
        let Some(remote) = backend::connect(args, false).await? else {
            return Ok(None);
        };
        remote
            .request(Command::Snapshot {
                scope: page.scope(),
                header: true,
            })
            .await
            .map(Some)
    })
    .await
    .context("读取后台超时；按 r 重试，或退出后导出诊断")?
}

async fn select(args: &Args, target: Target) -> Result<Option<Poll>> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let remote = backend::connect(args, false)
            .await?
            .context("后台已退出，未切换节点")?;
        let current: Poll = remote
            .request(Command::Snapshot {
                scope: Scope::Proxies,
                header: true,
            })
            .await?;
        target.validate(&current)?;
        let _: serde_json::Value = remote
            .request(Command::Select {
                group: target.group,
                node: target.node,
            })
            .await?;
        remote
            .request(Command::Snapshot {
                scope: Scope::Proxies,
                header: true,
            })
            .await
            .map(Some)
    })
    .await
    .context("节点切换等待超时；实际选择以刷新结果为准")?
}

pub async fn run(args: &Args) -> Result<()> {
    ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "cor ui 需要交互终端；脚本请使用 cor --json status"
    );
    let mut terminal = terminal::Terminal::enter(!args.flag("no-color"))?;
    let mut state = State::new(args.flag("demo"));
    let mut input = input::Decoder::default();
    let mut request: Option<Request> = None;
    let mut last_refresh = tokio::time::Instant::now() - Duration::from_secs(5);
    let mut dirty = true;
    let mut previous_size = (0, 0);
    loop {
        let size = terminal.size()?;
        if size != previous_size {
            previous_size = size;
            dirty = true;
        }
        let keys = input.feed(&terminal.read()?, std::time::Instant::now());
        for key in keys {
            if key == Key::Interrupt || (key == Key::Char('q') && !state.searching) {
                return Ok(());
            }
            if request.is_some()
                && state.saving
                && !(state.page == Page::Network && matches!(key, Key::Tab | Key::Char('1'..='4')))
            {
                continue;
            }
            let page = state.page;
            match state.key(key) {
                Action::None => {}
                Action::Refresh => last_refresh -= Duration::from_secs(5),
                Action::Select(target) => {
                    let args = args.clone();
                    state.saving = true;
                    request = Some(Request(tokio::spawn(async move {
                        select(&args, target)
                            .await
                            .map(|poll| Reply::Snapshot(poll.map(Box::new)))
                    })));
                }
                Action::CheckNetwork(target) => {
                    let args = args.clone();
                    state.saving = true;
                    request = Some(Request(tokio::spawn(async move {
                        Ok(Reply::Network(Box::new(
                            crate::commands::inspection::network_check(&args, &target)
                                .await
                                .map_err(|error| format!("{error:#}")),
                        )))
                    })));
                }
            }
            if state.page != page {
                request = None;
                state.saving = false;
                state.release_page();
                last_refresh -= Duration::from_secs(5);
            }
            dirty = true;
        }
        if request.as_ref().is_some_and(|job| job.0.is_finished()) {
            let mut completed = request.take().unwrap();
            match (&mut completed.0).await {
                Ok(Ok(Reply::Snapshot(poll))) => state.accept(poll.map(|poll| *poll)),
                Ok(Ok(Reply::Network(result))) => {
                    state.network_results = match *result {
                        Ok(report) => report.lines(),
                        Err(error) => vec![format!("检查失败 · {error}")],
                    };
                }
                Ok(Err(error)) => state.failure(format!("{error:#}")),
                Err(error) => state.failure(format!("读取任务失败：{error}")),
            }
            state.saving = false;
            last_refresh = tokio::time::Instant::now();
            dirty = true;
        }
        if !state.demo && request.is_none() && last_refresh.elapsed() >= Duration::from_secs(1) {
            let args = args.clone();
            let page = state.page;
            request = Some(Request(tokio::spawn(async move {
                read(&args, page)
                    .await
                    .map(|poll| Reply::Snapshot(poll.map(Box::new)))
            })));
            last_refresh = tokio::time::Instant::now();
        }
        if dirty {
            terminal.draw(&render::frame(&state, size.0, size.1))?;
            dirty = false;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}
