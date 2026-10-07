//! Native HTTP platform checks; upstream-derived decisions retain AGPL-3.0-only.
//! No interpreter, temporary response files, or HTTP utility processes.
use crate::ip_check::{CheckResult, Service, State, decorate_country};
use crate::probe::country_name;
use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine as _;
use regex::{Regex, RegexBuilder};
use reqwest::cookie::{CookieStore, Jar};
use reqwest::{Client, Method, Url};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[allow(
    clippy::all,
    unreachable_code,
    unused_assignments,
    unused_mut,
    unused_parens
)]
mod generated;
mod output;

pub async fn check(service: &Service, port: u16) -> Result<CheckResult> {
    let mut ctx = Context::new(port)?;
    let text = generated::execute(&service.id, &mut ctx).await?;
    let mut result = output::parse_output(&text);
    result.millis = ctx.first_response_ms.unwrap_or(0);
    if ctx.challenged {
        result.state = State::Unknown;
        result.summary = "验证拦截，未确认".into();
        result
            .detail
            .push_str("；响应包含浏览器验证，不能据此判断地区限制");
    } else if ctx.failed
        || result.millis == 0
        || (service.id == "AIUnlockTest_Claude" && ctx.denied)
    {
        result.state = State::Unknown;
        result.summary = "请求失败，未确认".into();
    } else if result.state == State::Restricted
        && result.summary == "不可用或受限"
        && matches!(
            service.id.as_str(),
            "MediaUnlockTest_ChatGPT" | "MediaUnlockTest_Sora"
        )
    {
        result.state = State::Unknown;
        result.summary = "未确认可用性".into();
    }
    Ok(result)
}

struct Context {
    client: Client,
    tls13_client: Client,
    first_response_ms: Option<u128>,
    failed: bool,
    challenged: bool,
    denied: bool,
    cookies: Jar,
    #[cfg(test)]
    test_origin: Option<Url>,
}

struct Request {
    url: String,
    method: String,
    headers: Vec<[String; 2]>,
    body: Option<String>,
    follow: bool,
    headers_only: bool,
    include_headers: bool,
    fail_status: bool,
    discard_body: bool,
    timeout: u64,
    writeout: String,
    cookie_store: bool,
    cookie_load: bool,
    retries: usize,
    tls13: bool,
}

impl Request {
    fn get(url: &str) -> Self {
        Self {
            url: url.into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            follow: false,
            headers_only: false,
            include_headers: false,
            fail_status: false,
            discard_body: false,
            timeout: 12,
            writeout: String::new(),
            cookie_store: false,
            cookie_load: false,
            retries: 0,
            tls13: false,
        }
    }
}

