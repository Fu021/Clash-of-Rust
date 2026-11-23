//! Bounded logs and stream framing, reusable by desktop and mobile core hosts.
use std::{
    borrow::Cow,
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

pub const LOG_LIMIT: usize = 500;
pub const LOG_FRAME_LIMIT: usize = 64 * 1024;
pub type LogBuffer = Arc<Mutex<VecDeque<Arc<str>>>>;

const CLIENT_LOG_PREFIX: &str = "[客户端] ";
const CORE_LOG_PREFIX: &str = "[内核] ";

pub fn display(message: &str, language: crate::config::Language) -> Cow<'_, str> {
    if language != crate::config::Language::English {
        return Cow::Borrowed(message);
    }
    let Some(body) = message.strip_prefix(CLIENT_LOG_PREFIX) else {
        return Cow::Borrowed(message);
    };
    let translated = match body {
        "继续使用上次运行的系统代理，保留原设置恢复记录" => {
            "Continuing the previous system proxy; original settings remain available for recovery"
        }
        "已恢复上次运行遗留的系统代理" => {
            "Restored system proxy settings from the previous run"
        }
        "已自动分配可用的内核控制端口" => "Assigned an available core control port",
        "mihomo 已启动，控制接口可用" => "mihomo started; control interface is available",
        "内核已停止" => "Core stopped",
        "规则已校验并保存；原始订阅文件保留" => {
            "Rules validated and saved; original subscription retained"
        }
        "订阅附加规则已保存并应用" => "Additional subscription rules saved and applied",
        "配置已导入/更新" => "Configuration imported or updated",
        "Claude 保护已启用：TUN、模式与节点锁定、代理 DoH；原设置恢复记录已保存" => {
            "Claude protection enabled; TUN, mode and node locked; proxy DoH active; original settings saved"
        }
        "Claude 保护已关闭，原代理模式、DNS 与规则已恢复" => {
            "Claude protection disabled; previous proxy mode, DNS and rules restored"
        }
        "全部 Geo 数据更新完成并通过校验" => "All Geo data updated and verified",
        "Windows 网络服务已接管内核，桌面界面保持普通用户运行" => {
            "Windows network service took over the core; the desktop remains a regular user process"
        }
        "已授权并重启 mihomo，桌面界面保持普通用户运行" => {
            "mihomo authorized and restarted; the desktop remains a regular user process"
        }
        "TUN 未配置 IPv6 地址（tun_ipv6_not_configured）；IPv6 接管尚未建立" => {
            "TUN has no IPv6 address (tun_ipv6_not_configured); IPv6 routing is not active"
        }
        "开始下载全部 Geo 数据，下载期间继续使用原数据" => {
            "Downloading Geo data; existing data remains in use"
        }
        "Claude 保护期间网络发生变化，已暂停自动刷新内核出站与 DNS" => {
            "Network changed during Claude protection; automatic core route and DNS refresh paused"
        }
        "网络已恢复或默认路由已变化，正在刷新内核出站与 DNS" => {
            "Network recovered or default route changed; refreshing core routes and DNS"
        }
        "网络刷新完成，已刷新出站与 DNS 并清理旧连接；目标服务是否可用仍需检查" => {
            "Network refresh complete; core routes and DNS refreshed, old connections closed. Target service availability still needs to be checked."
        }
        "正在手动刷新内核出站与 DNS" => "Refreshing core routes and DNS",
        "手动网络刷新完成；目标服务是否可用仍需检查" => {
            "Manual network refresh complete. Target service availability still needs to be checked."
        }
        _ => {
            if let Some(port) = body
                .strip_prefix("等待端口 ")
                .and_then(|value| value.strip_suffix(" 的退出交接或连接状态释放"))
                .filter(|value| value.parse::<u16>().is_ok())
            {
                return format!("[Client] Waiting for port {port} to be released").into();
            }
            if let Some(node) =
                body.strip_prefix("已恢复 Claude 保护；原状态恢复记录保留，使用节点 ")
            {
                return format!(
                    "[Client] Claude protection restored; original settings retained; node {node}"
                )
                .into();
            }
            for (prefix, translated) in [
                ("代理恢复失败：", "Proxy recovery failed: "),
                ("操作失败：", "Operation failed: "),
                ("定时测速失败：", "Scheduled speed test failed: "),
                (
                    "节点状态复测未完成，已保留网络设置：",
                    "Node state recheck did not finish; network settings retained: ",
                ),
                (
                    "全部节点经两个独立目标复测仍不可达；",
                    "All nodes remain unreachable after two independent checks; ",
                ),
            ] {
                if let Some(detail) = body.strip_prefix(prefix) {
                    let detail = crate::error_message::localize(detail, language);
                    return format!("[Client] {translated}{detail}").into();
                }
            }
            if let Some(status) = body
                .strip_prefix("内核意外退出：")
                .and_then(|value| value.strip_suffix("，已尝试恢复系统代理"))
            {
                return format!(
                    "[Client] Core exited unexpectedly: {status}; system proxy recovery attempted"
                )
                .into();
            }
            return Cow::Borrowed(message);
        }
    };
    format!("[Client] {translated}").into()
}

