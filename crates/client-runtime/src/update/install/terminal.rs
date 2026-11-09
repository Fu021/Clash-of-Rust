use super::*;
use crate::{
    config::Store,
    control::{self, Meta, Remote},
    engine::Engine,
};
use std::{
    io::IsTerminal,
    os::fd::AsFd,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Recovery {
    executable: PathBuf,
    resources: PathBuf,
    running: bool,
    settings_sha256: String,
}

fn settings_sha256(engine: &Engine) -> Result<String> {
    Ok(clash_client_core::file_hash::hex(Sha256::digest(
        serde_json::to_vec(&engine.settings)?,
    )))
}

fn session_lock(data_dir: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(data_dir.join("update-terminal.lock"))?;
    lock.try_lock()
        .context("已有终端安装正在进行，请先运行 cor update status")?;
    Ok(lock)
}

pub(crate) fn prepare_terminal_install(
    download: Arc<Downloaded>,
    engine: &Engine,
) -> Result<PathBuf> {
    let _lock = session_lock(&engine.store.root)?;
    anyhow::ensure!(
        crate::platform::is_elevated() || Path::new("/usr/bin/sudo").is_file(),
        "纯终端安装需要 sudo；请用 cor update download --output cor-update.deb 下载，再由管理员安装"
    );
    anyhow::ensure!(
        control::gui_pid(&engine.store)?.is_none(),
        "桌面客户端正在运行，请从桌面更新，或退出窗口后重新运行 cor update install"
    );
    let (mut plan, directory) = prepare_plan(
        &download,
        std::process::id(),
        false,
        true,
        Some(engine.store.root.clone()),
    )?;
    let executable = std::env::current_exe()?.canonicalize()?;
    anyhow::ensure!(
        executable == Path::new("/opt/clash-of-rust/cor"),
        "终端自动安装仅支持 DEB 安装的 cor"
    );
    plan.terminal = Some(Recovery {
        executable,
        resources: engine.resource_directory().to_owned(),
        running: engine.running(),
        settings_sha256: settings_sha256(engine)?,
    });
    fs::write(directory.join("plan.json"), serde_json::to_vec(&plan)?)?;
    fs::copy(
        &plan
            .terminal
            .as_ref()
            .expect("terminal recovery")
            .executable,
        directory.join("update-helper"),
    )
    .context("无法准备终端安装助手")?;
    record_installation(
        &plan,
        &directory,
        "preparing",
        "安装包已校验，等待终端授权；后台仍在运行",
        false,
    )?;
    download.relinquish();
    Ok(directory)
}

fn read_plan(directory: &Path) -> Result<Plan> {
    anyhow::ensure!(
        owned_update_directory(directory).is_some(),
        "终端安装目录无效"
    );
    let file = fs::File::open(directory.join("plan.json"))?;
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 64 * 1024, "更新信息过大");
    let plan: Plan = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        plan.require_commit
            && !plan.restart
            && plan
                .data_dir
                .as_ref()
                .is_some_and(|path| path.is_absolute())
            && plan.terminal.as_ref().is_some_and(|recovery| {
                recovery.executable == Path::new("/opt/clash-of-rust/cor")
                    && recovery.resources.is_absolute()
            }),
        "终端安装交接信息无效"
    );
    Ok(plan)
}

fn check_record(plan: &Plan, directory: &Path, data_dir: &Path) -> Result<()> {
    let status = load_installation_status(data_dir)?.context("安装记录已丢失")?;
    anyhow::ensure!(
        plan.data_dir.as_deref() == Some(data_dir)
            && status.directory == directory
            && status.state == "preparing"
            && status.version == plan.version
            && !directory.join("commit").exists(),
        "安装交接已失效，请重新运行 cor update install"
    );
    Ok(())
}

fn check_current(plan: &Plan, directory: &Path, engine: &Engine) -> Result<()> {
    let recovery = plan.terminal.as_ref().context("缺少终端安装恢复信息")?;
    check_record(plan, directory, &engine.store.root)?;
    check_owner(plan, control::Endpoint::read(&engine.store)?.as_ref())?;
    anyhow::ensure!(
        control::gui_pid(&engine.store)?.is_none()
            && recovery.running == engine.running()
            && recovery.settings_sha256 == settings_sha256(engine)?,
        "后台、窗口或设置已变化，已取消安装；请重新运行 cor update install"
    );
    Ok(())
}