impl Context {
    fn new(port: u16) -> Result<Self> {
        ensure!(port != 0, "代理端口无效");
        let build = |tls13| -> Result<Client> {
            let mut builder = Client::builder()
                .no_proxy()
                .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(12));
            if tls13 {
                builder = builder.min_tls_version(reqwest::tls::Version::TLS_1_3);
            }
            Ok(builder.build()?)
        };
        Ok(Self {
            client: build(false)?,
            tls13_client: build(true)?,
            first_response_ms: None,
            failed: false,
            challenged: false,
            denied: false,
            cookies: Jar::default(),
            #[cfg(test)]
            test_origin: None,
        })
    }

    async fn request(&mut self, request: Request) -> Result<String> {
        for attempt in 0..=request.retries {
            let response =
                tokio::time::timeout(Duration::from_secs(request.timeout), self.perform(&request))
                    .await;
            match response {
                Ok(Ok(text)) => return Ok(text),
                _ if attempt < request.retries => {
                    tokio::time::sleep(Duration::from_millis(250)).await
                }
                _ => {
                    self.failed = true;
                    // Compatibility marker is consumed only by generated decisions.
                    // Request errors never expose URL credentials or response bodies.
                    return Ok("curl: request failed (000)".into());
                }
            }
        }
        unreachable!()
    }

    async fn perform(&mut self, request: &Request) -> Result<String> {
        let mut url = Url::parse(&request.url)?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none(),
            "无效的检测地址"
        );
        ensure!(url.host_str().is_some(), "检测地址缺少主机");
        let original_origin = url.origin();
        let mut method = Method::from_bytes(request.method.as_bytes())?;
        let mut body = request.body.clone();
        let mut header_text = String::new();
        for _ in 0..=10 {
            let started = Instant::now();
            #[cfg(test)]
            let transport_url = self.test_origin.as_ref().map_or_else(
                || url.clone(),
                |base| {
                    let mut target = base.clone();
                    target.set_path(url.path());
                    target.set_query(url.query());
                    target
                },
            );
            #[cfg(not(test))]
            let transport_url = url.clone();
            let client = if request.tls13 {
                &self.tls13_client
            } else {
                &self.client
            };
            let mut builder = client.request(method.clone(), transport_url);
            for [name, value] in &request.headers {
                // Empty HTTP headers in the upstream are intentionally omitted.
                if value.is_empty() {
                    continue;
                }
                if url.origin() != original_origin
                    && matches!(
                        name.to_ascii_lowercase().as_str(),
                        "authorization" | "cookie" | "host"
                    )
                {
                    continue;
                }
                builder = builder.header(name, value);
            }
            if request.cookie_load
                && let Some(cookies) = self.cookies.cookies(&url)
            {
                builder = builder.header("cookie", cookies);
            }
            if let Some(data) = &body {
                if !request
                    .headers
                    .iter()
                    .any(|[key, _]| key.eq_ignore_ascii_case("content-type"))
                {
                    builder = builder.header("content-type", "application/x-www-form-urlencoded");
                }
                builder = builder.body(data.clone());
            }
            let mut response = builder.send().await?;
            self.first_response_ms
                .get_or_insert(started.elapsed().as_millis().max(1));
            let status = response.status();
            self.denied |= status.as_u16() >= 400;
            self.challenged |= response
                .headers()
                .get("cf-mitigated")
                .and_then(|h| h.to_str().ok())
                .is_some_and(|h| h.eq_ignore_ascii_case("challenge"));
            let location = response
                .headers()
                .get("location")
                .and_then(|h| h.to_str().ok())
                .map(str::to_owned);
            header_text.push_str(&format!(
                "HTTP/1.1 {} {}\r\n",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            ));
            for (key, value) in response.headers() {
                if let Ok(value) = value.to_str() {
                    header_text.push_str(&format!("{}: {value}\r\n", key.as_str()));
                }
            }
            header_text.push_str("\r\n");
            ensure!(header_text.len() <= 65536, "检测响应头超过限制");
            if request.cookie_store || request.cookie_load {
                self.cookies
                    .set_cookies(&mut response.headers().get_all("set-cookie").iter(), &url);
            }
            if request.follow
                && status.is_redirection()
                && let Some(location) = location
            {
                url = url.join(&location)?;
                ensure!(
                    matches!(url.scheme(), "http" | "https")
                        && url.username().is_empty()
                        && url.password().is_none(),
                    "无效的重定向地址"
                );
                if status.as_u16() == 303
                    || (matches!(status.as_u16(), 301 | 302) && method == Method::POST)
                {
                    method = Method::GET;
                    body = None;
                }
                continue;
            }
            if request.fail_status && status.as_u16() >= 400 {
                // Upstream's -f requests regard HTTP errors as failed transports.
                bail!("HTTP 请求失败");
            }
            let mut bytes = Vec::new();
            if !request.headers_only {
                while let Some(chunk) = response.chunk().await? {
                    ensure!(
                        bytes.len() + chunk.len() <= 2 * 1024 * 1024,
                        "检测响应超过 2 MiB"
                    );
                    bytes.extend_from_slice(&chunk);
                }
            }
            let mut text = if request.headers_only {
                header_text.clone()
            } else if request.discard_body {
                String::new()
            } else {
                String::from_utf8_lossy(&bytes).into_owned()
            };
            if request.include_headers && !request.headers_only {
                text = header_text + &text;
            }
            text.push_str(&decode_escapes(
                &request
                    .writeout
                    .replace("%{http_code}", &status.as_u16().to_string())
                    .replace("%{url_effective}", url.as_str()),
            ));
            return Ok(text);
        }
        bail!("检测重定向超过限制")
    }

    fn clear_cookies(&mut self) {
        self.cookies = Jar::default();
    }

    async fn resolve_host(&self, host: &str, family: &str) -> Result<String> {
        #[cfg(test)]
        if self.test_origin.is_some() {
            return Ok(format!("8.8.8.8 STREAM {host}"));
        }
        resolve_host(host, family).await
    }

    async fn detect_isp(&mut self, ip: &str) -> Result<String> {
        let address: std::net::IpAddr = ip.parse()?;
        if matches!(address, std::net::IpAddr::V4(ip) if ip.is_private()) {
            return Ok("LAN".into());
        }
        let body = self
            .request(Request::get(&format!("https://api.ip.sb/geoip/{address}")))
            .await?;
        Ok(json_field(&body, ".isp")?.trim_matches('"').to_owned())
    }
}

