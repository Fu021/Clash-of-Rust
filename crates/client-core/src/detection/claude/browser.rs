//! Short-lived loopback receiver; no WebView or persistent background server.
use super::*;
use serde::Deserialize;
use std::sync::{Arc, Mutex, mpsc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/claude-check/browser.html"
));
pub const SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/claude-check/browser.js"
));
const CSS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/claude-check/browser.css"
));
pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self' https://api.ipify.org https://api6.ipify.org https://www.cloudflare.com; img-src https://*.bash.ws; frame-ancestors 'none'; base-uri 'none'";

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserData {
    pub kind: String,
    pub schema: u8,
    pub origin: String,
    pub timezone: String,
    pub offset: i32,
    pub languages: Vec<String>,
    pub locale: String,
    pub date_format: String,
    pub platform: String,
    pub user_agent: String,
    pub accept_language: String,
    pub fonts: Vec<String>,
    pub emoji: Vec<String>,
    pub webrtc: WebRtc,
    pub ipv4: String,
    pub ipv6: String,
    pub dns_id: String,
    pub dns_error: String,
    pub trace: String,
    pub cf_trace: String,
    pub canvas_hash: String,
    pub webgl_vendor: String,
    pub webgl_renderer: String,
    pub cookie_enabled: Option<bool>,
    pub geolocation: Location,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebRtc {
    pub supported: bool,
    pub completed: bool,
    pub candidates: Vec<String>,
    pub error: String,
    pub servers: Vec<String>,
    pub samples: Vec<IceSample>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IceSample {
    pub address: String,
    pub server: String,
    pub protocol: String,
    #[serde(rename = "type")]
    pub kind: String,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Location {
    pub permission: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}
impl BrowserData {
    pub fn validate(&self) -> Result<()> {
        if self.kind != "browser"
            || self.schema != 1
            || !(-1440..=1440).contains(&self.offset)
            || self.languages.len() > 24
            || self.fonts.len() > 24
            || self.emoji.len() > 8
            || self.webrtc.candidates.len() > 24
            || self.webrtc.samples.len() > 24
            || self.webrtc.servers.len() > 4
        {
            bail!("浏览器报告格式无效");
        }
        for text in [
            &self.origin,
            &self.timezone,
            &self.locale,
            &self.date_format,
            &self.platform,
            &self.user_agent,
            &self.accept_language,
            &self.dns_error,
            &self.webrtc.error,
            &self.trace,
            &self.cf_trace,
            &self.canvas_hash,
            &self.webgl_vendor,
            &self.webgl_renderer,
            &self.geolocation.permission,
        ]
        .into_iter()
        .chain(self.languages.iter())
        .chain(self.fonts.iter())
        .chain(self.emoji.iter())
        .chain(self.webrtc.candidates.iter())
        .chain(self.webrtc.servers.iter())
        .chain(
            self.webrtc
                .samples
                .iter()
                .flat_map(|s| [&s.address, &s.server, &s.protocol, &s.kind]),
        ) {
            if text.len() > 2048 || text.contains('\0') {
                bail!("浏览器报告字段过长或无效");
            }
        }
        for (ip, v6) in [(&self.ipv4, false), (&self.ipv6, true)] {
            if !ip.is_empty() {
                let ip: IpAddr = ip.parse()?;
                if !public_ip(ip) || ip.is_ipv6() != v6 {
                    bail!("浏览器报告的出口 IP 无效");
                }
            }
        }
        if !self.dns_id.is_empty() && !valid_dns_id(&self.dns_id) {
            bail!("浏览器 DNS 测试编号无效");
        }
        if self
            .geolocation
            .latitude
            .is_some_and(|v| !v.is_finite() || !(-90.0..=90.0).contains(&v))
            || self
                .geolocation
                .longitude
                .is_some_and(|v| !v.is_finite() || !(-180.0..=180.0).contains(&v))
        {
            bail!("定位坐标无效");
        }
        Ok(())
    }
}

pub async fn report(data: BrowserData, client: &Client) -> Result<Report> {
    data.validate()?;
    let actual_site = reqwest::Url::parse(&data.origin)
        .ok()
        .filter(|url| url.scheme() == "https" && url.host_str().is_some_and(claude_host));
    let source = if actual_site.is_some() {
        "用户在 Claude 页面运行的浏览器采样"
    } else {
        "浏览器本地检测页；Claude 站点单独设置未确认"
    };
    let mut report = Report::new(Environment::Web, source);
    for (ip, title) in [
        (&data.ipv4, "浏览器 IPv4 出口"),
        (&data.ipv6, "浏览器 IPv6 出口"),
    ] {
        report.signals.push(if ip.is_empty() {
            Signal::unknown(
                Group::Exit,
                title,
                "浏览器直接访问 ipify；遵循浏览器代理与规则",
                "没有获得有效公网 IP，不能判断地区。",
            )
        } else {
            ip_signal(
                client,
                Group::Exit,
                title,
                ip,
                "浏览器直接访问 ipify；遵循浏览器代理与规则",
                "仅此测试域名的出口，不代替 Claude 目标出口。",
                Evidence::Measured,
            )
            .await
        });
    }
    let mut target = Signal::unknown(
        Group::Exit,
        "Claude 网页实际出口",
        source,
        "本地检测页不能跨域读取 Claude 的 trace；可在 Claude 页面执行专用采样脚本并导入结果。没有用 GUI 探测代替浏览器实际请求。",
    );
    if let Some(url) = actual_site
        && let Some(host) = url.host_str()
        && let Some((ip, code)) = parse_trace(&data.trace, host)
    {
        target = Signal::new(
            Group::Exit,
            "Claude 网页实际出口",
            format!("{ip} · {}", country_name(&code).unwrap()),
            source,
            Evidence::Measured,
            "采样脚本读取当前 Claude 页面同域 trace；来源为用户提供的页面采样结果，未验证账号资格。",
        );
        target.ip = ip;
        target.country = code;
    }
    report.signals.push(target);
    report.signals.push(preferences::preference(
        Group::Browser,
        "浏览器时区",
        &format!(
            "{} · UTC{:+.1}",
            data.timezone,
            -(data.offset as f64) / 60.0
        ),
        source,
        preferences::timezone_country(&data.timezone),
    ));
    for (index, language) in data.languages.iter().take(6).enumerate() {
        report.signals.push(preferences::preference(
            Group::Browser,
            &format!("语言偏好 {}", index + 1),
            language,
            source,
            preferences::region_tag(language).as_deref(),
        ));
    }
    report.signals.push(Signal::new(
        Group::Browser,
        "地区格式",
        format!("{} · {}", data.locale, data.date_format),
        source,
        Evidence::Measured,
        "Intl 的地区格式与日期示例；不从数字或日期格式推断物理所在地。",
    ));
    report.signals.push(if data.accept_language.is_empty() {
        Signal::unknown(Group::Browser, "Accept-Language 请求头", source, "页面脚本无法读取浏览器自动发送的请求头；本地回传入口能记录发送给检测页的值，但不能代表 Claude 站点的单独设置。")
    } else {
        Signal::new(Group::Browser, "Accept-Language 请求头", data.accept_language, "浏览器发给本机检测入口的 HTTP 请求头", Evidence::Measured, "与 navigator.languages 分别记录；Claude 站点的扩展或单独覆盖仍需确认。")
    });
    report.signals.push(Signal::new(
        Group::Browser,
        "平台与浏览器",
        format!("{} · {}", data.platform, data.user_agent),
        source,
        Evidence::Measured,
        "浏览器实际返回的字段，不使用 GUI 的请求头或操作系统代替。",
    ));
    report.signals.push(Signal::new(
        Group::Browser,
        "可识别字体",
        if data.fonts.is_empty() {
            "未识别到预设字体；不代表没有其他字体".into()
        } else {
            data.fonts.join("、")
        },
        "浏览器 Canvas 字形宽度比较",
        Evidence::Inferred,
        "字体识别可能受回退字体、隐私策略影响；中文字体全球通用，不按字体判定国家。",
    ));
    report.signals.push(Signal::new(
        Group::Browser,
        "Emoji 渲染",
        data.emoji.join("；"),
        "浏览器 Canvas 实际渲染",
        Evidence::Inferred,
        "记录国旗字形渲染特征，不将渲染能力、操作系统或字体推断为所在国家。",
    ));
    for (title, value) in [
        ("Canvas 指纹摘要", data.canvas_hash.as_str()),
        ("WebGL 厂商", data.webgl_vendor.as_str()),
        ("WebGL 渲染器", data.webgl_renderer.as_str()),
    ] {
        report.signals.push(if value.is_empty() {
            Signal::unknown(
                Group::Browser,
                title,
                source,
                "浏览器未提供该值；隐私限制或功能不可用均可能导致此结果。",
            )
        } else {
            Signal::new(
                Group::Browser,
                title,
                value,
                source,
                Evidence::Inferred,
                "本地采样的浏览器特征；不会上传原始图像，也不据此判断所在国家或账号风险。",
            )
        });
    }
    report
        .signals
        .push(if let Some(enabled) = data.cookie_enabled {
            Signal::new(
                Group::Browser,
                "浏览器 Cookie 功能",
                if enabled {
                    "允许 Cookie"
                } else {
                    "禁用 Cookie"
                },
                source,
                Evidence::Measured,
                "仅 navigator.cookieEnabled 的布尔值；不读取 Cookie 内容、账号或会话。",
            )
        } else {
            Signal::unknown(
                Group::Browser,
                "浏览器 Cookie 功能",
                source,
                "浏览器未提供状态；没有读取任何 Cookie 内容。",
            )
        });
    report.signals.push(super::enhanced::cloudflare_trace(
        &data.cf_trace,
        "www.cloudflare.com",
        "浏览器直接请求 Cloudflare trace",
    ));
    let mut public = BTreeSet::new();
    let mut hidden = 0;
    for candidate in &data.webrtc.candidates {
        if let Ok(ip) = candidate.parse::<IpAddr>() {
            if public_ip(ip) {
                public.insert(ip);
            } else {
                hidden += 1;
            }
        } else {
            hidden += 1;
        }
    }
    if public.is_empty() {
        report.signals.push(Signal::unknown(Group::WebRtc, "WebRTC 公网地址", source, format!("支持={}，收集完成={}；公网候选为空，私网或 mDNS 候选 {hidden} 项。浏览器策略、STUN 不可达或超时均可能导致此结果，不能据此确认没有泄漏。{}", data.webrtc.supported, data.webrtc.completed, data.webrtc.error)));
    } else {
        for ip in public.into_iter().take(4) {
            let mut signal = ip_signal(
                client,
                Group::WebRtc,
                "WebRTC 公网候选",
                &ip.to_string(),
                "浏览器 ICE 候选；Cloudflare 与 Google STUN",
                "公网候选可能揭示另一条网络路径；私网和 mDNS 地址没有当作公网泄漏。",
                Evidence::Measured,
            )
            .await;
            signal.advice = "核对 TUN 对 UDP/IPv6 的接管与浏览器 WebRTC 设置。".into();
            let samples = data
                .webrtc
                .samples
                .iter()
                .filter(|s| s.address == ip.to_string())
                .take(4)
                .map(|s| format!("{} · {} · {}", s.server, s.protocol, s.kind))
                .collect::<Vec<_>>();
            if !samples.is_empty() {
                signal
                    .detail
                    .push_str(&format!(" 探针明细：{}。", samples.join("；")));
            }
            report.signals.push(signal);
        }
    }
    let permission = if data.geolocation.permission.is_empty() {
        "未确认"
    } else {
        &data.geolocation.permission
    };
    report.signals.push(Signal::new(
        Group::Browser,
        "定位权限",
        permission,
        source,
        if permission == "未确认" {
            Evidence::Unknown
        } else {
            Evidence::Measured
        },
        "权限属于当前检测来源；本地页面的权限不能代表 Claude 网站。默认不请求定位。",
    ));
    if let (Some(latitude), Some(longitude)) =
        (data.geolocation.latitude, data.geolocation.longitude)
    {
        report.signals.push(Signal::new(
            Group::Browser,
            "主动检测的定位",
            format!("纬度 {latitude:.3} · 经度 {longitude:.3}"),
            source,
            Evidence::Measured,
            "用户主动允许后由浏览器返回；坐标只回传本地报告，不提交第三方反向地理编码。",
        ));
    }
    if data.dns_id.is_empty() {
        report.signals.push(Signal::unknown(
            Group::Dns,
            "浏览器递归 DNS",
            source,
            if data.dns_error.is_empty() {
                "未执行权威 DNS 实测。"
            } else {
                &data.dns_error
            },
        ));
    } else {
        report.signals.extend(
            dns_results(
                client,
                &data.dns_id,
                "浏览器触发随机域名解析；bash.ws 权威 DNS 记录",
            )
            .await
            .unwrap_or_else(|_| {
                vec![Signal::unknown(
                    Group::Dns,
                    "浏览器递归 DNS",
                    source,
                    "未获得有效 DNS 记录，不能判断是否泄漏。",
                )]
            }),
        )
    }
    report.validate()?;
    Ok(report)
}

struct Inner {
    receiver: Mutex<mpsc::Receiver<Report>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[derive(Clone)]
pub struct Session {
    pub url: String,
    inner: Arc<Inner>,
}
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiagnosticSession(loopback)")
    }
}
impl Session {
    pub async fn start(client: Client) -> Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let host = listener.local_addr()?.to_string();
        let prefix = format!("/{}/", uuid::Uuid::new_v4().simple());
        let url = format!("http://{host}{prefix}");
        let (sender, receiver) = mpsc::sync_channel(4);
        let task = tokio::spawn(async move {
            let slots = Arc::new(tokio::sync::Semaphore::new(4));
            let ids = Arc::new(Mutex::new(BTreeSet::<String>::new()));
            let mut children = tokio::task::JoinSet::new();
            let expires = tokio::time::sleep(Duration::from_secs(20 * 60));
            tokio::pin!(expires);
            loop {
                tokio::select! {
                    _ = &mut expires => break,
                    Some(_) = children.join_next(), if !children.is_empty() => {},
                    accepted = listener.accept() => {
                        let Ok((stream, peer)) = accepted else { break; };
                        if !peer.ip().is_loopback() { continue; }
                        let Ok(slot) = slots.clone().try_acquire_owned() else { continue; };
                        let (client, host, prefix, sender, ids) = (client.clone(), host.clone(), prefix.clone(), sender.clone(), ids.clone());
                        children.spawn(async move {
                            let _slot = slot;
                            let _ = tokio::time::timeout(Duration::from_secs(90), serve(stream, &host, &prefix, &client, sender, ids)).await;
                        });
                    }
                }
            }
        });
        Ok(Self {
            url,
            inner: Arc::new(Inner {
                receiver: Mutex::new(receiver),
                task,
            }),
        })
    }
    pub fn next_report(&self) -> Option<Report> {
        self.inner.receiver.lock().ok()?.try_recv().ok()
    }
    pub fn finished(&self) -> bool {
        self.inner.task.is_finished()
    }
    pub fn submit_url(&self) -> String {
        format!("{}report", self.url)
    }
}

pub fn validate_submit_url(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value)?;
    let parts: Vec<_> = url.path().split('/').collect();
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none_or(|p| p == 0)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || parts.len() != 3
        || parts[2] != "report"
        || parts[1].len() != 32
        || !parts[1].bytes().all(|c| c.is_ascii_hexdigit())
    {
        bail!("回传地址必须是本应用生成的本机检测地址");
    }
    Ok(())
}

