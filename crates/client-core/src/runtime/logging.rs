//! Bounded logs and stream framing, reusable by desktop and mobile core hosts.
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

pub const LOG_LIMIT: usize = 500;
pub const LOG_FRAME_LIMIT: usize = 64 * 1024;
pub type LogBuffer = Arc<Mutex<VecDeque<Arc<str>>>>;

#[derive(Default)]
pub struct LogFrames {
    pending: Vec<u8>,
    discarding: bool,
}

impl LogFrames {
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

pub async fn read_core_logs(reader: impl AsyncRead + Unpin, logs: LogBuffer) {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::with_capacity(4096);
    while let Ok(bytes) = reader.fill_buf().await {
        if bytes.is_empty() {
            if !line.is_empty() {
                append(&logs, format!("[内核] {}", String::from_utf8_lossy(&line)));
            }
            break;
        }
        let end = bytes.iter().position(|byte| *byte == b'\n');
        let used = end.map_or(bytes.len(), |end| end + 1);
        line.extend_from_slice(&bytes[..used.min(4096 - line.len())]);
        reader.consume(used);
        if end.is_some() {
            let text = String::from_utf8_lossy(&line);
            append(
                &logs,
                format!("[内核] {}", text.trim_end_matches(['\r', '\n'])),
            );
            line.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            writer.write_all("\n下一行\n".as_bytes()).await.unwrap();
        });
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        read_core_logs(reader, logs.clone()).await;
        producer.await.unwrap();
        let buffer = logs.lock().unwrap();
        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer[0].chars().count(), 1000);
        assert_eq!(buffer[1].as_ref(), "[内核] 下一行");
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
