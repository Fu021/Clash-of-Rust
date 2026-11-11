use super::*;
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementCode {
    NodeTimeout,
    NodeFailed,
    ControllerConnect,
    ControllerTimeout,
    ControllerAuthentication,
    ControllerHttp,
    ControllerResponse,
    MissingNode,
    InvalidProbe,
    QueryFailed,
}

impl MeasurementCode {
    fn message(self) -> &'static str {
        match self {
            Self::NodeTimeout => "节点测速超时",
            Self::NodeFailed => "节点测速未成功",
            Self::ControllerConnect => "无法连接内核控制接口",
            Self::ControllerTimeout => "内核控制请求超时",
            Self::ControllerAuthentication => "内核控制接口认证失败",
            Self::ControllerHttp => "内核控制接口返回异常状态",
            Self::ControllerResponse => "内核控制接口响应无效",
            Self::MissingNode => "测速节点不存在",
            Self::InvalidProbe => "测速参数或目标无效",
            Self::QueryFailed => "无法完成测速准备或策略组更新",
        }
    }

    fn node_failure(self) -> bool {
        matches!(self, Self::NodeTimeout | Self::NodeFailed)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MeasurementFailure {
    pub code: MeasurementCode,
    pub http_status: Option<u16>,
}

impl MeasurementFailure {
    pub(super) fn new(code: MeasurementCode, http_status: Option<u16>) -> Self {
        Self { code, http_status }
    }

    fn classify(error: &anyhow::Error) -> Self {
        if let Some(failure) = error.downcast_ref::<Self>() {
            return *failure;
        }
        if let Some(status) = error.downcast_ref::<ApiResponseError>() {
            return Self::new(
                if matches!(status.0.as_u16(), 401 | 403) {
                    MeasurementCode::ControllerAuthentication
                } else {
                    MeasurementCode::ControllerHttp
                },
                Some(status.0.as_u16()),
            );
        }
        let code = match error.downcast_ref::<reqwest::Error>() {
            Some(error) if error.is_timeout() => MeasurementCode::ControllerTimeout,
            Some(error) if error.is_connect() => MeasurementCode::ControllerConnect,
            Some(error) if error.is_decode() => MeasurementCode::ControllerResponse,
            _ if error.downcast_ref::<serde_json::Error>().is_some() => {
                MeasurementCode::ControllerResponse
            }
            _ => MeasurementCode::QueryFailed,
        };
        Self::new(code, None)
    }
}

impl std::fmt::Display for MeasurementFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code.message())?;
        if let Some(status) = self.http_status {
            write!(formatter, "（HTTP {status}）")?;
        }
        Ok(())
    }
}

impl std::error::Error for MeasurementFailure {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasurementDiagnostic {
    pub schema: u32,
    pub scope: String,
    pub started_at: String,
    pub duration_ms: u64,
    pub phase: String,
    pub completed: bool,
    pub probe_timeout_ms: u32,
    pub concurrency: usize,
    pub attempted: usize,
    pub succeeded: usize,
    pub node_failures: usize,
    pub request_failures: usize,
    pub targets: BTreeSet<String>,
    pub codes: BTreeMap<MeasurementCode, usize>,
    pub http_statuses: BTreeMap<u16, usize>,
    pub failure: Option<MeasurementFailure>,
}

pub(super) type MeasurementHistory = Arc<Mutex<VecDeque<MeasurementDiagnostic>>>;

pub(super) struct Measurement {
    started: Instant,
    diagnostic: MeasurementDiagnostic,
}

impl Measurement {
    pub(super) fn new(scope: &str) -> Self {
        Self {
            started: Instant::now(),
            diagnostic: MeasurementDiagnostic {
                schema: 1,
                scope: scope.into(),
                started_at: chrono::Utc::now().to_rfc3339(),
                duration_ms: 0,
                phase: "query_nodes".into(),
                completed: false,
                probe_timeout_ms: 5_000,
                concurrency: 6,
                attempted: 0,
                succeeded: 0,
                node_failures: 0,
                request_failures: 0,
                targets: BTreeSet::new(),
                codes: BTreeMap::new(),
                http_statuses: BTreeMap::new(),
                failure: None,
            },
        }
    }

    pub(super) fn phase(&mut self, phase: &str) {
        self.diagnostic.phase = phase.into();
    }

