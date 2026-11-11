use super::*;
use anyhow::Context;
use clash_client_core::{
    api::{Api, Connection},
    connection, ip_check, proxy_order,
};
use clash_client_runtime::claude_check;
use std::{
    collections::{BTreeMap, VecDeque},
    hash::{Hash, Hasher},
    time::Instant,
};

pub(super) async fn execute(args: &Args) -> Result<()> {
    match args.path.as_str() {
        "node list" | "node select" | "node test" => nodes(args).await,
        "connection list" | "connection show" | "connection close" => connections(args).await,
        "logs" | "logs clear" | "logs export" => logs(args).await,
        "diagnose" | "dns" => diagnose(args).await,
        "ip-check" | "ip-check list" => ip(args).await,
        _ => claude(args).await,
    }
}

async fn nodes(args: &Args) -> Result<()> {
    backend::live(args).await?;
    if args.path == "node select" {
        let _: Value = backend::request(
            args,
            Command::Select {
                group: args.positional[0].clone(),
                node: args.positional[1].clone(),
            },
        )
        .await?;
        return output::emit(
            args,
            json!({"group":args.positional[0],"selected":args.positional[1]}),
        );
    }
    if args.path == "node test" {
        let count = usize::from(!args.positional.is_empty())
            + usize::from(args.flag("all"))
            + usize::from(args.get("group").is_some());
        ensure!(count == 1, "请选择 NODE、--group GROUP 或 --all 中的一项");
        let value: Value = backend::request(
            args,
            Command::Delay {
                name: args
                    .get("group")
                    .map(str::to_owned)
                    .or_else(|| args.positional.first().cloned()),
                group: args.get("group").is_some(),
            },
        )
        .await?;
        return output::emit(args, value);
    }
    let poll: Poll = backend::request(
        args,
        Command::Snapshot {
            scope: Scope::Proxies,
            header: false,
        },
    )
    .await?;
    let proxies = poll.snapshot.proxies;
    let filter = args.get("filter").unwrap_or("").to_lowercase();
    let sort = match args.get("sort").unwrap_or("name") {
        "name" => proxy_order::NodeSort::Name,
        "latency" => {
            if args.flag("reverse") {
                proxy_order::NodeSort::LatencyDescending
            } else {
                proxy_order::NodeSort::LatencyAscending
            }
        }
        _ => bail!("节点排序支持 name、latency"),
    };
    let names: Vec<_> = if let Some(group) = args.positional.first() {
        proxies
            .proxies
            .get(group.as_str())
            .context("未找到策略组")?
            .all
            .clone()
    } else {
        proxies.proxies.keys().cloned().collect()
    };
    let selected = args
        .positional
        .first()
        .and_then(|name| proxies.proxies.get(name.as_str()))
        .map(|p| &p.now);
    let mut indices = proxy_order::sorted_indices(
        &names,
        sort,
        |name| proxies.proxies.get(name).and_then(|p| p.delay),
        |name| name.to_lowercase().contains(&filter),
    );
    if sort == proxy_order::NodeSort::Name && args.flag("reverse") {
        indices.reverse();
    }
    let rows=indices.into_iter().map(|i|{
        let name=&names[i]; let proxy=proxies.proxies.get(name.as_str());
        json!({"name":name,"type":proxy.map(|p|&p.kind),"selected":selected==Some(name),"now":proxy.map(|p|&p.now),"latency_ms":proxy.and_then(|p|p.delay),"members":proxy.map_or(0,|p|p.all.len())})
    }).collect();
    output::emit(args, output::page(args, rows)?)
}