struct Request {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}
use std::collections::BTreeMap;
async fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break index + 4;
        }
        if bytes.len() >= 8192 {
            bail!("HTTP 请求头过大");
        }
        let mut chunk = [0; 2048];
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            bail!("请求不完整");
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    if header_end > 8192 {
        bail!("HTTP 请求头过大");
    }
    let header = std::str::from_utf8(&bytes[..header_end])?;
    let mut lines = header.split("\r\n");
    let first: Vec<_> = lines.next().context("缺少请求行")?.split(' ').collect();
    if first.len() != 3 || first[2] != "HTTP/1.1" {
        bail!("请求格式无效");
    }
    let (method, path) = (first[0].into(), first[1].into());
    let mut headers = BTreeMap::new();
    for line in lines.filter(|s| !s.is_empty()) {
        let (key, value) = line.split_once(':').context("请求头无效")?;
        if headers
            .insert(key.to_ascii_lowercase(), value.trim().to_string())
            .is_some()
        {
            bail!("重复请求头");
        }
    }
    if headers.contains_key("transfer-encoding") {
        bail!("不支持分块请求");
    }
    let length = headers
        .get("content-length")
        .map_or(Ok(0), |s| s.parse::<usize>())?;
    if length > REPORT_LIMIT {
        bail!("请求报告过大");
    }
    let mut body = bytes.split_off(header_end);
    if body.len() > length {
        bail!("请求长度不符");
    }
    while body.len() < length {
        let mut chunk = [0; 4096];
        let count = stream
            .read(&mut chunk[..(length - body.len()).min(4096)])
            .await?;
        if count == 0 {
            bail!("请求报告不完整");
        }
        body.extend_from_slice(&chunk[..count]);
    }
    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

