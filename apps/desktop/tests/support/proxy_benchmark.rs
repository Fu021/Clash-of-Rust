use anyhow::{Context, Result, ensure};
use clash_client_core::{
    api::Api,
    download::{BUFFER_CAPACITY, Watchdog},
    file_hash,
    network::Route,
};
use clash_of_rust::platform;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    net::SocketAddr,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;

#[derive(Deserialize)]
struct Fixture {
    urls: Vec<String>,
    latency_url: String,
    size: u64,
    sha256: String,
    certificate: PathBuf,
    report: PathBuf,
    system: String,
    arch: String,
    host: serde_json::Value,
}

#[derive(Serialize)]
struct LatencySample {
    route: String,
    round: usize,
    elapsed_ms: f64,
    reported_delay_ms: Option<u32>,
    server_wait_ms: Option<f64>,
    peer: SocketAddr,
}

#[derive(Serialize)]
struct Sample {
    protocol: String,
    route: String,
    sink: String,
    round: usize,
    bytes: u64,
    first_byte_ms: f64,
    elapsed_ms: f64,
    mib_per_second: f64,
    peer: SocketAddr,
    sha256: String,
}

#[derive(Serialize)]
struct SequenceSample {
    route: String,
    round: usize,
    requests: usize,
    bytes: u64,
    elapsed_ms: f64,
    final_mib_per_second: f64,
}

async fn pooled_sequence(
    fixture: &Fixture,
    url: &reqwest::Url,
    route: &Route,
    round: usize,
    mixed_port: u16,
) -> Result<SequenceSample> {
    let certificate = reqwest::Certificate::from_pem(&std::fs::read(&fixture.certificate)?)?;
    let client = route
        .apply(
            clash_client_core::network::client_builder()
                .tls_certs_only([certificate])
                .http1_only()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(20)),
        )?
        .build()?;
    let small: Vec<_> = (0u8..=255).cycle().take(64 * 1024).collect();
    let small_digest = file_hash::hex(Sha256::digest(&small));
    let started = Instant::now();
    let mut bytes = 0;
    let mut final_mib_per_second = 0.;
    for index in 0..30 {
        let mut target = url.clone();
        let (size, expected) = if index == 29 {
            (fixture.size, fixture.sha256.as_str())
        } else {
            target.set_path("/small");
            (small.len() as u64, small_digest.as_str())
        };
        let request_started = Instant::now();
        let mut response = client.get(target).send().await?;
        ensure!(
            response.status() == reqwest::StatusCode::OK && response.content_length() == Some(size),
            "pooled sequence returned an unexpected response"
        );
        let peer = response.remote_addr().context("missing pooled peer")?;
        let expected_port = if route.proxy.is_some() {
            mixed_port
        } else {
            url.port_or_known_default().unwrap()
        };
        ensure!(
            peer.ip().is_loopback() && peer.port() == expected_port,
            "pooled sequence bypassed the intended proxy"
        );
        let mut watchdog = Watchdog::new(false);
        let mut received = 0;
        let mut digest = Sha256::new();
        while let Some(chunk) = watchdog.chunk(&mut response).await? {
            let chunk = chunk.as_ref();
            received += chunk.len() as u64;
            ensure!(received <= size, "oversized pooled response");
            digest.update(chunk);
        }
        ensure!(
            received == size && file_hash::hex(digest.finalize()) == expected,
            "pooled sequence returned corrupt bytes"
        );
        bytes += received;
        if index == 29 {
            final_mib_per_second =
                received as f64 / (1024. * 1024.) / request_started.elapsed().as_secs_f64();
        }
    }
    Ok(SequenceSample {
        route: route.label.into(),
        round,
        requests: 30,
        bytes,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.,
        final_mib_per_second,
    })
}