fn connection_json(c: &Connection) -> Value {
    json!({"id":c.id,"name":connection::name(c),"host":c.metadata.host,"sniff_host":connection::hostname(c),
        "ip":c.metadata.destination_ip,"port":c.metadata.destination_port,"network":c.metadata.network,
        "rule":c.rule,"rule_payload":c.rule_payload,"chain":c.chains,"upload":c.upload,"download":c.download,
        "upload_rate":c.rates.map(|r|r.upload),"download_rate":c.rates.map(|r|r.download),"start":c.start})
}
async fn sample(args: &Args) -> Result<Vec<Connection>> {
    let poll: Poll = backend::request(
        args,
        Command::Snapshot {
            scope: Scope::Connections,
            header: false,
        },
    )
    .await?;
    Ok(poll.snapshot.connections.connections)
}
async fn connections(args: &Args) -> Result<()> {
    let meta = backend::live(args).await?;
    if args.path == "connection close" {
        ensure!(
            args.flag("all") != !args.positional.is_empty(),
            "请选择 ID 或 --all"
        );
        if args.flag("all") {
            output::confirm(args, "关闭全部现有连接？").await?;
        }
        let _: Value = backend::request(
            args,
            Command::Close {
                id: args.positional.first().cloned(),
            },
        )
        .await?;
        return output::emit(args, json!({"closed":true}));
    }
    if args.path == "connection show" {
        let entries = sample(args).await?;
        let c = entries
            .iter()
            .find(|c| c.id.as_ref() == args.positional[0])
            .context("连接已关闭或 ID 不存在")?;
        let mut row = connection_json(c);
        if c.metadata.destination_ip.is_empty() && !connection::hostname(c).is_empty() {
            match connection::resolve_target(
                &Api::new(&meta.settings)?,
                connection::hostname(c),
                meta.settings.mixed_port,
            )
            .await
            {
                Ok(ip) => {
                    row["ip"] = json!(ip);
                    row["ip_source"] = json!("resolver");
                }
                Err(error) => row["resolution_error"] = json!(format!("{error:#}")),
            }
        }
        return output::emit(args, row);
    }
    let field = match args.get("sort") {
        None | Some("none") => None,
        Some("name") => Some(connection::Field::Name),
        Some("rule") => Some(connection::Field::Rule),
        Some("chain") => Some(connection::Field::Chain),
        Some("upload") => Some(connection::Field::UploadTotal),
        Some("download") => Some(connection::Field::DownloadTotal),
        Some("upload-rate") => Some(connection::Field::UploadRate),
        Some("download-rate") => Some(connection::Field::DownloadRate),
        Some("start") => Some(connection::Field::Start),
        _ => bail!("未知连接排序字段"),
    };
    let sort = connection::Sort {
        field,
        descending: field.is_some_and(|f| {
            !matches!(
                f,
                connection::Field::Name | connection::Field::Rule | connection::Field::Chain
            )
        }) ^ args.flag("reverse"),
    };
    let filter = args.get("filter").unwrap_or("").to_lowercase();
    let mut previous = sample(args).await?;
    let mut sampled = Instant::now();
    slim(&mut previous);
    loop {
        tokio::time::sleep(args.interval()?).await;
        let mut current = sample(args).await?;
        let now = Instant::now();
        connection::sample_rates(&mut current, &previous, now.duration_since(sampled));
        sampled = now;
        let mut indices: Vec<_> = (0..current.len())
            .filter(|&i| {
                let c = &current[i];
                connection::name(c).to_lowercase().contains(&filter)
                    || c.rule.to_lowercase().contains(&filter)
                    || c.chains.iter().any(|v| v.to_lowercase().contains(&filter))
                    || c.id.contains(&filter)
            })
            .collect();
        indices.sort_by(|&a, &b| sort.compare(&current[a], &current[b]));
        let rows = indices
            .into_iter()
            .map(|i| connection_json(&current[i]))
            .collect();
        output::emit(args, output::page(args, rows)?)?;
        if !args.flag("watch") {
            break;
        }
        slim(&mut current);
        previous = current;
    }
    Ok(())
}
fn slim(connections: &mut [Connection]) {
    for c in connections {
        c.metadata = Default::default();
        c.chains = Vec::new();
        c.rule = Default::default();
        c.rule_payload = Default::default();
        c.rates = None;
    }
}

