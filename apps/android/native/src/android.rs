use anyhow::{Context, Result, bail, ensure};
use clash_mobile_runtime::{NativeHost, Session, core};
use core::{api, config, proxy_order, rule_manager};
use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JClass, JObject, JString, JValue},
};
use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{
    collections::VecDeque,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::mpsc;

slint::include_modules!();
static EVENTS: OnceLock<mpsc::Sender<String>> = OnceLock::new();

struct AndroidHost {
    vm: JavaVM,
    activity: GlobalRef,
}
impl AndroidHost {
    fn new(app: &slint::android::AndroidApp) -> Result<Self> {
        // android-activity owns the VM and Activity for this process.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
        let activity = {
            let env = vm.attach_current_thread()?;
            let object = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
            env.new_global_ref(object)?
        };
        Ok(Self { vm, activity })
    }
    fn root(&self) -> Result<PathBuf> {
        let mut env = self.vm.attach_current_thread()?;
        let directory = env
            .call_method(
                self.activity.as_obj(),
                "getFilesDir",
                "()Ljava/io/File;",
                &[],
            )?
            .l()?;
        let path = env
            .call_method(directory, "getAbsolutePath", "()Ljava/lang/String;", &[])?
            .l()?;
        let text: String = env.get_string(&JString::from(path))?.into();
        Ok(PathBuf::from(text).join("client"))
    }
}
impl NativeHost for AndroidHost {
    fn request(&self, value: Value) -> Result<Value> {
        let mut env = self.vm.attach_current_thread()?;
        let result: Result<Value> = env.with_local_frame(16, |env| {
            let input = env.new_string(value.to_string())?;
            let response = env
                .call_method(
                    self.activity.as_obj(),
                    "request",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    &[JValue::Object(input.as_ref())],
                )?
                .l()?;
            let response: String = env.get_string(&JString::from(response))?.into();
            Ok(serde_json::from_str(&response)?)
        });
        if env.exception_check()? {
            env.exception_clear()?;
            bail!("Android 原生操作失败");
        }
        let result = result?;
        if let Some(message) = result.get("error").and_then(Value::as_str) {
            bail!("{message}");
        }
        Ok(result)
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_clashofrust_android_MainActivity_nativeEvent(
    mut env: JNIEnv<'_>,
    _: JClass<'_>,
    text: JString<'_>,
) {
    if let Ok(value) = env.get_string(&text)
        && let Some(sender) = EVENTS.get()
    {
        let _ = sender.try_send(value.into());
    }
}

struct Request {
    intent: MobileIntent,
    a: String,
    b: String,
    selected: String,
}
struct State {
    session: Session,
    page: i32,
    query: String,
    selected: String,
    notice: String,
    logs: VecDeque<(String, String)>,
    log_level: String,
    connection_sort: String,
    ip_results: Vec<core::ip_report::ResultSlot>,
    claude: Option<core::claude_check::Report>,
}

#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("ClashOfRust"),
    );
    if let Err(error) = run(app) {
        log::error!("Android startup: {error:#}");
    }
}
fn run(app: slint::android::AndroidApp) -> Result<()> {
    let native = Arc::new(AndroidHost::new(&app)?);
    let root = native.root()?;
    slint::android::init(app)?;
    let window = AndroidWindow::new()?;
    window
        .global::<MobileData>()
        .set_version(core::VERSION.into());
    let (commands, mut receive) = mpsc::channel::<Request>(32);
    let (events, mut native_events) = mpsc::channel::<String>(16);
    let _ = EVENTS.set(events);
    let weak = window.as_weak();
    window
        .global::<MobileActions>()
        .on_request(move |intent, a, b| {
            let Some(ui) = weak.upgrade() else { return };
            if commands
                .try_send(Request {
                    intent,
                    a: a.into(),
                    b: b.into(),
                    selected: ui.get_selected_id().into(),
                })
                .is_err()
            {
                ui.invoke_notify("操作过于频繁，请稍后再试".into());
            }
        });
    let host = native.clone();
    window
        .global::<AndroidActions>()
        .on_request(move |intent, value| {
            let op = match intent {
                AndroidIntent::VpnSettings => "vpn-settings",
                AndroidIntent::ApplicationRouting => "application-routing",
                AndroidIntent::NotificationSpeeds => "notification-speeds",
                AndroidIntent::BatterySettings => "battery-settings",
                AndroidIntent::CheckUpdate => "open-url",
                AndroidIntent::SavePorts => return, // Routed through MobileActions below.
                _ => return,
            };
            let request = if op == "open-url" {
                json!({"op":op,"url":"https://github.com/Fu021/Clash-of-Rust/releases"})
            } else {
                json!({"op":op,"value":value.as_str() == "true"})
            };
            if let Err(error) = host.request(request) {
                log::error!("Android request: {error:#}");
            }
        });
    let current_page = Arc::new(std::sync::atomic::AtomicI32::new(0));
    let page = current_page.clone();
    let observed = window.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(250),
        move || {
            if let Some(ui) = observed.upgrade() {
                page.store(ui.get_current_page(), std::sync::atomic::Ordering::Relaxed);
            }
        },
    );
    let weak = window.as_weak();
    std::thread::Builder::new().name("mobile-runtime".into()).spawn(move || {
        let result = (|| -> Result<()> {
            let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
            runtime.block_on(async move {
                let mut state = State { session: Session::open(root, native)?, page: 0, query: String::new(),
                    selected: String::new(), notice: String::new(), logs: VecDeque::new(), log_level: "全部等级".into(),
                    connection_sort: "下载速度 ↓".into(), ip_results: vec![None; core::ip_check::services().len()], claude: None };
                if let Err(error) = state.session.initialize() { state.notice = format!("内核启动失败：{error:#}"); }
                let mut poll = tokio::time::interval(Duration::from_secs(1));
                loop {
                    state.page = current_page.load(std::sync::atomic::Ordering::Relaxed);
                    tokio::select! {
                        value = receive.recv() => {
                            let Some(request) = value else { break; };
                            state.selected = request.selected.clone();
                            if let Err(error) = handle(&mut state, request).await { state.notice = format!("{error:#}"); }
                        }
                        value = native_events.recv() => {
                            if let Some(value) = value {
                                if let Err(error) = native_event(&mut state, &value).await { state.notice = format!("{error:#}"); }
                            }
                        }
                        _ = poll.tick() => {
                            if let Err(error) = state.session.refresh(state.page).await {
                                if state.logs.back().is_none_or(|(_, old)| old != &error.to_string()) {
                                    state.logs.push_back(("WARN".into(), error.to_string()));
                                }
                            }
                        }
                    }
                    while state.logs.len() > 300 { state.logs.pop_front(); }
                    let view = project(&state);
                    let notice = std::mem::take(&mut state.notice);
                    let weak = weak.clone();
                    slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak.upgrade() { apply(&ui, view); if !notice.is_empty() { ui.invoke_notify(notice.into()); } }
                    })?;
                }
                Ok(())
            })
        })();
        if let Err(error) = result { log::error!("Mobile runtime stopped: {error:#}"); }
    })?;
    window.run()?;
    Ok(())
}

