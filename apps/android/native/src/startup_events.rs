use anyhow::{Context, Result, ensure};
use clash_mobile_runtime::core::config::atomic_write;
use serde_json::Value;
use std::{
    collections::VecDeque,
    fs,
    io::ErrorKind,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI32, AtomicU8, Ordering},
    },
};
use tokio::sync::{mpsc, watch};

const EVENT_CAPACITY: usize = 16;
static AUTHORIZATION_ALLOCATION: Mutex<()> = Mutex::new(());

pub(crate) fn next_vpn_authorization(root: &Path) -> Result<i64> {
    let _allocation = AUTHORIZATION_ALLOCATION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let path = root.join("vpn-authorization-sequence");
    let previous = match fs::read_to_string(&path) {
        Ok(text) => {
            let previous: i64 = text.parse().context("Invalid VPN authorization sequence")?;
            ensure!(previous < 0, "Invalid VPN authorization sequence");
            previous
        }
        Err(error) if error.kind() == ErrorKind::NotFound => 0,
        Err(error) => return Err(error).context("Cannot read the VPN authorization sequence"),
    };
    let request = previous
        .checked_sub(1)
        .context("VPN authorization sequence exhausted")?;
    atomic_write(&path, request.to_string().as_bytes())
        .context("Cannot save the VPN authorization sequence")?;
    Ok(request)
}

pub(crate) fn vpn_authorization_result(event: &Value, request: i64) -> Option<&Value> {
    let result = &event["value"];
    (event["event"] == "vpn-authorized" && result["request"].as_i64() == Some(request))
        .then_some(result)
}

pub(crate) struct NativeEvents {
    queue: Mutex<Option<NativeEventSender>>,
    visible: watch::Sender<bool>,
}

#[derive(Default)]
struct EventBuffer {
    events: VecDeque<Value>,
    ordinary: usize,
    authorization: Option<i64>,
}

struct NativeEventSender {
    buffer: Arc<Mutex<EventBuffer>>,
    wake: mpsc::Sender<()>,
}

pub(crate) struct NativeEventReceiver {
    buffer: Arc<Mutex<EventBuffer>>,
    wake: mpsc::Receiver<()>,
}

impl NativeEventReceiver {
    fn pop(&self) -> Option<Value> {
        let mut buffer = self
            .buffer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let event = buffer.events.pop_front()?;
        if event["event"] != "vpn-authorized" {
            buffer.ordinary -= 1;
        }
        Some(event)
    }

    pub(crate) async fn recv(&mut self) -> Option<Value> {
        loop {
            if let Some(event) = self.pop() {
                return Some(event);
            }
            self.wake.recv().await?;
        }
    }
}

impl NativeEvents {
    pub(crate) fn new() -> Self {
        Self {
            queue: Mutex::new(None),
            visible: watch::Sender::new(true),
        }
    }

    pub(crate) fn connect(&self) -> (NativeEventReceiver, watch::Receiver<bool>) {
        let buffer = Arc::new(Mutex::new(EventBuffer::default()));
        let (send, wake) = mpsc::channel(1);
        *self.queue.lock().unwrap_or_else(|error| error.into_inner()) = Some(NativeEventSender {
            buffer: buffer.clone(),
            wake: send,
        });
        (
            NativeEventReceiver { buffer, wake },
            self.visible.subscribe(),
        )
    }

    pub(crate) fn disconnect(&self) {
        self.queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }

    pub(crate) fn expect_authorization(&self, request: i64) -> bool {
        let queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        let Some(sender) = queue.as_ref() else {
            return false;
        };
        let mut buffer = sender
            .buffer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        buffer
            .events
            .retain(|event| event["event"] != "vpn-authorized");
        buffer.authorization = Some(request);
        true
    }

    pub(crate) fn cancel_authorization(&self, request: i64) {
        let queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(sender) = queue.as_ref() {
            let mut buffer = sender
                .buffer
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if buffer.authorization == Some(request) {
                buffer.authorization = None;
            }
            buffer
                .events
                .retain(|event| vpn_authorization_result(event, request).is_none());
        }
    }