fn log_level(line: &str) -> &'static str {
    let lower = line.to_lowercase();
    if lower.starts_with("[error]")
        || lower.contains("level=error")
        || lower.contains("\"type\":\"error\"")
        || lower.contains("失败")
    {
        "error"
    } else if lower.starts_with("[warning]")
        || lower.starts_with("[warn]")
        || lower.contains("level=warning")
        || lower.contains("level=warn")
        || lower.contains("\"type\":\"warning\"")
    {
        "warning"
    } else if lower.starts_with("[debug]")
        || lower.contains("level=debug")
        || lower.contains("\"type\":\"debug\"")
    {
        "debug"
    } else {
        "info"
    }
}
async fn logs(args: &Args) -> Result<()> {
    if args.path == "logs clear" {
        output::confirm(args, "清空客户端日志？").await?;
        let _: Value = backend::request(args, Command::Logs { clear: true }).await?;
        return output::emit(args, json!({"cleared":true}));
    }
    if args.path == "logs export" {
        let path = args.get("output").context("请提供 --output FILE")?;
        let lines: Vec<String> = backend::request(args, Command::Logs { clear: false }).await?;
        let secret = backend::meta(args).await?.settings.secret;
        clash_client_core::config::atomic_write(
            &backend::absolute(path)?,
            lines.join("\n").replace(&secret, "[密钥]").as_bytes(),
        )?;
        if !args.flag("quiet") {
            eprintln!("已保存：{path}");
        }
        if lines.iter().any(|line| log_level(line) == "error") {
            crate::diagnostics::failure_hint(Some(args));
        }
        return Ok(());
    }
    let level = args.get("level");
    ensure!(
        level.is_none_or(|v| matches!(v, "debug" | "info" | "warning" | "error" | "silent")),
        "日志等级应为 debug/info/warning/error/silent"
    );
    let filter = args.get("filter").unwrap_or("").to_lowercase();
    let limit = args.number("limit", 100, 10_000)?;
    let mut previous: Vec<u64> = Vec::new();
    loop {
        let lines: Vec<String> = backend::request(args, Command::Logs { clear: false }).await?;
        let hashes: Vec<_> = lines
            .iter()
            .map(|line| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                line.hash(&mut h);
                h.finish()
            })
            .collect();
        // Match the longest surviving suffix; identical consecutive lines retain their multiplicity.
        let overlap = (1..=previous.len().min(hashes.len()))
            .rev()
            .find(|&n| previous[previous.len() - n..] == hashes[..n])
            .unwrap_or(0);
        let start = if previous.is_empty() {
            lines.len().saturating_sub(limit)
        } else {
            overlap
        };
        for line in &lines[start..] {
            let detected = log_level(line);
            if level.is_some_and(|level| level != detected)
                || !line.to_lowercase().contains(&filter)
            {
                continue;
            }
            output::emit(
                args,
                if args.flag("json") || args.flag("jsonl") {
                    json!({"level":detected,"message":line})
                } else {
                    json!(line)
                },
            )?;
            if detected == "error" {
                crate::diagnostics::failure_hint(Some(args));
            }
        }
        previous = hashes;
        if !args.flag("follow") {
            break;
        }
        tokio::time::sleep(args.interval()?).await;
    }
    Ok(())
}