async fn native_event(state: &mut State, raw: &str) -> Result<()> {
    let value: Value = serde_json::from_str(raw)?;
    match value["event"].as_str().unwrap_or("") {
        "vpn" => {
            state.session.vpn = value["value"].as_bool().unwrap_or(false);
        }
        "error" | "notice" => {
            state.notice = value["value"].as_str().unwrap_or("").into();
        }
        "file" => {
            state
                .session
                .save_profile(
                    None,
                    value["value"]["name"].as_str().unwrap_or("导入配置"),
                    "",
                    Some(value["value"]["content"].as_str().context("导入内容为空")?),
                )
                .await?;
            state.notice = "配置已导入，请选择启用".into();
        }
        _ => {}
    }
    Ok(())
}

async fn handle(state: &mut State, request: Request) -> Result<()> {
    let Request {
        intent,
        a,
        b,
        selected,
    } = request;
    let session = &mut state.session;
    match intent {
        MobileIntent::Connect | MobileIntent::Disconnect => {
            session.native.request(
                json!({"op":if intent == MobileIntent::Connect {"connect"} else {"disconnect"}}),
            )?;
        }
        MobileIntent::RestartCore => session.initialize()?,
        MobileIntent::Mode => {
            let mut next = session.settings.clone();
            next.run_mode = ["rule", "global", "direct"]
                .get(a.parse::<usize>()?)
                .context("运行模式无效")?
                .to_string();
            session.commit_settings(next)?;
        }
        MobileIntent::SelectNode => {
            session.api.select(&a, &b).await?;
        }
        MobileIntent::NodeSort => {
            let mut next = session.settings.clone();
            next.node_sort = *proxy_order::NodeSort::ALL
                .get(a.parse::<usize>()?)
                .context("排序无效")?;
            session.commit_settings(next)?;
        }
        MobileIntent::NodeDelay => {
            if a == "all" {
                session.api.delay_all_nodes().await?;
            } else {
                session.api.delay(&a).await?;
            }
            state.notice = "测速完成".into();
        }
        MobileIntent::Query => state.query = b,
        MobileIntent::PickFile => {
            session.native.request(json!({"op":"pick-file"}))?;
        }
        MobileIntent::SaveProfile => {
            session.save_profile(None, &a, &b, None).await?;
            state.notice = "订阅已保存".into();
        }
        MobileIntent::ActivateProfile => {
            session.activate(&a)?;
        }
        MobileIntent::DeleteProfile => {
            session.delete(&a)?;
            state.notice = "订阅已删除".into();
        }
        MobileIntent::UpdateProfile => {
            bail!("当前订阅更新接入中；可先添加新地址并切换");
        }
        MobileIntent::CloseConnection => session.api.close_connection(&a).await?,
        MobileIntent::CloseAll => session.api.close_connections().await?,
        MobileIntent::ConnectionSort => state.connection_sort = a,
        MobileIntent::ConnectionDetail => state.selected = a,
        MobileIntent::Copy => {
            session.native.request(json!({"op":"copy","text":a}))?;
            state.notice = "已复制".into();
        }
        MobileIntent::LogLevel => state.log_level = a,
        MobileIntent::ClearLogs => state.logs.clear(),
        MobileIntent::ExportLogs => {
            session.native.request(json!({"op":"export","name":"clash-of-rust-logs.txt",
                "content":state.logs.iter().map(|(level,line)|format!("[{level}] {line}")).collect::<Vec<_>>().join("\n")}))?;
        }
        MobileIntent::RuleSave => {
            session.save_rule(&selected, &a, b.parse()?)?;
            state.notice = "已保存到草稿，请应用修改".into();
        }
        MobileIntent::RuleShortcut => {
            let index: usize = a.parse()?;
            *session
                .draft
                .enabled
                .get_mut(index)
                .context("快捷规则不存在")? = b == "true";
        }
        MobileIntent::RuleDelete => {
            session.draft.custom.retain(|r| r.id != a);
            session.draft.order.retain(|id| id != &a);
        }
        MobileIntent::RuleMove => {
            let (rows, _) = session.document()?.rows(&session.draft);
            let mut ids: Vec<_> = rows.into_iter().map(|r| r.id).collect();
            let index = ids
                .iter()
                .position(|id| id == &selected)
                .context("请先选择规则")?;
            let next = (index as i32 + a.parse::<i32>()?)
                .clamp(0, ids.len().saturating_sub(1) as i32) as usize;
            ids.swap(index, next);
            session.draft.order = ids;
        }
        MobileIntent::RuleApply => {
            session.apply_rules()?;
            state.notice = "规则已应用".into();
        }
        MobileIntent::RuleDiscard => {
            session.draft = rule_manager::ProfileRules::for_profile(
                &session.settings,
                &session.profile().id,
                false,
            )
            .as_ref()
            .clone();
        }
        MobileIntent::RuleSelect if b == "profile" => {
            let id = session
                .profiles
                .iter()
                .find(|p| p.name == a)
                .context("订阅不存在")?
                .id
                .clone();
            session.activate(&id)?;
        }
        MobileIntent::DnsOverride | MobileIntent::Ipv6 | MobileIntent::Udp => {
            ensure!(
                !session.vpn || intent != MobileIntent::Ipv6,
                "请先断开 VPN，再修改 IPv6"
            );
            let mut next = session.settings.clone();
            match intent {
                MobileIntent::DnsOverride => next.dns_override = a == "true",
                MobileIntent::Ipv6 => next.ipv6 = a == "true",
                _ => next.udp = a == "true",
            }
            session.commit_settings(next)?;
        }
        MobileIntent::Appearance | MobileIntent::Accent | MobileIntent::TextScale => {
            let mut next = session.settings.clone();
            match intent {
                MobileIntent::Appearance => {
                    next.theme_mode = Some(match a.as_str() {
                        "0" => config::ThemeMode::Dark,
                        "1" => config::ThemeMode::Light,
                        _ => config::ThemeMode::System,
                    })
                }
                MobileIntent::Accent => {
                    next.accent_color = *config::AccentColor::ALL
                        .get(a.parse::<usize>()?)
                        .context("强调色无效")?
                }
                _ => next.text_scale = a.trim_end_matches('%').parse()?,
            }
            // Appearance changes do not reload the network configuration.
            session.store.save_settings(&next)?;
            session.settings = next;
        }
        MobileIntent::Interval => {
            let mut next = session.settings.clone();
            next.delay_interval_minutes = a.parse()?;
            session.store.save_settings(&next)?;
            session.settings = next;
        }
        MobileIntent::UpdateGeo => {
            core::geodata::download_geo(
                &session.store.runtime(),
                session.running.then_some(session.settings.mixed_port),
            )
            .await?;
            state.notice = "Geo 数据已更新".into();
        }
        MobileIntent::Probe => {
            let result = core::probe::website(a, session.settings.mixed_port, false).await?;
            state.notice = format!("{} · {} ms", result.detail, result.millis);
        }
        MobileIntent::Dns => {
            let ips = session.api.resolve_domain(&a).await?;
            state.notice = ips
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" · ");
        }
        MobileIntent::SystemNotification => {
            session.native.request(json!({"op":"notify"}))?;
        }
        MobileIntent::SiteStart | MobileIntent::SiteRetest => {
            for (index, service) in core::ip_check::services().iter().enumerate() {
                if intent == MobileIntent::SiteRetest && service.group != a {
                    continue;
                }
                state.ip_results[index] = Some(
                    core::ip_check::check(&service.id, session.settings.mixed_port)
                        .await
                        .map_err(|e| e.to_string()),
                );
            }
        }
        MobileIntent::ClaudeStart => {
            let client = core::claude_check::client(Some(session.settings.mixed_port))?;
            let mut report = core::claude_check::Report::new(
                core::claude_check::Environment::Core,
                "Android 网络环境",
            );
            core::claude_check::network(&client, &mut report, "内核代理").await;
            report
                .signals
                .extend(core::claude_check::system_dns(&client, "Android 系统").await);
            state.claude = Some(report);
        }
        MobileIntent::ExportReport => {
            session
                .native
                .request(json!({"op":"export","name":"claude-report.json",
                "content":serde_json::to_string_pretty(&state.claude)?}))?;
        }
        MobileIntent::ClaudeProtection => {
            bail!("Android 保护链路尚未通过真实设备验证，暂未开放此开关");
        }
        _ => {}
    }
    Ok(())
}

