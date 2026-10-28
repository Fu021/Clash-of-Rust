//! Shared bounds for streamed Geo and installer bodies.
use anyhow::{Context, Result, bail};
use std::time::Duration;
use tokio::time::Instant;

pub const BUFFER_CAPACITY: usize = 256 * 1024;
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
const SPEED_WINDOW: Duration = Duration::from_secs(20);
const MIN_BYTES_PER_SECOND: u64 = 32 * 1024;

pub struct Watchdog {
    retry_slow: bool,
    window_started: Instant,
    window_bytes: u64,
}

impl Watchdog {
    /// Only reject a slow route when another route can still be attempted.
    /// The final route retains the idle and overall request timeouts.
    pub fn new(retry_slow: bool) -> Self {
        Self {
            retry_slow,
            window_started: Instant::now(),
            window_bytes: 0,
        }
    }

    fn check_window(&mut self, now: Instant) -> Result<()> {
        let elapsed = now.duration_since(self.window_started);
        if self.retry_slow && elapsed >= SPEED_WINDOW {
            if (self.window_bytes as f64) < MIN_BYTES_PER_SECOND as f64 * elapsed.as_secs_f64() {
                bail!("下载持续低于 32 KiB/s，尝试下一条连接路线");
            }
            self.window_started = now;
            self.window_bytes = 0;
        }
        Ok(())
    }

    pub async fn chunk(
        &mut self,
        response: &mut reqwest::Response,
    ) -> Result<Option<impl AsRef<[u8]> + Send + use<>>> {
        // reqwest's per-read timeout alone cannot detect a trickle of tiny chunks.
        self.check_window(Instant::now())?;
        let idle_deadline = Instant::now() + STALL_TIMEOUT;
        loop {
            let deadline = if self.retry_slow {
                idle_deadline.min(self.window_started + SPEED_WINDOW)
            } else {
                idle_deadline
            };
            match tokio::time::timeout_at(deadline, response.chunk()).await {
                Ok(result) => {
                    let chunk = result.context("下载中断或超时")?;
                    if let Some(bytes) = &chunk {
                        self.window_bytes += bytes.len() as u64;
                    }
                    return Ok(chunk);
                }
                Err(_) => {
                    self.check_window(Instant::now())?;
                    if Instant::now() >= idle_deadline {
                        bail!("下载超过 30 秒没有收到数据，尝试下一条连接路线")
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_trickles_are_retried_but_fast_bursts_and_the_final_route_are_preserved() {
        let mut slow = Watchdog::new(true);
        let start = slow.window_started;
        slow.window_bytes = 64 * 1024;
        slow.check_window(start + Duration::from_secs(19)).unwrap();
        assert!(slow.check_window(start + SPEED_WINDOW).is_err());

        let mut fast = Watchdog::new(true);
        let start = fast.window_started;
        fast.window_bytes = MIN_BYTES_PER_SECOND * SPEED_WINDOW.as_secs();
        fast.check_window(start + SPEED_WINDOW).unwrap();
        assert_eq!(fast.window_bytes, 0);
        assert!(
            fast.check_window(start + SPEED_WINDOW + SPEED_WINDOW)
                .is_err()
        );

        let mut last_route = Watchdog::new(false);
        last_route
            .check_window(last_route.window_started + Duration::from_secs(60))
            .unwrap();
    }
}