struct Variables(HashMap<String, String>);
impl Variables {
    fn new() -> Self {
        Self(HashMap::from([
            ("1".into(), "4".into()), ("?".into(), "0".into()),
            ("UA_Browser".into(), "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/112.0.0.0 Safari/537.36 Edg/112.0.1722.64".into()),
            ("UA_Dalvik".into(), "Dalvik/2.1.0 (Linux; U; Android 9; ALP-AL00 Build/HUAWEIALP-AL00)".into()),
            ("UA_SecCHUA".into(), "\"Chromium\";v=\"112\", \"Microsoft Edge\";v=\"112\"".into()),
            ("Media_Cookie".into(), include_str!("../vendor/region-restriction-check/cookies").into()),
            ("IATACode".into(), include_str!("../vendor/region-restriction-check/IATACode.txt").into()),
        ]))
    }
    fn get(&self, key: &str) -> String {
        self.0.get(key).cloned().unwrap_or_default()
    }
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.into(), value);
    }
}

fn header_pair(text: &str) -> Result<[String; 2]> {
    let (key, value) = text.split_once(':').context("检测请求头格式无效")?;
    Ok([key.trim().into(), value.trim().into()])
}

fn number(text: &str) -> i64 {
    text.parse().unwrap_or(i64::MIN)
}
fn random_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
fn random_digit() -> String {
    (uuid::Uuid::new_v4().as_u128() % 10).to_string()
}
fn timestamp(millis: bool) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    if millis {
        now.as_millis().to_string()
    } else {
        now.as_secs().to_string()
    }
}

async fn resolve_host(host: &str, family: &str) -> Result<String> {
    ensure!(
        matches!(host, "www.netflix.com" | "api.bilibili.com"),
        "检测 DNS 主机无效"
    );
    let mut addresses = Vec::new();
    for address in tokio::net::lookup_host((host, 443)).await? {
        if address.is_ipv4() == (family == "ahostsv4") {
            addresses.push(format!("{} STREAM {host}", address.ip()));
        }
    }
    Ok(addresses.join("\n"))
}

fn decode_escapes(text: &str) -> String {
    text.replace("\\r", "\r")
        .replace("\\n", "\n")
        .replace("\\t", "\t")
        .replace("\\\"", "\"")
        .replace("\\/", "/")
}

fn slice_text(text: &str, start: i64, length: Option<i64>) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len() as i64;
    let start = if start < 0 { n + start } else { start }.clamp(0, n);
    let end = match length {
        None => n,
        Some(len) if len < 0 => n + len,
        Some(len) => start + len,
    }
    .clamp(start, n);
    chars[start as usize..end as usize].iter().collect()
}