async fn sample(
    fixture: &Fixture,
    url: &reqwest::Url,
    route: &Route,
    disk: bool,
    round: usize,
    mixed_port: u16,
) -> Result<Sample> {
    let certificate = reqwest::Certificate::from_pem(&std::fs::read(&fixture.certificate)?)?;
    let client = route
        .apply(
            reqwest::Client::builder()
                .tls_certs_only([certificate])
                .http1_only()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(20))
                .pool_max_idle_per_host(0),
        )?
        .build()?;
    let directory = tempfile::tempdir()?;
    let mut output = if disk {
        Some(tokio::io::BufWriter::with_capacity(
            BUFFER_CAPACITY,
            tokio::fs::File::create(directory.path().join("payload")).await?,
        ))
    } else {
        None
    };
    let started = Instant::now();
    let mut response = client.get(url.clone()).send().await?;
    ensure!(
        response.status() == reqwest::StatusCode::OK
            && response.content_length() == Some(fixture.size),
        "loopback fixture returned an unexpected response"
    );
    let peer = response.remote_addr().context("missing loopback peer")?;
    let expected_port = if route.proxy.is_some() {
        mixed_port
    } else {
        url.port_or_known_default().unwrap()
    };
    ensure!(
        peer.ip().is_loopback() && peer.port() == expected_port,
        "throughput sample bypassed the intended proxy"
    );
    let mut watchdog = Watchdog::new(false);
    let mut received = 0;
    let mut digest = Sha256::new();
    let mut first_byte_ms = None;
    while let Some(chunk) = watchdog.chunk(&mut response).await? {
        let chunk = chunk.as_ref();
        if chunk.is_empty() {
            continue;
        }
        first_byte_ms.get_or_insert_with(|| started.elapsed().as_secs_f64() * 1000.);
        received += chunk.len() as u64;
        ensure!(received <= fixture.size, "oversized loopback payload");
        digest.update(chunk);
        if let Some(output) = &mut output {
            output.write_all(chunk).await?;
        }
    }
    if let Some(mut output) = output {
        output.flush().await?;
        output.into_inner().sync_all().await?;
    }
    let elapsed = started.elapsed();
    let sha256 = file_hash::hex(digest.finalize());
    ensure!(
        received == fixture.size && sha256 == fixture.sha256,
        "loopback payload is incomplete or corrupt"
    );
    Ok(Sample {
        protocol: url.scheme().into(),
        route: route.label.into(),
        sink: if disk { "file" } else { "memory" }.into(),
        round,
        bytes: received,
        first_byte_ms: first_byte_ms.context("empty loopback payload")?,
        elapsed_ms: elapsed.as_secs_f64() * 1000.,
        mib_per_second: received as f64 / (1024. * 1024.) / elapsed.as_secs_f64(),
        peer,
        sha256,
    })
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