#[derive(Default)]
pub struct LogFrames {
    pending: Vec<u8>,
    discarding: bool,
}

impl LogFrames {
    pub fn push_core_events(&mut self, chunk: &[u8], logs: &LogBuffer) {
        #[derive(serde::Deserialize)]
        struct Entry<'a> {
            #[serde(rename = "type", default)]
            kind: &'a str,
            #[serde(default, borrow)]
            payload: Cow<'a, str>,
        }

        self.push(chunk, |frame| {
            if let Ok(entry) = serde_json::from_slice::<Entry<'_>>(frame) {
                let kind = if entry.kind.is_empty() {
                    "info"
                } else {
                    entry.kind
                };
                append_core(logs, format!("[{kind}] {}", entry.payload));
            }
        });
    }

    pub fn push(&mut self, mut chunk: &[u8], mut emit: impl FnMut(&[u8])) {
        while !chunk.is_empty() {
            let end = chunk.iter().position(|byte| *byte == b'\n');
            let used = end.map_or(chunk.len(), |end| end + 1);
            if !self.discarding {
                if self.pending.len() + used > LOG_FRAME_LIMIT {
                    self.pending.clear();
                    self.discarding = true;
                } else if self.pending.is_empty() && end.is_some() {
                    emit(&chunk[..used]);
                } else {
                    self.pending.extend_from_slice(&chunk[..used]);
                    if end.is_some() {
                        emit(&self.pending);
                        self.pending.clear();
                    }
                }
            }
            if end.is_some() {
                self.discarding = false;
            }
            chunk = &chunk[used..];
        }
    }
}

pub fn append(logs: &LogBuffer, message: impl Into<String>) {
    if let Ok(mut buffer) = logs.lock() {
        if buffer.len() >= LOG_LIMIT {
            buffer.pop_front();
        }
        let mut message = message.into();
        if let Some((end, _)) = message.char_indices().nth(1000) {
            message.truncate(end);
        }
        buffer.push_back(Arc::from(message));
    }
}

fn append_core(logs: &LogBuffer, message: impl Into<String>) {
    let mut message = message.into();
    if message.starts_with(CLIENT_LOG_PREFIX) {
        message.insert_str(0, CORE_LOG_PREFIX);
    }
    append(logs, message);
}