fn rows<T: Clone + 'static>(values: Vec<T>) -> ModelRc<T> {
    Rc::new(VecModel::from(values)).into()
}
fn bytes(value: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB"];
    let mut size = value as f64;
    let mut index = 0;
    while size >= 1024. && index < units.len() - 1 {
        size /= 1024.;
        index += 1;
    }
    if index == 0 {
        format!("{value} B")
    } else {
        format!("{size:.1} {}", units[index])
    }
}
struct Projection {
    settings: config::Settings,
    running: bool,
    vpn: bool,
    profile: String,
    node: String,
    total: usize,
    upload: String,
    download: String,
    groups: Vec<(String, api::Proxy, Vec<(String, api::Proxy)>)>,
    profiles: Vec<config::Profile>,
    connections: Vec<api::Connection>,
    rules: Vec<rule_manager::Row>,
    policies: Vec<String>,
    shortcuts: [bool; 5],
    dirty: bool,
    logs: Vec<(String, String)>,
    details: Vec<(String, String)>,
}
fn project(state: &State) -> Projection {
    let s = &state.session;
    let query = state.query.to_lowercase();
    let mut groups = vec![];
    for (name, group) in &s.proxies.proxies {
        if group.all.is_empty() {
            continue;
        }
        let indices = proxy_order::sorted_indices(
            &group.all,
            s.settings.node_sort,
            |node| s.proxies.proxies.get(node).and_then(|n| n.delay),
            |node| {
                query.is_empty()
                    || node.to_lowercase().contains(&query)
                    || name.as_str().to_lowercase().contains(&query)
            },
        );
        let nodes = indices
            .into_iter()
            .map(|i| {
                let name = group.all[i].to_string();
                let proxy = s
                    .proxies
                    .proxies
                    .get(name.as_str())
                    .cloned()
                    .unwrap_or_default();
                (name, proxy)
            })
            .collect();
        groups.push((name.to_string(), group.clone(), nodes));
    }
    let node = s
        .raw()
        .ok()
        .and_then(|raw| core::active_route::root(&raw).ok())
        .and_then(|root| core::active_route::from_snapshot(&root, &s.proxies))
        .unwrap_or_default();
    let document = s.document().ok();
    let (rules, _) = document
        .as_ref()
        .map(|d| d.rows(&s.draft))
        .unwrap_or_default();
    let saved = rule_manager::ProfileRules::for_profile(&s.settings, &s.profile().id, false);
    let mut connections = s
        .connections
        .connections
        .iter()
        .filter(|c| {
            query.is_empty()
                || format!(
                    "{} {} {} {:?}",
                    core::connection::name(c),
                    c.metadata.destination_ip,
                    c.rule,
                    c.chains
                )
                .to_lowercase()
                .contains(&query)
        })
        .cloned()
        .collect::<Vec<_>>();
    let field = if state.connection_sort.starts_with("名称") {
        core::connection::Field::Name
    } else if state.connection_sort.starts_with("上传速度") {
        core::connection::Field::UploadRate
    } else if state.connection_sort.starts_with("下载量") {
        core::connection::Field::DownloadTotal
    } else {
        core::connection::Field::DownloadRate
    };
    let sort = core::connection::Sort {
        field: Some(field),
        descending: state.connection_sort.ends_with('↓') || state.connection_sort.ends_with("Z–A"),
    };
    connections.sort_by(|a, b| sort.compare(a, b));
    let mut details = vec![];
    if let Some(c) = s
        .connections
        .connections
        .iter()
        .find(|c| c.id.as_ref() == state.selected)
    {
        details = vec![
            ("连接名称".into(), core::connection::name(c).into()),
            ("域名".into(), c.metadata.host.to_string()),
            (
                "嗅探域名".into(),
                if c.metadata.sniff_host.as_str().is_empty() {
                    c.metadata.host.to_string()
                } else {
                    c.metadata.sniff_host.to_string()
                },
            ),
            ("目标 IP".into(), c.metadata.destination_ip.to_string()),
            ("目标端口".into(), c.metadata.destination_port.to_string()),
            ("协议".into(), c.metadata.network.to_string()),
            ("匹配规则".into(), format!("{} {}", c.rule, c.rule_payload)),
            (
                "代理链".into(),
                c.chains
                    .iter()
                    .rev()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" → "),
            ),
            (
                "上传速度".into(),
                format!("{}/s", bytes(c.rates.map(|r| r.upload).unwrap_or(0))),
            ),
            (
                "下载速度".into(),
                format!("{}/s", bytes(c.rates.map(|r| r.download).unwrap_or(0))),
            ),
            ("累计上传".into(), bytes(c.upload)),
            ("累计下载".into(), bytes(c.download)),
            ("开始时间".into(), c.start.to_string()),
            ("连接 ID".into(), c.id.to_string()),
        ];
    }
    Projection {
        settings: s.settings.clone(),
        running: s.running,
        vpn: s.vpn,
        profile: s.profile().name.clone(),
        node,
        total: s.connections.connections.len(),
        upload: bytes(s.connections.upload_total),
        download: bytes(s.connections.download_total),
        groups,
        profiles: s
            .profiles
            .iter()
            .filter(|p| query.is_empty() || p.name.to_lowercase().contains(&query))
            .cloned()
            .collect(),
        connections,
        rules: rules
            .into_iter()
            .filter(|r| query.is_empty() || r.raw.to_lowercase().contains(&query))
            .collect(),
        policies: document.map(|d| d.policies).unwrap_or_default(),
        shortcuts: s.draft.enabled,
        dirty: s.draft != *saved,
        logs: state
            .logs
            .iter()
            .filter(|(level, line)| {
                (state.log_level == "全部等级" || level == &state.log_level)
                    && (query.is_empty() || line.to_lowercase().contains(&query))
            })
            .cloned()
            .collect(),
        details,
    }
}

