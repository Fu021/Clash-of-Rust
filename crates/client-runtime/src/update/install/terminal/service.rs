use super::*;
use crate::platform::cli_service;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct UserService {
    unit_file: PathBuf,
    unit_sha256: String,
    enabled: String,
    tun_authorization: bool,
}

struct State {
    file: PathBuf,
    enabled: String,
    pid: u32,
    overrides: bool,
}

fn in_service(cgroup: &str) -> bool {
    cgroup.lines().any(|line| {
        line.splitn(3, ':')
            .nth(2)
            .and_then(|path| path.rsplit('/').next())
            == Some(cli_service::NAME)
    })
}

impl State {
    fn parse(text: &str) -> Result<Self> {
        let field = |name| {
            text.lines()
                .filter_map(|line| line.split_once('='))
                .find_map(|(key, value)| (key == name).then_some(value))
                .context("CLI 自启动服务状态不完整")
        };
        anyhow::ensure!(field("LoadState")? == "loaded", "CLI 自启动服务未加载");
        Ok(Self {
            file: PathBuf::from(field("FragmentPath")?),
            enabled: field("UnitFileState")?.into(),
            pid: field("MainPID")?
                .parse()
                .context("CLI 自启动服务进程号无效")?,
            overrides: !field("DropInPaths")?.is_empty(),
        })
    }

    fn read() -> Result<Self> {
        let mut child = Command::new("/usr/bin/systemctl")
            .args([
                "--user",
                "--no-pager",
                "--no-ask-password",
                "show",
                cli_service::NAME,
                "--property=LoadState,FragmentPath,UnitFileState,MainPID,DropInPaths",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("无法查询 CLI 自启动服务")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!("查询 CLI 自启动服务超时");
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        anyhow::ensure!(status.success(), "无法读取当前用户的 CLI 自启动服务");
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .context("服务查询没有输出")?
            .take(8193)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 8192, "CLI 自启动服务状态过大");
        Self::parse(std::str::from_utf8(&bytes)?)
    }
}

impl UserService {
    pub(super) fn capture(engine: &Engine, executable: &Path) -> Result<Option<Self>> {
        let managed = fs::read_to_string("/proc/self/cgroup")
            .ok()
            .is_some_and(|cgroup| in_service(&cgroup));
        let result = (|| {
            let unit_file = cli_service::path()?;
            let bytes = fs::read(&unit_file)?;
            let state = State::read()?;
            let definitions = [
                cli_service::definition(executable, &engine.store.root, None)?,
                cli_service::definition(
                    executable,
                    &engine.store.root,
                    Some(engine.resource_directory()),
                )?,
            ];
            Self::select(
                unit_file,
                &bytes,
                &state,
                &definitions,
                managed,
                engine.settings.proxy_mode == crate::engine::ProxyMode::Tun
                    || engine.settings.claude_protection.is_some(),
            )
        })();
        if managed {
            result.context("无法准备原 CLI 自启动服务的更新恢复")
        } else {
            Ok(result.ok().flatten())
        }
    }

    fn select(
        unit_file: PathBuf,
        bytes: &[u8],
        state: &State,
        definitions: &[String],
        managed: bool,
        tun_authorization: bool,
    ) -> Result<Option<Self>> {
        if !managed && !matches!(state.enabled.as_str(), "enabled" | "enabled-runtime") {
            return Ok(None);
        }
        anyhow::ensure!(
            state.file == unit_file && !state.overrides,
            "CLI 自启动服务包含其他配置，未自动接管"
        );
        anyhow::ensure!(
            state.pid == 0 || state.pid == std::process::id(),
            "CLI 自启动服务正在管理其他后台"
        );
        let matches = definitions
            .iter()
            .any(|definition| bytes == definition.as_bytes());
        anyhow::ensure!(matches, "CLI 自启动服务与当前程序或数据目录不匹配");
        Ok(Some(Self {
            unit_file,
            unit_sha256: clash_client_core::file_hash::hex(Sha256::digest(bytes)),
            enabled: state.enabled.clone(),
            tun_authorization,
        }))
    }

    fn validate(&self, state: &State) -> Result<()> {
        let bytes = fs::read(&self.unit_file).context("CLI 自启动服务配置已无法读取")?;
        anyhow::ensure!(
            clash_client_core::file_hash::hex(Sha256::digest(bytes)) == self.unit_sha256
                && state.file == self.unit_file
                && state.enabled == self.enabled
                && !state.overrides,
            "CLI 自启动服务配置已变化，未自动启动"
        );
        Ok(())
    }