fn matches_pattern(text: &str, pattern: &str) -> bool {
    let mut expression = String::from("(?s)^");
    for c in pattern.chars() {
        match c {
            '*' => expression.push_str(".*"),
            '?' => expression.push('.'),
            _ => expression.push_str(&regex::escape(&c.to_string())),
        }
    }
    expression.push('$');
    Regex::new(&expression).is_ok_and(|r| r.is_match(text))
}

fn trim_prefix_pattern(text: &str, pattern: &str) -> String {
    for (i, _) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        if matches_pattern(&text[..i], pattern) {
            return text[i..].into();
        }
    }
    text.into()
}

fn json_field(text: &str, path: &str) -> Result<String> {
    // jq accepts whitespace-delimited JSON values. Google's batchexecute replies
    // include an anti-XSSI line; its selected JSON line is parsed independently.
    let mut value: serde_json::Value =
        serde_json::from_str(text.trim()).context("检测响应不是有效 JSON")?;
    if path != "." {
        let segments = path
            .trim_start_matches('.')
            .replace('[', ".")
            .replace(']', "");
        for key in segments.split('.').filter(|k| !k.is_empty()) {
            value = if let Ok(index) = key.parse::<usize>() {
                value.get(index)
            } else {
                value.get(key)
            }
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        }
    }
    Ok(serde_json::to_string_pretty(&value)?)
}

fn basic_regex(pattern: &str) -> String {
    // Translate the basic-regex grouping/quantifier spelling used by the pinned
    // upstream; escaped dollar signs and literal brackets remain escaped.
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek().is_some_and(|c| "()+?|{}".contains(*c)) {
            out.push(chars.next().unwrap());
        } else if "()+?|{}".contains(c) {
            out.push('\\');
            out.push(c);
        } else {
            out.push(c);
        }
    }
    out
}

fn select_lines(text: &str, args: &[String]) -> Result<String> {
    let (mut insensitive, mut only, mut extended, mut word, mut invert) =
        (false, false, false, false, false);
    let mut pattern = None;
    for arg in args {
        if arg.starts_with('-') && pattern.is_none() {
            insensitive |= arg.contains('i');
            only |= arg.contains('o');
            extended |= arg.contains('E') || arg.contains('P');
            word |= arg.contains('w');
            invert |= arg.contains('v');
        } else if arg.starts_with('-') && arg.chars().skip(1).all(|c| "iEoPqwv".contains(c)) {
            insensitive |= arg.contains('i');
        } else {
            ensure!(pattern.is_none(), "多个检测匹配表达式");
            pattern = Some(arg.as_str());
        }
    }
    let pattern = pattern.context("缺少匹配表达式")?;
    let (pattern, capture) = if let Some((prefix, suffix)) = pattern.split_once("\\K") {
        (format!("(?:{prefix})({suffix})"), true)
    } else {
        (
            if extended {
                pattern.into()
            } else {
                basic_regex(pattern)
            },
            false,
        )
    };
    let regex = RegexBuilder::new(&pattern)
        .case_insensitive(insensitive)
        .build()?;
    let mut result = Vec::new();
    for line in text.lines() {
        if only {
            for captures in regex.captures_iter(line) {
                let m = captures.get(usize::from(capture)).unwrap();
                if word {
                    let before = line[..m.start()].chars().next_back();
                    let after = line[m.end()..].chars().next();
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                        || after.is_some_and(|c| c.is_alphanumeric() || c == '_')
                    {
                        continue;
                    }
                }
                result.push(m.as_str().to_owned());
            }
        } else if regex.is_match(line) != invert {
            result.push(line.to_owned());
        }
    }
    Ok(result.join("\n"))
}