fn apply(ui: &AndroidWindow, p: Projection) {
    let data = ui.global::<MobileData>();
    data.set_core_running(p.running);
    data.set_vpn_connected(p.vpn);
    data.set_profile(p.profile.into());
    data.set_node(p.node.into());
    data.set_connection_count(p.total.to_string().into());
    data.set_upload_total(p.upload.into());
    data.set_download_total(p.download.into());
    data.set_dns_override(p.settings.dns_override);
    data.set_ipv6(p.settings.ipv6);
    data.set_udp(p.settings.udp);
    data.set_mixed_port(p.settings.mixed_port.to_string().into());
    data.set_controller_port(p.settings.controller_port.to_string().into());
    data.set_interval(p.settings.delay_interval_minutes.to_string().into());
    data.set_mode(match p.settings.run_mode.as_str() {
        "global" => 1,
        "direct" => 2,
        _ => 0,
    });
    data.set_rule_dirty(p.dirty);
    data.set_shortcuts(rows(
        [
            "拦截广告",
            "私有域名直连",
            "私有 IP 直连",
            "国内域名直连",
            "国内 IP 直连",
        ]
        .into_iter()
        .map(Into::into)
        .collect(),
    ));
    data.set_shortcut_values(rows(p.shortcuts.into()));
    data.set_rule_profiles(rows(
        p.profiles.iter().map(|p| p.name.clone().into()).collect(),
    ));
    data.set_rule_policies(rows(p.policies.into_iter().map(Into::into).collect()));
    data.set_connection_details(rows(
        p.details
            .into_iter()
            .map(|(label, value)| DetailItem {
                label: label.into(),
                value: if value.is_empty() {
                    "—".into()
                } else {
                    value.into()
                },
            })
            .collect(),
    ));
    data.set_profiles(rows(
        p.profiles
            .into_iter()
            .map(|profile| ProfileItem {
                id: profile.id.clone().into(),
                name: profile.name.into(),
                source: profile.source.clone().into(),
                detail: profile
                    .usage
                    .unwrap_or_else(|| format!("更新：{}", profile.updated))
                    .into(),
                active: p.settings.active_profile.as_deref() == Some(&profile.id),
                local: profile.source.is_empty(),
            })
            .collect(),
    ));
    data.set_groups(rows(
        p.groups
            .into_iter()
            .map(|(name, group, nodes)| {
                let node_items = nodes
                    .into_iter()
                    .map(|(name, node)| NodeItem {
                        group: group.now.to_string().into(),
                        selected: name == group.now.as_str(),
                        name: name.into(),
                        kind: node.kind.to_string().into(),
                        delay: node
                            .delay
                            .map(|d| format!("{d} ms"))
                            .unwrap_or_else(|| "未测速".into())
                            .into(),
                        valid: true,
                    })
                    .collect::<Vec<_>>();
                let count = node_items.len();
                GroupItem {
                    id: name.clone().into(),
                    title: name.into(),
                    detail: format!("{} · {count} 个节点", group.kind).into(),
                    current: group.now.to_string().into(),
                    expanded: true,
                    nodes: rows(
                        node_items
                            .chunks(2)
                            .map(|pair| NodePair {
                                left: pair[0].clone(),
                                right: pair.get(1).cloned().unwrap_or_default(),
                            })
                            .collect(),
                    ),
                    page: format!("{count} 个节点").into(),
                    ..Default::default()
                }
            })
            .collect(),
    ));
    data.set_connections(rows(
        p.connections
            .into_iter()
            .map(|c| ConnectionItem {
                id: c.id.to_string().into(),
                host: format!(
                    "{}:{}",
                    core::connection::name(&c),
                    c.metadata.destination_port
                )
                .into(),
                detail: c.start.to_string().into(),
                rule: format!("{} {}", c.rule, c.rule_payload).into(),
                route: c
                    .chains
                    .iter()
                    .rev()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" → ")
                    .into(),
                up: format!("{}/s", bytes(c.rates.map(|r| r.upload).unwrap_or(0))).into(),
                down: format!("{}/s", bytes(c.rates.map(|r| r.download).unwrap_or(0))).into(),
                upload: bytes(c.upload).into(),
                download: bytes(c.download).into(),
            })
            .collect(),
    ));
    data.set_rules(rows(
        p.rules
            .into_iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let parts = rule_manager::parts(&row.raw).ok()?;
                Some(RuleItem {
                    id: row.id.into(),
                    position: (index + 1).to_string().into(),
                    kind: parts.kind.into(),
                    value: parts.payload.into(),
                    policy: parts.policy.into(),
                    source: row.source.to_string().into(),
                    selected: false,
                    enabled: row.enabled,
                })
            })
            .collect(),
    ));
    data.set_logs(rows(
        p.logs
            .into_iter()
            .map(|(level, message)| LogItem {
                tone: if level == "ERROR" {
                    3
                } else if level == "WARN" {
                    2
                } else {
                    4
                },
                level: level.into(),
                message: message.into(),
                time: "".into(),
            })
            .collect(),
    ));
    let theme = ui.global::<Theme>();
    theme.set_appearance(match p.settings.theme_mode {
        Some(config::ThemeMode::Dark) => 0,
        Some(config::ThemeMode::Light) => 1,
        _ => 2,
    });
    theme.set_text_scale(p.settings.text_scale as f32 / 100.);
    theme.set_accent_index(
        config::AccentColor::ALL
            .iter()
            .position(|c| c == &p.settings.accent_color)
            .unwrap_or(0) as i32,
    );
}
