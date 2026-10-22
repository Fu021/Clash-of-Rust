//! Retry discovery, never installer execution or acceptance of unverified bytes.
use anyhow::{Result, anyhow};
use std::{future::Future, time::Duration};

/// Re-fetch release metadata up to four times, with a shared overall deadline.
/// Dropping the caller's future also cancels requests and backoff sleeps.
pub async fn check<T, F, Fut>(operation: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    run(
        operation,
        &[
            Duration::from_millis(500),
            Duration::from_secs(1),
            Duration::from_secs(2),
        ],
        Duration::from_secs(75),
    )
    .await
}

async fn run<T, F, Fut>(mut operation: F, delays: &[Duration], budget: Duration) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let deadline = tokio::time::Instant::now() + budget;
    let mut last = None;
    for attempt in 0..=delays.len() {
        if attempt > 0
            && let Err(elapsed) =
                tokio::time::timeout_at(deadline, tokio::time::sleep(delays[attempt - 1])).await
        {
            return Err(failure(attempt, last, Some(elapsed)));
        }
        match tokio::time::timeout_at(deadline, operation()).await {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(error)) => last = Some(error),
            Err(elapsed) => return Err(failure(attempt + 1, last, Some(elapsed))),
        }
    }
    Err(failure(delays.len() + 1, last, None))
}

fn failure(
    attempts: usize,
    last: Option<anyhow::Error>,
    elapsed: Option<tokio::time::error::Elapsed>,
) -> anyhow::Error {
    let state = if elapsed.is_some() {
        "超时"
    } else {
        "失败"
    };
    let detail = last.map(|error| format!("：{error:#}")).unwrap_or_default();
    let message = format!("检查更新{state}（已尝试 {attempts} 次）{detail}");
    match elapsed {
        Some(elapsed) => anyhow::Error::new(elapsed).context(message),
        None => anyhow!(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn success_and_no_update_stop_immediately() {
        for value in [Some("0.6.3"), None] {
            let count = AtomicUsize::new(0);
            let result = run(
                || {
                    count.fetch_add(1, Ordering::Relaxed);
                    async { Ok(value) }
                },
                &[Duration::from_millis(1); 3],
                Duration::from_secs(1),
            )
            .await
            .unwrap();
            assert_eq!(result, value);
            assert_eq!(count.load(Ordering::Relaxed), 1);
        }
    }

    #[tokio::test]
    async fn repeated_errors_stop_after_four_attempts_and_retain_the_cause() {
        let count = AtomicUsize::new(0);
        let error = run::<(), _, _>(
            || {
                count.fetch_add(1, Ordering::Relaxed);
                async { Err(anyhow!("缺少本平台安装包或校验文件")) }
            },
            &[Duration::from_millis(1); 3],
            Duration::from_secs(1),
        )
        .await
        .unwrap_err()
        .to_string();
        assert_eq!(count.load(Ordering::Relaxed), 4);
        assert!(error.contains("4 次") && error.contains("校验文件"));
    }

    #[tokio::test]
    async fn total_deadline_bounds_a_stalled_request() {
        let error = run::<(), _, _>(
            std::future::pending::<Result<()>>,
            &[Duration::from_millis(1); 3],
            Duration::from_millis(10),
        )
        .await
        .unwrap_err();
        assert!(error.is::<tokio::time::error::Elapsed>());
        let error = error.to_string();
        assert!(error.contains("超时") && error.contains("1 次"));
    }
}