fn select_fields(text: &str, args: &[String]) -> Result<String> {
    let mut delimiter = None;
    let mut rule = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-F" {
            i += 1;
            delimiter = Some(args[i].as_str());
        } else if let Some(d) = args[i].strip_prefix("-F") {
            delimiter = Some(d);
        } else {
            rule = Some(args[i].as_str());
        }
        i += 1;
    }
    let rule = rule.context("缺少字段选择规则")?;
    if rule.contains("END { print code }") {
        return Ok(text
            .lines()
            .filter(|l| l.starts_with("HTTP/"))
            .filter_map(|l| l.split_whitespace().nth(1))
            .next_back()
            .unwrap_or("")
            .into());
    }
    if let Some(n) = rule.strip_prefix("NR==") {
        return Ok(text
            .lines()
            .nth(n.parse::<usize>()?.saturating_sub(1))
            .unwrap_or("")
            .into());
    }
    let field: usize = rule
        .trim()
        .strip_prefix("{print $")
        .and_then(|r| r.strip_suffix('}'))
        .context("无效字段规则")?
        .trim()
        .parse()?;
    Ok(text
        .lines()
        .map(|l| {
            if let Some(d) = delimiter {
                l.split(d).nth(field.saturating_sub(1)).unwrap_or("")
            } else {
                l.split_whitespace()
                    .nth(field.saturating_sub(1))
                    .unwrap_or("")
            }
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn split_fields(text: &str, args: &[String]) -> Result<String> {
    let (mut delimiter, mut field, mut count) = ("\t".to_owned(), None, None);
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if matches!(arg.as_str(), "-d" | "-f" | "-c") {
            i += 1;
            match arg.as_str() {
                "-d" => delimiter.clone_from(&args[i]),
                "-f" => field = Some(args[i].parse::<usize>()?),
                _ => count = Some(args[i].split('-').next_back().unwrap().parse::<usize>()?),
            }
        } else if let Some(d) = arg.strip_prefix("-d") {
            delimiter = d.into();
        } else if let Some(f) = arg.strip_prefix("-f") {
            field = Some(f.parse()?);
        } else if let Some(c) = arg.strip_prefix("-c") {
            count = Some(c.split('-').next_back().unwrap().parse()?);
        } else {
            bail!("无效的字段分隔参数");
        }
        i += 1;
    }
    Ok(text
        .lines()
        .map(|line| {
            if let Some(count) = count {
                line.chars().take(count).collect()
            } else if !line.contains(&delimiter) {
                line.to_owned()
            } else {
                line.split(&delimiter)
                    .nth(field.unwrap_or(1).saturating_sub(1))
                    .unwrap_or("")
                    .into()
            }
        })
        .collect::<Vec<String>>()
        .join("\n"))
}

fn map_characters(text: &str, args: &[String]) -> Result<String> {
    if args.len() == 2 && args[0] == "-d" {
        return Ok(text
            .chars()
            .filter(|c| {
                if args[1] == "[:cntrl:]" {
                    !c.is_control()
                } else if args[1] == "\\r" {
                    *c != '\r'
                } else {
                    !args[1].contains(*c)
                }
            })
            .collect());
    }
    ensure!(args.len() == 2, "无效字符转换参数");
    match (args[0].as_str(), args[1].as_str()) {
        ("a-z" | "[:lower:]", "A-Z" | "[:upper:]") => Ok(text.to_uppercase()),
        _ => bail!("不支持的字符转换"),
    }
}

fn take_lines(text: &str, args: &[String]) -> Result<String> {
    let count = if args.is_empty() {
        10
    } else {
        args.last().unwrap().trim_start_matches('-').parse()?
    };
    Ok(text.lines().take(count).collect::<Vec<_>>().join("\n"))
}
fn sort_lines(text: &str) -> String {
    let mut lines: Vec<_> = text.lines().collect();
    lines.sort();
    lines.join("\n")
}
fn unique_lines(text: &str) -> String {
    let mut lines: Vec<_> = text.lines().collect();
    lines.dedup();
    lines.join("\n")
}

fn replace_text(text: &str, args: &[String]) -> Result<String> {
    let selected = args.iter().any(|a| a == "-n");
    let rules: Vec<_> = args
        .iter()
        .filter(|a| !matches!(a.as_str(), "-n" | "-e"))
        .collect();
    ensure!(rules.len() == 1, "无效替换规则");
    let rule = rules[0];
    if selected && rule.ends_with('p') && rule[..rule.len() - 1].chars().all(|c| c.is_ascii_digit())
    {
        return Ok(text
            .lines()
            .nth(rule[..rule.len() - 1].parse::<usize>()?.saturating_sub(1))
            .unwrap_or("")
            .into());
    }
    if selected && rule.starts_with('/') && rule.ends_with("/=") {
        let regex = Regex::new(&basic_regex(&rule[1..rule.len() - 2]))?;
        return Ok(text
            .lines()
            .enumerate()
            .filter(|(_, l)| regex.is_match(l))
            .map(|(i, _)| (i + 1).to_string())
            .collect::<Vec<_>>()
            .join("\n"));
    }
    let mut current = text.to_owned();
    // The one semicolon-separated substitution chain is URL percent encoding.
    // Splitting on ';s' keeps the literal semicolon replacement intact.
    for (index, part) in rule.split(";s").enumerate() {
        let owned;
        let part = if index == 0 {
            part
        } else {
            owned = format!("s{part}");
            &owned
        };
        ensure!(part.starts_with('s') && part.len() >= 2, "无效文本替换");
        let delimiter = part.as_bytes()[1] as char;
        let mut pieces = Vec::new();
        let mut chunk = String::new();
        let mut chars = part[2..].chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' && chars.peek() == Some(&delimiter) {
                chunk.push(chars.next().unwrap());
            } else if c == delimiter {
                pieces.push(std::mem::take(&mut chunk));
            } else {
                chunk.push(c);
            }
        }
        pieces.push(chunk);
        ensure!(pieces.len() == 3, "无效文本替换字段");
        let regex = RegexBuilder::new(&basic_regex(&pieces[0]))
            .case_insensitive(pieces[2].contains('i'))
            .build()?;
        let mut replacement = String::new();
        let mut replacement_chars = pieces[1].chars().peekable();
        while let Some(c) = replacement_chars.next() {
            if c == '\\' && replacement_chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                replacement.push('$');
                replacement.push(replacement_chars.next().unwrap());
            } else if c == '$' {
                replacement.push_str("$$");
            } else {
                replacement.push(c);
            }
        }
        let mut lines = Vec::new();
        for line in current.lines() {
            if selected && !regex.is_match(line) {
                continue;
            }
            let replaced = if pieces[2].contains('g') {
                regex.replace_all(line, replacement.as_str())
            } else {
                regex.replace(line, replacement.as_str())
            };
            lines.push(replaced.into_owned());
        }
        current = lines.join("\n");
    }
    Ok(current)
}

