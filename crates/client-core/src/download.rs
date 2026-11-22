//! Shared bounds for streamed subscriptions, Geo data, and installers.
use anyhow::{Context, Result, ensure};
use std::time::Duration;
use tokio::time::Instant;

pub const BUFFER_CAPACITY: usize = 256 * 1024;
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
const SPEED_WINDOW: Duration = Duration::from_secs(20);
const SAMPLE_WINDOW: Duration = Duration::from_secs(5);
const MIN_BYTES_PER_SECOND: u64 = 32 * 1024;
const DROP_CEILING_BYTES_PER_SECOND: u64 = 128 * 1024;
const DROP_BASELINE_BYTES_PER_SECOND: u64 = 256 * 1024;
const DROP_FACTOR: f64 = 4.0;

#[derive(Debug)]
struct SlowTransfer(&'static str);

impl std::fmt::Display for SlowTransfer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for SlowTransfer {}

pub async fn response(request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    response_with_timeout(request, STALL_TIMEOUT).await
}

pub fn require_complete_response(response: &reqwest::Response) -> Result<()> {
    ensure!(
        response.status() != reqwest::StatusCode::PARTIAL_CONTENT,
        "下载响应不完整（HTTP 206）"
    );
    Ok(())
}

async fn response_with_timeout(
    request: reqwest::RequestBuilder,
    timeout: Duration,
) -> Result<reqwest::Response> {
    tokio::time::timeout(timeout, request.send())
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "下载等待响应超时"))?
        .map_err(|error| crate::network::request_error(error, "下载"))
}

pub fn retryable(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|error| error.is_timeout() || error.is_body())
            || cause.is::<tokio::time::error::Elapsed>()
            || cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(retryable_io)
    })
}

fn retryable_io(mut error: &std::io::Error) -> bool {
    loop {
        if matches!(
            error.kind(),
            std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::UnexpectedEof
        ) {
            return true;
        }
        match error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<std::io::Error>())
        {
            Some(inner) => error = inner,
            None => return false,
        }
    }
}

pub fn monitor_slow(has_fallback: bool, attempt: usize) -> bool {
    has_fallback || attempt == 0
}

pub fn retry_same_route(error: &anyhow::Error, attempt: usize) -> bool {
    attempt == 0 && (retryable(error) || error.chain().any(|cause| cause.is::<SlowTransfer>()))
}

pub struct Watchdog {
    retry_slow: bool,
    window_started: Instant,
    window_bytes: u64,
    sample_started: Instant,
    sample_bytes: u64,
    peak_bytes_per_second: f64,
    last_returned: Option<Instant>,
}

impl Watchdog {
    pub fn new(retry_slow: bool) -> Self {
        let now = Instant::now();
        Self {
            retry_slow,
            window_started: now,
            window_bytes: 0,
            sample_started: now,
            sample_bytes: 0,
            peak_bytes_per_second: 0.0,
            last_returned: None,
        }
    }

    fn record(&mut self, bytes: u64, now: Instant) {
        self.window_bytes = self.window_bytes.saturating_add(bytes);
        if !self.retry_slow {
            return;
        }
        self.sample_bytes = self.sample_bytes.saturating_add(bytes);
        let elapsed = now.duration_since(self.sample_started);
        if elapsed >= SAMPLE_WINDOW {
            self.peak_bytes_per_second = self
                .peak_bytes_per_second
                .max(self.sample_bytes as f64 / elapsed.as_secs_f64());
            self.sample_started = now;
            self.sample_bytes = 0;
        }
    }