async fn native_proxy(scheme: &str, mixed_port: u16) -> Result<String> {
    for _ in 0..20 {
        if let Some(value) = platform::configured_proxy(scheme)?
            && let Ok(url) = reqwest::Url::parse(&value)
            && url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port_or_known_default() == Some(mixed_port)
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
        {
            return Ok(value);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("native system proxy does not point to the managed mixed listener")
}

async fn client_latency(
    url: &reqwest::Url,
    route: &Route,
    round: usize,
    mixed_port: u16,
) -> Result<LatencySample> {
    let client = route
        .apply(
            reqwest::Client::builder()
                .http1_only()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(8))
                .pool_max_idle_per_host(0),
        )?
        .build()?;
    let started = Instant::now();
    let response = client.head(url.clone()).send().await?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.;
    ensure!(
        response.status() == reqwest::StatusCode::NO_CONTENT,
        "latency fixture returned an unexpected response"
    );
    let server_wait_ms: f64 = response
        .headers()
        .get("x-cor-fixture-wait-ms")
        .context("missing fixture timing")?
        .to_str()?
        .parse()?;
    ensure!(
        server_wait_ms.is_finite() && server_wait_ms >= 0.,
        "invalid fixture timing"
    );
    let peer = response.remote_addr().context("missing latency peer")?;
    let expected_port = if route.proxy.is_some() {
        mixed_port
    } else {
        url.port_or_known_default().unwrap()
    };
    ensure!(
        peer.ip().is_loopback() && peer.port() == expected_port,
        "latency sample bypassed the intended proxy"
    );
    Ok(LatencySample {
        route: route.label.into(),
        round,
        elapsed_ms,
        reported_delay_ms: None,
        server_wait_ms: Some(server_wait_ms),
        peer,
    })
}

async fn controller_latency(
    api: &Api,
    url: &reqwest::Url,
    node: &str,
    label: &str,
    round: usize,
) -> Result<LatencySample> {
    let mut endpoint = api.url(&["proxies", node, "delay"])?;
    let controller_port = endpoint.port_or_known_default().unwrap();
    endpoint
        .query_pairs_mut()
        .append_pair("url", url.as_str())
        .append_pair("timeout", "5000")
        .append_pair("expected", "204");
    let started = Instant::now();
    let response = api.request(reqwest::Method::GET, endpoint).send().await?;
    let peer = response.remote_addr().context("missing controller peer")?;
    ensure!(
        peer.ip().is_loopback() && peer.port() == controller_port,
        "delay query did not reach the local controller"
    );
    let body: serde_json::Value = response.error_for_status()?.json().await?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.;
    let delay = body["delay"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .context("mihomo did not report a valid node delay")?;
    Ok(LatencySample {
        route: label.into(),
        round,
        elapsed_ms,
        reported_delay_ms: Some(delay),
        server_wait_ms: None,
        peer,
    })
}

fn report(
    fixture: &Fixture,
    samples: &[Sample],
    latencies: &[LatencySample],
    sequences: &[SequenceSample],
    error: Option<String>,
) -> Result<()> {
    let mut summaries = Vec::new();
    for protocol in ["http", "https"] {
        for route in ["direct", "native-http", "socks5"] {
            for sink in ["memory", "file"] {
                let selected: Vec<_> = samples
                    .iter()
                    .filter(|sample| {
                        sample.protocol == protocol && sample.route == route && sample.sink == sink
                    })
                    .collect();
                if selected.is_empty() {
                    continue;
                }
                summaries.push(json!({
                    "protocol": protocol, "route": route, "sink": sink,
                    "samples": selected.len(),
                    "median_mib_per_second": median(selected.iter().map(|sample| sample.mib_per_second).collect()),
                    "median_first_byte_ms": median(selected.iter().map(|sample| sample.first_byte_ms).collect()),
                }));
            }
        }
    }
    let mut latency_summaries = Vec::new();
    for route in [
        "direct",
        "native-http",
        "socks5",
        "controller-direct",
        "controller-loopback-socks",
    ] {
        let selected: Vec<_> = latencies
            .iter()
            .filter(|sample| sample.route == route)
            .collect();
        if selected.is_empty() {
            continue;
        }
        let reported: Vec<_> = selected
            .iter()
            .filter_map(|sample| sample.reported_delay_ms.map(f64::from))
            .collect();
        let server_waits: Vec<_> = selected
            .iter()
            .filter_map(|sample| sample.server_wait_ms)
            .collect();
        latency_summaries.push(json!({
            "route": route,
            "samples": selected.len(),
            "median_elapsed_ms": median(selected.iter().map(|sample| sample.elapsed_ms).collect()),
            "median_reported_delay_ms": if reported.is_empty() { None } else { Some(median(reported)) },
            "median_server_wait_ms": if server_waits.is_empty() { None } else { Some(median(server_waits)) },
        }));
    }
    let report = json!({
        "schema": 1, "system": fixture.system, "arch": fixture.arch, "host": fixture.host,
        "scope": "local-loopback-no-public-network",
        "http_version": "HTTP/1.1", "ip_version": 4,
        "tls_certificate_verified": samples.iter().any(|sample| sample.protocol == "https"),
        "complete": error.is_none() && samples.len() == 36 && latencies.len() == 25
            && sequences.len() == 30,
        "error": error, "bytes_per_sample": fixture.size,
        "samples": samples, "summaries": summaries,
        "latency_samples": latencies, "latency_summaries": latency_summaries,
        "pooled_sequences": sequences,
    });
    std::fs::write(&fixture.report, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}

pub async fn check(api: &Api, mixed_port: u16) -> Result<()> {
    let Some(path) = std::env::var_os("CLASH_PROXY_BENCHMARK_FIXTURE") else {
        return Ok(());
    };
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut samples = Vec::new();
    let mut latencies = Vec::new();
    let mut sequences = Vec::new();
    report(&fixture, &samples, &latencies, &sequences, None)?;
    let result = tokio::time::timeout(Duration::from_secs(180), async {
        let latency_url = reqwest::Url::parse(&fixture.latency_url)?;
        ensure!(
            latency_url.scheme() == "http"
                && latency_url.host_str() == Some("127.0.0.1")
                && latency_url.path() == "/latency"
                && latency_url.query().is_none()
                && latency_url.fragment().is_none(),
            "invalid loopback latency fixture"
        );
        let latency_routes = [
            Route {
                label: "direct",
                proxy: None,
            },
            Route {
                label: "native-http",
                proxy: Some(native_proxy("http", mixed_port).await?),
            },
            Route {
                label: "socks5",
                proxy: Some(format!("socks5h://127.0.0.1:{mixed_port}")),
            },
        ];
        for round in 0..5 {
            for offset in 0..latency_routes.len() {
                let route = &latency_routes[(round + offset) % latency_routes.len()];
                latencies.push(client_latency(&latency_url, route, round, mixed_port).await?);
                report(&fixture, &samples, &latencies, &sequences, None)?;
            }
            for (node, label) in [
                ("DIRECT", "controller-direct"),
                ("CI-Loopback", "controller-loopback-socks"),
            ] {
                latencies.push(controller_latency(api, &latency_url, node, label, round).await?);
                report(&fixture, &samples, &latencies, &sequences, None)?;
            }
        }
        for url in &fixture.urls {
            let url = reqwest::Url::parse(url)?;
            ensure!(
                url.host_str() == Some("127.0.0.1")
                    && matches!(url.scheme(), "http" | "https")
                    && fixture.size == 16 * 1024 * 1024,
                "invalid loopback benchmark fixture"
            );
            let proxy = native_proxy(url.scheme(), mixed_port).await?;
            let routes = [
                Route {
                    label: "direct",
                    proxy: None,
                },
                Route {
                    label: "native-http",
                    proxy: Some(proxy),
                },
                Route {
                    label: "socks5",
                    proxy: Some(format!("socks5h://127.0.0.1:{mixed_port}")),
                },
            ];
            for round in 0..3 {
                for offset in 0..routes.len() {
                    let route = &routes[(round + offset) % routes.len()];
                    for disk in [round % 2 == 0, round % 2 != 0] {
                        let sample = sample(&fixture, &url, route, disk, round, mixed_port).await?;
                        println!(
                            "PROXY_BENCHMARK: {} {} {} round={} {:.2} MiB/s first_byte={:.2} ms",
                            sample.protocol,
                            sample.route,
                            sample.sink,
                            round,
                            sample.mib_per_second,
                            sample.first_byte_ms,
                        );
                        samples.push(sample);
                        report(&fixture, &samples, &latencies, &sequences, None)?;
                    }
                }
            }
            if url.scheme() == "https" {
                for round in 0..10 {
                    for offset in 0..routes.len() {
                        let route = &routes[(round + offset) % routes.len()];
                        sequences
                            .push(pooled_sequence(&fixture, &url, route, round, mixed_port).await?);
                        report(&fixture, &samples, &latencies, &sequences, None)?;
                    }
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("loopback throughput benchmark exceeded its 180-second budget")
    .and_then(|result| result);
    report(
        &fixture,
        &samples,
        &latencies,
        &sequences,
        result.as_ref().err().map(|error| format!("{error:#}")),
    )?;
    result
}
