mod catalog;
mod fake_ip;
mod form;
mod home;
mod input;
mod model;
mod network;
mod panels;
mod render;
mod rules;
mod settings;
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
    Snapshot(Option<Box<Poll>>, Option<catalog::Shortcuts>),
    Saved(Box<Poll>, Option<catalog::Shortcuts>),
    Network(Box<Result<clash_client_core::network_tools::Report, String>>),
    Exported(Result<String, String>),
}

struct Request<T = Reply>(JoinHandle<Result<T>>);

impl<T> Drop for Request<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn check_update(
    state: &mut State,
) -> Result<Request<Option<clash_client_runtime::update::Available>>> {
    let (ports, channel) = home::begin_check(state, std::time::Instant::now())?;
    Ok(Request(tokio::spawn(async move {
        clash_client_runtime::update::check_with_prereleases(ports, channel).await
    })))
}

async fn read(
    args: &Args,
    page: Page,
    cached: Option<catalog::Shortcuts>,
    include_rules: bool,
) -> Result<Reply> {
    tokio::time::timeout(Duration::from_secs(4), async {
        let Some((remote, poll)) = backend::observed_snapshot(args, page.scope()).await? else {
            return Ok(Reply::Snapshot(None, None));
        };
        let shortcuts = if page == Page::Rules {
            catalog::read_shortcuts(&remote, &poll, cached, include_rules).await
        } else {
            None
        };
        Ok(Reply::Snapshot(Some(Box::new(poll)), shortcuts))
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

async fn set_node_sort(
    args: &Args,
    mode: clash_client_core::proxy_order::NodeSort,
) -> Result<Option<Poll>> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let remote = backend::connect(args, false)
            .await?
            .context("后台已退出，未保存节点排序")?;
        let _: serde_json::Value = remote
            .request(Command::ConfigSet {
                key: "tui_node_sort".into(),
                value: serde_json::to_value(mode)?,
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
    .context("保存节点排序等待超时；实际状态以刷新结果为准")?
}

async fn save_setting(context: &Args, operation: &Args) -> Result<Reply> {
    let command = settings::command(operation)?;
    save_command(context, command, operation.path == "rule shortcut", false).await
}

async fn save_command(
    context: &Args,
    command: Command,
    shortcut: bool,
    include_rules: bool,
) -> Result<Reply> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let remote = backend::connect(context, false)
            .await?
            .context("后台已退出，未保存更改")?;
        let _: serde_json::Value = remote.request(command).await.map_err(|error| {
            if shortcut
                && error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|cause| cause.kind() == std::io::ErrorKind::UnexpectedEof)
            {
                error.context("后台已断开；若仍为旧版，请退出旧版 GUI／cor 后台后重试")
            } else {
                error
            }
        })?;
        let poll: Poll = remote
            .request(Command::Snapshot {
                scope: Scope::Other,
                header: true,
            })
            .await?;
        let shortcuts = if shortcut {
            catalog::read_shortcuts(&remote, &poll, None, include_rules).await
        } else {
            None
        };
        Ok(Reply::Saved(Box::new(poll), shortcuts))
    })
    .await
    .context("保存或读取结果超时；实际状态以刷新结果为准，草稿仍保留")?
}

