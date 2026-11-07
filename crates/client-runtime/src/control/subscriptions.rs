use crate::engine::Engine;
use clash_client_core::subscription::schedule::Schedule;
use std::{
    sync::Weak,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, watch};

pub(super) fn start(
    engine: Weak<Mutex<Engine>>,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut schedule = Schedule::default();
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = tick.tick() => {},
                _ = shutdown.changed() => break,
            }
            if *shutdown.borrow() {
                break;
            }
            let Some(shared) = engine.upgrade() else {
                break;
            };
            let pending = {
                let owner = shared.lock().await;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                match owner.prepare_scheduled_subscription(&mut schedule, now) {
                    Ok(pending) => pending,
                    Err(_) => {
                        owner.record_failure("订阅定时更新失败，已保留原配置");
                        None
                    }
                }
            };
            let Some((mut job, pending, port)) = pending else {
                continue;
            };
            let mut disconnected = Some(shutdown.clone());
            let result = job
                .download(
                    async {
                        let url = reqwest::Url::parse(&pending.profile.source)?;
                        crate::subscription::download(&url, port).await
                    },
                    &mut disconnected,
                )
                .await;
            if *shutdown.borrow() {
                break;
            }
            let mut owner = shared.lock().await;
            if job.ensure_current(&disconnected).is_err() {
                continue;
            }
            match result {
                Ok((raw, usage)) => {
                    if let Err(error) = owner.update_subscription(&pending, raw, usage).await {
                        owner.record_subscription_format_failure(&error);
                        owner.record_failure("订阅定时更新失败，已保留原配置与运行状态");
                    }
                }
                Err(error) => {
                    if pending
                        .is_current(&owner.settings, &owner.profiles, &owner.store)
                        .unwrap_or(false)
                    {
                        owner.record_subscription_format_failure(&error);
                    }
                    owner.record_failure(&format!(
                        "订阅定时下载失败，已保留原配置与运行状态：{error}"
                    ));
                }
            }
        }
    })
}
