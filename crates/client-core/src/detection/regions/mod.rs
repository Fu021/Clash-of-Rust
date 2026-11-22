//! Native HTTP platform checks; upstream-derived decisions retain AGPL-3.0-only.
//! No interpreter, temporary response files, or HTTP utility processes.
use crate::ip_check::{CheckResult, Service, State, decorate_country};
use crate::probe::country_name;
use anyhow::{Context as _, Result, bail, ensure};
use regex::{Regex, RegexBuilder};
use reqwest::cookie::{CookieStore, Jar};
use reqwest::{Client, Method, StatusCode, Url};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

mod output;
#[allow(
    clippy::all,
    unreachable_code,
    unused_assignments,
    unused_mut,
    unused_parens
)]
mod platforms;

pub async fn check(service: &Service, port: u16) -> Result<CheckResult> {
    let mut ctx = Context::new(port)?;
    let Some(text) = platforms::execute(&service.id, &mut ctx).await? else {
        return Ok(CheckResult {
            state: State::Unknown,
            summary: "未确认可用性".into(),
            country: "未提供".into(),
            millis: None,
            authentication_check_skipped: true,
            detail: "未执行认证检测".into(),
            target_exit: None,
        });
    };
    Ok(platform_result(service, &ctx, &text))
}

fn platform_result(service: &Service, ctx: &Context, text: &str) -> CheckResult {
    let mut result = output::parse_output(text);
    result.millis = ctx.first_response_ms;
    if service.information_only()
        && matches!(
            result.state,
            State::Confirmed | State::Partial | State::Reachable
        )
    {
        result.state = State::Identified;
        result.summary = "已获取地区或网络信息".into();
    }
    if ctx.challenged {
        result.state = State::Unknown;
        result.summary = "验证拦截，未确认".into();
        result.detail.push_str("；响应包含浏览器验证");
    } else if ctx.failed || result.millis.is_none() {
        result.state = State::Failed;
        result.summary = "请求失败，未确认".into();
    } else if service.id == "AIUnlockTest_Claude" && ctx.denied {
        result.state = State::Unknown;
        result.summary = "请求被拒绝，未确认".into();
    } else if result.state == State::Restricted
        && result.summary == "不可用或受限"
        && service.id == "MediaUnlockTest_ChatGPT"
    {
        result.state = State::Unknown;
        result.summary = "未确认可用性".into();
    }
    result
}

struct Context {
    clients: std::sync::Arc<Clients>,
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

#[derive(Clone, Copy)]
struct PrefixRead {
    status: StatusCode,
    limit_for_first_byte: fn(u8) -> usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyEnd {
    Complete,
    Truncated,
    LimitReached,
    Skipped,
}

struct ByteResponse {
    status: StatusCode,
    url: Url,
    body: Vec<u8>,
    end: BodyEnd,
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

struct Clients {
    normal: Client,
    tls13: std::sync::Mutex<Option<Client>>,
    port: u16,
}
impl Clients {
    fn build(port: u16, tls13: bool) -> Result<Client> {
        let mut builder = crate::network::client_builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .pool_max_idle_per_host(1)
            .pool_idle_timeout(Duration::from_secs(15));
        if tls13 {
            builder = builder.tls_version_min(reqwest::tls::Version::TLS_1_3);
        }
        Ok(builder.build()?)
    }
    fn shared(port: u16) -> Result<std::sync::Arc<Self>> {
        type Cache = std::sync::Mutex<Vec<(u16, std::sync::Weak<Clients>)>>;
        static CACHE: Cache = std::sync::Mutex::new(Vec::new());
        let mut cache = CACHE
            .lock()
            .map_err(|_| anyhow::anyhow!("检测客户端缓存不可用"))?;
        cache.retain(|(_, clients)| clients.strong_count() > 0);
        if let Some(clients) = cache.iter().find_map(|(cached_port, clients)| {
            (*cached_port == port).then(|| clients.upgrade()).flatten()
        }) {
            return Ok(clients);
        }
        let clients = std::sync::Arc::new(Self {
            normal: Self::build(port, false)?,
            tls13: std::sync::Mutex::new(None),
            port,
        });
        if cache.len() == 8 {
            cache.remove(0);
        }
        cache.push((port, std::sync::Arc::downgrade(&clients)));
        Ok(clients)
    }
    fn client(&self, tls13: bool) -> Result<Client> {
        if !tls13 {
            return Ok(self.normal.clone());
        }
        let mut client = self
            .tls13
            .lock()
            .map_err(|_| anyhow::anyhow!("检测客户端缓存不可用"))?;
        if client.is_none() {
            *client = Some(Self::build(self.port, true)?);
        }
        Ok(client.as_ref().unwrap().clone())
    }
}

impl Context {
    fn new(port: u16) -> Result<Self> {
        ensure!(port != 0, "代理端口无效");
        Ok(Self {
            clients: Clients::shared(port)?,
            first_response_ms: None,
            failed: false,
            challenged: false,
            denied: false,
            // Cookies stay private to this detector, independently of shared transports.
            cookies: Jar::default(),
            #[cfg(test)]
            test_origin: None,
        })
    }

    #[cfg(test)]
    fn set_test_client(&mut self, client: Client) {
        self.clients = std::sync::Arc::new(Clients {
            normal: client.clone(),
            tls13: std::sync::Mutex::new(Some(client)),
            port: self.clients.port,
        });
    }

    fn record_response(&mut self, elapsed: Duration) {
        self.first_response_ms.get_or_insert(elapsed.as_millis());
    }