    pub(crate) fn dispatch(&self, event: Value) {
        if event["event"] == "visible" {
            let current = event["value"].as_bool().unwrap_or(true);
            self.visible.send_if_modified(|previous| {
                let changed = *previous != current;
                *previous = current;
                changed
            });
        } else if let Ok(queue) = self.queue.lock()
            && let Some(sender) = queue.as_ref()
        {
            let mut buffer = sender
                .buffer
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if event["event"] == "vpn-authorized" {
                if !buffer
                    .authorization
                    .is_some_and(|request| vpn_authorization_result(&event, request).is_some())
                {
                    return;
                }
                buffer.authorization = None;
            } else if buffer.ordinary < EVENT_CAPACITY {
                buffer.ordinary += 1;
            } else {
                return;
            }
            buffer.events.push_back(event);
            let _ = sender.wake.try_send(());
        }
    }
}

#[derive(Default)]
pub(crate) struct StartupEvents(VecDeque<Value>);

impl StartupEvents {
    pub(crate) fn record(&mut self, event: Value) {
        if !matches!(
            event["event"].as_str(),
            Some("vpn" | "vpn-cancelled" | "vpn-authorized")
        ) && self.0.len() < EVENT_CAPACITY
        {
            self.0.push_back(event);
        }
    }

    pub(crate) async fn next(&mut self, live: &mut NativeEventReceiver) -> Option<Value> {
        match self.0.pop_front() {
            Some(event) => Some(event),
            None => live.recv().await,
        }
    }
}

#[derive(Default)]
pub(crate) struct StartupRecovery {
    phase: AtomicU8,
    page: AtomicI32,
}

impl StartupRecovery {
    const OPENING: u8 = 0;
    const FAILED: u8 = 1;
    const READY: u8 = 2;

    pub(crate) fn opening(&self) -> bool {
        self.phase.load(Ordering::Acquire) == Self::OPENING
    }

    pub(crate) fn ready(&self) -> bool {
        self.phase.load(Ordering::Acquire) == Self::READY
    }

    pub(crate) fn fail(&self) {
        self.phase.store(Self::FAILED, Ordering::Release);
    }