pub async fn run(args: &Args) -> Result<()> {
    ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "cor ui 需要交互终端；脚本请使用 cor --json status"
    );
    let mut terminal = Some(terminal::Terminal::enter(!args.flag("no-color"))?);
    let mut state = State::new(args.flag("demo"));
    let mut input = input::Decoder::default();
    let mut request: Option<Request> = None;
    let mut update_request: Option<Request<Option<clash_client_runtime::update::Available>>> = None;
    let mut last_refresh = tokio::time::Instant::now() - Duration::from_secs(5);
    let mut dirty = true;
    let mut previous_size = (0, 0);
    loop {
        let size = terminal.as_ref().unwrap().size()?;
        if size != previous_size {
            if state.page == Page::Network && previous_size != (0, 0) {
                network::resize(
                    &mut state,
                    render::content_width(previous_size.0, previous_size.1),
                    render::content_width(size.0, size.1),
                );
            }
            previous_size = size;
            dirty = true;
        }
        let keys = input.feed(
            &terminal.as_mut().unwrap().read()?,
            std::time::Instant::now(),
        );
        for key in keys {
            if key == Key::Interrupt || (key == Key::Char('q') && !state.entering_text()) {
                return Ok(());
            }
            if request.is_some()
                && state.saving
                && !(state.page == Page::Network
                    && state.form.is_none()
                    && matches!(
                        key,
                        Key::Tab
                            | Key::Escape
                            | Key::Left
                            | Key::Right
                            | Key::Char('1'..='8' | '[' | ']' | 'o')
                    ))
            {
                continue;
            }
            let page = state.page;
            match state.key(key) {
                Action::None => {}
                Action::ReviewForm => {
                    if state.form.as_ref().and_then(|form| form.network)
                        == Some(network::Editor::Export)
                    {
                        match network::prepare_export(&state) {
                            Ok(export) => {
                                state.saving = true;
                                state.message = None;
                                request = Some(Request(tokio::spawn(async move {
                                    Ok(Reply::Exported(
                                        tokio::task::spawn_blocking(move || export.save())
                                            .await?
                                            .map_err(|error| format!("{error:#}")),
                                    ))
                                })));
                            }
                            Err(error) => state.message = Some(format!("{error:#}")),
                        }
                    } else if state
                        .form
                        .as_ref()
                        .is_some_and(|form| form.network.is_some())
                    {
                        if let Err(error) = network::apply(&mut state) {
                            state.message = Some(format!("{error:#}"));
                        }
                    } else if let Some(form) = &state.form {
                        match form.args(args) {
                            Ok(operation) => {
                                state.execution = Some(operation);
                                state.message = None;
                            }
                            Err(error) => state.message = Some(format!("{error:#}")),
                        }
                    }
                }
                Action::Execute(operation) => {
                    request = None;
                    update_request = None;
                    state.updates.checking = false;
                    state.saving = false;
                    drop(terminal.take());
                    eprintln!("\ncor {} · 使用相同账户和数据目录\n", operation.path);
                    let result = Box::pin(crate::commands::execute(&operation)).await;
                    if operation.path == "update install" {
                        eprintln!(
                            "\n已退出 TUI 以释放程序文件；安装结果请用 cor update status 查看"
                        );
                        return result;
                    }
                    match result {
                        Ok(()) => eprintln!("\n命令已完成"),
                        Err(error) => {
                            eprintln!("\n操作失败：{error:#}");
                            crate::diagnostics::failure_hint(Some(&operation));
                        }
                    }
                    crate::output::read_line("按 Enter 返回 TUI；Ctrl+C 退出：").await?;
                    terminal = Some(terminal::Terminal::enter(!args.flag("no-color"))?);
                    input = input::Decoder::default();
                    state.form = None;
                    state.execution = None;
                    state.release_page();
                    last_refresh -= Duration::from_secs(5);
                    dirty = true;
                    break;
                }
                Action::ApplySetting(operation) => {
                    let args = args.clone();
                    state.saving = true;
                    state.message = None;
                    request = Some(Request(tokio::spawn(async move {
                        save_setting(&args, &operation).await
                    })));
                }
                Action::SaveFakeIp(command) => {
                    let args = args.clone();
                    state.saving = true;
                    state.message = None;
                    request = Some(Request(tokio::spawn(async move {
                        save_command(&args, *command, false, false).await
                    })));
                }
                Action::SaveRules(command) => {
                    let args = args.clone();
                    state.saving = true;
                    request = Some(Request(tokio::spawn(async move {
                        save_command(&args, *command, true, true).await
                    })));
                }
                Action::ScrollDetail(down) => {
                    let width = render::content_width(size.0, size.1);
                    let limit = panels::connection_rows(&state, width)
                        .len()
                        .saturating_sub(1);
                    state.detail_offset = if down {
                        state.detail_offset.saturating_add(1).min(limit)
                    } else {
                        state.detail_offset.min(limit).saturating_sub(1)
                    };
                }
                Action::ScrollNetwork(down) => {
                    network::scroll(&mut state, render::content_width(size.0, size.1), down);
                }
                Action::Refresh => last_refresh -= Duration::from_secs(5),
                Action::Select(target) => {
                    let args = args.clone();
                    state.saving = true;
                    request = Some(Request(tokio::spawn(async move {
                        select(&args, target)
                            .await
                            .map(|poll| Reply::Snapshot(poll.map(Box::new), None))
                    })));
                }
                Action::SetNodeSort(mode) => {
                    let args = args.clone();
                    state.saving = true;
                    request = Some(Request(tokio::spawn(async move {
                        set_node_sort(&args, mode)
                            .await
                            .map(|poll| Reply::Snapshot(poll.map(Box::new), None))
                    })));
                }
                Action::CancelNetwork => {
                    request = None;
                    state.saving = false;
                }
                Action::CheckNetwork(tool, target, options) => {
                    let args = args.clone();
                    state.saving = true;
                    request = Some(Request(tokio::spawn(async move {
                        Ok(Reply::Network(Box::new(
                            crate::commands::inspection::network_tool(
                                &args, tool, &target, options,
                            )
                            .await
                            .map_err(|error| format!("{error:#}")),
                        )))
                    })));
                }
                Action::CheckUpdate => match check_update(&mut state) {
                    Ok(job) => update_request = Some(job),
                    Err(error) => state.message = Some(format!("{error:#}")),
                },
                Action::InstallUpdate(available) => {
                    drop(request.take());
                    drop(update_request.take());
                    drop(terminal.take());
                    drop(state);
                    let operation = form::Form::new(
                        form::commands()
                            .find(|spec| spec.path == "update install")
                            .unwrap(),
                    )
                    .args(args)?;
                    let result =
                        crate::commands::install_selected_update(&operation, *available).await;
                    eprintln!("\n已退出 TUI；安装结果请用 cor update status 查看");
                    return result;
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
                Ok(Ok(Reply::Snapshot(poll, shortcuts))) => {
                    state.accept(poll.map(|poll| *poll));
                    state.shortcuts = shortcuts;
                    rules::sync(&mut state);
                }
                Ok(Ok(Reply::Saved(poll, shortcuts))) => {
                    let fake_ip = state.fake_ip.is_some();
                    let rule_selection = state
                        .rules
                        .as_ref()
                        .and_then(|rules| rules.menu.as_ref().map(|(id, _)| id.clone()));
                    let added_rule = state
                        .rules
                        .as_ref()
                        .and_then(|rules| rules.input.as_ref())
                        .filter(|input| input.id.is_none())
                        .map(|input| input.raw.clone());
                    let key = state.form.as_ref().and_then(|form| form.setting.clone());
                    let message = key.map(|key| {
                        format!(
                            "已保存 · {}：{}",
                            settings::title(&key),
                            settings::display(&poll, &key)
                        )
                    });
                    state.accept(Some(*poll));
                    state.shortcuts = shortcuts;
                    let saved_rules = state.rules.as_ref().is_some_and(rules::Browser::busy)
                        && !state.shortcuts_open;
                    if saved_rules {
                        state.rules = None;
                    }
                    rules::sync(&mut state);
                    if saved_rules {
                        let selected = rule_selection.or_else(|| {
                            state
                                .rules
                                .as_ref()?
                                .rows
                                .iter()
                                .find(|row| {
                                    row.source == clash_client_core::rule_manager::Source::Custom
                                        && row.enabled
                                        && added_rule.as_deref() == Some(row.raw.as_ref())
                                })
                                .map(|row| row.id.clone())
                        });
                        if let Some(index) = selected.and_then(|id| {
                            state.items().iter().position(|candidate| *candidate == id)
                        }) {
                            state.selected = index;
                        }
                    }
                    state.form = None;
                    state.execution = None;
                    if fake_ip {
                        if let Some(editor) = &mut state.fake_ip
                            && editor.saving_addition
                        {
                            editor.draft.clear();
                            editor.saving_addition = false;
                        }
                        state.message = Some("已保存 Fake-IP 排除项".into());
                    } else if saved_rules {
                        state.message = Some("已保存规则".into());
                    } else {
                        state.message = message;
                    }
                }
                Ok(Ok(Reply::Network(result))) => match *result {
                    Ok(report) => {
                        state.network_results = report.lines();
                        state.network_report = Some(report);
                    }
                    Err(error) => {
                        state.network_results = vec![format!("检查失败 · {error}")];
                        state.network_report = None;
                    }
                },
                Ok(Ok(Reply::Exported(result))) => match result {
                    Ok(path) => {
                        state.form = None;
                        state.message = Some(format!("已导出：{path}"));
                    }
                    Err(error) => {
                        state.message = Some(format!("导出失败，路径和报告保留：{error}"));
                    }
                },
                Ok(Err(error)) => {
                    if state.saving
                        && (state.fake_ip.is_some()
                            || state.rules.as_ref().is_some_and(rules::Browser::busy)
                            || state
                                .form
                                .as_ref()
                                .is_some_and(|form| form.setting.is_some()))
                    {
                        state.message = Some(format!("保存失败或结果未确认，草稿保留：{error:#}"));
                    }
                    state.failure(format!("{error:#}"));
                }
                Err(error) => state.failure(format!("读取任务失败：{error}")),
            }
            state.saving = false;
            last_refresh = tokio::time::Instant::now();
            dirty = true;
        }
        if home::sync(&mut state) {
            update_request = None;
            dirty = true;
        }
        if update_request
            .as_ref()
            .is_some_and(|job| job.0.is_finished())
        {
            let mut completed = update_request.take().unwrap();
            let result = match (&mut completed.0).await {
                Ok(result) => result.map_err(|error| format!("{error:#}")),
                Err(error) => Err(format!("更新检查任务失败：{error}")),
            };
            home::accept(&mut state, result);
            dirty = true;
        }
        if update_request.is_none() && home::due(&mut state, std::time::Instant::now()) {
            match check_update(&mut state) {
                Ok(job) => update_request = Some(job),
                Err(error) => state.updates.result = Some(Err(format!("{error:#}"))),
            }
            dirty = true;
        }
        if !state.demo && request.is_none() && last_refresh.elapsed() >= Duration::from_secs(1) {
            let args = args.clone();
            let page = state.page;
            let cached = state.shortcuts.clone();
            let include_rules = !state.shortcuts_open;
            request = Some(Request(tokio::spawn(async move {
                read(&args, page, cached, include_rules).await
            })));
            last_refresh = tokio::time::Instant::now();
        }
        if dirty {
            terminal
                .as_mut()
                .unwrap()
                .draw(&render::frame(&state, size.0, size.1))?;
            dirty = false;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}
