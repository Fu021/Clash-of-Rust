//! Read-only audit of known Claude projects. Never walks the home directory,
//! executes project hooks, or retains credentials and conversation content.
use super::preferences::redact_url;
use serde::{
    Deserialize, Serialize,
    de::{self, IgnoredAny, MapAccess, Visitor},
};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const PROJECT_LIMIT: usize = 4096;
const FINDING_LIMIT: usize = 512;
const SETTINGS_LIMIT: u64 = 256 * 1024;
const REGISTRY_LIMIT: u64 = 16 * 1024 * 1024;
const KEYS: &[&str] = &[
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_BEDROCK_BASE_URL",
    "ANTHROPIC_VERTEX_BASE_URL",
    "ANTHROPIC_FOUNDRY_BASE_URL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "CLOUD_ML_REGION",
    "NODE_TLS_REJECT_UNAUTHORIZED",
    "CLAUDE_CONFIG_DIR",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    Issue,
    Review,
    Incomplete,
}
impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Self::Issue => "配置问题",
            Self::Review => "需核对",
            Self::Incomplete => "未完成检查",
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub level: Level,
    pub path: String,
    pub field: String,
    pub value: String,
    pub reason: String,
    pub advice: String,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct Audit {
    pub projects: usize,
    pub files: usize,
    pub missing_projects: usize,
    pub omitted: usize,
    pub incomplete: bool,
    pub findings: Vec<Finding>,
    pub clean_count: usize,
    pub clean_files: Vec<String>,
    pub issues: usize,
    pub reviews: usize,
    pub unreadable: usize,
}
impl Audit {
    fn add(
        &mut self,
        level: Level,
        path: &Path,
        field: &str,
        value: String,
        reason: &str,
        advice: &str,
    ) {
        self.incomplete |= level == Level::Incomplete;
        match level {
            Level::Issue => self.issues += 1,
            Level::Review => self.reviews += 1,
            Level::Incomplete => self.unreadable += 1,
        }
        if self.findings.len() == FINDING_LIMIT {
            self.omitted += 1;
            return;
        }
        self.findings.push(Finding {
            level,
            path: display(path),
            field: field.into(),
            value,
            reason: reason.into(),
            advice: advice.into(),
        });
    }
    fn unavailable(&mut self, path: &Path) {
        self.add(
            Level::Incomplete,
            path,
            "文件检查",
            String::new(),
            "文件不可读、格式无效或超过读取上限，相关覆盖尚未确认。",
            "核对文件权限和 JSON 格式后重新检测。",
        );
    }
}
fn display(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .filter(|c| !c.is_control())
        .take(1024)
        .collect()
}

/// Cancels blocking filesystem work when the user stops or leaves the page.
#[derive(Debug, Clone)]
pub struct Scan(Arc<AtomicBool>);
impl Default for Scan {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
}
impl Scan {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

pub struct Inputs {
    pub home: PathBuf,
    pub config: PathBuf,
    pub current: Option<PathBuf>,
    pub managed: Vec<PathBuf>,
    pub mixed_port: u16,
}

#[derive(Default)]
struct Projects {
    paths: BTreeSet<PathBuf>,
    omitted: usize,
}
impl<'de> Deserialize<'de> for Projects {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Paths;
        impl<'de> Visitor<'de> for Paths {
            type Value = Projects;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a project map")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Projects, M::Error> {
                let mut projects = Projects::default();
                while let Some(key) = map.next_key::<String>()? {
                    map.next_value::<IgnoredAny>()?;
                    let path = PathBuf::from(&key);
                    if path.is_absolute() && !key.chars().any(char::is_control) {
                        if projects.paths.len() < PROJECT_LIMIT {
                            projects.paths.insert(path);
                        } else {
                            projects.omitted += 1;
                        }
                    }
                }
                Ok(projects)
            }
        }
        d.deserialize_map(Paths)
    }
}
#[derive(Default, Deserialize)]
struct Registry {
    #[serde(default)]
    projects: Projects,
}
#[derive(Deserialize)]
struct ProjectIndex {
    #[serde(default, rename = "originalPath")]
    original: Option<PathBuf>,
    #[serde(default)]
    entries: Vec<ProjectIndexEntry>,
}
#[derive(Deserialize)]
struct ProjectIndexEntry {
    #[serde(default, rename = "projectPath")]
    project: Option<PathBuf>,
}