    fn check_window(&mut self, now: Instant) -> Result<()> {
        let elapsed = now.duration_since(self.window_started);
        if self.retry_slow && elapsed >= SPEED_WINDOW {
            let rate = self.window_bytes as f64 / elapsed.as_secs_f64();
            if rate < MIN_BYTES_PER_SECOND as f64 {
                return Err(SlowTransfer("下载持续低于 32 KiB/s").into());
            }
            if rate < DROP_CEILING_BYTES_PER_SECOND as f64
                && self.peak_bytes_per_second >= DROP_BASELINE_BYTES_PER_SECOND as f64
                && rate * DROP_FACTOR < self.peak_bytes_per_second
            {
                return Err(SlowTransfer("下载速度持续下降").into());
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
        self.chunk_with_idle_timeout(response, STALL_TIMEOUT).await
    }

    async fn chunk_with_idle_timeout(
        &mut self,
        response: &mut reqwest::Response,
        idle_timeout: Duration,
    ) -> Result<Option<impl AsRef<[u8]> + Send + use<>>> {
        require_complete_response(response)?;
        if let Some(last_returned) = self.last_returned.take() {
            let processing_time = Instant::now().duration_since(last_returned);
            self.window_started += processing_time;
            self.sample_started += processing_time;
        }
        // reqwest's per-read timeout alone cannot detect a trickle of tiny chunks.
        let idle_deadline = Instant::now() + idle_timeout;
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
                        let now = Instant::now();
                        self.record(bytes.len() as u64, now);
                        self.check_window(now)?;
                        self.last_returned = Some(now);
                    }
                    return Ok(chunk);
                }
                Err(_) => {
                    self.check_window(Instant::now())?;
                    if Instant::now() >= idle_deadline {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "下载超过 30 秒没有收到数据",
                        )
                        .into());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) async fn serve_responses(
        responses: Vec<Vec<u8>>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for wire in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut buffer = [0; 1024];
                    let size = socket.read(&mut buffer).await.unwrap();
                    assert!(size > 0 && request.len() + size <= 8192);
                    request.extend_from_slice(&buffer[..size]);
                }
                socket.write_all(&wire).await.unwrap();
            }
        });
        (format!("http://{address}"), server)
    }

    #[tokio::test]
    async fn partial_subscription_response_falls_back_to_a_complete_download() {
        let (proxy, partial) = serve_responses(vec![
            b"HTTP/1.1 206 Partial Content\r\nContent-Length: 12\r\nContent-Range: bytes 0-11/29\r\nsubscription-userinfo: upload=1\r\n\r\nproxies: []\n".to_vec(),
        ])
        .await;
        let (origin, complete) = serve_responses(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 29\r\nsubscription-userinfo: upload=2\r\n\r\nproxies: []\nproxy-groups: []\n".to_vec(),
        ])
        .await;
        let url = reqwest::Url::parse(&origin).unwrap();
        let routes = crate::network::routes(None, Some(proxy));
        let (raw, usage) = crate::subscription::download_routes(&url, &routes)
            .await
            .unwrap();
        assert_eq!(raw, "proxies: []\nproxy-groups: []\n");
        assert_eq!(usage.as_deref(), Some("upload=2"));
        partial.await.unwrap();
        complete.await.unwrap();
    }

    #[tokio::test]
    async fn truncated_subscription_reconnects_then_uses_the_next_route() {
        for truncated in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 29\r\nConnection: close\r\nsubscription-userinfo: upload=1\r\n\r\nproxies: []\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\nsubscription-userinfo: upload=1\r\n\r\nc\r\nproxies: []\n\r\n".as_slice(),
        ] {
            let (proxy, interrupted) =
                serve_responses(vec![truncated.to_vec(), truncated.to_vec()]).await;
            let (origin, complete) = serve_responses(vec![
                b"HTTP/1.1 200 OK\r\nContent-Length: 29\r\nsubscription-userinfo: upload=2\r\n\r\nproxies: []\nproxy-groups: []\n".to_vec(),
            ])
            .await;
            let url = reqwest::Url::parse(&origin).unwrap();
            let routes = crate::network::routes(None, Some(proxy));
            let (raw, usage) = tokio::time::timeout(
                Duration::from_secs(5),
                crate::subscription::download_routes(&url, &routes),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(raw, "proxies: []\nproxy-groups: []\n");
            assert_eq!(usage.as_deref(), Some("upload=2"));
            tokio::time::timeout(Duration::from_secs(2), interrupted)
                .await
                .expect("the interrupted route should be attempted twice")
                .unwrap();
            complete.await.unwrap();
        }
    }

    #[tokio::test]
    async fn watchdog_rejects_a_partial_response_before_reading_its_body() {
        let (origin, server) = serve_responses(vec![
            b"HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\nContent-Range: bytes 0-3/8\r\n\r\ndone".to_vec(),
        ])
        .await;
        let mut response = response(
            crate::network::client_builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(origin),
        )
        .await
        .unwrap();
        let mut watchdog = Watchdog::new(false);
        let error = match watchdog.chunk(&mut response).await {
            Err(error) => error,
            Ok(_) => panic!("HTTP 206 must not be accepted as a complete response"),
        };
        assert!(error.to_string().contains("HTTP 206"), "{error:#}");
        assert!(!retry_same_route(&error, 0));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn final_slow_route_still_times_out_when_the_body_stops() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\na")
                .await
                .unwrap();
            match socket.read(&mut request).await {
                Ok(0) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                    ) => {}
                result => panic!("download connection remained open: {result:?}"),
            }
        });
        let mut response = response(
            crate::network::client_builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!("http://{address}/package")),
        )
        .await
        .unwrap();
        let mut watchdog = Watchdog::new(false);
        assert_eq!(
            watchdog
                .chunk(&mut response)
                .await
                .unwrap()
                .unwrap()
                .as_ref(),
            b"a"
        );
        let processing_time = SPEED_WINDOW + Duration::from_secs(1);
        watchdog.window_started -= processing_time;
        watchdog.sample_started -= processing_time;
        watchdog.last_returned = watchdog
            .last_returned
            .map(|instant| instant - processing_time);
        let started = Instant::now();
        let idle_timeout = Duration::from_millis(100);
        let error = match tokio::time::timeout(
            Duration::from_secs(3),
            watchdog.chunk_with_idle_timeout(&mut response, idle_timeout),
        )
        .await
        .unwrap()
        {
            Err(error) => error,
            Ok(_) => panic!("stalled download must time out"),
        };
        assert!(started.elapsed() >= idle_timeout);
        assert!(error.to_string().contains("没有收到数据"), "{error:#}");
        assert!(retryable(&error));
        drop(response);
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn completed_body_does_not_retry_when_the_caller_checks_eof_after_the_speed_window() {
        let (origin, server) = serve_responses(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ndone".to_vec(),
        ])
        .await;
        let mut response = response(
            crate::network::client_builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(origin),
        )
        .await
        .unwrap();
        let mut watchdog = Watchdog::new(true);
        let bytes = watchdog.chunk(&mut response).await.unwrap().unwrap();
        assert_eq!(bytes.as_ref(), b"done");

        watchdog.window_started = Instant::now() - SPEED_WINDOW;
        assert!(watchdog.chunk(&mut response).await.unwrap().is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn caller_processing_time_does_not_trigger_a_slow_download_retry() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (resume, resume_rx) = tokio::sync::oneshot::channel();
        let (sent, sent_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nfirst")
                .await
                .unwrap();
            resume_rx.await.unwrap();
            socket.write_all(b"end").await.unwrap();
            sent.send(()).unwrap();
        });
        let mut response = response(
            crate::network::client_builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!("http://{address}/package")),
        )
        .await
        .unwrap();
        let mut watchdog = Watchdog::new(true);
        assert_eq!(
            watchdog
                .chunk(&mut response)
                .await
                .unwrap()
                .unwrap()
                .as_ref(),
            b"first"
        );
        let processing_time = SPEED_WINDOW + Duration::from_secs(1);
        watchdog.window_started -= processing_time;
        watchdog.sample_started -= processing_time;
        watchdog.last_returned = watchdog
            .last_returned
            .map(|instant| instant - processing_time);
        resume.send(()).unwrap();
        sent_rx.await.unwrap();

        assert_eq!(
            watchdog
                .chunk(&mut response)
                .await
                .unwrap()
                .unwrap()
                .as_ref(),
            b"end"
        );
        assert!(watchdog.chunk(&mut response).await.unwrap().is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_ready_chunk_counts_toward_the_expired_speed_window() {
        let body = vec![b'.'; 8 * 1024];
        let mut wire =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        wire.extend_from_slice(&body);
        let (origin, server) = serve_responses(vec![wire]).await;
        let mut response = response(
            crate::network::client_builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(origin),
        )
        .await
        .unwrap();
        let mut watchdog = Watchdog::new(true);
        watchdog.window_started = Instant::now() - SPEED_WINDOW;
        watchdog.window_bytes = MIN_BYTES_PER_SECOND * SPEED_WINDOW.as_secs() - 1;

        let mut received = Vec::new();
        let first = watchdog.chunk(&mut response).await.unwrap().unwrap();
        assert_eq!(watchdog.window_bytes, 0);
        received.extend_from_slice(first.as_ref());
        while let Some(chunk) = watchdog.chunk(&mut response).await.unwrap() {
            received.extend_from_slice(chunk.as_ref());
        }
        assert_eq!(received, body);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn tls_eof_keeps_the_transport_cause_available_for_bounded_reconnect() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut hello = [0; 4096];
            assert!(socket.read(&mut hello).await.unwrap() > 0);
        });
        let error = response(
            crate::network::client_builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!("https://{address}/package")),
        )
        .await
        .unwrap_err();
        server.await.unwrap();
        assert!(retryable(&error), "{error:#}");
    }

    #[tokio::test]
    async fn tiny_continuous_chunks_do_not_reset_the_slow_route_deadline() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10000\r\n\r\n")
                .await
                .unwrap();
            loop {
                if socket.write_all(b".").await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        let mut response = response(
            crate::network::client_builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!("http://{address}/package")),
        )
        .await
        .unwrap();
        let mut watchdog = Watchdog::new(true);
        watchdog.window_started = Instant::now() - SPEED_WINDOW + Duration::from_millis(100);
        let error = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Err(error) = watchdog.chunk(&mut response).await {
                    return error;
                }
            }
        })
        .await
        .unwrap();
        assert!(error.to_string().contains("32 KiB/s"), "{error:#}");
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn response_deadline_bounds_a_connection_that_never_sends_headers() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            assert!(socket.read(&mut bytes).await.unwrap() > 0);
            std::future::pending::<()>().await;
            drop(socket);
        });
        let error = response_with_timeout(
            crate::network::client_builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!("http://{address}/package")),
            Duration::from_millis(100),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("等待响应超时"), "{error:#}");
        assert!(retryable(&error));
        server.abort();
        let _ = server.await;
    }

    #[test]
    fn retries_only_transient_connections_not_integrity_disk_or_speed_failures() {
        for kind in [
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::UnexpectedEof,
        ] {
            let error = anyhow::Error::new(std::io::Error::from(kind)).context("下载中断");
            assert!(retryable(&error));
            let wrapped = std::io::Error::other(std::io::Error::other(std::io::Error::from(kind)));
            assert!(retryable(&wrapped.into()));
        }
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::InvalidData,
        ] {
            assert!(!retryable(&std::io::Error::from(kind).into()));
            let wrapped = std::io::Error::other(std::io::Error::other(std::io::Error::from(kind)));
            assert!(!retryable(&wrapped.into()));
        }
        assert!(!retryable(&anyhow::anyhow!("SHA256 校验失败")));
        let mut slow = Watchdog::new(true);
        let error = slow
            .check_window(slow.window_started + SPEED_WINDOW)
            .unwrap_err();
        assert!(!retryable(&error));
    }

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

    #[test]
    fn sustained_drop_uses_observed_speed_without_rejecting_steady_slow_networks() {
        let mut dropped = Watchdog::new(true);
        let start = dropped.window_started;
        dropped.record(512 * 1024 * 5, start + Duration::from_secs(5));
        for seconds in [10, 15, 20] {
            dropped.record(48 * 1024 * 5, start + Duration::from_secs(seconds));
        }
        dropped.check_window(start + SPEED_WINDOW).unwrap();
        for seconds in [25, 30, 35, 40] {
            dropped.record(48 * 1024 * 5, start + Duration::from_secs(seconds));
        }
        let error = dropped
            .check_window(start + SPEED_WINDOW + SPEED_WINDOW)
            .unwrap_err();
        assert!(error.to_string().contains("速度持续下降"));

        let mut steady = Watchdog::new(true);
        let start = steady.window_started;
        for seconds in [5, 10, 15, 20, 25, 30, 35, 40] {
            steady.record(48 * 1024 * 5, start + Duration::from_secs(seconds));
            steady
                .check_window(start + Duration::from_secs(seconds))
                .unwrap();
        }
    }

    #[test]
    fn slow_last_route_reconnects_once_and_then_permits_slow_completion() {
        let error = anyhow::Error::new(SlowTransfer("slow"));
        assert!(monitor_slow(true, 0));
        assert!(monitor_slow(false, 0));
        assert!(!monitor_slow(false, 1));
        assert!(retry_same_route(&error, 0));
        assert!(!retry_same_route(&error, 1));
        assert!(!retry_same_route(&anyhow::anyhow!("SHA256 校验失败"), 0));
    }

    #[tokio::test]
    async fn outer_request_timeout_is_retryable() {
        let error = tokio::time::timeout(Duration::from_millis(1), std::future::pending::<()>())
            .await
            .unwrap_err();
        assert!(retry_same_route(&anyhow::Error::new(error), 0));
    }

    #[tokio::test]
    async fn continuous_dropped_speed_above_the_absolute_floor_is_detected() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10000000\r\n\r\n")
                .await
                .unwrap();
            loop {
                if socket.write_all(b".").await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        let mut response = response(
            crate::network::client_builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!("http://{address}/package")),
        )
        .await
        .unwrap();
        let mut watchdog = Watchdog::new(true);
        watchdog.window_started = Instant::now() - SPEED_WINDOW + Duration::from_millis(100);
        watchdog.window_bytes = 48 * 1024 * SPEED_WINDOW.as_secs();
        watchdog.peak_bytes_per_second = 512.0 * 1024.0;
        let error = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Err(error) = watchdog.chunk(&mut response).await {
                    return error;
                }
            }
        })
        .await
        .unwrap();
        assert!(error.to_string().contains("速度持续下降"), "{error:#}");
        server.abort();
        let _ = server.await;
    }
}