async fn diagnose(args: &Args) -> Result<()> {
    if args.path == "diagnose" {
        crate::diagnostics::validate_output(args)?;
        if args.flag("offline") {
            let (report, routes) = tokio::join!(
                crate::diagnostics::collect(args),
                crate::host_network::diagnose(args)
            );
            let mut report = report?;
            report["offline"] = json!(true);
            report["host_routes"] = routes;
            return output::emit(args, report);
        }
    }
    if args.path == "dns" && args.flag("system") {
        return output::emit(
            args,
            json!({"source":"system","answers":clash_client_core::probe::dns(args.positional[0].clone()).await?}),
        );
    }
    if args.path == "dns" {
        let meta = backend::live(args).await?;
        let api = Api::new(&meta.settings)?;
        let domain = &args.positional[0];
        let answers = match api.resolve_domain(domain).await {
            Ok(a) => a,
            Err(_) => {
                clash_client_core::dns::through_proxy(domain, meta.settings.mixed_port).await?
            }
        };
        return output::emit(args, json!({"source":"core/proxy","answers":answers}));
    }
    let host = args.get("host").unwrap_or("github.com");
    let url = args
        .get("url")
        .unwrap_or("https://www.cloudflare.com/cdn-cgi/trace");
    let display = |r: Result<clash_client_core::probe::ProbeResult>| match r {
        Ok(p) => json!({"url":p.url,"status":p.status,"millis":p.millis,"detail":p.detail}),
        Err(e) => json!({"error":format!("{e:#}")}),
    };
    let backend_probes = async {
        let (remote, meta): (_, Meta) =
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let remote = backend::connect(args, false)
                    .await?
                    .context("当前用户后台未运行")?;
                let meta = remote.request(Command::Meta).await?;
                Ok::<_, anyhow::Error>((remote, meta))
            })
            .await
            .context("连接当前用户后台超过 5 秒；可用 --offline 保存本地现场")??;
        ensure!(meta.running, "当前用户内核未运行");
        let api = Api::new(&meta.settings)?;
        let logs = async {
            if args.get("output").is_none() {
                return json!({"state":"not_requested"});
            }
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                remote.request::<Vec<String>>(Command::Logs { clear: false }),
            )
            .await
            {
                Ok(Ok(lines)) => crate::diagnostics::core_logs(lines, &meta),
                Ok(Err(error)) => json!({"state":"unavailable","error":format!("{error:#}")}),
                Err(_) => json!({"state":"unavailable","error":"读取内核日志超过 5 秒"}),
            }
        };
        let (core, http, v4, v6, logs) = tokio::join!(
            bounded_probe(api.resolve_domain(host)),
            bounded_probe(clash_client_core::probe::website(
                url.into(),
                meta.settings.mixed_port,
                false
            )),
            bounded_probe(clash_client_core::probe::website(
                "https://api.ipify.org?format=json".into(),
                meta.settings.mixed_port,
                true
            )),
            bounded_probe(clash_client_core::probe::website(
                "https://api6.ipify.org?format=json".into(),
                meta.settings.mixed_port,
                true
            )),
            logs
        );
        Ok::<_, anyhow::Error>(json!({
            "core_dns":core.map_err(|error|format!("{error:#}")),
            "http":display(http),
            "ipv4":display(v4),
            "ipv6":display(v6),
            "core_logs":logs,
            "measurements":meta.measurements
        }))
    };
    let local = async {
        if args.get("output").is_some() {
            crate::diagnostics::collect(args).await
        } else {
            Ok(json!({}))
        }
    };
    let (system, system_http, backend_result, routes, local) = tokio::join!(
        bounded_probe(clash_client_core::probe::dns(host.into())),
        bounded_probe(clash_client_core::probe::website_system_route(url.into())),
        backend_probes,
        crate::host_network::diagnose(args),
        local
    );
    let (backend, backend_error) = match backend_result {
        Ok(value) => (value, None),
        Err(error) => (json!({}), Some(format!("{error:#}"))),
    };
    let mut report = json!({
        "dns_host":host,
        "system_dns":system.map_err(|e|format!("{e:#}")),
        "core_dns":backend["core_dns"],
        "http":backend["http"],
        "system_route_http":display(system_http),
        "host_routes":routes,
        "proxy_environment":{
            "http":proxy_environment_set("http_proxy"),
            "https":proxy_environment_set("https_proxy"),
            "all":proxy_environment_set("all_proxy"),
            "no_proxy":proxy_environment_set("no_proxy")
        },
        "ipv4":backend["ipv4"],
        "ipv6":backend["ipv6"],
        "core_logs":backend["core_logs"],
        "measurements":backend["measurements"],
        "backend_error":backend_error
    });
    match local {
        Ok(Value::Object(fields)) => report.as_object_mut().unwrap().extend(fields),
        Ok(_) => {}
        Err(error) => report["local_evidence_error"] = json!(format!("{error:#}")),
    }
    output::emit(args, report)
}

async fn bounded_probe<T>(probe: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(std::time::Duration::from_secs(15), probe)
        .await
        .context("单项检测超过 15 秒")?
}

fn proxy_environment_set(name: &str) -> bool {
    [name.to_owned(), name.to_ascii_uppercase()]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
}

