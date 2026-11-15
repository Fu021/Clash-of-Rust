use super::*;
use reqwest::{Client, Url};

fn client(context: &Environment, seconds: u64) -> Result<Client> {
    Ok(crate::network::client_builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(format!(
            "http://127.0.0.1:{}",
            proxy_port(context)?
        ))?)
        .connect_timeout(Duration::from_secs(seconds.min(5)))
        .timeout(Duration::from_secs(seconds))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

async fn route(response: &reqwest::Response, context: &Environment, report: &mut Report) {
    let mut observer = crate::update::RouteObserver::new(response, context.api.clone());
    let detail = observer
        .observation()
        .await
        .map(|route| format!("{} · {} ({})", route.chain, route.rule, route.rule_payload));
    report.entries.push(match detail {
        Some(detail) => Entry::ok("实际线路", detail),
        None => Entry::unknown("实际线路", "未捕获到本次连接记录", "route_not_observed"),
    });
}

pub(super) async fn dns(
    url: &Url,
    context: &Environment,
    options: &Options,
    report: &mut Report,
) -> Result<()> {
    let host = url
        .host_str()
        .context("目标缺少域名")?
        .trim_matches(['[', ']']);
    if host.parse::<std::net::IpAddr>().is_ok() {
        report.entries.push(Entry::unknown(
            "DNS",
            "目标已经是 IP 地址，无需解析",
            "literal_ip",
        ));
        return Ok(());
    }
    let system = async {
        if context.protected {
            return Entry::unknown(
                "系统 DNS",
                "保护开启或状态未知，未单独查询系统解析器",
                "protection_restricted",
            );
        }
        match crate::probe::dns(host.to_owned()).await {
            Ok(addresses) => Entry::ok(
                "系统 DNS",
                addresses
                    .into_iter()
                    .take(16)
                    .collect::<Vec<_>>()
                    .join("、"),
            ),
            Err(_) => Entry::error("系统 DNS", "系统解析失败或超时", "dns_failed"),
        }
    };
    let core = async {
        if options.record_type != "ADDR" {
            return match dns_records(host, context, &options.record_type).await {
                Ok(entries) => entries,
                Err(error) => vec![error::entry("内核 DNS", &error)],
            };
        }
        vec![match resolve(url, context).await {
            Ok(addresses) => Entry::ok(
                "内核 DNS",
                addresses
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("、"),
            ),
            Err(error) => error::entry("内核 DNS", &error),
        }]
    };
    let (system, core) = tokio::join!(system, core);
    report.entries.push(system);
    report.entries.extend(core);
    report.entries.push(Entry::unknown(
        "解析方式",
        "内核查询返回真实上游地址；两种解析结果不同不单独视为故障",
        "information",
    ));
    Ok(())
}

async fn dns_records(host: &str, context: &Environment, kind: &str) -> Result<Vec<Entry>> {
    let api = context
        .api
        .as_ref()
        .ok_or_else(|| error::failure("controller_unavailable", "内核解析接口不可用"))?;
    let mut url = api.url(&["dns", "query"])?;
    url.query_pairs_mut()
        .append_pair("name", host)
        .append_pair("type", kind);
    let response = api.request(reqwest::Method::GET, url).send().await?;
    let value = controller_json(response, 64 * 1024).await?;
    dns_entries(&value, kind)
}

pub(super) fn dns_entries(value: &serde_json::Value, kind: &str) -> Result<Vec<Entry>> {
    let status = value
        .get("Status")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| error::failure("dns_response_invalid", "DNS 响应缺少状态码"))?;
    if status != 0 {
        return Ok(vec![Entry::error(
            "内核 DNS",
            format!("{kind} · DNS 状态 {status}"),
            "dns_failed",
        )]);
    }
    let answers = value.get("Answer").and_then(serde_json::Value::as_array);
    let mut entries = vec![Entry::ok(
        "DNS 响应",
        format!(
            "{kind} · {} 条回答 · AA={} · RA={}",
            answers.map_or(0, Vec::len),
            value
                .get("AA")
                .and_then(serde_json::Value::as_bool)
                .map_or("未提供", |flag| if flag { "是" } else { "否" }),
            value
                .get("RA")
                .and_then(serde_json::Value::as_bool)
                .map_or("未提供", |flag| if flag { "是" } else { "否" })
        ),
    )];
    if let Some(answers) = answers {
        for answer in answers.iter().take(16) {
            let record_type = answer.get("type").and_then(serde_json::Value::as_u64);
            let data = answer
                .get("data")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("未提供");
            entries.push(Entry::ok(
                "记录",
                format!(
                    "类型 {} · TTL {} s · {}",
                    record_type.map_or_else(|| "未提供".into(), |number| number.to_string()),
                    answer
                        .get("TTL")
                        .and_then(serde_json::Value::as_u64)
                        .map_or_else(|| "未提供".into(), |number| number.to_string()),
                    if matches!(kind, "TXT" | "SOA") || matches!(record_type, Some(6 | 16)) {
                        "正文已隐藏".into()
                    } else {
                        bounded(data, 256)
                    }
                ),
            ));
        }
        if answers.len() > 16 {
            entries.push(Entry::unknown(
                "回答上限",
                "仅显示前 16 条记录",
                "output_truncated",
            ));
        }
    }
    if answers.is_none_or(Vec::is_empty) {
        entries.push(Entry::unknown(
            "记录",
            "查询成功，但没有此类型的回答",
            "dns_no_record",
        ));
    }
    Ok(entries)
}