    pub(super) fn observe(&mut self, result: Result<u32>, target: &str) -> Result<u32> {
        self.diagnostic.attempted += 1;
        let target = match target {
            "https://www.gstatic.com/generate_204" => "gstatic_generate_204",
            "https://www.cloudflare.com/cdn-cgi/trace" => "cloudflare_trace",
            _ => "subscription_custom",
        };
        self.diagnostic.targets.insert(target.into());
        let failure = match result {
            Ok(delay) if delay > 0 => {
                self.diagnostic.succeeded += 1;
                return Ok(delay);
            }
            Ok(_) => MeasurementFailure::new(MeasurementCode::NodeFailed, Some(200)),
            Err(error) => MeasurementFailure::classify(&error),
        };
        *self.diagnostic.codes.entry(failure.code).or_default() += 1;
        if let Some(status) = failure.http_status {
            *self.diagnostic.http_statuses.entry(status).or_default() += 1;
        }
        if failure.code.node_failure() {
            self.diagnostic.node_failures += 1;
            Ok(0)
        } else {
            self.diagnostic.request_failures += 1;
            Err(failure.into())
        }
    }
}

impl Api {
    pub fn measurement_diagnostics(&self) -> Vec<MeasurementDiagnostic> {
        self.measurements
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    pub fn restore_measurement_diagnostics(&self, mut reports: Vec<MeasurementDiagnostic>) {
        reports.sort_by(|left, right| left.started_at.cmp(&right.started_at));
        reports.dedup_by(|left, right| {
            left.started_at == right.started_at && left.scope == right.scope
        });
        *self
            .measurements
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = reports
            .into_iter()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
    }

    pub(super) fn finish_measurement<T>(&self, mut measurement: Measurement, result: &Result<T>) {
        measurement.diagnostic.duration_ms = measurement
            .started
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        measurement.diagnostic.completed = result.is_ok();
        if let Err(error) = result {
            measurement.diagnostic.failure = Some(MeasurementFailure::classify(error));
        } else {
            measurement.phase("complete");
        }
        let mut history = self
            .measurements
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        history.push_back(measurement.diagnostic);
        while history.len() > 4 {
            history.pop_front();
        }
    }

    pub(super) async fn measurement_response(response: reqwest::Response) -> Result<u32> {
        #[derive(Deserialize)]
        struct MeasuredDelay {
            delay: u32,
        }
        let status = response.status().as_u16();
        let code = match status {
            200 => None,
            503 => Some(MeasurementCode::NodeFailed),
            504 => Some(MeasurementCode::NodeTimeout),
            401 | 403 => Some(MeasurementCode::ControllerAuthentication),
            404 => Some(MeasurementCode::MissingNode),
            400 | 422 => Some(MeasurementCode::InvalidProbe),
            _ => Some(MeasurementCode::ControllerHttp),
        };
        if let Some(code) = code {
            return Err(MeasurementFailure::new(code, Some(status)).into());
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 4096 {
                return Err(MeasurementFailure::new(
                    MeasurementCode::ControllerResponse,
                    Some(status),
                )
                .into());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice::<MeasuredDelay>(&bytes)
            .map(|response| response.delay)
            .map_err(|_| {
                MeasurementFailure::new(MeasurementCode::ControllerResponse, Some(status)).into()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_failures_are_never_reported_as_failed_nodes() {
        for code in [
            MeasurementCode::ControllerConnect,
            MeasurementCode::ControllerTimeout,
            MeasurementCode::ControllerAuthentication,
            MeasurementCode::ControllerHttp,
            MeasurementCode::ControllerResponse,
            MeasurementCode::MissingNode,
            MeasurementCode::InvalidProbe,
            MeasurementCode::QueryFailed,
        ] {
            let mut measurement = Measurement::new("all");
            let result = measurement.observe(
                Err(MeasurementFailure::new(code, None).into()),
                "https://user:secret@example.test/private?token=secret",
            );
            assert!(result.is_err());
            assert_eq!(measurement.diagnostic.node_failures, 0);
            assert_eq!(measurement.diagnostic.request_failures, 1);
            let encoded = serde_json::to_string(&measurement.diagnostic).unwrap();
            assert!(!encoded.contains("secret"));
            assert!(!encoded.contains("example.test"));
        }
    }

    #[test]
    fn node_failures_and_successes_retain_numeric_delay_compatibility() {
        let mut measurement = Measurement::new("all");
        for (result, expected) in [
            (Ok(42), 42),
            (Ok(0), 0),
            (
                Err(MeasurementFailure::new(MeasurementCode::NodeTimeout, Some(504)).into()),
                0,
            ),
            (
                Err(MeasurementFailure::new(MeasurementCode::NodeFailed, Some(503)).into()),
                0,
            ),
        ] {
            assert_eq!(measurement.observe(result, "").unwrap(), expected);
        }
        assert_eq!(measurement.diagnostic.succeeded, 1);
        assert_eq!(measurement.diagnostic.node_failures, 3);
        assert_eq!(measurement.diagnostic.request_failures, 0);
    }

    #[tokio::test]
    async fn batch_controller_failures_do_not_publish_zero_delays_as_unreachable_nodes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (status, success, code) in [
            (
                "500 Internal Server Error",
                false,
                MeasurementCode::ControllerHttp,
            ),
            ("504 Gateway Timeout", true, MeasurementCode::NodeTimeout),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let api = Api::new(&Settings {
                controller_port: listener.local_addr().unwrap().port(),
                ..Default::default()
            })
            .unwrap();
            let server = tokio::spawn(async move {
                for _ in 0..2 {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        let mut buffer = [0; 1024];
                        let count = socket.read(&mut buffer).await.unwrap();
                        assert!(count > 0 && request.len() < 8192);
                        request.extend_from_slice(&buffer[..count]);
                    }
                    let (status, body) = if request.starts_with(b"GET /proxies HTTP") {
                        (
                            "200 OK",
                            r#"{"proxies":{"group":{"type":"Selector","all":["node"]},"node":{"type":"Socks5"}}}"#,
                        )
                    } else {
                        assert!(request.starts_with(b"GET /proxies/node/delay?"));
                        (status, "private-response")
                    };
                    socket
                        .write_all(
                            format!(
                                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                }
            });
            let result = tokio::time::timeout(Duration::from_secs(5), api.delay_all_nodes())
                .await
                .unwrap();
            assert_eq!(result.is_ok(), success);
            if let Ok(delays) = result {
                assert_eq!(delays, BTreeMap::from([("node".into(), 0)]));
            }
            let reports = api.measurement_diagnostics();
            assert_eq!(reports.len(), 1);
            assert_eq!(reports[0].completed, success);
            assert_eq!(reports[0].attempted, 1);
            assert_eq!(reports[0].node_failures, usize::from(success));
            assert_eq!(reports[0].request_failures, usize::from(!success));
            assert_eq!(reports[0].codes[&code], 1);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn http_and_json_failures_preserve_their_meaning_and_hide_response_bodies() {
        for (status, body, expected, code) in [
            ("200 OK", r#"{"delay":42}"#, Some(42), None),
            (
                "200 OK",
                r#"{"delay":0}"#,
                Some(0),
                Some(MeasurementCode::NodeFailed),
            ),
            (
                "503 Service Unavailable",
                "private-response",
                Some(0),
                Some(MeasurementCode::NodeFailed),
            ),
            (
                "504 Gateway Timeout",
                "private-response",
                Some(0),
                Some(MeasurementCode::NodeTimeout),
            ),
            (
                "500 Internal Server Error",
                "private-response",
                None,
                Some(MeasurementCode::ControllerHttp),
            ),
            (
                "401 Unauthorized",
                "private-response",
                None,
                Some(MeasurementCode::ControllerAuthentication),
            ),
            (
                "403 Forbidden",
                "private-response",
                None,
                Some(MeasurementCode::ControllerAuthentication),
            ),
            (
                "404 Not Found",
                "private-response",
                None,
                Some(MeasurementCode::MissingNode),
            ),
            (
                "400 Bad Request",
                "private-response",
                None,
                Some(MeasurementCode::InvalidProbe),
            ),
            (
                "200 OK",
                r#"{"message":"private-response"}"#,
                None,
                Some(MeasurementCode::ControllerResponse),
            ),
            (
                "200 OK",
                r#"{"delay":42} trailing"#,
                None,
                Some(MeasurementCode::ControllerResponse),
            ),
        ] {
            let wire = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let (response, server) = super::super::tests::mock_response(wire.into_bytes()).await;
            let mut measurement = Measurement::new("node");
            let result = measurement.observe(Api::measurement_response(response).await, "");
            match expected {
                Some(delay) => assert_eq!(result.unwrap(), delay, "{status}"),
                None => assert!(result.is_err(), "{status}"),
            }
            assert_eq!(
                measurement.diagnostic.codes.keys().copied().next(),
                code,
                "{status}"
            );
            assert!(
                !serde_json::to_string(&measurement.diagnostic)
                    .unwrap()
                    .contains("private-response")
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn delay_response_bytes_are_bounded_without_a_content_length_header() {
        let wire = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{}",
            " ".repeat(4097)
        );
        let (response, server) = super::super::tests::mock_response(wire.into_bytes()).await;
        let error = Api::measurement_response(response).await.unwrap_err();
        assert_eq!(
            MeasurementFailure::classify(&error).code,
            MeasurementCode::ControllerResponse
        );
        server.await.unwrap();
    }

    #[test]
    fn diagnostics_are_shared_by_api_clones_bounded_and_safe_to_restore() {
        let api = Api::new(&Settings::default()).unwrap();
        let clone = api.clone();
        for index in 0..6 {
            let mut measurement = Measurement::new("all");
            measurement.diagnostic.started_at = format!("2026-11-11T00:00:0{index}Z");
            clone.finish_measurement(measurement, &Ok::<_, anyhow::Error>(()));
        }
        let reports = api.measurement_diagnostics();
        assert_eq!(reports.len(), 4);
        assert_eq!(reports[0].started_at, "2026-11-11T00:00:02Z");
        let mut duplicated = reports.clone();
        duplicated.extend(reports);
        api.restore_measurement_diagnostics(duplicated);
        assert_eq!(api.measurement_diagnostics().len(), 4);
        let changed = api
            .reconfigured(&Settings {
                controller_port: 19090,
                secret: "new-controller-secret".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(changed.measurement_diagnostics().len(), 4);
        assert_eq!(changed.base.port(), Some(19090));
        changed.finish_measurement(Measurement::new("node"), &Ok::<_, anyhow::Error>(()));
        assert_eq!(api.measurement_diagnostics().last().unwrap().scope, "node");
    }
}