#[derive(Default)]
struct Settings(Vec<(String, Option<String>)>);
impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Env;
        impl<'de> Visitor<'de> for Env {
            type Value = Settings;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an environment map")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Settings, M::Error> {
                let mut settings = Settings::default();
                while let Some(key) = map.next_key::<String>()? {
                    if KEYS.contains(&key.as_str()) {
                        let value = map.next_value::<serde_json::Value>()?;
                        let value = value
                            .as_str()
                            .filter(|s| s.len() <= 4096)
                            .map(str::to_owned);
                        if let Some(old) = settings.0.iter_mut().find(|(k, _)| k == &key) {
                            old.1 = value;
                        } else {
                            settings.0.push((key, value));
                        }
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(settings)
            }
        }
        d.deserialize_map(Env)
    }
}
#[derive(Deserialize)]
struct Config {
    #[serde(default)]
    env: Settings,
}
struct Object<T>(T);
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ObjectVisitor<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
            type Value = Object<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                T::deserialize(de::value::MapAccessDeserializer::new(map)).map(Object)
            }
        }
        d.deserialize_map(ObjectVisitor(std::marker::PhantomData))
    }
}

fn read<T: for<'de> Deserialize<'de>>(path: &Path, limit: u64) -> anyhow::Result<T> {
    let file = fs::File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= limit,
        "size/type"
    );
    let mut json =
        serde_json::Deserializer::from_reader(std::io::BufReader::new(file.take(limit + 1)));
    let Object(value) = Object::<T>::deserialize(&mut json)?;
    json.end()?;
    Ok(value)
}
fn present(path: &Path, audit: &mut Audit) -> bool {
    match fs::metadata(path) {
        Ok(m) if m.is_file() => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        _ => {
            audit.unavailable(path);
            false
        }
    }
}
fn bypasses_claude(value: &str) -> bool {
    value
        .split(|c: char| c == ',' || c.is_whitespace())
        .any(|part| {
            let part = part.trim().to_ascii_lowercase();
            if part == "*" {
                return true;
            }
            // Domain suffixes are matched at a label boundary, never by substring.
            let host = part
                .trim_start_matches("*.")
                .trim_start_matches('.')
                .split(':')
                .next()
                .unwrap_or("");
            super::claude_host(host)
                || [
                    "claude.ai",
                    "claude.com",
                    "anthropic.com",
                    "api.anthropic.com",
                ]
                .iter()
                .any(|target| {
                    !host.is_empty() && (*target == host || target.ends_with(&format!(".{host}")))
                })
        })
}
fn inspect(audit: &mut Audit, path: &Path, settings: Settings, mixed_port: u16) {
    for (key, value) in settings.0 {
        let Some(value) = value else {
            audit.add(
                Level::Issue,
                path,
                &key,
                String::new(),
                "相关环境字段不是字符串或过长。",
                "核对字段类型，不要把密钥写入代理或地址字段。",
            );
            continue;
        };
        let lower = key.to_ascii_lowercase();
        if lower == "no_proxy" {
            if bypasses_claude(&value) {
                audit.add(
                    Level::Review,
                    path,
                    &key,
                    "包含 Claude 域名或全部地址".into(),
                    "此配置可能让 Claude 请求绕过 HTTP 代理；TUN 是否接管需要结合实时请求确认。",
                    "移除不需要的绕过项，并重新启动相应会话验证。",
                );
            }
        } else if lower.ends_with("proxy") {
            if value.is_empty() {
                audit.add(
                    Level::Review,
                    path,
                    &key,
                    "空值".into(),
                    "此配置清空了代理变量，可能覆盖终端中原有的代理。",
                    "核对是否有其他代理变量或 TUN 接管。",
                );
                continue;
            }
            let url = reqwest::Url::parse(&value);
            let valid = url
                .as_ref()
                .ok()
                .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
            if lower == "all_proxy" {
                audit.add(
                    Level::Review,
                    path,
                    &key,
                    redact_url(&value),
                    "Claude Code 不保证使用 ALL_PROXY；仅设置此变量可能无法代理其请求。",
                    "核对 HTTPS_PROXY 或 HTTP_PROXY，并观察实际线路。",
                );
            } else if valid.is_none() {
                audit.add(
                    Level::Issue,
                    path,
                    &key,
                    redact_url(&value),
                    "代理地址无效或协议不受 Claude Code 支持。",
                    "使用有效的 HTTP/HTTPS 代理地址。",
                );
            } else if let Some(url) = valid {
                let local = url.host_str().is_some_and(|h| {
                    h == "localhost"
                        || h.trim_matches(['[', ']'])
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                });
                if !local || url.port_or_known_default() != Some(mixed_port) {
                    audit.add(
                        Level::Review,
                        path,
                        &key,
                        redact_url(&value),
                        "代理地址与本应用的混合端口不同，实际出口可能使用另一条线路。",
                        "核对是否为有意设置；本项不证明该代理不可用。",
                    );
                }
            }
        } else if key.ends_with("BASE_URL") && !value.is_empty() {
            let official = key == "ANTHROPIC_BASE_URL"
                && reqwest::Url::parse(&value).is_ok_and(|u| {
                    u.scheme() == "https"
                        && u.host_str() == Some("api.anthropic.com")
                        && u.username().is_empty()
                        && u.password().is_none()
                        && u.port_or_known_default() == Some(443)
                });
            if !official {
                audit.add(
                    Level::Review,
                    path,
                    &key,
                    redact_url(&value),
                    "API 地址被覆盖；本机官方域名的出口检测不能代表中转服务的远端出口。",
                    "确认目标地址是预期服务，并在实际会话中核对来源。",
                );
            }
        } else if key == "NODE_TLS_REJECT_UNAUTHORIZED" && value == "0" {
            audit.add(
                Level::Issue,
                path,
                &key,
                "0".into(),
                "此配置关闭 TLS 证书验证，可能掩盖代理或证书问题。",
                "恢复证书验证，使用正确的受信任证书。",
            );
        } else if key.starts_with("CLAUDE_CODE_USE_") && matches!(value.as_str(), "1" | "true") {
            audit.add(
                Level::Review,
                path,
                &key,
                "已启用".into(),
                "会话可能使用云平台提供方，官方域名探测不能代表该提供方的实际出口。",
                "核对提供方、区域和实际会话配置。",
            );
        } else if matches!(
            key.as_str(),
            "AWS_REGION" | "AWS_DEFAULT_REGION" | "CLOUD_ML_REGION"
        ) && !value.is_empty()
        {
            audit.add(
                Level::Review,
                path,
                &key,
                "已设置区域覆盖".into(),
                "云平台区域配置可能改变模型请求目的地；是否启用对应提供方尚未确认。",
                "结合提供方设置核对区域，本项不等同于本机所在地区。",
            );
        } else if key == "CLAUDE_CONFIG_DIR" && !value.is_empty() {
            audit.add(
                Level::Review,
                path,
                &key,
                "已设置其他配置目录".into(),
                "此配置可能使会话读取另一套配置；项目级设置是否被忽略取决于 Code 版本。",
                "核对 Code 的 /status 与实际配置目录。",
            );
        }
    }
}