pub(super) async fn controller_json(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<serde_json::Value> {
    if !response.status().is_success() {
        return Err(error::failure(
            "controller_query_failed",
            "内核未支持此查询或拒绝请求",
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(error::failure(
                "controller_response_limit",
                "控制接口响应超过本次上限",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| error::failure("controller_response_invalid", "控制接口响应格式无效"))
}

pub(super) async fn inspect(
    url: &Url,
    context: &Environment,
    options: &Options,
    report: &mut Report,
) -> Result<()> {
    let mut response = client(context, options.seconds)?
        .request(
            reqwest::Method::from_bytes(options.method.as_bytes())?,
            url.clone(),
        )
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;
    report.entries.push(Entry::ok(
        "网站响应",
        format!(
            "{} · HTTP {} · {:?}",
            options.method,
            response.status().as_u16(),
            response.version()
        ),
    ));
    for name in [
        "content-type",
        "content-length",
        "content-encoding",
        "cache-control",
        "allow",
        "strict-transport-security",
    ] {
        if let Some(value) = response.headers().get(name) {
            report.entries.push(Entry::ok(
                name,
                bounded(value.to_str().unwrap_or("无法显示"), 256),
            ));
        }
    }
    if response.status().is_redirection() {
        report.entries.push(Entry::unknown(
            "重定向",
            "未跟随；Location、Cookie 和认证响应头不进入报告",
            "information",
        ));
    }
    route(&response, context, report).await;
    if options.method != "HEAD" {
        let mut size = 0usize;
        while size < 4096 {
            match response.chunk().await {
                Ok(Some(chunk)) => size += chunk.len().min(4096 - size),
                Ok(None) => break,
                Err(error) => {
                    report.entries.push(error::entry("响应正文", &error.into()));
                    break;
                }
            }
        }
        report.entries.push(Entry::unknown(
            "响应正文",
            format!("接收 {size} B（最多 4096 B），已丢弃"),
            if size == 4096 {
                "byte_limit"
            } else {
                "information"
            },
        ));
    }
    Ok(())
}

pub(super) async fn stability(
    url: &Url,
    context: &Environment,
    options: &Options,
    report: &mut Report,
) -> Result<()> {
    let client = client(context, options.seconds)?;
    let mut delays = Vec::with_capacity(usize::from(options.count));
    let mut failed = 0;
    for index in 0..options.count {
        let start = Instant::now();
        match client.get(url.clone()).send().await {
            Ok(response) => {
                let millis = start.elapsed().as_millis();
                delays.push(millis);
                report.entries.push(Entry::ok(
                    &format!("请求 {}", index + 1),
                    format!("HTTP {} · {millis} ms", response.status().as_u16()),
                ));
                if index == 0 {
                    route(&response, context, report).await;
                }
            }
            Err(error) => {
                failed += 1;
                report
                    .entries
                    .push(error::entry(&format!("请求 {}", index + 1), &error.into()));
            }
        }
        if index + 1 < options.count {
            tokio::time::sleep(Duration::from_millis(options.interval_ms)).await;
        }
    }
    delays.sort_unstable();
    report.entries.insert(
        0,
        Entry::new(
            "请求结果",
            format!(
                "成功 {} / {} · 失败 {failed}；HTTP 错误状态也属于收到网站响应",
                delays.len(),
                options.count
            ),
            Some(failed == 0),
            if failed == 0 { "ok" } else { "http_failed" },
        ),
    );
    if let (Some(first), Some(last)) = (delays.first(), delays.last()) {
        report.entries.push(Entry::ok(
            "响应头耗时",
            format!(
                "最低 {first} · 中位 {} · 最高 {last} ms",
                delays[delays.len() / 2]
            ),
        ));
    }
    Ok(())
}

pub(super) async fn download(
    url: &Url,
    context: &Environment,
    options: &Options,
    report: &mut Report,
) -> Result<()> {
    let client = client(context, options.seconds)?;
    let start = Instant::now();
    let mut response = client
        .get(url.clone())
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;
    if !response.status().is_success() {
        report.entries.push(Entry::error(
            "网站响应",
            format!(
                "HTTP {}，未接收正文；重定向不会自动跳转",
                response.status().as_u16()
            ),
            "download_http_status",
        ));
        return Ok(());
    }
    route(&response, context, report).await;
    let deadline = tokio::time::Instant::from_std(start) + Duration::from_secs(options.seconds);
    let mut bytes = 0u64;
    let reason = loop {
        if bytes >= options.max_bytes {
            break "byte_limit";
        }
        match tokio::time::timeout_at(deadline, response.chunk()).await {
            Err(_) => break "time_limit",
            Ok(Ok(None)) => break "completed",
            Ok(Ok(Some(chunk))) => {
                bytes += (chunk.len() as u64).min(options.max_bytes - bytes);
            }
            Ok(Err(error)) if error.is_timeout() => break "time_limit",
            Ok(Err(error)) => {
                report.entries.push(error::entry("接收中断", &error.into()));
                break "interrupted";
            }
        }
    };
    let elapsed = start.elapsed().as_secs_f64();
    report.entries.insert(
        0,
        Entry::new(
            "已接收",
            format!("{bytes} B / {} B 上限", options.max_bytes),
            Some(bytes > 0 && reason != "interrupted"),
            if bytes == 0 { "empty_response" } else { "ok" },
        ),
    );
    report.entries.push(Entry::unknown(
        "平均速度",
        format!(
            "{:.1} KiB/s · {elapsed:.2} 秒",
            bytes as f64 / elapsed.max(0.001) / 1024.0
        ),
        "information",
    ));
    report.entries.push(Entry::unknown(
        "结束原因",
        match reason {
            "byte_limit" => "达到流量上限",
            "time_limit" => "达到时间上限",
            "completed" => "目标响应结束",
            _ => "连接中断",
        },
        reason,
    ));
    report.entries.push(Entry::unknown(
        "接收内容",
        "即时丢弃，不写文件或保留正文",
        "information",
    ));
    Ok(())
}