    async fn request(&mut self, request: Request) -> Result<String> {
        Ok(match self.request_bytes(request, None).await {
            Ok(response) => decode_body(response.body),
            Err(_) => "curl: request failed (000)".into(),
        })
    }

    async fn request_bytes(
        &mut self,
        request: Request,
        prefix: Option<PrefixRead>,
    ) -> Result<ByteResponse> {
        for attempt in 0..=request.retries {
            let response = tokio::time::timeout(
                Duration::from_secs(request.timeout),
                self.perform_bytes(&request, prefix),
            )
            .await;
            match response {
                Ok(Ok(response)) => return Ok(response),
                _ if attempt < request.retries => {
                    tokio::time::sleep(Duration::from_millis(250)).await
                }
                _ => {
                    self.failed = true;
                    // Request errors never expose URL credentials or response bodies.
                    bail!("检测请求失败");
                }
            }
        }
        unreachable!()
    }

    #[cfg(test)]
    async fn perform(&mut self, request: &Request) -> Result<String> {
        Ok(decode_body(self.perform_bytes(request, None).await?.body))
    }

    async fn perform_bytes(
        &mut self,
        request: &Request,
        prefix: Option<PrefixRead>,
    ) -> Result<ByteResponse> {
        const BODY_LIMIT: usize = 2 * 1024 * 1024;
        if prefix.is_some() {
            ensure!(
                !request.headers_only
                    && !request.include_headers
                    && !request.discard_body
                    && request.writeout.is_empty(),
                "无效的检测响应前缀选项"
            );
        }
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
        let mut body = request.body.as_deref();
        let mut header_text = String::new();
        for _ in 0..=10 {
            let started = Instant::now();
            let same_origin = url.origin() == original_origin;
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
            let client = self.clients.client(request.tls13)?;
            let mut builder = client.request(method.clone(), transport_url);
            for [name, value] in &request.headers {
                // Empty HTTP headers in the upstream are intentionally omitted.
                if value.is_empty() {
                    continue;
                }
                if !same_origin
                    && !matches!(
                        name.to_ascii_lowercase().as_str(),
                        "user-agent" | "accept-language" | "accept-encoding"
                    )
                {
                    continue;
                }
                builder = builder.header(name, value);
            }
            if same_origin
                && request.cookie_load
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
                builder = builder.body((*data).to_owned());
            }
            let mut response = builder.send().await?;
            self.record_response(started.elapsed());
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
            if request.headers_only || request.include_headers {
                use std::fmt::Write as _;
                write!(
                    header_text,
                    "HTTP/1.1 {} {}\r\n",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or("")
                )?;
                for (key, value) in response.headers() {
                    if let Ok(value) = value.to_str() {
                        write!(header_text, "{}: {value}\r\n", key.as_str())?;
                    }
                }
                header_text.push_str("\r\n");
                ensure!(header_text.len() <= 65536, "检测响应头超过限制");
            }
            if same_origin && (request.cookie_store || request.cookie_load) {
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
                ensure!(
                    body.is_none() || url.origin() == original_origin,
                    "跨源重定向不能重发请求体"
                );
                continue;
            }
            if request.fail_status && status.as_u16() >= 400 {
                // Upstream's -f requests regard HTTP errors as failed transports.
                bail!("HTTP 请求失败");
            }
            if prefix.is_some_and(|read| status != read.status) {
                return Ok(ByteResponse {
                    status,
                    url,
                    body: Vec::new(),
                    end: BodyEnd::Skipped,
                });
            }
            let mut bytes = Vec::new();
            let mut received = 0usize;
            let mut prefix_limit = None;
            let mut end = BodyEnd::Complete;
            let content_length = response.content_length();
            if !request.headers_only {
                while let Some(chunk) = response.chunk().await? {
                    if let Some(read) = prefix
                        && prefix_limit.is_none()
                        && let Some(&first) = chunk.first()
                    {
                        let limit = (read.limit_for_first_byte)(first);
                        ensure!((1..=BODY_LIMIT).contains(&limit), "无效的检测响应前缀长度");
                        prefix_limit = Some(limit);
                    }
                    let length = if let Some(limit) = prefix_limit {
                        chunk.len().min(limit - received)
                    } else {
                        ensure!(chunk.len() <= BODY_LIMIT - received, "检测响应超过 2 MiB");
                        chunk.len()
                    };
                    received += length;
                    if !request.discard_body {
                        bytes.extend_from_slice(&chunk[..length]);
                    }
                    if prefix_limit == Some(received) {
                        let retained = received as u64;
                        end = if length < chunk.len()
                            || content_length.is_some_and(|length| length > retained)
                        {
                            BodyEnd::Truncated
                        } else if content_length == Some(retained) {
                            BodyEnd::Complete
                        } else {
                            BodyEnd::LimitReached
                        };
                        break;
                    }
                }
            }
            let mut output = if request.headers_only {
                header_text.into_bytes()
            } else if request.include_headers {
                let mut output = header_text.into_bytes();
                output.extend_from_slice(&bytes);
                output
            } else {
                bytes
            };
            output.extend_from_slice(
                decode_escapes(
                    &request
                        .writeout
                        .replace("%{http_code}", &status.as_u16().to_string())
                        .replace("%{url_effective}", url.as_str()),
                )
                .as_bytes(),
            );
            return Ok(ByteResponse {
                status,
                url,
                body: output,
                end,
            });
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
}

fn decode_body(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

struct Variables(HashMap<&'static str, std::borrow::Cow<'static, str>>);
impl Variables {
    fn new() -> Self {
        use std::borrow::Cow::Borrowed;
        Self(HashMap::from([
            ("1", Borrowed("4")),
            ("?", Borrowed("0")),
            (
                "UA_Browser",
                Borrowed(
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/112.0.0.0 Safari/537.36 Edg/112.0.1722.64",
                ),
            ),
            (
                "UA_Dalvik",
                Borrowed("Dalvik/2.1.0 (Linux; U; Android 9; ALP-AL00 Build/HUAWEIALP-AL00)"),
            ),
            (
                "UA_SecCHUA",
                Borrowed("\"Chromium\";v=\"112\", \"Microsoft Edge\";v=\"112\""),
            ),
            (
                "IATACode",
                Borrowed(include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../resources/ip-check/IATACode.txt"
                ))),
            ),
        ]))
    }
    fn get(&self, key: &str) -> String {
        self.0
            .get(key)
            .map(|value| value.clone().into_owned())
            .unwrap_or_default()
    }
    fn set(&mut self, key: &'static str, value: String) {
        self.0.insert(key, std::borrow::Cow::Owned(value));
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
    let n = text.chars().count() as i64;
    let start = if start < 0 {
        n.saturating_add(start)
    } else {
        start
    }
    .clamp(0, n);
    let end = match length {
        None => n,
        Some(len) if len < 0 => n.saturating_add(len),
        Some(len) => start.saturating_add(len),
    }
    .clamp(start, n);
    let byte_start = text
        .char_indices()
        .nth(start as usize)
        .map_or(text.len(), |(i, _)| i);
    let byte_end = text
        .char_indices()
        .nth(end as usize)
        .map_or(text.len(), |(i, _)| i);
    text[byte_start..byte_end].to_owned()
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
    let root: serde_json::Value =
        serde_json::from_str(text.trim()).context("检测响应不是有效 JSON")?;
    let mut value = &root;
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
            .unwrap_or(&serde_json::Value::Null);
        }
    }
    Ok(serde_json::to_string_pretty(value)?)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn static_detector_data_is_borrowed_and_valid_utf8_reuses_the_body() {
        let mut values = Variables::new();
        assert!(matches!(
            values.0["IATACode"],
            std::borrow::Cow::Borrowed(_)
        ));
        values.set("region", "US".into());
        assert_eq!(values.get("region"), "US");
        let bytes = "response 内容".as_bytes().to_vec();
        let pointer = bytes.as_ptr();
        let text = decode_body(bytes);
        assert_eq!(text.as_ptr(), pointer);
        assert_eq!(text, "response 内容");
        assert_eq!(decode_body(vec![b'a', 0xff]), "a�");
    }
    #[test]
    fn shared_transports_are_lazy_and_do_not_keep_completed_detectors_alive() {
        let a = Context::new(10001).unwrap();
        let b = Context::new(10001).unwrap();
        assert!(std::sync::Arc::ptr_eq(&a.clients, &b.clients));
        assert!(a.clients.tls13.lock().unwrap().is_none());
        let weak = std::sync::Arc::downgrade(&a.clients);
        drop(a);
        drop(b);
        assert!(weak.upgrade().is_none());
    }
    #[tokio::test]
    async fn discarded_bodies_still_enforce_the_response_limit() {
        let (mut ctx, server) = mock(&"x".repeat(2 * 1024 * 1024 + 1), "").await;
        let mut request = Request::get("https://example.test/");
        request.discard_body = true;
        assert!(
            ctx.perform(&request)
                .await
                .unwrap_err()
                .to_string()
                .contains("2 MiB")
        );
        server.abort();
        let (mut ctx, server) = mock("discard this", "").await;
        assert_eq!(ctx.perform(&request).await.unwrap(), "");
        server.abort();
    }

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
        (loopback_context(address), task)
    }