pub async fn read_core_logs(reader: impl AsyncRead + Unpin, logs: LogBuffer) {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::with_capacity(4096);
    while let Ok(bytes) = reader.fill_buf().await {
        if bytes.is_empty() {
            if !line.is_empty() {
                append_core(&logs, String::from_utf8_lossy(&line));
            }
            break;
        }
        let end = bytes.iter().position(|byte| *byte == b'\n');
        let used = end.map_or(bytes.len(), |end| end + 1);
        line.extend_from_slice(&bytes[..used.min(4096 - line.len())]);
        reader.consume(used);
        if end.is_some() {
            let text = String::from_utf8_lossy(&line);
            append_core(&logs, text.trim_end_matches(['\r', '\n']));
            line.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Language;

    #[test]
    fn known_client_logs_follow_language_and_preserve_external_details() {
        let chinese = "[客户端] 配置已导入/更新";
        assert_eq!(
            display(chinese, crate::config::Language::English),
            "[Client] Configuration imported or updated"
        );
        assert_eq!(
            display(chinese, crate::config::Language::SimplifiedChinese),
            chinese
        );
        assert_eq!(
            display(
                "[客户端] 代理恢复失败：provider 原文",
                crate::config::Language::English
            ),
            "[Client] Proxy recovery failed: provider 原文"
        );
        assert_eq!(
            display(
                "[客户端] 定时测速失败：下载响应不完整（HTTP 206）",
                Language::English
            ),
            "[Client] Scheduled speed test failed: Incomplete download response (HTTP 206)"
        );
        assert_eq!(
            display(
                "[客户端] 节点状态复测未完成，已保留网络设置：provider 原文",
                Language::English
            ),
            "[Client] Node state recheck did not finish; network settings retained: provider 原文"
        );
        for external in ["[内核] 配置已导入/更新", "[客户端] 未知事件：provider 原文"]
        {
            assert_eq!(
                display(external, crate::config::Language::English),
                external
            );
        }
    }

    #[test]
    fn protection_restore_log_preserves_node_name_verbatim() {
        for node in ["直连：香港", "订阅下载失败", "香港 · 東京  "] {
            let raw = format!("[客户端] 已恢复 Claude 保护；原状态恢复记录保留，使用节点 {node}");
            let expected = format!(
                "[Client] Claude protection restored; original settings retained; node {node}"
            );
            assert_eq!(display(&raw, Language::English), expected);
            assert_eq!(display(&raw, Language::SimplifiedChinese), raw);
        }
    }

    #[test]
    fn network_refresh_logs_keep_target_availability_caveat() {
        for (raw, expected) in [
            (
                "[客户端] 网络刷新完成，已刷新出站与 DNS 并清理旧连接；目标服务是否可用仍需检查",
                "[Client] Network refresh complete; core routes and DNS refreshed, old connections closed. Target service availability still needs to be checked.",
            ),
            (
                "[客户端] 手动网络刷新完成；目标服务是否可用仍需检查",
                "[Client] Manual network refresh complete. Target service availability still needs to be checked.",
            ),
        ] {
            assert_eq!(display(raw, Language::English), expected);
            assert_eq!(display(raw, Language::SimplifiedChinese), raw);
        }
    }

    #[test]
    fn controller_log_frames_preserve_levels_and_cannot_be_projected_as_client_logs() {
        let logs = LogBuffer::default();
        let mut frames = LogFrames::default();
        frames.push_core_events(br#"{"type":"warn","#, &logs);
        let batch = [
            r#""payload":"内核警告"}"#,
            r#"{"payload":"[客户端] 配置已导入/更新"}"#,
            r#"{"type":"vendor","payload":"第三方原文"}"#,
            r#"{"type":"客户端","payload":"配置已导入/更新"}"#,
            r#"{"type":"客户端] 操作失败：","payload":"provider 原文"}"#,
            "broken frame",
            "",
        ]
        .join("\n");
        frames.push_core_events(batch.as_bytes(), &logs);
        let buffer = logs.lock().unwrap();
        let expected = [
            "[warn] 内核警告",
            "[info] [客户端] 配置已导入/更新",
            "[vendor] 第三方原文",
            "[内核] [客户端] 配置已导入/更新",
            "[内核] [客户端] 操作失败：] provider 原文",
        ];
        assert_eq!(buffer.len(), expected.len());
        for (raw, expected) in buffer.iter().zip(expected) {
            assert_eq!(display(raw, Language::English), raw.as_ref());
            assert_eq!(raw.as_ref(), expected);
        }
    }

    #[test]
    fn log_frames_handle_fragmentation_batches_and_oversized_records() {
        let mut frames = LogFrames::default();
        let mut emitted = Vec::new();
        frames.push(b"first", |line| emitted.push(line.to_vec()));
        frames.push(b"\nsecond\nthird", |line| emitted.push(line.to_vec()));
        frames.push(b"\n", |line| emitted.push(line.to_vec()));
        assert_eq!(
            emitted,
            [
                b"first\n".to_vec(),
                b"second\n".to_vec(),
                b"third\n".to_vec()
            ]
        );
        emitted.clear();
        for _ in 0..100 {
            frames.push(&[b'x'; 2048], |line| emitted.push(line.to_vec()));
            assert!(frames.pending.len() <= LOG_FRAME_LIMIT);
        }
        frames.push(b"tail\nvalid\n", |line| emitted.push(line.to_vec()));
        assert_eq!(emitted, [b"valid\n".to_vec()]);
        // A large network chunk may contain many small valid records.
        emitted.clear();
        frames.push(&b"ok\n".repeat(30_000), |line| emitted.push(line.to_vec()));
        assert_eq!(emitted.len(), 30_000);
    }
    #[tokio::test]
    async fn long_log_lines_are_bounded_and_next_line_survives() {
        use tokio::io::AsyncWriteExt;
        let (reader, mut writer) = tokio::io::duplex(64);
        let producer = tokio::spawn(async move {
            writer.write_all(&vec![b'x'; 100_000]).await.unwrap();
            writer
                .write_all("\n下一行\n[warn] 原始等级\n[客户端] 配置已导入/更新".as_bytes())
                .await
                .unwrap();
        });
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        read_core_logs(reader, logs.clone()).await;
        producer.await.unwrap();
        let buffer = logs.lock().unwrap();
        assert_eq!(buffer.len(), 4);
        assert_eq!(buffer[0].chars().count(), 1000);
        assert_eq!(buffer[1].as_ref(), "下一行");
        assert_eq!(buffer[2].as_ref(), "[warn] 原始等级");
        let external = "[内核] [客户端] 配置已导入/更新";
        assert_eq!(buffer[3].as_ref(), external);
        assert_eq!(display(&buffer[3], Language::English), external);
    }
    #[test]
    fn log_snapshots_share_text_and_history_is_bounded() {
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        for i in 0..LOG_LIMIT + 10 {
            append(&logs, format!("entry {i}"));
        }
        let buffer = logs.lock().unwrap();
        assert_eq!(buffer.len(), LOG_LIMIT);
        assert_eq!(buffer[0].as_ref(), "entry 10");
        let snapshot: Vec<_> = buffer.iter().cloned().collect();
        assert!(Arc::ptr_eq(&buffer[0], &snapshot[0]));
    }
}