async fn send(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> Result<()> {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: {CSP}\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn serve(
    mut stream: TcpStream,
    host: &str,
    prefix: &str,
    client: &Client,
    sender: mpsc::SyncSender<Report>,
    ids: Arc<Mutex<BTreeSet<String>>>,
) -> Result<()> {
    let request =
        match tokio::time::timeout(Duration::from_secs(6), read_request(&mut stream)).await {
            Ok(Ok(request)) => request,
            _ => {
                return send(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain; charset=utf-8",
                    b"Invalid diagnostic request",
                )
                .await;
            }
        };
    let origin = format!("http://{host}");
    if request.headers.get("host").map(String::as_str) != Some(host)
        || request
            .headers
            .get("origin")
            .is_some_and(|value| value != &origin)
        || !request.path.starts_with(prefix)
    {
        return send(&mut stream, "403 Forbidden", "text/plain", b"Forbidden").await;
    }
    let path = &request.path[prefix.len()..];
    let result: Result<(String, Vec<u8>)> = async {
        match (request.method.as_str(), path) {
            ("GET", "") => Ok(("text/html; charset=utf-8".into(), HTML.as_bytes().to_vec())),
            ("GET", "browser.js") => Ok((
                "text/javascript; charset=utf-8".into(),
                SCRIPT.as_bytes().to_vec(),
            )),
            ("GET", "browser.css") => {
                Ok(("text/css; charset=utf-8".into(), CSS.as_bytes().to_vec()))
            }
            ("POST", "dns/start")
                if request
                    .headers
                    .get("x-clash-diagnostic")
                    .is_some_and(|s| s == "1") =>
            {
                if ids.lock().unwrap().len() >= 4 {
                    bail!("DNS 测试次数已达上限，请重新开启检测入口");
                }
                let id = dns_id(client).await?;
                ids.lock().unwrap().insert(id.clone());
                Ok((
                    "application/json".into(),
                    serde_json::to_vec(&serde_json::json!({"id":id}))?,
                ))
            }
            ("POST", "report")
                if request
                    .headers
                    .get("x-clash-diagnostic")
                    .is_some_and(|s| s == "1") =>
            {
                let json: serde_json::Value = serde_json::from_slice(&request.body)?;
                let report = if json.get("kind").and_then(|v| v.as_str()) == Some("browser") {
                    let mut data: BrowserData = serde_json::from_value(json)?;
                    data.accept_language = request
                        .headers
                        .get("accept-language")
                        .cloned()
                        .unwrap_or_default();
                    if !data.dns_id.is_empty() && !ids.lock().unwrap().remove(&data.dns_id) {
                        bail!("DNS 编号不属于当前检测会话");
                    }
                    report(data, client).await?
                } else {
                    let report: Report = serde_json::from_value(json)?;
                    if report.environment != Environment::Code {
                        bail!("回传报告来源无效");
                    }
                    report.validate()?;
                    report
                };
                sender
                    .try_send(report)
                    .context("报告队列已满，请稍后重试")?;
                Ok(("application/json".into(), b"{\"ok\":true}".to_vec()))
            }
            _ => bail!("不支持的检测请求"),
        }
    }
    .await;
    match result {
        Ok((kind, body)) => send(&mut stream, "200 OK", &kind, &body).await,
        // Remote errors may contain URLs/credentials; send a fixed message.
        Err(_) => {
            send(
                &mut stream,
                "400 Bad Request",
                "application/json",
                b"{\"error\":\"Diagnostic request failed or report was invalid\"}",
            )
            .await
        }
    }
}

pub fn site_script() -> String {
    format!(
        "{SCRIPT}\ncollectClaudeRegion({{site:true}}).then(data=>{{const value=JSON.stringify(data);console.log(value);if(typeof copy==='function')copy(value);}});"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_cannot_send_reports_to_external_or_redirecting_locations() {
        let token = "a".repeat(32);
        assert!(validate_submit_url(&format!("http://127.0.0.1:7891/{token}/report")).is_ok());
        for url in [
            format!("https://example.com/{token}/report"),
            format!("http://localhost:7891/{token}/report"),
            format!("http://127.0.0.1:7891/{token}/report?redirect=external"),
            "http://127.0.0.1:7891/short/report".into(),
        ] {
            assert!(validate_submit_url(&url).is_err());
        }
    }
    #[tokio::test]
    async fn receiver_requires_host_token_header_and_same_origin_and_stops_on_drop() {
        let session = Session::start(client(None).unwrap()).await.unwrap();
        let client = client(None).unwrap();
        let reply = client.get(&session.url).send().await.unwrap();
        assert_eq!(reply.status(), 200);
        assert!(reply.headers().get("access-control-allow-origin").is_none());
        let report = Report::new(Environment::Code, "fixture");
        assert_eq!(
            client
                .post(session.submit_url())
                .json(&report)
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        assert_eq!(
            client
                .post(session.submit_url())
                .header("X-Clash-Diagnostic", "1")
                .header("Origin", "https://evil.test")
                .json(&report)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        assert_eq!(
            client
                .post(session.submit_url())
                .header("X-Clash-Diagnostic", "1")
                .json(&report)
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            session.next_report().unwrap().environment,
            Environment::Code
        );
        let url = session.url.clone();
        drop(session);
        tokio::task::yield_now().await;
        assert!(client.get(url).send().await.is_err());
    }
    #[test]
    fn imported_browser_report_is_bounded_and_rejects_private_exit_and_invalid_coordinates() {
        let mut data = BrowserData {
            kind: "browser".into(),
            schema: 1,
            ..Default::default()
        };
        assert!(data.validate().is_ok());
        data.ipv6 = "::1".into();
        assert!(data.validate().is_err());
        data.ipv6.clear();
        data.geolocation.latitude = Some(91.0);
        assert!(data.validate().is_err());
        data.geolocation.latitude = None;
        data.languages = vec!["en".into(); 25];
        assert!(data.validate().is_err());
    }
}