    pub(super) fn restore(&self, plan: &Plan, recovery: &Recovery) -> Result<()> {
        self.validate(&State::read()?)?;
        if self.tun_authorization {
            let status = wait_command(
                foreground(&recovery.executable)?.arg("__authorize-tun-for-service"),
                "等待 TUN 授权超时",
            )?;
            anyhow::ensure!(status.success(), "无法完成自启动服务的 TUN 授权");
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let state = State::read()?;
            self.validate(&state)?;
            if state.pid == 0 {
                let status = wait_command(
                    foreground(Path::new("/usr/bin/systemctl"))?.args([
                        "--user",
                        "--no-ask-password",
                        "start",
                        cli_service::NAME,
                    ]),
                    "等待 CLI 自启动服务超时",
                )?;
                anyhow::ensure!(status.success(), "无法启动 CLI 自启动服务（{status}）");
                return Ok(());
            }
            anyhow::ensure!(
                state.pid == plan.parent,
                "CLI 自启动服务已被其他后台接管，未重复启动"
            );
            anyhow::ensure!(Instant::now() < deadline, "原 CLI 自启动服务尚未退出");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub(super) fn verify(&self, pid: u32) -> Result<()> {
        let state = State::read()?;
        self.verify_state(&state, pid)
    }

    fn verify_state(&self, state: &State, pid: u32) -> Result<()> {
        self.validate(state)?;
        anyhow::ensure!(
            state.pid != 0 && state.pid == pid,
            "后台已恢复，但未由原 CLI 自启动服务管理"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_cgroup_requires_the_exact_unit_in_either_hierarchy() {
        assert!(in_service(
            "0::/user.slice/user@1000.service/app.slice/clash-of-rust-cli.service\n"
        ));
        assert!(in_service(
            "1:name=systemd:/user.slice/clash-of-rust-cli.service\n"
        ));
        for value in [
            "",
            "0::/other.service",
            "0::/clash-of-rust-cli.service-extra",
            "0::/clash-of-rust-cli.service/child",
        ] {
            assert!(!in_service(value), "{value}");
        }
    }

    #[test]
    fn service_properties_cannot_treat_a_missing_or_incomplete_unit_as_running() {
        let complete = "MainPID=1234\nUnitFileState=enabled\nFragmentPath=/home/example/service\nDropInPaths=\nLoadState=loaded\n";
        assert_eq!(State::parse(complete).unwrap().pid, 1234);
        assert!(!State::parse(complete).unwrap().overrides);
        for text in [
            complete.replace("LoadState=loaded", "LoadState=not-found"),
            complete.replace("MainPID=1234", "MainPID=invalid"),
            complete.replace("DropInPaths=\n", ""),
        ] {
            assert!(State::parse(&text).is_err());
        }
    }

    fn state(file: &Path) -> State {
        State {
            file: file.into(),
            enabled: "enabled".into(),
            pid: 0,
            overrides: false,
        }
    }

    #[test]
    fn adoption_requires_the_enabled_exact_instance_without_overrides() {
        let data = tempfile::tempdir().unwrap();
        let file = data.path().join(cli_service::NAME);
        let executable = Path::new("/opt/clash-of-rust/cor");
        let resources = Path::new("/opt/clash-of-rust/resources");
        let bytes = cli_service::definition(executable, data.path(), Some(resources)).unwrap();
        let definitions = [
            bytes.clone(),
            cli_service::definition(executable, data.path(), None).unwrap(),
        ];
        let select = |state: &State, bytes: &[u8], managed| {
            UserService::select(file.clone(), bytes, state, &definitions, managed, true)
        };
        let mut state = state(&file);
        assert!(select(&state, bytes.as_bytes(), false).unwrap().is_some());
        state.enabled = "disabled".into();
        assert!(select(&state, bytes.as_bytes(), false).unwrap().is_none());
        assert!(select(&state, bytes.as_bytes(), true).unwrap().is_some());
        state.enabled = "enabled".into();
        state.overrides = true;
        assert!(select(&state, bytes.as_bytes(), true).is_err());
        state.overrides = false;
        state.pid = std::process::id() + 1;
        assert!(select(&state, bytes.as_bytes(), false).is_err());
        state.pid = 0;
        let other = cli_service::definition(executable, Path::new("/other/profile"), None).unwrap();
        assert!(select(&state, other.as_bytes(), false).is_err());
        let other = cli_service::definition(Path::new("/other/cor"), data.path(), None).unwrap();
        assert!(select(&state, other.as_bytes(), false).is_err());
    }

    #[test]
    fn changed_or_removed_units_and_foreign_fragments_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join(cli_service::NAME);
        fs::write(&file, b"original").unwrap();
        let service = UserService {
            unit_file: file.clone(),
            unit_sha256: clash_client_core::file_hash::hex(Sha256::digest(b"original")),
            enabled: "enabled".into(),
            tun_authorization: false,
        };
        let mut state = state(&file);
        service.validate(&state).unwrap();
        assert!(service.verify_state(&state, 1234).is_err());
        state.pid = 1234;
        service.verify_state(&state, 1234).unwrap();
        assert!(service.verify_state(&state, 4321).is_err());
        state.enabled = "disabled".into();
        assert!(service.validate(&state).is_err());
        state.enabled = "enabled".into();
        state.file = directory.path().join("other.service");
        assert!(service.validate(&state).is_err());
        state.file = file.clone();
        fs::write(&file, b"changed").unwrap();
        assert!(service.validate(&state).is_err());
        fs::remove_file(file).unwrap();
        assert!(service.validate(&state).is_err());
    }
}