    pub(crate) fn retry(&self) -> bool {
        self.phase
            .compare_exchange(
                Self::FAILED,
                Self::OPENING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    pub(crate) fn navigate(&self, page: i32) {
        self.page.store(page, Ordering::Release);
    }

    pub(crate) fn page(&self) -> i32 {
        self.page.load(Ordering::Acquire)
    }

    pub(crate) fn publish(&self, page: i32) -> bool {
        page == self.page()
            && self
                .phase
                .compare_exchange(
                    Self::OPENING,
                    Self::READY,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn latest_visibility_survives_a_full_queue_and_receiver_replacement() {
        let ingress = NativeEvents::new();
        ingress.dispatch(json!({"event":"visible","value":false}));
        let (events, mut visible) = ingress.connect();
        assert!(!*visible.borrow());
        for index in 0..EVENT_CAPACITY {
            ingress.dispatch(json!({"event":"notice","value":index}));
        }
        ingress.dispatch(json!({"event":"visible","value":true}));
        assert!(visible.has_changed().unwrap());
        assert!(*visible.borrow_and_update());
        for index in 0..EVENT_CAPACITY {
            assert_eq!(events.pop().unwrap()["value"], index);
        }
        assert!(events.pop().is_none());
        ingress.disconnect();
        ingress.dispatch(json!({"event":"visible","value":false}));
        let (_, visible) = ingress.connect();
        assert!(!*visible.borrow());
    }

    #[tokio::test]
    async fn full_ordinary_queue_still_delivers_only_the_current_authorization_in_order() {
        let ingress = NativeEvents::new();
        let (mut events, _) = ingress.connect();
        assert!(ingress.expect_authorization(2));
        for index in 0..EVENT_CAPACITY {
            ingress.dispatch(json!({"event":"notice","value":index}));
        }
        ingress.dispatch(json!({"event":"notice","value":"overflow"}));
        ingress.dispatch(json!({"event":"vpn-authorized","value":{"request":1,"granted":true}}));
        let denial = json!({"event":"vpn-authorized","value":{"request":2,"granted":false}});
        ingress.dispatch(denial.clone());
        ingress.dispatch(json!({"event":"vpn-authorized","value":{"request":2,"granted":true}}));
        assert_eq!(events.recv().await.unwrap()["value"], 0);
        ingress.dispatch(json!({"event":"notice","value":"after authorization"}));
        for index in 1..EVENT_CAPACITY {
            assert_eq!(events.pop().unwrap()["value"], index);
        }
        assert_eq!(events.pop(), Some(denial));
        assert_eq!(events.pop().unwrap()["value"], "after authorization");
        assert!(events.pop().is_none());
    }

    #[test]
    fn cancelled_or_replaced_authorization_cannot_complete_a_retry() {
        let ingress = NativeEvents::new();
        let (events, _) = ingress.connect();
        assert!(ingress.expect_authorization(3));
        ingress.dispatch(json!({"event":"vpn-authorized","value":{"request":3,"granted":true}}));
        ingress.cancel_authorization(3);
        assert!(ingress.expect_authorization(4));
        ingress.dispatch(json!({"event":"vpn-authorized","value":{"request":3,"granted":true}}));
        let grant = json!({"event":"vpn-authorized","value":{"request":4,"granted":true}});
        ingress.dispatch(grant.clone());
        assert_eq!(events.pop(), Some(grant));
        assert!(events.pop().is_none());
        ingress.disconnect();
        assert!(!ingress.expect_authorization(5));
    }

    #[test]
    fn reopened_authorization_sequence_rejects_old_bundle_results() {
        let root = tempfile::tempdir().unwrap();
        let previous = next_vpn_authorization(root.path()).unwrap();
        assert_eq!(previous, -1);

        let ingress = NativeEvents::new();
        let (events, _) = ingress.connect();
        let current = next_vpn_authorization(root.path()).unwrap();
        assert_eq!(current, -2);
        assert!(ingress.expect_authorization(current));
        for stale in [1, previous] {
            ingress.dispatch(
                json!({"event":"vpn-authorized","value":{"request":stale,"granted":true}}),
            );
            assert!(events.pop().is_none());
        }
        let denial = json!({"event":"vpn-authorized","value":{"request":current,"granted":false}});
        ingress.dispatch(denial.clone());
        assert_eq!(events.pop(), Some(denial));
    }

    #[test]
    fn invalid_or_exhausted_authorization_sequence_is_not_reset() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("vpn-authorization-sequence");
        for saved in ["", "0", "1", "-9223372036854775808"] {
            fs::write(&path, saved).unwrap();
            assert!(next_vpn_authorization(root.path()).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), saved);
        }
    }

    #[tokio::test]
    async fn recovery_replays_pending_notices_in_order_without_replaying_vpn_results() {
        let ingress = NativeEvents::new();
        let (mut events, _) = ingress.connect();
        let mut pending = StartupEvents::default();
        pending.record(json!({"event":"error","value":"old error"}));
        pending.record(json!({"event":"vpn-cancelled","value":true}));
        pending.record(json!({"event":"vpn-authorized","value":{"request":1,"granted":false}}));
        pending.record(json!({"event":"vpn","value":false}));
        pending.record(json!({"event":"critical","value":"next error"}));
        ingress.dispatch(json!({"event":"notice","value":"live notice"}));
        for expected in ["old error", "next error", "live notice"] {
            assert_eq!(pending.next(&mut events).await.unwrap()["value"], expected);
        }
    }

    #[test]
    fn only_a_matching_authorization_result_can_complete_a_protection_request() {
        let request = 2;
        let cancelled_before_recovery = json!({"event":"vpn-cancelled","value":true});
        let previous_result =
            json!({"event":"vpn-authorized","value":{"request":1,"granted":false}});
        let current_result =
            json!({"event":"vpn-authorized","value":{"request":2,"granted":false}});
        assert!(vpn_authorization_result(&cancelled_before_recovery, request).is_none());
        assert!(vpn_authorization_result(&previous_result, request).is_none());
        assert_eq!(
            vpn_authorization_result(&current_result, request).unwrap()["granted"],
            false
        );
    }

    #[test]
    fn retry_stays_reserved_until_failure_or_first_projection_of_the_current_page() {
        let recovery = StartupRecovery::default();
        assert!(!recovery.retry());
        recovery.fail();
        assert!(recovery.retry());
        assert!(!recovery.retry());
        recovery.navigate(9);
        assert!(!recovery.publish(0));
        assert!(recovery.opening());
        recovery.fail();
        assert!(recovery.retry());
        assert!(recovery.publish(9));
        assert!(recovery.ready());
        assert!(!recovery.retry());
    }
}
