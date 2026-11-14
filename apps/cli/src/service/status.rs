use crate::{args::Args, backend};
#[cfg(unix)]
use anyhow::ensure;
#[cfg(any(unix, test))]
use anyhow::{Context, Result};
use clash_client_runtime::{
    config::Store,
    control::{Endpoint, Owner},
};
use serde_json::{Value, json};

pub(super) async fn enrich(args: &Args, mut value: Value) -> Value {
    let owner = async {
        let Some(_) = backend::connect(args, false).await? else {
            return Ok(None);
        };
        Endpoint::read(&Store {
            root: backend::data_root(args)?,
        })
    };
    match tokio::time::timeout(std::time::Duration::from_secs(3), owner).await {
        Ok(Ok(endpoint)) => {
            value["owner_pid"] = json!(endpoint.as_ref().map(|endpoint| endpoint.pid));
            value["ownership"] = json!(ownership(
                value["active"].as_bool(),
                value["main_pid"].as_u64(),
                endpoint
                    .as_ref()
                    .map(|endpoint| (endpoint.pid, endpoint.owner)),
            ));
        }
        result => {
            value["ownership"] = json!("unknown");
            value["owner_pid"] = Value::Null;
            value["backend_error"] = json!(match result {
                Ok(Err(error)) => format!("{error:#}"),
                _ => "查询后台超过 3 秒".into(),
            });
        }
    }
    value
}

fn ownership(
    active: Option<bool>,
    main_pid: Option<u64>,
    owner: Option<(u32, Owner)>,
) -> &'static str {
    match owner {
        None => "none",
        Some((_, Owner::Gui)) => "gui",
        Some((pid, _)) if active == Some(true) && main_pid == Some(u64::from(pid)) => "service",
        Some(_) if active.is_none() || main_pid.is_none() => "unknown",
        Some(_) => "detached",
    }
}

#[cfg(unix)]
pub(super) async fn query() -> Result<Value> {
    #[cfg(target_os = "linux")]
    {
        let text = query_output(
            "systemctl",
            &[
                "--user",
                "--no-pager",
                "--no-ask-password",
                "show",
                clash_client_runtime::platform::cli_service::NAME,
                "--property=LoadState,UnitFileState,ActiveState,SubState,MainPID",
            ],
        )
        .await?;
        linux(&text)
    }
    #[cfg(target_os = "macos")]
    {
        let domain = format!("gui/{}", unsafe { libc::geteuid() });
        let disabled = query_output("launchctl", &["print-disabled", &domain]).await?;
        let target = format!("{domain}/org.clash-of-rust.cli");
        let result = query_output("launchctl", &["print", &target]).await;
        let mut value = match result {
            Ok(text) => macos(&text)?,
            Err(error) => {
                return Err(error.context("无法确认 launchd 服务是否已加载"));
            }
        };
        value["enabled"] = json!(
            !disabled
                .lines()
                .any(|line| line.trim() == "\"org.clash-of-rust.cli\" => true")
        );
        Ok(value)
    }
}

#[cfg(unix)]
async fn query_output(program: &str, arguments: &[&str]) -> Result<String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new(program)
            .args(arguments)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("查询登录服务超过 5 秒")?
    .context("无法查询当前用户的登录服务")?;
    ensure!(
        output.status.success(),
        "登录服务查询失败：{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?)
}

#[cfg(any(target_os = "linux", test))]
fn linux(text: &str) -> Result<Value> {
    let field = |name| {
        text.lines()
            .filter_map(|line| line.split_once('='))
            .find_map(|(key, value)| (key == name).then_some(value))
            .context("登录服务查询缺少状态字段")
    };
    let loaded = field("LoadState")?;
    let enabled_state = field("UnitFileState")?;
    let active_state = field("ActiveState")?;
    let enabled = match enabled_state {
        "enabled" | "enabled-runtime" => Some(true),
        "disabled" | "masked" | "masked-runtime" => Some(false),
        "" if loaded == "not-found" => Some(false),
        _ => None,
    };
    Ok(json!({
        "loaded":loaded,
        "enabled":enabled,
        "enabled_state":enabled_state,
        "active":match active_state {
            "active" | "reloading" => Some(true),
            "inactive" | "failed" => Some(false),
            _ => None,
        },
        "active_state":active_state,
        "sub_state":field("SubState")?,
        "main_pid":field("MainPID")?.parse::<u32>().context("服务进程号无效")?,
    }))
}

#[cfg(any(target_os = "macos", test))]
fn macos(text: &str) -> Result<Value> {
    let field = |name: &str| {
        text.lines()
            .filter_map(|line| line.trim().split_once(" = "))
            .find_map(|(key, value)| (key == name).then_some(value.trim()))
    };
    let state = field("state").context("launchd 查询缺少运行状态")?;
    let pid = field("pid")
        .map(str::parse::<u32>)
        .transpose()
        .context("launchd 进程号无效")?
        .unwrap_or(0);
    Ok(json!({"active":state=="running","active_state":state,"main_pid":pid}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_states_distinguish_configuration_activity_and_unknown() {
        let text = "LoadState=loaded\nUnitFileState=enabled\nActiveState=inactive\nSubState=dead\nMainPID=0\n";
        let value = linux(text).unwrap();
        assert_eq!(value["enabled"], true);
        assert_eq!(value["active"], false);
        assert_eq!(value["main_pid"], 0);
        let value = linux(&text.replace("inactive", "active").replace("=0", "=123")).unwrap();
        assert_eq!(value["active"], true);
        assert_eq!(value["main_pid"], 123);
        let value = linux(
            &text
                .replace("enabled", "static")
                .replace("inactive", "activating"),
        )
        .unwrap();
        assert!(value["enabled"].is_null());
        assert!(value["active"].is_null());
        assert!(linux("ActiveState=active").is_err());
        assert!(linux(&text.replace("MainPID=0", "MainPID=bad")).is_err());
    }

    #[test]
    fn backend_identity_is_separate_from_service_configuration() {
        assert_eq!(
            ownership(Some(true), Some(12), Some((12, Owner::Daemon))),
            "service"
        );
        assert_eq!(
            ownership(Some(false), Some(0), Some((12, Owner::Daemon))),
            "detached"
        );
        assert_eq!(
            ownership(Some(true), Some(13), Some((12, Owner::Daemon))),
            "detached"
        );
        assert_eq!(ownership(None, None, Some((12, Owner::Daemon))), "unknown");
        assert_eq!(ownership(Some(true), Some(12), None), "none");
        assert_eq!(
            ownership(Some(true), Some(12), Some((12, Owner::Gui))),
            "gui"
        );
    }

    #[test]
    fn launchd_loaded_and_running_states_are_not_conflated() {
        let running = macos("service = {\n\tstate = running\n\tpid = 321\n}").unwrap();
        assert_eq!(running["active"], true);
        assert_eq!(running["main_pid"], 321);
        let waiting = macos("service = {\n\tstate = not running\n}").unwrap();
        assert_eq!(waiting["active"], false);
        assert_eq!(waiting["main_pid"], 0);
        assert!(macos("unavailable").is_err());
    }
}