    fn loopback_context(address: std::net::SocketAddr) -> Context {
        let mut ctx = Context::new(1).unwrap();
        ctx.set_test_client(
            crate::network::client_builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        );
        ctx.test_origin = Some(Url::parse(&format!("http://{address}")).unwrap());
        ctx
    }

    #[tokio::test]
    async fn every_catalog_detector_executes_with_native_mock_http() {
        // Broad fixture exercises the native request/parse chains without
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
            if let Err(error) = platforms::execute(&service.id, &mut ctx).await {
                failures.push(format!("{}: {error:#}", service.id));
            }
            task.abort();
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[tokio::test]
    async fn platforms_requiring_credentials_report_unknown_without_sending_requests() {
        let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        proxy.set_nonblocking(true).unwrap();
        let port = proxy.local_addr().unwrap().port();
        for id in [
            "MediaUnlockTest_DisneyPlus",
            "MediaUnlockTest_ESPNPlus",
            "MediaUnlockTest_NeonTV",
            "MediaUnlockTest_HBOMax",
            "MediaUnlockTest_NLZIET",
            "MediaUnlockTest_EurosportRO",
            "MediaUnlockTest_OptusSports",
            "MediaUnlockTest_HuluUS",
            "MediaUnlockTest_Catchplay",
            "MediaUnlockTest_TrueID",
            "MediaUnlockTest_EPIX",
            "MediaUnlockTest_Joyn",
            "MediaUnlockTest_SetantaSports",
            "MediaUnlockTest_TVer",
            "MediaUnlockTest_Tving",
            "MediaUnlockTest_Wavve",
            "MediaUnlockTest_DMM",
            "MediaUnlockTest_encoreTVB",
            "MediaUnlockTest_SKY_DE",
            "MediaUnlockTest_MaoriTV",
            "MediaUnlockTest_SpotvNow",
            "MediaUnlockTest_NetflixCDN",
            "MediaUnlockTest_7plus",
            "MediaUnlockTest_NaverTV",
            "MediaUnlockTest_TV360",
            "MediaUnlockTest_KPlus",
            "MediaUnlockTest_ofiii",
            "MediaUnlockTest_NowE",
            "MediaUnlockTest_ViuTV",
            "MediaUnlockTest_4GTV",
            "MediaUnlockTest_Channel5",
        ] {
            let service = crate::ip_check::services()
                .iter()
                .find(|service| service.id == id)
                .unwrap();
            let result = tokio::time::timeout(Duration::from_secs(1), check(service, port))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(result.state, State::Unknown, "{id}");
            assert_eq!(result.summary, "未确认可用性", "{id}");
            assert_eq!(result.detail, "未执行认证检测", "{id}");
            assert!(result.authentication_check_skipped, "{id}");
            assert_eq!(result.response_millis(), None, "{id}");
        }
        assert_eq!(
            proxy.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    async fn capture_platform_request(
        service: &Service,
        status: &str,
        response: &str,
    ) -> (CheckResult, String, Vec<u8>) {
        capture_platform_request_bytes(service, status, response.as_bytes()).await
    }

    async fn capture_platform_request_bytes(
        service: &Service,
        status: &str,
        response: &[u8],
    ) -> (CheckResult, String, Vec<u8>) {
        let mut reply = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        )
        .into_bytes();
        reply.extend_from_slice(response);
        capture_platform_reply(service, reply).await
    }

    async fn capture_platform_reply(
        service: &Service,
        reply: Vec<u8>,
    ) -> (CheckResult, String, Vec<u8>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut ctx = loopback_context(listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let captured = read_redirect_fixture_request(&mut socket).await;
            socket.write_all(&reply).await.unwrap();
            captured
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            let text = platforms::execute(&service.id, &mut ctx)
                .await
                .unwrap()
                .expect("anonymous platform probe skipped");
            let (headers, body) = server.await.unwrap();
            (platform_result(service, &ctx, &text), headers, body)
        })
        .await
        .unwrap()
    }

    fn fixture_transport_stream() -> Vec<u8> {
        let mut pat = [0xff; 188];
        let mut pmt = [0xff; 188];
        let mut audio = [0xff; 188];
        let pat_payload = [
            0x47, 0x40, 0x00, 0x10, 0x00, 0x00, 0xb0, 0x0d, 0x00, 0x01, 0xc1, 0x00, 0x00, 0x00,
            0x01, 0xe1, 0x00, 0xe8, 0xf9, 0x5e, 0x7d,
        ];
        let pmt_payload = [
            0x47, 0x41, 0x00, 0x10, 0x00, 0x02, 0xb0, 0x12, 0x00, 0x01, 0xc1, 0x00, 0x00, 0xe1,
            0x01, 0xf0, 0x00, 0x03, 0xe1, 0x01, 0xf0, 0x00, 0x8d, 0xff, 0x34, 0x11,
        ];
        let pes_header = [
            0x00, 0x00, 0x01, 0xc0, 0x00, 0x28, 0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x46, 0x51,
        ];
        let mut silent_audio_frame = [0; 32];
        silent_audio_frame[..4].copy_from_slice(&[0xff, 0xff, 0x10, 0xc0]);
        let frame_offset = audio.len() - silent_audio_frame.len();
        let pes_offset = frame_offset - pes_header.len();
        pat[..pat_payload.len()].copy_from_slice(&pat_payload);
        pmt[..pmt_payload.len()].copy_from_slice(&pmt_payload);
        audio[..4].copy_from_slice(&[0x47, 0x41, 0x01, 0x30]);
        audio[4] = (pes_offset - 5) as u8;
        audio[5..12].copy_from_slice(&[0x50, 0x00, 0x00, 0x00, 0x00, 0x7e, 0x00]);
        audio[pes_offset..frame_offset].copy_from_slice(&pes_header);
        audio[frame_offset..].copy_from_slice(&silent_audio_frame);
        [pat, pmt, audio].concat()
    }

    fn fox_service() -> &'static Service {
        crate::ip_check::services()
            .iter()
            .find(|service| service.id == "MediaUnlockTest_Fox")
            .unwrap()
    }

