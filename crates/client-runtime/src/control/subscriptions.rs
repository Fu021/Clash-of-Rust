use crate::engine::Engine;
use clash_client_core::subscription::schedule::{PendingUpdate, Schedule};
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
                let Some(profile) = schedule.next(&owner.settings, &owner.profiles, now) else {
                    continue;
                };
                match PendingUpdate::capture(profile, &owner.store) {
                    Ok(pending) => Some((
                        pending,
                        owner.running().then_some(owner.settings.mixed_port),
                    )),
                    Err(_) => {
                        owner.record_failure("订阅定时更新失败，已保留原配置");
                        None
                    }
                }
            };
            let Some((pending, port)) = pending else {
                continue;
            };
            // Downloads must not hold the engine lock or block GUI/CLI operations.
            let result = async {
                let url = reqwest::Url::parse(&pending.profile.source)?;
                crate::subscription::download(&url, port).await
            }
            .await;
            if *shutdown.borrow() {
                break;
            }
            let mut owner = shared.lock().await;
            match result {
                Ok((raw, usage)) => {
                    if owner
                        .update_subscription(&pending, raw, usage)
                        .await
                        .is_err()
                    {
                        owner.record_failure("订阅定时更新失败，已保留原配置与运行状态");
                    }
                }
                Err(_) => owner.record_failure("订阅定时下载失败，已保留原配置与运行状态"),
            }
        }
    })
}