fn check_owner(plan: &Plan, endpoint: Option<&control::Endpoint>) -> Result<()> {
    anyhow::ensure!(
        endpoint.is_some_and(|endpoint| {
            endpoint.pid == plan.parent
                && endpoint.pid == std::process::id()
                && endpoint.owner == control::Owner::Daemon
        }),
        "原后台已变化，已取消安装；请重新运行 cor update install"
    );
    Ok(())
}

pub(crate) async fn commit_terminal_install(directory: &Path, engine: &mut Engine) -> Result<()> {
    let plan = read_plan(directory)?;
    check_current(&plan, directory, engine)?;
    fs::write(directory.join("commit"), b"")?;
    engine.stop().await?;
    Ok(())
}

fn foreground(executable: &Path) -> Result<Command> {
    let mut command = Command::new(executable);
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::from(std::io::stderr().as_fd().try_clone_to_owned()?))
        .stderr(Stdio::inherit());
    Ok(command)
}

pub fn run_terminal_install(directory: &Path) -> Result<InstallationStatus> {
    let plan = read_plan(directory)?;
    let mut child = foreground(&directory.join("update-helper"))?;
    child.arg("--install-update-terminal").arg(directory);
    let result = child.status().context("无法启动终端安装助手")?;
    let status = installation_status(plan.data_dir.as_deref().context("缺少安装数据目录")?)?
        .context("安装助手没有保存结果")?;
    anyhow::ensure!(
        result.success()
            && status.directory == directory
            && status.version == plan.version
            && matches!(status.state.as_str(), "succeeded" | "failed"),
        "安装助手已中断，尚未确认结果；请运行 cor update status 并检查 cor status"
    );
    Ok(status)
}

fn authorize(interactive: bool) -> Result<()> {
    if crate::platform::is_elevated() {
        return Ok(());
    }
    anyhow::ensure!(
        Path::new("/usr/bin/sudo").is_file(),
        "纯终端安装需要 sudo；请用 cor update download --output cor-update.deb 下载，再由管理员安装"
    );
    let mut command = foreground(Path::new("/usr/bin/sudo"))?;
    if !interactive {
        command.arg("-n");
    }
    let status = command.arg("-v").status().context("无法请求终端安装授权")?;
    anyhow::ensure!(
        status.success(),
        "未获得安装授权，未执行停止后台；请在交互终端重试，或由管理员手动安装"
    );
    Ok(())
}

fn install_package(plan: &Plan, interactive: bool) -> Result<()> {
    verify(plan, plan.package.parent().context("安装包目录无效")?)?;
    let mut command = if crate::platform::is_elevated() {
        foreground(Path::new("/usr/bin/dpkg"))?
    } else {
        let mut command = foreground(Path::new("/usr/bin/sudo"))?;
        if !interactive {
            command.arg("-n");
        }
        command.args(["--", "/usr/bin/dpkg"]);
        command
    };
    command.arg("-i").arg(&plan.package);
    let (status, detail) = wait_installer(&mut command)?;
    anyhow::ensure!(status.success(), "DEB 安装失败（{status}）：{detail}");
    Ok(())
}