    #[tokio::test]
    async fn fox_detects_binary_transport_stream_without_mistaking_other_bodies() {
        let service = fox_service();
        let stream = fixture_transport_stream();
        let (result, _, _) = capture_platform_request_bytes(service, "200 OK", &stream).await;
        assert_eq!(result.state, State::Confirmed);

        let (result, _, _) =
            capture_platform_request_bytes(service, "403 Forbidden", &stream).await;
        assert_eq!(result.state, State::Failed);

        for (offset, invalid) in [
            (188, 0x00),
            (189, 0xc1),
            (191, 0x00),
            (2 * 188 + 3, 0x20),
            (2 * 188 + 4, 183),
            (2 * 188 + 4, 255),
        ] {
            let mut corrupt = stream.clone();
            corrupt[offset] = invalid;
            let (result, _, _) = capture_platform_request_bytes(service, "200 OK", &corrupt).await;
            assert_eq!(
                result.state,
                State::Unknown,
                "byte {offset} value {invalid}"
            );
        }

        let (result, _, _) =
            capture_platform_request_bytes(service, "200 OK", &stream[..188]).await;
        assert_eq!(result.state, State::Unknown);
    }

    #[tokio::test]
    async fn fox_rejects_terminal_redirects_and_short_http_bodies() {
        let service = fox_service();
        let stream = fixture_transport_stream();
        let (result, _, _) = capture_platform_request_bytes(service, "302 Found", &stream).await;
        assert_eq!(result.state, State::Unknown);

        let mut reply =
            b"HTTP/1.1 200 OK\r\nContent-Length: 564\r\nConnection: close\r\n\r\n".to_vec();
        reply.extend_from_slice(&stream[..563]);
        let (result, _, _) = capture_platform_reply(service, reply).await;
        assert_eq!(result.state, State::Failed);
    }

