//! Process-scoped enforcement, independent of the Slint Activity's lifetime.
use anyhow::{Context, Result};
use clash_mobile_runtime::{core, native_response};
use jni::{
    Env, EnvUnowned, JValue, jni_sig, jni_str,
    objects::{JClass, JString},
    refs::Global,
    vm::JavaVM,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Clone, PartialEq, Eq, Deserialize)]
struct Configuration {
    enabled: bool,
    #[serde(default)]
    controller_port: u16,
    #[serde(default)]
    mixed_port: u16,
    #[serde(default)]
    secret: String,
    #[serde(default)]
    node: String,
}
struct Command {
    configuration: Configuration,
    ready: std::sync::mpsc::Sender<()>,
}
static COMMANDS: Mutex<Option<mpsc::UnboundedSender<Command>>> = Mutex::new(None);
static STATE: Mutex<(bool, core::claude_check::guard::Snapshot)> = Mutex::new((
    false,
    core::claude_check::guard::Snapshot {
        total: 0,
        blocked: 0,
        alerts: 0,
        notifications_completed: 0,
        connected: false,
        error: String::new(),
        routes: std::collections::VecDeque::new(),
    },
));

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_clashofrust_android_ProtectionRuntime_nativeRecover(
    mut unowned_env: EnvUnowned<'_>,
    _: JClass<'_>,
    root: JString<'_>,
) -> jni::sys::jstring {
    unowned_env
        .with_env(|env| {
            let result = (|| -> Result<()> {
                let root = root.try_to_string(env)?;
                core::profile_transaction::recover(std::path::Path::new(&root))?;
                Ok(())
            })();
            let response = match result {
                Ok(()) => json!({"ok":true}),
                Err(error) => json!({"error":format!("恢复配置事务失败：{error:#}")}),
            };
            env.new_string(response.to_string()).map(JString::into_raw)
        })
        .resolve::<jni::errors::LogErrorAndDefault>()
}

struct Notifications {
    vm: JavaVM,
    class: Global<JClass<'static>>,
}
impl Notifications {
    fn show(&self, title: &str, message: &str) -> Result<()> {
        self.vm.attach_current_thread(|env| -> Result<()> {
            let title = env.new_string(title)?;
            let body = env.new_string(message)?;
            let result = env
                .call_static_method(
                    &self.class,
                    jni_str!("notify"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;"),
                    &[
                        JValue::Object(title.as_ref()),
                        JValue::Object(body.as_ref()),
                    ],
                )?
                .l()?;
            let result = env.cast_local::<JString>(result)?.try_to_string(env)?;
            native_response(serde_json::from_str(&result)?)?;
            Ok(())
        })
    }
}

fn configure(env: &mut Env<'_>, class: JClass<'_>, value: JString<'_>) -> Result<()> {
    let value = value.try_to_string(env)?;
    let configuration: Configuration = serde_json::from_str(&value)?;
    if configuration.enabled {
        anyhow::ensure!(
            !core::active_route::terminal(&configuration.node) && !configuration.node.is_empty(),
            "保护节点无效"
        );
        core::api::Api::new(&core::config::Settings {
            controller_port: configuration.controller_port,
            mixed_port: configuration.mixed_port,
            secret: configuration.secret.clone(),
            ..Default::default()
        })?;
    }
    let mut commands = COMMANDS
        .lock()
        .map_err(|_| anyhow::anyhow!("保护服务锁不可用"))?;
    if !configuration.enabled
        && commands
            .as_ref()
            .is_none_or(mpsc::UnboundedSender::is_closed)
    {
        return Ok(());
    }
    if commands
        .as_ref()
        .is_none_or(mpsc::UnboundedSender::is_closed)
    {
        let notifications = Arc::new(Notifications {
            vm: env.get_java_vm()?,
            class: env.new_global_ref(class)?,
        });
        let (send, mut receive) = mpsc::unbounded_channel::<Command>();
        std::thread::Builder::new().name("claude-protection".into()).spawn(move || {
            let result = (|| -> Result<()> {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
                runtime.block_on(async move {
                    let mut current = None;
                    let mut guard: Option<core::claude_check::guard::Guard> = None;
                    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
                    loop {
                        tokio::select! {
                            value = receive.recv() => {
                                let Some(command) = value else { break; };
                                let next = command.configuration;
                                if current.as_ref() == Some(&next) {
                                    let _ = command.ready.send(());
                                    continue;
                                }
                                guard = None;
                                if next.enabled {
                                    let api = core::api::Api::new(&core::config::Settings {
                                        controller_port: next.controller_port, mixed_port: next.mixed_port,
                                        secret: next.secret.clone(), ..Default::default()
                                    })?;
                                    let host = notifications.clone();
                                    guard = Some(core::claude_check::guard::Guard::start(api, next.node.clone(),
                                        move |title, message| {
                                            let host = host.clone();
                                            async move {
                                                tokio::task::spawn_blocking(move || host.show(&title, &message)).await?
                                            }
                                        }));
                                }
                                current = Some(next);
                                if let Ok(mut state) = STATE.lock() {
                                    *state = (guard.is_some(), guard.as_ref().map(|g|g.snapshot()).unwrap_or_default());
                                }
                                let _ = command.ready.send(());
                            }
                            _ = tick.tick() => {}
                        }
                        if let Ok(mut state) = STATE.lock() {
                            *state = (guard.is_some(), guard.as_ref().map(|g|g.snapshot()).unwrap_or_default());
                        }
                    }
                    Ok::<_, anyhow::Error>(())
                })
            })();
            if let Err(error) = result {
                log::error!("Protection service: {error:#}");
                if let Ok(mut state) = STATE.lock() {
                    state.0 = false;
                    state.1.error = format!("实时拦截停止：{error:#}");
                }
            }
        }).context("无法启动后台保护服务")?;
        *commands = Some(send);
    }
    let (ready, response) = std::sync::mpsc::channel();
    commands
        .as_ref()
        .context("保护服务不可用")?
        .send(Command {
            configuration,
            ready,
        })
        .map_err(|_| anyhow::anyhow!("保护服务已停止"))?;
    response
        .recv_timeout(std::time::Duration::from_secs(3))
        .context("保护服务未确认状态切换")?;
    Ok(())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_clashofrust_android_ProtectionRuntime_nativeConfigure(
    mut unowned_env: EnvUnowned<'_>,
    class: JClass<'_>,
    value: JString<'_>,
) -> jni::sys::jstring {
    unowned_env
        .with_env(|env| {
            let result = match configure(env, class, value) {
                Ok(()) => json!({"ok":true}),
                Err(error) => json!({"error":format!("{error:#}")}),
            };
            env.new_string(result.to_string()).map(JString::into_raw)
        })
        .resolve::<jni::errors::LogErrorAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_clashofrust_android_ProtectionRuntime_nativeState(
    mut unowned_env: EnvUnowned<'_>,
    _: JClass<'_>,
) -> jni::sys::jstring {
    unowned_env
        .with_env(|env| {
            let result = STATE
                .lock()
                .map(|state| json!({"enabled":state.0,"snapshot":state.1}))
                .unwrap_or_else(|_| json!({"enabled":false,"snapshot":{"error":"保护状态不可用"}}));
            env.new_string(result.to_string()).map(JString::into_raw)
        })
        .resolve::<jni::errors::LogErrorAndDefault>()
}