async fn ip(args: &Args) -> Result<()> {
    use clash_client_core::ip_report::{Counts, Status};
    let category = args.get("category");
    let filter = args.get("filter").unwrap_or("").to_lowercase();
    let services: Vec<_> = ip_check::services()
        .iter()
        .filter(|s| {
            category.is_none_or(|c| s.group.eq_ignore_ascii_case(c))
                && args.positional.first().is_none_or(|id| &s.id == id)
                && (s.name.to_lowercase().contains(&filter)
                    || s.id.to_lowercase().contains(&filter))
        })
        .collect();
    ensure!(
        !services.is_empty(),
        "未找到检测项目；使用 cor ip-check list 查看 ID 与分类"
    );
    if args.path == "ip-check list" {
        return output::emit(
            args,
            json!(
                services
                    .iter()
                    .map(|s| json!({"id":s.id,"name":s.name,"category":s.group,"url":s.url}))
                    .collect::<Vec<_>>()
            ),
        );
    }
    let meta = backend::live(args).await?;
    let port = meta.settings.mixed_port;
    let mut waiting: VecDeque<_> = services.into_iter().collect();
    let mut tasks = tokio::task::JoinSet::new();
    let mut results = Vec::new();
    let mut counts = BTreeMap::<String, usize>::new();
    let mut totals = Counts::default();
    let mut categories = BTreeMap::<String, Counts>::new();
    loop {
        while tasks.len() < 3 {
            let Some(service) = waiting.pop_front() else {
                break;
            };
            tasks.spawn(async move {
                let mut row = json!({"id":service.id,"name":service.name,"category":service.group});
                let outcome = ip_check::check(&service.id, port)
                    .await
                    .map_err(|error| format!("{error:#}"));
                let status = Status::of(Some(&outcome));
                match outcome {
                    Ok(result) => {
                        row["state"] = json!(format!("{:?}", result.state));
                        row["summary"] = json!(result.summary);
                        row["country"] = json!(result.country);
                        row["millis"] = json!(result.millis);
                        row["detail"] = json!(result.detail);
                    }
                    Err(error) => {
                        row["state"] = json!("Failed");
                        row["detail"] = json!(format!("{error:#}"));
                    }
                }
                (row, status)
            });
        }
        let Some(row) = tasks.join_next().await else {
            break;
        };
        let (row, status) = row?;
        totals.0[status as usize] += 1;
        categories
            .entry(row["category"].as_str().unwrap_or("其他").into())
            .or_default()
            .0[status as usize] += 1;
        *counts
            .entry(row["state"].as_str().unwrap_or("Failed").into())
            .or_default() += 1;
        if args.flag("jsonl") && args.get("output").is_none() {
            output::emit(args, json!({"event":"result","result":row}))?;
        } else if !args.flag("quiet") {
            eprintln!(
                "已完成 {}：{}",
                results.len() + 1,
                row["name"].as_str().unwrap_or("")
            );
        }
        results.push(row);
    }
    results.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    let categories:Vec<_>=categories.into_iter().map(|(name,counts)|json!({"name":name,"total":counts.total(),"completed":counts.completed(),"non_red":counts.non_red(),"confirmed_available":counts.count(Status::Available)})).collect();
    output::emit(
        args,
        json!({"event":"report","total":results.len(),"non_red":totals.non_red(),"confirmed_available":totals.count(Status::Available),"counts":counts,"categories":categories,"results":results}),
    )
}