    #[tokio::test]
    async fn fox_prefix_reports_terminal_status_and_body_end() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut ctx = loopback_context(listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for (path, reply) in [
                (
                    "/redirect",
                    "HTTP/1.1 302 Found\r\nLocation: https://cdn.example.test/exact\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                ),
                (
                    "/exact",
                    "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc",
                ),
                (
                    "/longer",
                    "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd",
                ),
                (
                    "/chunked",
                    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
                ),
                (
                    "/short",
                    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\nab\r\n0\r\n\r\n",
                ),
                (
                    "/terminal",
                    "HTTP/1.1 302 Found\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc",
                ),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (headers, _) = read_redirect_fixture_request(&mut socket).await;
                assert!(headers.starts_with(&format!("get {path} ")));
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        let prefix = PrefixRead {
            status: StatusCode::OK,
            limit_for_first_byte: |_| 3,
        };
        for (path, status, body, end) in [
            ("redirect", StatusCode::OK, "abc", BodyEnd::Complete),
            ("longer", StatusCode::OK, "abc", BodyEnd::Truncated),
            ("chunked", StatusCode::OK, "abc", BodyEnd::LimitReached),
            ("short", StatusCode::OK, "ab", BodyEnd::Complete),
            ("terminal", StatusCode::FOUND, "", BodyEnd::Skipped),
        ] {
            let mut request = Request::get(&format!("https://origin.example.test/{path}"));
            request.follow = true;
            request.timeout = 3;
            let response = ctx.request_bytes(request, Some(prefix)).await.unwrap();
            assert_eq!(response.status, status, "{path}");
            assert_eq!(response.body, body.as_bytes(), "{path}");
            assert_eq!(response.end, end, "{path}");
            if path == "redirect" {
                assert_eq!(response.url.as_str(), "https://cdn.example.test/exact");
            }
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn fox_hls_requires_a_valid_entry_and_complete_uri() {
        let service = fox_service();
        for (body, expected) in [
            (
                format!("#EXTM3U\n#EXTINF:6,{}\nsegment.ts\n", "x".repeat(1024)),
                State::Confirmed,
            ),
            (
                "#EXTM3U\r\n#EXT-X-STREAM-INF:CODECS=\"avc1.42e01e,mp4a.40.2\",BANDWIDTH=128000\r\n../variant.m3u8\r\n".into(),
                State::Confirmed,
            ),
            (
                "#EXTM3U\n#EXTINF:6,\nhttps://media.example.test/segment?index=2\n".into(),
                State::Confirmed,
            ),
            ("#EXTM3U\n#EXTINF:abc,\nsegment.ts\n".into(), State::Unknown),
            ("#EXTM3U\n#EXT-X-STREAM-INF:CODECS=\"test\"\nvariant.m3u8\n".into(), State::Unknown),
            ("#EXTM3U\n#EXTINF:6,\nnot a URI\n".into(), State::Unknown),
            ("#EXTM3U\n#EXTINF:6,\nError\n".into(), State::Unknown),
            ("#EXTM3U\n#EXTINF:6,\nhttps://user@example.test/segment.ts\n".into(), State::Unknown),
            ("#EXTM3U\n#EXTINF:6,\nfile:///segment.ts\n".into(), State::Unknown),
            ("#EXTM3U\n#EXTINF:6,\nsegment%2.ts\n".into(), State::Unknown),
            (
                format!("#EXTM3U\n#{}\n#EXTINF:6,\nhttps://[bad\n", "x".repeat(8192 - 22)),
                State::Unknown,
            ),
            (
                format!("#EXTM3U\n#EXTINF:6,\nsegment.ts\n#{}中\n", "x".repeat(8192 - 32)),
                State::Confirmed,
            ),
        ] {
            let (result, _, _) = capture_platform_request(service, "200 OK", &body).await;
            assert_eq!(
                result.state,
                expected,
                "playlist prefix: {:?}",
                &body[..body.len().min(96)]
            );
        }
    }

    #[tokio::test]
    async fn fox_finishes_after_a_valid_prefix_without_draining_the_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut ctx = loopback_context(listener.local_addr().unwrap());
        let (release, finish) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_redirect_fixture_request(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            let stream = fixture_transport_stream();
            for part in [&stream[..19], &stream[19..271], &stream[271..]] {
                socket
                    .write_all(format!("{:x}\r\n", part.len()).as_bytes())
                    .await
                    .unwrap();
                socket.write_all(part).await.unwrap();
                socket.write_all(b"\r\n").await.unwrap();
            }
            let _ = finish.await;
        });
        let service = fox_service();
        let text = tokio::time::timeout(
            Duration::from_secs(3),
            platforms::execute(&service.id, &mut ctx),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert_eq!(
            platform_result(service, &ctx, &text).state,
            State::Confirmed
        );
        let _ = release.send(());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn active_probes_send_public_parameters_and_fresh_anonymous_ids() {
        for (id, response, expected, query_fields) in [
            (
                "MediaUnlockTest_LiTV",
                r#"{"error":{"code":42000075}}"#,
                State::Confirmed,
                &[][..],
            ),
            (
                "MediaUnlockTest_Paravi",
                r#"{"type":"Unauthorized"}"#,
                State::Reachable,
                &[][..],
            ),
            (
                "MediaUnlockTest_unext",
                r#"{"data":{"playlist":{"resultStatus":200}}}"#,
                State::Confirmed,
                &["operationname", "variables", "extensions"][..],
            ),
            (
                "MediaUnlockTest_Fox",
                "#EXTM3U\n#EXTINF:6,\nsegment.ts\n",
                State::Confirmed,
                &["cloud", "si"][..],
            ),
            (
                "MediaUnlockTest_MegogoTV",
                r#"{"status":0}"#,
                State::Confirmed,
                &[
                    "elementid",
                    "elementalias",
                    "elementtype",
                    "withupgradesubscriptionreturnamount",
                    "forcesvod",
                    "includeproductsforupsale",
                ][..],
            ),
        ] {
            let service = crate::ip_check::services()
                .iter()
                .find(|service| service.id == id)
                .unwrap();
            let before = timestamp(true).parse::<u128>().unwrap();
            let (result, headers, body) =
                capture_platform_request(service, "200 OK", response).await;
            let after = timestamp(true).parse::<u128>().unwrap();
            assert_eq!(result.state, expected, "{id}");
            assert!(!result.authentication_check_skipped, "{id}");
            assert!(
                !headers.lines().any(|line| {
                    line.starts_with("authorization:") || line.starts_with("cookie:")
                }),
                "{id}: unexpected authentication header"
            );
            let target = headers.split_whitespace().nth(1).unwrap();
            let catalog = Url::parse(&service.url).unwrap();
            let url = Url::parse(&format!("http://fixture.example{target}")).unwrap();
            assert!(
                url.path() == catalog.path().to_ascii_lowercase()
                    && url.query() == catalog.query().map(str::to_ascii_lowercase).as_deref(),
                "{id}: request target differs from the sanitized directory"
            );
            let query: HashMap<_, _> = url.query_pairs().collect();
            assert!(
                query.len() == query_fields.len()
                    && query_fields
                        .iter()
                        .all(|field| query.get(*field).is_some_and(|value| !value.is_empty())),
                "{id}: expected only the public query parameters"
            );
            if matches!(id, "MediaUnlockTest_LiTV" | "MediaUnlockTest_Paravi") {
                let (identity, public_fields, expected_length) = if id == "MediaUnlockTest_LiTV" {
                    ("puid", &["AssetId", "MediaType"][..], 36)
                } else {
                    ("vuid", &["meta_id", "device_code", "app_id"][..], 32)
                };
                assert!(headers.starts_with("post "), "{id}");
                assert!(
                    headers.contains("\r\ncontent-type: application/json\r\n"),
                    "{id}"
                );
                let payload = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
                let (_, _, next_body) = capture_platform_request(service, "200 OK", response).await;
                let next_payload = serde_json::from_slice::<serde_json::Value>(&next_body).unwrap();
                let first = payload[identity].as_str().unwrap_or_default();
                let next = next_payload[identity].as_str().unwrap_or_default();
                assert!(
                    [first, next].iter().all(|value| {
                        value.len() == expected_length
                            && uuid::Uuid::parse_str(value)
                                .is_ok_and(|value| value.get_version_num() == 4)
                    }) && first != next,
                    "{id}: expected a fresh UUID in {identity}"
                );
                assert!(
                    payload.as_object().is_some_and(|object| {
                        object.len() == public_fields.len() + 1
                            && public_fields.iter().all(|field| {
                                object.get(*field).is_some_and(|value| {
                                    !value.is_null() && *value == next_payload[*field]
                                })
                            })
                    }),
                    "{id}: public request body fields"
                );
            } else {
                assert!(headers.starts_with("get ") && body.is_empty(), "{id}");
            }
            if id == "MediaUnlockTest_MegogoTV" {
                let sent = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("x-scrapi-client-ts: "))
                    .and_then(|value| value.parse::<u128>().ok());
                assert!(
                    sent.is_some_and(|value| (before..=after).contains(&value)),
                    "{id}: expected a timestamp generated for this request"
                );
            }
        }
    }

    #[tokio::test]
    async fn changed_probes_require_service_evidence_for_region_decisions() {
        for (id, status, response, expected) in [
            ("LiTV", "200 OK", "", State::Unknown),
            (
                "LiTV",
                "403 Forbidden",
                r#"{"error":{"code":42000087}}"#,
                State::Restricted,
            ),
            (
                "Paravi",
                "403 Forbidden",
                r#"{"type":"Forbidden"}"#,
                State::Unknown,
            ),
            ("Paravi", "200 OK", "", State::Unknown),
            ("unext", "200 OK", "", State::Unknown),
            (
                "unext",
                "200 OK",
                r#"{"data":{"playlist":{"resultStatus":467}}}"#,
                State::Restricted,
            ),
            (
                "unext",
                "200 OK",
                r#"{"resultStatus":475}"#,
                State::Reachable,
            ),
            ("Fox", "200 OK", "", State::Unknown),
            ("Fox", "200 OK", "<html>Error</html>", State::Unknown),
            ("Fox", "403 Forbidden", "", State::Failed),
            ("MegogoTV", "200 OK", "", State::Unknown),
            (
                "MegogoTV",
                "502 Bad Gateway",
                r#"{"status":502}"#,
                State::Unknown,
            ),
        ] {
            let service = crate::ip_check::services()
                .iter()
                .find(|service| service.id == format!("MediaUnlockTest_{id}"))
                .unwrap();
            let (result, _, _) = capture_platform_request(service, status, response).await;
            assert_eq!(result.state, expected, "{id} ({status})");
        }
    }

    #[tokio::test]
    async fn discovery_plus_uses_fresh_anonymous_sessions_without_tracking_cookies() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut ctx = loopback_context(listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for index in 0..6 {
                let (mut socket, _) = listener.accept().await.unwrap();
                requests.push(read_redirect_fixture_request(&mut socket).await);
                let body = if index % 2 == 0 {
                    format!("{{\"{}\":\"fixture-value\"}}", ["to", "ken"].concat())
                } else if index == 1 {
                    r#"{"currentLocationTerritory":"us"}"#.to_owned()
                } else if index == 3 {
                    "{}".to_owned()
                } else {
                    r#"{"currentLocationTerritory":"gb"}"#.to_owned()
                };
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
            requests
        });
        for expected in [State::Confirmed, State::Failed, State::Restricted] {
            let text = platforms::execute("MediaUnlockTest_DiscoveryPlus", &mut ctx)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(output::parse_output(&text).state, expected);
        }
        let requests = server.await.unwrap();
        let mut device_ids = Vec::new();
        for pair in requests.as_chunks::<2>().0 {
            let token_request = &pair[0].0;
            let request_target = token_request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let url = Url::parse(&format!("https://fixture.example{request_target}")).unwrap();
            let device_id = url
                .query_pairs()
                .find_map(|(key, value)| (key == "deviceid").then_some(value.into_owned()))
                .unwrap();
            assert!(uuid::Uuid::parse_str(&device_id).is_ok());
            device_ids.push(device_id);
            assert!(
                !token_request
                    .lines()
                    .any(|line| line.starts_with("cookie:"))
            );
            let cookies: Vec<_> = pair[1]
                .0
                .lines()
                .filter(|line| line.starts_with("cookie:"))
                .collect();
            let expected_cookie = format!("cookie: {}=fixture-value", ["s", "t"].concat());
            assert!(cookies.len() == 1 && cookies[0] == expected_cookie);
        }
        assert!(device_ids[0] != device_ids[1] && device_ids[1] != device_ids[2]);
    }

    #[tokio::test]
    async fn discovery_plus_does_not_request_user_data_without_a_session() {
        for service in [
            "MediaUnlockTest_DiscoveryPlus",
            "MediaUnlockTest_DiscoveryPlusUK",
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut ctx = loopback_context(listener.local_addr().unwrap());
            let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
            let server = tokio::spawn(async move {
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    sender
                        .send(read_redirect_fixture_request(&mut socket).await)
                        .unwrap();
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                        )
                        .await
                        .unwrap();
                }
            });
            let text = platforms::execute(service, &mut ctx)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(output::parse_output(&text).state, State::Failed);
            server.abort();
            let mut requests = Vec::new();
            while let Ok(request) = receiver.try_recv() {
                requests.push(request);
            }
            assert_eq!(requests.len(), 1, "{service}");
            assert!(
                !requests[0]
                    .0
                    .lines()
                    .any(|line| line.starts_with("cookie:"))
            );
        }
    }