pub fn scan(inputs: Inputs, cancel: &Scan) -> Audit {
    let mut audit = Audit::default();
    let mut projects = BTreeSet::new();
    let mut registries = BTreeSet::from([
        inputs.home.join(".claude.json"),
        inputs.config.join(".claude.json"),
    ]);
    registries.insert(inputs.config.with_extension("json"));
    for path in registries {
        if cancel.cancelled() {
            audit.incomplete = true;
            return audit;
        }
        if !present(&path, &mut audit) {
            continue;
        }
        match read::<Registry>(&path, REGISTRY_LIMIT) {
            Ok(registry) => {
                if registry.projects.omitted > 0 {
                    audit.add(
                        Level::Incomplete,
                        &path,
                        "项目清单",
                        String::new(),
                        "项目清单超过 4096 项，部分目录未检查。",
                        "缩减过期项目记录后重新检测。",
                    );
                }
                projects.extend(registry.projects.paths);
            }
            Err(_) => audit.unavailable(&path),
        }
    }
    if let Some(current) = &inputs.current {
        for path in current.ancestors() {
            if path.join(".claude").is_dir() {
                projects.insert(path.to_owned());
                break;
            }
        }
    }
    // Supplement older project registries with session index metadata. Never
    // open transcript JSONL files or guess paths from encoded folder names.
    for config in BTreeSet::from([inputs.config.clone(), inputs.home.join(".claude")]) {
        let directory = config.join("projects");
        match fs::read_dir(&directory) {
            Ok(entries) => {
                for (count, entry) in entries.enumerate() {
                    if cancel.cancelled() {
                        audit.incomplete = true;
                        return audit;
                    }
                    if count >= PROJECT_LIMIT {
                        audit.add(
                            Level::Incomplete,
                            &directory,
                            "项目索引",
                            String::new(),
                            "项目索引超过检查上限，部分目录未检查。",
                            "清理过期项目索引后重新检测。",
                        );
                        break;
                    }
                    let Ok(entry) = entry else {
                        audit.unavailable(&directory);
                        continue;
                    };
                    if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                        continue;
                    }
                    let path = entry.path().join("sessions-index.json");
                    if !present(&path, &mut audit) {
                        continue;
                    }
                    match read::<ProjectIndex>(&path, 1024 * 1024) {
                        Ok(index) => {
                            for path in index
                                .original
                                .into_iter()
                                .chain(index.entries.into_iter().filter_map(|entry| entry.project))
                            {
                                if path.is_absolute()
                                    && !path.to_string_lossy().chars().any(char::is_control)
                                {
                                    if projects.len() < PROJECT_LIMIT || projects.contains(&path) {
                                        projects.insert(path);
                                    } else {
                                        audit.incomplete = true;
                                    }
                                }
                            }
                        }
                        Err(_) => audit.unavailable(&path),
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => audit.unavailable(&directory),
        }
    }
    let mut files = BTreeSet::from([
        inputs.config.join("settings.json"),
        inputs.home.join(".claude/settings.json"),
    ]);
    let mut directories = BTreeSet::new();
    for project in projects.iter().take(PROJECT_LIMIT) {
        if cancel.cancelled() {
            audit.incomplete = true;
            return audit;
        }
        match fs::metadata(project) {
            Ok(m) if m.is_dir() => {
                if !directories
                    .insert(fs::canonicalize(project).unwrap_or_else(|_| project.clone()))
                {
                    continue;
                }
                audit.projects += 1;
                files.insert(project.join(".claude/settings.json"));
                files.insert(project.join(".claude/settings.local.json"));
                // Code also stores project-local settings at the repository
                // root when a session starts in a nested working directory.
                if let Some(root) = project.ancestors().find(|p| p.join(".git").exists()) {
                    files.insert(root.join(".claude/settings.local.json"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => audit.missing_projects += 1,
            _ => audit.unavailable(project),
        }
    }
    if projects.len() > PROJECT_LIMIT {
        audit.add(
            Level::Incomplete,
            &inputs.config,
            "项目清单",
            String::new(),
            "合并后的项目清单超过 4096 项，部分项目未检查。",
            "清理过期项目记录后重新检测。",
        );
    }
    for managed in inputs.managed {
        if let Some(parent) = managed.parent() {
            let directory = parent.join("managed-settings.d");
            match fs::read_dir(&directory) {
                Ok(entries) => {
                    for (count, entry) in entries.enumerate() {
                        if count >= 128 {
                            audit.add(
                                Level::Incomplete,
                                &directory,
                                "托管配置",
                                String::new(),
                                "托管配置目录超过检查上限，部分文件未检查。",
                                "缩减过期文件后重新检测。",
                            );
                            break;
                        }
                        let Ok(entry) = entry else {
                            audit.unavailable(&directory);
                            continue;
                        };
                        let hidden = entry.file_name().to_string_lossy().starts_with('.');
                        if !hidden && entry.path().extension().is_some_and(|s| s == "json") {
                            files.insert(entry.path());
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => audit.unavailable(&directory),
            }
        }
        files.insert(managed);
    }
    let mut checked = BTreeSet::new();
    for path in files {
        if cancel.cancelled() {
            audit.incomplete = true;
            break;
        }
        if !present(&path, &mut audit) {
            continue;
        }
        if !checked.insert(fs::canonicalize(&path).unwrap_or_else(|_| path.clone())) {
            continue;
        }
        audit.files += 1;
        match read::<Config>(&path, SETTINGS_LIMIT) {
            Ok(config) => {
                let before = audit.findings.len() + audit.omitted;
                inspect(&mut audit, &path, config.env, inputs.mixed_port);
                if audit.findings.len() + audit.omitted == before {
                    audit.clean_count += 1;
                    if audit.clean_files.len() < FINDING_LIMIT {
                        audit.clean_files.push(display(&path));
                    }
                }
            }
            Err(error) if error.downcast_ref::<serde_json::Error>().is_some() => audit.add(
                Level::Issue,
                &path,
                "配置格式",
                String::new(),
                "配置不是有效的 JSON 对象，或 env 字段格式错误。",
                "修正 JSON 格式后重新检测。",
            ),
            Err(_) => audit.unavailable(&path),
        }
    }
    audit.findings.sort_by_key(|f| match f.level {
        Level::Issue => 0,
        Level::Review => 1,
        Level::Incomplete => 2,
    });
    audit
}

pub fn scan_available(inputs: Option<Inputs>, cancel: &Scan) -> Audit {
    inputs
        .map(|inputs| scan(inputs, cancel))
        .unwrap_or_else(|| {
            let mut audit = Audit::default();
            audit.add(
                Level::Incomplete,
                Path::new("用户目录"),
                "扫描",
                String::new(),
                "无法获取用户目录，尚未检查 Claude 配置。",
                "核对运行账户后重新检测。",
            );
            audit
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn write(path: &Path, value: serde_json::Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn inputs(root: &Path) -> Inputs {
        Inputs {
            home: root.join("home"),
            config: root.join("home/.claude"),
            current: None,
            managed: vec![root.join("managed/managed-settings.json")],
            mixed_port: 7890,
        }
    }
    #[test]
    fn scans_all_recorded_projects_and_redacts_credentials_without_running_hooks() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("outside home/project ' 中文");
        let b = root.path().join("other project");
        let stale = root.path().join("deleted project");
        let config = a.join(".claude/settings.local.json");
        write(
            &config,
            serde_json::json!({"env":{
            "NO_PROXY":".anthropic.com", "ANTHROPIC_BASE_URL":"https://user:secret@relay.example/private?key=secret",
            "ANTHROPIC_API_KEY":"never-retain"}, "hooks":{"SessionStart":[{"command":"must-not-execute"}]}}),
        );
        write(
            &b.join(".claude/settings.json"),
            serde_json::json!({"env":{"HTTPS_PROXY":"http://127.0.0.1:7890","LANG":"zh_CN.UTF-8","TZ":"Asia/Shanghai"}}),
        );
        write(
            &root.path().join("home/.claude.json"),
            serde_json::json!({
            "oauthAccount":{"token":"never-retain"}, "projects":{
                a.to_str().unwrap(): {"history":["never-retain"]}, b.to_str().unwrap(): {}, stale.to_str().unwrap(): {}}}),
        );
        let result = scan(inputs(root.path()), &Scan::default());
        assert_eq!(
            (result.projects, result.files, result.missing_projects),
            (2, 2, 1)
        );
        assert_eq!((result.reviews, result.clean_count), (2, 1));
        assert!(result.findings.iter().all(|f| f.path == display(&config)));
        let json = serde_json::to_string(&result).unwrap();
        for secret in [
            "secret",
            "never-retain",
            "must-not-execute",
            "user:",
            "?key",
        ] {
            assert!(!json.contains(secret));
        }
        assert!(!result.incomplete);
    }
    #[test]
    fn project_index_metadata_finds_projects_without_opening_transcripts() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("source");
        write(
            &project.join(".claude/settings.json"),
            serde_json::json!({"env":{"NODE_TLS_REJECT_UNAUTHORIZED":"0"}}),
        );
        write(
            &root
                .path()
                .join("home/.claude/projects/encoded/sessions-index.json"),
            serde_json::json!({"originalPath":project,"entries":[{"projectPath":project,"summary":"must-not-retain"}]}),
        );
        fs::write(
            root.path()
                .join("home/.claude/projects/encoded/session.jsonl"),
            "not valid JSON and must not be opened",
        )
        .unwrap();
        let result = scan(inputs(root.path()), &Scan::default());
        assert_eq!((result.projects, result.files, result.issues), (1, 1, 1));
        assert!(!result.incomplete);
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("must-not-retain")
        );
    }
    #[test]
    fn bypass_domains_use_label_boundaries_and_include_subdomains() {
        for value in [
            "*",
            ".anthropic.com",
            "api.anthropic.com:443",
            "*.claude.ai",
            "statsig.anthropic.com",
            "example.org,claude.com",
        ] {
            assert!(bypasses_claude(value), "{value}");
        }
        for value in [
            "localhost,127.0.0.1",
            "evilclaude.ai",
            "anthropic.com.evil.test",
            "not-anthropic.com",
            "",
        ] {
            assert!(!bypasses_claude(value), "{value}");
        }
    }
    #[test]
    fn global_custom_and_managed_configs_are_checked_and_safe_fields_are_not_risks() {
        let root = tempfile::tempdir().unwrap();
        let mut input = inputs(root.path());
        input.config = root.path().join("custom-code");
        write(
            &input.home.join(".claude/settings.json"),
            serde_json::json!({"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com/","LANG":"zh_CN.UTF-8"}}),
        );
        write(
            &input.config.join("settings.json"),
            serde_json::json!({"env":{"HTTPS_PROXY":"socks5://127.0.0.1:7890"}}),
        );
        write(
            &root
                .path()
                .join("managed/managed-settings.d/10-region.json"),
            serde_json::json!({"env":{"CLAUDE_CODE_USE_VERTEX":"1","CLOUD_ML_REGION":"private-metadata-do-not-retain"}}),
        );
        write(
            &root.path().join("managed/managed-settings.d/.ignored.json"),
            serde_json::json!({"env":{"NO_PROXY":"*"}}),
        );
        let result = scan(input, &Scan::default());
        assert_eq!(
            (
                result.files,
                result.clean_count,
                result.issues,
                result.reviews
            ),
            (3, 1, 1, 2)
        );
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("private-metadata")
        );
    }
    #[test]
    fn malformed_settings_and_unreadable_size_limits_do_not_report_normal() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("home/.claude/settings.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{\"env\":{\"HTTPS_PROXY\":\"private-secret\"").unwrap();
        let result = scan(inputs(root.path()), &Scan::default());
        assert_eq!((result.issues, result.clean_count), (1, 0));
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("private-secret")
        );
        for invalid in ["[]", "null", "{\"env\":null}"] {
            fs::write(&path, invalid).unwrap();
            let result = scan(inputs(root.path()), &Scan::default());
            assert_eq!((result.issues, result.clean_count), (1, 0));
        }
        fs::write(&path, vec![b' '; SETTINGS_LIMIT as usize + 1]).unwrap();
        let result = scan(inputs(root.path()), &Scan::default());
        assert!(result.incomplete);
        assert_eq!((result.unreadable, result.clean_count), (1, 0));
    }
    #[test]
    fn retained_findings_are_bounded_but_counts_remain_exact() {
        let mut audit = Audit::default();
        for _ in 0..1000 {
            inspect(
                &mut audit,
                Path::new("settings.json"),
                Settings(vec![("NO_PROXY".into(), Some("*".into()))]),
                7890,
            );
        }
        assert_eq!(
            (audit.findings.len(), audit.reviews, audit.omitted),
            (512, 1000, 488)
        );
        let cancel = Scan::default();
        cancel.cancel();
        let result = scan(inputs(Path::new("must-not-open")), &cancel);
        assert!(result.incomplete && result.files == 0 && result.projects == 0);
    }
    #[test]
    fn local_ipv6_proxy_and_official_api_are_not_flagged_as_other_routes() {
        let mut audit = Audit::default();
        inspect(
            &mut audit,
            Path::new("settings.json"),
            Settings(vec![
                ("HTTPS_PROXY".into(), Some("http://[::1]:7890".into())),
                (
                    "ANTHROPIC_BASE_URL".into(),
                    Some("https://api.anthropic.com".into()),
                ),
                ("NO_PROXY".into(), Some("localhost,127.0.0.1".into())),
            ]),
            7890,
        );
        assert!(audit.findings.is_empty());
    }
    #[test]
    fn nested_project_records_also_check_repository_local_settings() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repository");
        let project = repo.join("source/nested");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();
        write(
            &repo.join(".claude/settings.local.json"),
            serde_json::json!({"env":{"NO_PROXY":"*"}}),
        );
        write(
            &root.path().join("home/.claude.json"),
            serde_json::json!({"projects":{project.to_str().unwrap():{}}}),
        );
        let result = scan(inputs(root.path()), &Scan::default());
        assert_eq!((result.projects, result.files, result.reviews), (1, 1, 1));
    }
    #[test]
    fn project_registry_is_bounded_and_skips_account_fields() {
        let root = tempfile::tempdir().unwrap();
        let mut projects = serde_json::Map::new();
        for n in 0..PROJECT_LIMIT + 1 {
            projects.insert(
                root.path()
                    .join(format!("{n}"))
                    .to_string_lossy()
                    .into_owned(),
                serde_json::json!({"private":"never-retain"}),
            );
        }
        let registry: Registry = serde_json::from_value(
            serde_json::json!({"projects":projects,"oauthAccount":{"private":"never-retain"}}),
        )
        .unwrap();
        assert_eq!(
            (registry.projects.paths.len(), registry.projects.omitted),
            (PROJECT_LIMIT, 1)
        );
    }
    #[cfg(unix)]
    #[test]
    fn linked_config_aliases_are_checked_once() {
        let root = tempfile::tempdir().unwrap();
        let input = inputs(root.path());
        write(
            &input.config.join("settings.json"),
            serde_json::json!({"env":{"NO_PROXY":"*"}}),
        );
        let project = root.path().join("project");
        fs::create_dir_all(project.join(".claude")).unwrap();
        std::os::unix::fs::symlink(
            input.config.join("settings.json"),
            project.join(".claude/settings.json"),
        )
        .unwrap();
        write(
            &input.home.join(".claude.json"),
            serde_json::json!({"projects":{project.to_str().unwrap():{}}}),
        );
        let result = scan(input, &Scan::default());
        assert_eq!((result.files, result.reviews), (1, 1));
    }
}
