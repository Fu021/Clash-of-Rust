use anyhow::{Result, anyhow};
use clash_mobile_runtime::core::claude_check::guard::Snapshot;
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

pub(crate) fn acknowledge_stopped_pause<T>(
    enabled: bool,
    commands: Option<&UnboundedSender<T>>,
    state: &Mutex<(bool, Snapshot)>,
) -> Result<bool> {
    if !enabled && commands.is_none_or(UnboundedSender::is_closed) {
        *state.lock().map_err(|_| anyhow!("保护状态不可用"))? = (false, Snapshot::default());
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pausing_a_stopped_worker_clears_its_previous_failure_snapshot() {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<()>();
        drop(receiver);
        for commands in [Some(&sender), None] {
            let state = Mutex::new((
                true,
                Snapshot {
                    connected: true,
                    error: "fixture worker failure".into(),
                    ..Default::default()
                },
            ));
            assert!(!acknowledge_stopped_pause(true, commands, &state).unwrap());
            assert_eq!(state.lock().unwrap().1.error, "fixture worker failure");
            assert!(acknowledge_stopped_pause(false, commands, &state).unwrap());
            let snapshot = state.lock().unwrap();
            assert!(!snapshot.0);
            assert!(!snapshot.1.connected);
            assert!(snapshot.1.error.is_empty());
        }
    }

    #[test]
    fn pause_must_get_an_acknowledgment_from_a_running_worker() {
        let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel::<()>();
        let state = Mutex::new((
            true,
            Snapshot {
                error: "fixture live worker failure".into(),
                ..Default::default()
            },
        ));
        assert!(!acknowledge_stopped_pause(false, Some(&sender), &state).unwrap());
        let snapshot = state.lock().unwrap();
        assert!(snapshot.0);
        assert_eq!(snapshot.1.error, "fixture live worker failure");
    }
}