fn sign_sha1(text: &str, key: &str) -> Result<String> {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(key.as_bytes())?;
    mac.update(text.as_bytes());
    Ok(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn mock(body: &str, extra_headers: &str) -> (Context, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = body.to_owned();
        let headers = extra_headers.to_owned();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let body = body.clone();
                let headers = headers.clone();
                tokio::spawn(async move {
                    let mut data = Vec::new();
                    let mut buffer = [0; 4096];
                    loop {
                        let count = socket.read(&mut buffer).await.unwrap();
                        if count == 0 {
                            return;
                        }
                        data.extend_from_slice(&buffer[..count]);
                        if data.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        let mut ctx = Context::new(1).unwrap();
        ctx.client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        ctx.tls13_client = ctx.client.clone();
        ctx.test_origin = Some(Url::parse(&format!("http://{address}")).unwrap());
        (ctx, task)
    }

    #[tokio::test]
    async fn every_catalog_detector_executes_with_native_mock_http() {
        // Broad fixture exercises the generated request/parse chains without
        // depending on public services, credentials, DNS, or bundled tools.
        let fixture = r#"{"isp":"Netflix","client":{"isp":"Netflix"},"targets":[{"location":{"city":"Tokyo","country":"JP"},"url":"https://example.test/cdn"}],"error":{"name":"LOGIN_FORBIDDEN"},"cf_details":null,"result":{"value":"Success"},"data":{"country_code":86,"country":"CN"},"Region":{"isAllowed":true,"GeolocatedCountry":"JP"}}"#;
        let mut failures = Vec::new();
        for service in crate::ip_check::services()
            .iter()
            .filter(|s| !matches!(s.id.as_str(), "exit-ip" | "github"))
        {
            let special;
            let body = if matches!(
                service.id.as_str(),
                "AIUnlockTest_Gemini_location" | "MediaUnlockTest_Google"
            ) {
                special = serde_json::json!([["K4WWud", null, "[[\"JP\",\"S"]]).to_string();
                &special
            } else if service.id == "MediaUnlockTest_Netflix" {
                "netflix.reactContext = {\"models\":{\"geo\":{\"data\":{\"requestCountry\":{\"id\":\"JP\"}}}}};\n<meta og:video=yes>"
            } else {
                fixture
            };
            let (mut ctx, task) = mock(body, "").await;
            if let Err(error) = generated::execute(&service.id, &mut ctx).await {
                failures.push(format!("{}: {error:#}", service.id));
            }
            task.abort();
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[tokio::test]
    async fn json_decisions_distinguish_allowed_restricted_and_malformed() {
        for (body, expected) in [
            (
                r#"{"Region":{"isAllowed":true,"GeolocatedCountry":"JP"}}"#,
                State::Confirmed,
            ),
            (r#"{"Region":{"isAllowed":false}}"#, State::Restricted),
        ] {
            let (mut ctx, task) = mock(body, "").await;
            let text = generated::execute("MediaUnlockTest_Dazn", &mut ctx)
                .await
                .unwrap();
            assert_eq!(output::parse_output(&text).state, expected);
            task.abort();
        }
        let (mut ctx, task) = mock("<html>verification</html>", "").await;
        assert!(
            generated::execute("MediaUnlockTest_Dazn", &mut ctx)
                .await
                .is_err()
        );
        task.abort();
    }

    #[tokio::test]
    async fn browser_challenges_and_cookie_store_stay_in_process() {
        let (mut ctx, task) = mock(
            "{}",
            "cf-mitigated: challenge\r\nSet-Cookie: session=abc; Path=/; HttpOnly\r\n",
        )
        .await;
        let mut request = Request::get("https://example.test/start");
        request.cookie_store = true;
        assert_eq!(ctx.request(request).await.unwrap(), "{}");
        assert!(ctx.challenged);
        let url = Url::parse("https://example.test/start").unwrap();
        assert_eq!(ctx.cookies.cookies(&url).unwrap(), "session=abc");
        assert!(
            ctx.cookies
                .cookies(&Url::parse("https://other.test/").unwrap())
                .is_none()
        );
        ctx.clear_cookies();
        assert!(ctx.cookies.cookies(&url).is_none());
        task.abort();
    }

    #[tokio::test]
    async fn native_requests_are_forced_through_loopback_proxy() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut ctx = Context::new(listener.local_addr().unwrap().port()).unwrap();
        let pending = tokio::spawn(async move {
            ctx.request(Request::get("http://destination.invalid/check"))
                .await
                .unwrap()
        });
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut data = [0; 4096];
        let count = socket.read(&mut data).await.unwrap();
        let text = String::from_utf8_lossy(&data[..count]);
        assert!(text.starts_with("GET http://destination.invalid/check HTTP/1.1"));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
            .await
            .unwrap();
        assert_eq!(pending.await.unwrap(), "OK");
    }

    #[tokio::test]
    async fn cross_origin_redirect_does_not_forward_credentials_or_post_body() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            for redirected in [false, true] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                let text = String::from_utf8_lossy(&buffer[..count]).to_ascii_lowercase();
                if redirected {
                    assert!(text.starts_with("get /next "));
                    assert!(
                        !text.contains("authorization:")
                            && !text.contains("cookie:")
                            && !text.contains("secret")
                    );
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK",
                        )
                        .await
                        .unwrap();
                } else {
                    assert!(text.contains("authorization: bearer secret"));
                    socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://other.test/next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                }
            }
        });
        let mut ctx = Context::new(1).unwrap();
        ctx.client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        ctx.test_origin = Some(origin);
        let mut request = Request::get("https://example.test/start");
        request.method = "POST".into();
        request.body = Some("secret".into());
        request.headers = vec![
            ["Authorization".into(), "Bearer secret".into()],
            ["Cookie".into(), "session=secret".into()],
        ];
        request.follow = true;
        assert_eq!(ctx.request(request).await.unwrap(), "OK");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn oversized_native_response_is_not_treated_as_success() {
        let (mut ctx, task) = mock(&"x".repeat(2 * 1024 * 1024 + 1), "").await;
        assert_eq!(
            ctx.request(Request::get("https://example.test/large"))
                .await
                .unwrap(),
            "curl: request failed (000)"
        );
        assert!(ctx.failed);
        task.abort();
    }

    #[tokio::test]
    async fn bahamut_multi_step_detection_preserves_device_id_and_scoped_cookie() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            for step in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                let text = String::from_utf8_lossy(&buffer[..count]);
                let (body, cookie) = if step == 0 {
                    assert!(text.starts_with("GET /ajax/getdeviceid.php "));
                    (
                        r#"{"deviceid":"device-123"}"#,
                        "Set-Cookie: session=abc; Path=/ajax/; Secure\r\n",
                    )
                } else {
                    assert!(text.contains("device=device-123"));
                    assert!(text.contains("cookie: session=abc"));
                    assert!(text.contains(if step == 1 { "sn=38832" } else { "sn=37783" }));
                    (r#"{"animeSn":38832}"#, "")
                };
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n{cookie}\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        let mut ctx = Context::new(1).unwrap();
        ctx.client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        ctx.test_origin = Some(origin);
        let output = generated::execute("MediaUnlockTest_BahamutAnime", &mut ctx)
            .await
            .unwrap();
        assert_eq!(output::parse_output(&output).state, State::Confirmed);
        assert!(output.contains("Region: TW"));
        assert!(
            ctx.cookies
                .cookies(&Url::parse("https://ani.gamer.com.tw/ajax/").unwrap())
                .is_none()
        );
        server.await.unwrap();
    }

    #[test]
    fn native_transformations_preserve_pinned_extraction_and_signatures() {
        assert_eq!(
            replace_text(
                "x RevIpCC:\"jp\" end",
                &["-n".into(), r#"s/.*RevIpCC:\"\([^\"]*\)\".*/\1/p"#.into()]
            )
            .unwrap(),
            "jp"
        );
        assert_eq!(
            select_lines(
                "token country=JP more",
                &["-woP".into(), r"country=\K[A-Z]{2}".into()]
            )
            .unwrap(),
            "JP"
        );
        assert_eq!(
            json_field(r#"{"a":[{"b":true}]}"#, ".a[0].b").unwrap(),
            "true"
        );
        assert_eq!(slice_text("abcdef", 2, Some(-1)), "cde");
        assert!(matches_pattern("curl: failed", "curl*"));
        assert_eq!(
            sign_sha1("The quick brown fox jumps over the lazy dog", "key").unwrap(),
            "3nybhbi3iqa8ino29wqQcBydtNk="
        );
    }

    #[tokio::test]
    async fn cancellation_drops_native_http_without_descendant_processes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut ctx = Context::new(1).unwrap();
        ctx.client = Client::builder().no_proxy().build().unwrap();
        let address = listener.local_addr().unwrap();
        let request = Request::get(&format!("http://{address}/hang"));
        let pending = tokio::spawn(async move { ctx.request(request).await });
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 4096];
        assert!(socket.read(&mut buffer).await.unwrap() > 0);
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), socket.read(&mut buffer))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
}