    #[tokio::test]
    async fn instagram_music_uses_no_captured_session_and_unexpected_results_fail() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut ctx = loopback_context(listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for index in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                requests.push(read_redirect_fixture_request(&mut socket).await);
                let body = if index == 0 {
                    r#"{"data":{"xdt_shortcode_media":{"clips_music_attribution_info":{"should_mute_audio":false}}}}"#
                } else {
                    "{}"
                };
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
            requests
        });
        for expected in [State::Confirmed, State::Failed] {
            let text = platforms::execute("MediaUnlockTest_Instagram.Music", &mut ctx)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(output::parse_output(&text).state, expected);
        }
        for (headers, body) in server.await.unwrap() {
            assert!(!headers.lines().any(|line| line.starts_with("cookie:")));
            let body = String::from_utf8(body).unwrap();
            let url = Url::parse(&format!("https://fixture.example/?{body}")).unwrap();
            let keys: Vec<_> = url.query_pairs().map(|(key, _)| key.into_owned()).collect();
            assert_eq!(
                keys,
                [
                    "fb_api_caller_class",
                    "fb_api_req_friendly_name",
                    "variables",
                    "server_timestamps",
                    "doc_id",
                ]
            );
        }
    }

    #[tokio::test]
    async fn youtube_premium_does_not_send_a_captured_browser_cookie() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut ctx = loopback_context(listener.local_addr().unwrap());
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            let mut count = 0;
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                sender
                    .send(read_redirect_fixture_request(&mut socket).await)
                    .unwrap();
                let body = if count == 0 {
                    r#"{"countryCode":"US","purchaseButtonOverride":true}"#
                } else {
                    "{}"
                };
                count += 1;
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        for expected in [State::Confirmed, State::Failed] {
            let text = platforms::execute("MediaUnlockTest_YouTube_Premium", &mut ctx)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(output::parse_output(&text).state, expected);
        }
        server.abort();
        let mut requests = Vec::new();
        while let Ok(request) = receiver.try_recv() {
            requests.push(request);
        }
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|(headers, _)| !headers.lines().any(|line| line.starts_with("cookie:")))
        );
    }

    #[tokio::test]
    async fn telasa_generates_a_fresh_device_identifier_for_each_detection() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut ctx = loopback_context(listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                requests.push(read_redirect_fixture_request(&mut socket).await);
                let body = r#"{"type":"OK"}"#;
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
            requests
        });
        for _ in 0..2 {
            let text = platforms::execute("MediaUnlockTest_Telasa", &mut ctx)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(output::parse_output(&text).state, State::Confirmed);
        }
        let requests = server.await.unwrap();
        let device_ids: Vec<_> = requests
            .iter()
            .map(|(headers, _)| {
                headers
                    .lines()
                    .find_map(|line| line.strip_prefix("x-device-id: "))
                    .unwrap()
            })
            .collect();
        assert!(
            device_ids
                .iter()
                .all(|id| uuid::Uuid::parse_str(id).is_ok())
        );
        assert!(device_ids[0] != device_ids[1]);
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
            let text = platforms::execute("MediaUnlockTest_Dazn", &mut ctx)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(output::parse_output(&text).state, expected);
            task.abort();
        }
        let (mut ctx, task) = mock("<html>verification</html>", "").await;
        assert!(
            platforms::execute("MediaUnlockTest_Dazn", &mut ctx)
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
        ctx.set_test_client(
            crate::network::client_builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        );
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

    async fn read_redirect_fixture_request(
        socket: &mut tokio::net::TcpStream,
    ) -> (String, Vec<u8>) {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut reader = BufReader::new(socket);
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
            headers.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        let headers = headers.to_ascii_lowercase();
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .map_or(0, |length| length.trim().parse().unwrap());
        let mut body = vec![0; length];
        reader.read_exact(&mut body).await.unwrap();
        (headers, body)
    }

    #[tokio::test]
    async fn method_preserving_redirects_confine_payloads_to_the_original_origin() {
        for status in [307, 308] {
            for (cross_origin, post, send_fixture_headers) in [
                (true, true, true),
                (true, false, true),
                (true, false, false),
                (false, true, true),
            ] {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let mut ctx = loopback_context(listener.local_addr().unwrap());
                let initial_url = "https://first.example.test/start";
                let location = if cross_origin {
                    "https://second.example.test/next"
                } else {
                    "/next"
                };
                let (finished, mut finish) = tokio::sync::oneshot::channel();
                let server = tokio::spawn(async move {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let first = read_redirect_fixture_request(&mut socket).await;
                    socket
                        .write_all(
                            format!(
                                "HTTP/1.1 {status} Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                    let second = tokio::select! {
                        biased;
                        second = listener.accept() => {
                            let (mut socket, _) = second.unwrap();
                            let request = read_redirect_fixture_request(&mut socket).await;
                            socket
                                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
                                .await
                                .unwrap();
                            Some(request)
                        }
                        _ = &mut finish => None,
                    };
                    (first, second)
                });

                let mut request = Request::get(initial_url);
                request.follow = true;
                request
                    .headers
                    .push(["User-Agent".into(), "redirect-fixture".into()]);
                if post {
                    request.method = "POST".into();
                    request.body = Some("fixture-body".into());
                }
                if send_fixture_headers {
                    request
                        .headers
                        .push(["X-Fixture-Auth".into(), "fixture-header".into()]);
                    request.cookie_load = true;
                    ctx.cookies.add_cookie_str(
                        "fixture_cookie=value; Domain=example.test; Path=/",
                        &Url::parse(initial_url).unwrap(),
                    );
                }
                let result = tokio::time::timeout(Duration::from_secs(3), ctx.perform(&request))
                    .await
                    .unwrap();
                let _ = finished.send(());
                let ((first_headers, first_body), second) = server.await.unwrap();
                let method = if post { "post" } else { "get" };
                assert!(first_headers.starts_with(&format!("{method} /start ")));
                if send_fixture_headers {
                    assert!(first_headers.contains("x-fixture-auth:"));
                    assert!(first_headers.contains("cookie:"));
                }
                if post {
                    assert_eq!(first_body, b"fixture-body");
                }
                if cross_origin && post {
                    assert!(result.is_err());
                    assert!(second.is_none());
                    continue;
                }
                assert_eq!(result.unwrap(), "OK");
                let (headers, body) = second.unwrap();
                assert!(headers.starts_with(&format!("{method} /next ")));
                assert!(headers.contains("user-agent: redirect-fixture"));
                assert_eq!(headers.contains("x-fixture-auth:"), !cross_origin);
                assert_eq!(headers.contains("cookie:"), !cross_origin);
                assert_eq!(body, first_body);
            }
        }
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
        ctx.set_test_client(
            crate::network::client_builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        );
        ctx.test_origin = Some(origin);
        let output = platforms::execute("MediaUnlockTest_BahamutAnime", &mut ctx)
            .await
            .unwrap()
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
    fn native_transformations_preserve_pinned_extraction() {
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
    }

    #[tokio::test]
    async fn cancellation_drops_native_http_without_descendant_processes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut ctx = Context::new(1).unwrap();
        ctx.set_test_client(crate::network::client_builder().no_proxy().build().unwrap());
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

    #[test]
    fn platform_result_distinguishes_submillisecond_response_from_no_response() {
        let service = Service {
            id: "fixture".into(),
            name: "Fixture".into(),
            group: "Fixture".into(),
            url: "https://fixture.example".into(),
        };
        let mut ctx = Context::new(8080).unwrap();
        let unmeasured = platform_result(&service, &ctx, "Fixture: Unexpected response");
        assert_eq!(unmeasured.state, State::Failed);
        assert_eq!(unmeasured.response_millis(), None);

        ctx.record_response(Duration::from_micros(500));
        ctx.record_response(Duration::from_millis(20));
        let measured = platform_result(&service, &ctx, "Fixture: Unexpected response");
        assert_eq!(measured.state, State::Unknown);
        assert_eq!(measured.response_millis(), Some(0));

        ctx.failed = true;
        let measured_failure = platform_result(&service, &ctx, "Fixture: Yes");
        assert_eq!(measured_failure.state, State::Failed);
        assert_eq!(measured_failure.response_millis(), Some(0));
    }
}