fn wait_installer(command: &mut Command) -> Result<(std::process::ExitStatus, String)> {
    use std::io::Write;
    let mut child = command
        .stderr(Stdio::piped())
        .spawn()
        .context("无法启动 DEB 安装程序")?;
    let mut output = child.stderr.take().context("无法读取安装错误输出")?;
    let mut tail = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        match output.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                let _ = std::io::stderr().write_all(&buffer[..count]);
                tail.extend_from_slice(&buffer[..count]);
                if tail.len() > 2048 {
                    tail.drain(..tail.len() - 2048);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let status = child.wait().context("等待 DEB 安装结束失败")?;
    let detail = String::from_utf8_lossy(&tail)
        .chars()
        .rev()
        .take(500)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    Ok((
        status,
        if detail.trim().is_empty() {
            "安装程序未提供详情，请检查权限、包管理器、依赖及磁盘空间".into()
        } else {
            detail.trim().into()
        },
    ))
}

async fn stop_owner(plan: &Plan, directory: &Path) -> Result<()> {
    let store = Store::at(plan.data_dir.as_ref().context("缺少安装数据目录")?.clone())?;
    let remote =
        Remote::discover(&store, Duration::from_secs(30))?.context("原后台已退出，已取消安装")?;
    let result: serde_json::Value = remote
        .request(control::Command::UpdateCommitTerminal {
            directory: directory.into(),
        })
        .await?;
    anyhow::ensure!(result["shutdown"] == true, "原后台未确认安装交接");
    remote.wait_closed(&store).await?;
    Ok(())
}

fn restore_owner(plan: &Plan) -> Result<()> {
    let recovery = plan.terminal.as_ref().context("缺少后台恢复信息")?;
    let data_dir = plan.data_dir.as_ref().context("缺少安装数据目录")?;
    let mut command = foreground(&recovery.executable)?;
    command
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--resources")
        .arg(&recovery.resources);
    if recovery.running {
        let mut child = command.arg("start").spawn().context("无法重新启动原后台")?;
        let deadline = Instant::now() + Duration::from_secs(90);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!("等待原内核恢复超时，请运行 cor status");
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        anyhow::ensure!(status.success(), "恢复原内核失败（{status}）");
    } else {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::process::CommandExt;
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(data_dir.join("daemon.log"))?;
        let _child = command
            .args(["run", "--no-core", "--quiet"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .process_group(0)
            .spawn()
            .context("无法重新启动原后台")?;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let store = Store::at(data_dir.clone())?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if let Some(remote) = Remote::discover(&store, Duration::from_secs(2))?
                && let Ok(meta) = remote.request::<Meta>(control::Command::Meta).await
            {
                anyhow::ensure!(
                    meta.running == recovery.running,
                    "后台已恢复，但内核状态与更新前不同，请运行 cor status"
                );
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        bail!("等待原后台恢复超时，请运行 cor status")
    })
}

fn failure_message(error: &anyhow::Error, recovery: Option<Result<()>>) -> String {
    let recovery = match recovery {
        None => "本次未停止原后台".into(),
        Some(Ok(())) => "已恢复更新前的后台和内核状态".into(),
        Some(Err(error)) => format!("恢复后台也失败：{error:#}；请运行 cor start"),
    };
    format!("更新失败：{error:#}；{recovery}")
}

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn interrupted(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

fn check_interrupted() -> Result<()> {
    anyhow::ensure!(!INTERRUPTED.load(Ordering::Relaxed), "安装已取消");
    Ok(())
}

fn transaction(
    version: &str,
    authorize: impl FnOnce() -> Result<()>,
    commit: impl FnOnce() -> Result<()>,
    install: impl FnOnce() -> Result<()>,
    committed: impl FnOnce() -> bool,
    recover: impl FnOnce() -> Result<()>,
) -> Outcome {
    let result = authorize().and_then(|()| commit()).and_then(|()| install());
    match result {
        Ok(()) => Outcome {
            message: format!("已更新至 {version}；可用 cor start 启动后台"),
            error: false,
        },
        Err(error) => Outcome {
            message: failure_message(&error, committed().then(recover)),
            error: true,
        },
    }
}

pub(super) fn helper(directory: &Path) {
    let result = (|| -> Result<(Plan, fs::File)> {
        let plan = read_plan(directory)?;
        let lock = session_lock(plan.data_dir.as_deref().context("缺少安装数据目录")?)?;
        Ok((plan, lock))
    })();
    let (plan, _lock) = match result {
        Ok(prepared) => prepared,
        Err(error) => {
            eprintln!("cor: 无法准备终端安装：{error:#}");
            return;
        }
    };
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        unsafe { libc::signal(signal, interrupted as *const () as libc::sighandler_t) };
    }
    let interactive = std::io::stdin().is_terminal();
    let outcome = transaction(
        &plan.version,
        || {
            verify(&plan, directory)?;
            authorize(interactive)?;
            check_interrupted()
        },
        || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(stop_owner(&plan, directory))
        },
        || {
            check_interrupted()?;
            record_installation(
                &plan,
                directory,
                "installing",
                "正在安装，后台已暂停",
                false,
            )?;
            install_package(&plan, interactive)
        },
        || directory.join("commit").exists(),
        || restore_owner(&plan),
    );
    let saved = record_installation(
        &plan,
        directory,
        if outcome.error { "failed" } else { "succeeded" },
        &outcome.message,
        outcome.error,
    )
    .and_then(|()| {
        crate::config::atomic_write(
            &directory.join("outcome.json"),
            &serde_json::to_vec(&outcome)?,
        )
    });
    if let Err(error) = saved {
        eprintln!("cor: 保存安装结果失败：{error:#}；{}", outcome.message);
    } else if let Some(owned) = owned_update_directory(directory) {
        let _ = fs::remove_dir_all(owned);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn plan(data_dir: &Path, directory: &Path) -> Plan {
        Plan {
            package: directory.join("package.deb"),
            executable: "/opt/clash-of-rust/clash-of-rust".into(),
            sha256: clash_client_core::file_hash::hex(Sha256::digest(b"verified package")),
            version: "0.6.7".into(),
            parent: std::process::id(),
            restart: false,
            require_commit: true,
            data_dir: Some(data_dir.into()),
            terminal: Some(Recovery {
                executable: "/opt/clash-of-rust/cor".into(),
                resources: "/opt/clash-of-rust/resources".into(),
                running: true,
                settings_sha256: "0".repeat(64),
            }),
        }
    }

    #[test]
    fn commit_requires_the_exact_pending_installation_and_cannot_be_replayed() {
        let data = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let next = tempfile::tempdir().unwrap();
        let plan = plan(data.path(), first.path());
        assert!(check_record(&plan, first.path(), data.path()).is_err());
        record_installation(&plan, first.path(), "preparing", "waiting", false).unwrap();
        check_record(&plan, first.path(), data.path()).unwrap();
        fs::write(first.path().join("commit"), b"").unwrap();
        assert!(check_record(&plan, first.path(), data.path()).is_err());
        fs::remove_file(first.path().join("commit")).unwrap();
        record_installation(&plan, first.path(), "failed", "cancelled", true).unwrap();
        assert!(check_record(&plan, first.path(), data.path()).is_err());
        record_installation(&plan, next.path(), "preparing", "new request", false).unwrap();
        assert!(check_record(&plan, first.path(), data.path()).is_err());
        assert!(check_record(&plan, next.path(), next.path()).is_err());
    }

    #[test]
    fn handoff_requires_the_original_daemon_in_the_same_data_directory() {
        let data = tempfile::tempdir().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let plan = plan(data.path(), directory.path());
        let endpoint = |pid, owner| {
            serde_json::from_value::<control::Endpoint>(serde_json::json!({
                "protocol": control::PROTOCOL,
                "port": 12345,
                "pid": pid,
                "owner": owner,
                "token": "0".repeat(64),
            }))
            .unwrap()
        };
        check_owner(&plan, Some(&endpoint(plan.parent, "daemon"))).unwrap();
        assert!(check_owner(&plan, None).is_err());
        assert!(check_owner(&plan, Some(&endpoint(plan.parent + 1, "daemon"))).is_err());
        assert!(check_owner(&plan, Some(&endpoint(plan.parent, "gui"))).is_err());
    }

    #[test]
    fn changed_package_is_rejected_before_authorization_or_shutdown() {
        let data = tempfile::tempdir().unwrap();
        let download = tempfile::tempdir().unwrap();
        let plan = plan(data.path(), download.path());
        fs::write(&plan.package, b"verified package").unwrap();
        verify(&plan, download.path()).unwrap();
        fs::write(&plan.package, b"changed package").unwrap();
        let outcome = transaction(
            &plan.version,
            || verify(&plan, download.path()),
            || panic!("must not stop the owner"),
            || panic!("must not install changed bytes"),
            || false,
            || panic!("the owner must remain running"),
        );
        assert!(outcome.error);
        assert!(outcome.message.contains("安装包校验失败"));
    }

    #[test]
    fn authorization_denial_never_stops_the_owner_or_installs() {
        let outcome = transaction(
            "0.6.7",
            || bail!("authorization denied"),
            || panic!("must not stop the owner"),
            || panic!("must not install"),
            || false,
            || panic!("must not start a second owner"),
        );
        assert!(outcome.error);
        assert!(outcome.message.contains("authorization denied"));
        assert!(outcome.message.contains("本次未停止原后台"));
    }

    #[test]
    fn installation_failure_restores_owner_and_preserves_both_errors() {
        for recovery_fails in [false, true] {
            let events = RefCell::new(Vec::new());
            let committed = Cell::new(false);
            let outcome = transaction(
                "0.6.7",
                || {
                    events.borrow_mut().push("authorize");
                    Ok(())
                },
                || {
                    events.borrow_mut().push("stop");
                    committed.set(true);
                    Ok(())
                },
                || {
                    events.borrow_mut().push("install");
                    bail!("disk full")
                },
                || committed.get(),
                || {
                    events.borrow_mut().push("recover");
                    if recovery_fails {
                        bail!("core start failed");
                    }
                    Ok(())
                },
            );
            assert_eq!(
                *events.borrow(),
                ["authorize", "stop", "install", "recover"]
            );
            assert!(outcome.error);
            assert!(outcome.message.contains("disk full"));
            assert_eq!(
                outcome.message.contains("core start failed"),
                recovery_fails
            );
            assert_eq!(outcome.message.contains("已恢复"), !recovery_fails);
        }
    }

    #[test]
    fn partial_shutdown_failure_recovers_but_a_stale_handoff_does_not() {
        for stopped in [false, true] {
            let recovered = Cell::new(false);
            let outcome = transaction(
                "0.6.7",
                || Ok(()),
                || bail!("commit failed"),
                || panic!("must not install after an unacknowledged shutdown"),
                || stopped,
                || {
                    recovered.set(true);
                    Ok(())
                },
            );
            assert!(outcome.error);
            assert_eq!(recovered.get(), stopped);
        }
    }

    #[test]
    fn successful_install_waits_for_completion_and_does_not_restart_the_old_core() {
        let installed = Cell::new(false);
        let outcome = transaction(
            "0.6.7",
            || Ok(()),
            || Ok(()),
            || {
                installed.set(true);
                Ok(())
            },
            || panic!("must not recover a successful installation"),
            || panic!("must not restart a successful installation"),
        );
        assert!(installed.get());
        assert!(!outcome.error);
        assert!(outcome.message.contains("已更新至 0.6.7"));
    }

    #[test]
    fn terminal_installs_are_serialized_per_data_directory() {
        let directory = tempfile::tempdir().unwrap();
        let first = session_lock(directory.path()).unwrap();
        assert!(session_lock(directory.path()).is_err());
        drop(first);
        assert!(session_lock(directory.path()).is_ok());
    }

    #[test]
    fn installer_failure_keeps_the_exit_code_and_bounded_error_tail() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "printf '%4096s' old-output >&2; printf '\\npermission denied\\n' >&2; exit 7",
        ]);
        let (status, detail) = wait_installer(&mut command).unwrap();
        assert_eq!(status.code(), Some(7));
        assert!(detail.ends_with("permission denied"));
        assert!(detail.chars().count() <= 500);
        let mut success = Command::new("/bin/sh");
        success.args(["-c", "exit 0"]);
        assert!(wait_installer(&mut success).unwrap().0.success());
    }

    #[test]
    fn failed_install_reports_recovery_without_claiming_installation_succeeded() {
        let error = anyhow::anyhow!("dpkg failed");
        let recovered = failure_message(&error, Some(Ok(())));
        assert!(recovered.contains("dpkg failed"));
        assert!(recovered.contains("已恢复"));
        assert!(!recovered.contains("已更新至"));
        let failed = failure_message(&error, Some(Err(anyhow::anyhow!("core failed"))));
        assert!(failed.contains("dpkg failed"));
        assert!(failed.contains("core failed"));
        assert!(failed.contains("cor start"));
        assert!(failure_message(&error, None).contains("未停止原后台"));
    }
}