async fn claude(args: &Args) -> Result<()> {
    if args.path == "claude protect" {
        if let Some(value) = args.positional.first() {
            let enabled = output::on_off(value)?;
            if enabled {
                output::confirm(
                    args,
                    "Claude 保护会启用 TUN、覆盖 DNS 和广泛规则，锁定节点并关闭已有内核连接，可能影响其他服务。继续？",
                ).await?;
                super::maintenance::authorize_tun(args).await?;
            }
            let _: Value = backend::request(args, Command::Protect { enabled }).await?;
        }
        let meta = backend::meta(args).await?;
        return output::emit(
            args,
            json!({"enabled":meta.settings.claude_protection.is_some(),"protection":meta.settings.claude_protection}),
        );
    }
    if args.path == "claude watch" {
        let meta = backend::live(args).await?;
        let mut identity = (meta.settings.controller_port, meta.settings.secret.clone());
        let mut observer = claude_check::observe::Session::start(Api::new(&meta.settings)?);
        let mut revision = 0;
        let mut observed = Value::Null;
        loop {
            let poll: Poll = backend::request(
                args,
                Command::Snapshot {
                    scope: Scope::Claude,
                    header: false,
                },
            )
            .await?;
            let next = (
                poll.meta.settings.controller_port,
                poll.meta.settings.secret.clone(),
            );
            if next != identity {
                observer = claude_check::observe::Session::start(Api::new(&poll.meta.settings)?);
                identity = next;
                revision = 0;
            }
            if let Some(snapshot) = observer.after(revision) {
                revision = snapshot.revision;
                observed = json!({"connected":snapshot.connected,"total":snapshot.total,"failures":snapshot.failures,"routes":snapshot.routes});
            }
            output::emit(
                args,
                json!({"running":poll.meta.running,"enabled":poll.meta.settings.claude_protection.is_some(),"protection_status":poll.snapshot.protection_status,"guard":poll.snapshot.claude_guard,"observed":observed}),
            )?;
            tokio::time::sleep(args.interval()?).await;
        }
    }
    if args.path == "claude audit" && args.flag("offline") {
        let cancel = AuditScan(claude_check::audit::Scan::default());
        let worker = cancel.0.clone();
        let audit =
            tokio::task::spawn_blocking(move || claude_check::audit::local(0, &worker)).await?;
        let report = claude_check::system::code(true).await?;
        return output::emit(args, json!({"terminal":report,"projects":audit}));
    }
    let meta = backend::live(args).await?;
    if args.path == "claude audit" {
        let cancel = AuditScan(claude_check::audit::Scan::default());
        let worker = cancel.0.clone();
        let port = meta.settings.mixed_port;
        let task = tokio::task::spawn_blocking(move || claude_check::audit::local(port, &worker));
        let report = claude_check::system::code(false).await?;
        return output::emit(args, json!({"terminal":report,"projects":task.await?}));
    }
    let client = claude_check::client(Some(meta.settings.mixed_port))?;
    if args.path == "claude browser" {
        let session = claude_check::browser::Session::start(client).await?;
        // URL goes to stderr as well so machine-readable output remains a report stream.
        eprintln!("请在需要检测的浏览器中打开：{}", session.url);
        if args.flag("open") {
            clash_client_runtime::platform::open_diagnostic(&session.url)?;
        }
        let deadline = tokio::time::Instant::now() + backend::timeout(args)?;
        loop {
            if let Some(report) = session.next_report() {
                return output::emit(args, serde_json::to_value(report)?);
            }
            ensure!(
                tokio::time::Instant::now() < deadline && !session.finished(),
                "等待浏览器采样超时或入口已关闭"
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
    let mut report = claude_check::system::local(claude_check::Environment::Core).await;
    claude_check::network(&client, &mut report, "内核代理").await;
    claude_check::enhanced::extend(&client, &mut report).await;
    if let Ok(observed) = Api::new(&meta.settings)?
        .get::<claude_check::ObservedConnections>("connections")
        .await
    {
        report.routes = observed.connections;
    }
    let (normal, risk, clues) = report.counts();
    output::emit(
        args,
        json!({"normal":normal,"risk":risk,"clues":clues,"report":report}),
    )
}

struct AuditScan(claude_check::audit::Scan);
impl Drop for AuditScan {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slimming_keeps_only_rate_baseline_and_restart_invalidates_it() {
        let mut previous = vec![Connection {
            id: "1".into(),
            start: "a".into(),
            upload: 100,
            download: 200,
            ..Default::default()
        }];
        slim(&mut previous);
        let mut current = vec![Connection {
            id: "1".into(),
            start: "a".into(),
            upload: 150,
            download: 300,
            ..Default::default()
        }];
        connection::sample_rates(&mut current, &previous, std::time::Duration::from_secs(2));
        assert_eq!(current[0].rates.unwrap().download, 50);
        current[0].start = "b".into();
        connection::sample_rates(&mut current, &previous, std::time::Duration::from_secs(2));
        assert!(current[0].rates.is_none());
    }
}
