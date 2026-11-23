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
        entries.extend(
            answers
                .iter()
                .take(16)
                .map(|answer| dns_record(answer, kind)),
        );
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

fn dns_record(answer: &serde_json::Value, query_type: &str) -> Entry {
    let record_type = answer.get("type").and_then(serde_json::Value::as_u64);
    let hidden = matches!(query_type, "TXT" | "SOA") || matches!(record_type, Some(6 | 16));
    let data = if hidden {
        None
    } else {
        answer
            .get("data")
            .and_then(serde_json::Value::as_str)
            .map(|data| bounded(data, 256))
    };
    let record_type = record_type.map(|number| number.to_string());
    let ttl = answer
        .get("TTL")
        .and_then(serde_json::Value::as_u64)
        .map(|number| number.to_string());
    let detail = |language: Language| {
        let unavailable = language.text("未提供", "unavailable");
        let data = if hidden {
            language.text("正文已隐藏", "Content hidden")
        } else {
            data.as_deref().unwrap_or(unavailable)
        };
        format!(
            "{} {} · TTL {} s · {data}",
            language.text("类型", "Type"),
            record_type.as_deref().unwrap_or(unavailable),
            ttl.as_deref().unwrap_or(unavailable),
        )
    };
    Entry::ok("记录", detail(Language::SimplifiedChinese))
        .with_english_detail(detail(Language::English))
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

pub(super) fn header_entry(name: &str, value: &reqwest::header::HeaderValue) -> Entry {
    if name == "content-security-policy" {
        return Entry::ok(name, "已设置（策略正文已隐藏）")
            .with_english_detail("Set; policy content hidden");
    }
    match value.to_str() {
        Ok(value) => Entry::ok(name, bounded(value, 256)),
        Err(_) => Entry::ok(name, "无法显示").with_english_detail("Cannot display value"),
    }
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
    report.entries.push(Entry::unknown(
        "业务状态",
        if response.status().is_client_error() || response.status().is_server_error() {
            "已收到 HTTP 错误响应；传输连通不代表业务可用"
        } else {
            "已收到响应；业务是否可用还取决于目标服务"
        },
        "information",
    ));
    for name in [
        "content-type",
        "content-length",
        "content-encoding",
        "cache-control",
        "allow",
        "strict-transport-security",
        "content-security-policy",
        "x-content-type-options",
        "referrer-policy",
    ] {
        if let Some(value) = response.headers().get(name) {
            report.entries.push(header_entry(name, value));
        }
    }
    if response.status().is_redirection() {
        report.entries.push(Entry::unknown(
            "重定向",
            "未跟随；Location、Cookie 和认证响应头不进入报告",
            "information",
        ));
    }
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
    let mut statuses = std::collections::BTreeMap::<u16, usize>::new();
    for index in 0..options.count {
        let start = Instant::now();
        match client.get(url.clone()).send().await {
            Ok(response) => {
                let millis = start.elapsed().as_millis();
                delays.push(millis);
                *statuses.entry(response.status().as_u16()).or_default() += 1;
                report.entries.push(Entry::ok(
                    &format!("请求 {}", index + 1),
                    format!("HTTP {} · {millis} ms", response.status().as_u16()),
                ));
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
                "收到响应 {} / {} · 传输失败 {failed}；HTTP 业务状态另列",
                delays.len(),
                options.count
            ),
            Some(failed == 0),
            if failed == 0 { "ok" } else { "http_failed" },
        ),
    );
    report.entries.push(Entry::unknown(
        "HTTP 状态分布",
        if statuses.is_empty() {
            "未收到 HTTP 响应".into()
        } else {
            statuses
                .iter()
                .map(|(status, count)| format!("{status} × {count}"))
                .collect::<Vec<_>>()
                .join(" · ")
        },
        "information",
    ));
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
        "即时丢弃，不写文件或保留正文；平均吞吐包含建连等待，仅代表本次限量样本",
        "information",
    ));
    Ok(())
}
