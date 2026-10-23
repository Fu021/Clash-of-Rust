//! Typed, renderer-independent desktop state and bounded private-pipe messages.
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    io::{self, Read, Write},
    rc::Rc,
};
pub mod models;

/// Cheap snapshots share text inside a process; the pipe carries owned UTF-8.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Text(std::sync::Arc<str>);
impl Text {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl std::ops::Deref for Text {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
impl std::fmt::Display for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl From<String> for Text {
    fn from(value: String) -> Self {
        Self(value.into())
    }
}
impl From<&str> for Text {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}
impl PartialEq<&str> for Text {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
impl PartialEq<str> for Text {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct List<T>(Rc<RefCell<Vec<T>>>);
impl<T> Default for List<T> {
    fn default() -> Self {
        Self(Rc::default())
    }
}
impl<T> From<Vec<T>> for List<T> {
    fn from(rows: Vec<T>) -> Self {
        Self(Rc::new(RefCell::new(rows)))
    }
}
impl<T: Clone> List<T> {
    pub fn row_count(&self) -> usize {
        self.0.borrow().len()
    }
    pub fn row_data(&self, index: usize) -> Option<T> {
        self.0.borrow().get(index).cloned()
    }
    pub fn rows(&self) -> Vec<T> {
        self.0.borrow().clone()
    }
    pub fn with_rows<R>(&self, operation: impl FnOnce(&[T]) -> R) -> R {
        operation(&self.0.borrow())
    }
    pub fn set_vec(&self, rows: Vec<T>) {
        *self.0.borrow_mut() = rows;
    }
}
include!(concat!(env!("OUT_DIR"), "/contract.rs"));

pub const PROTOCOL: u32 = 3;
pub const LIMIT: usize = 8 * 1024 * 1024;
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    pub page: i32,
    pub scroll: [f32; 13],
    pub accent: Option<i32>,
    pub position: Option<(i32, i32)>,
    pub maximized: bool,
    pub fullscreen: bool,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub size: Size,
    pub available: Option<Size>,
    pub maximized: bool,
    pub minimized: bool,
    pub system_dark: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WindowCommand {
    Show,
    Destroy,
    Minimize(bool),
    Maximize(bool),
    Focus,
    Resize(Size),
    MinSize(Option<Size>),
    FocusInput(String),
    PickFile(u64),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostPacket {
    Open {
        protocol: u32,
        version: String,
        id: u64,
        size: Size,
        visible: bool,
        view: Option<ViewState>,
        frame: Frame,
        view_ready: bool,
    },
    Frame {
        data: Frame,
        view_ready: bool,
        accepted: u64,
    },
    Command(WindowCommand),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiEvent {
    Opened {
        protocol: u32,
        version: String,
        metrics: Metrics,
        preserve_native_state: bool,
    },
    Intent {
        revision: u64,
        intent: Intent,
        a: String,
        b: String,
        view: ViewState,
    },
    Geometry {
        metrics: Metrics,
        view: ViewState,
    },
    Rescaled,
    Focused,
    Close(ViewState),
    FilePicked {
        id: u64,
        result: Result<Option<std::path::PathBuf>, String>,
    },
    Error(String),
}
pub fn encode<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    let bytes = bincode::serde::encode_to_vec(value, bincode::config::standard())
        .map_err(io::Error::other)?;
    if bytes.len() > LIMIT {
        return Err(io::Error::other("Desktop pipe frame exceeds limit"));
    }
    Ok(bytes)
}
pub fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> io::Result<T> {
    if bytes.len() > LIMIT {
        return Err(io::Error::other("Desktop pipe frame exceeds limit"));
    }
    let (value, consumed) =
        bincode::serde::decode_from_slice(bytes, bincode::config::standard().with_limit::<LIMIT>())
            .map_err(io::Error::other)?;
    if consumed != bytes.len() {
        return Err(io::Error::other("Unexpected trailing desktop pipe data"));
    }
    Ok(value)
}
pub fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > LIMIT {
        return Err(io::Error::other("Desktop pipe frame exceeds limit"));
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()
}
pub fn read_frame(reader: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0; 4];
    loop {
        match reader.read(&mut length[..1]) {
            Ok(0) => return Ok(None),
            Ok(1) => break,
            Ok(_) => unreachable!(),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    reader.read_exact(&mut length[1..])?;
    let length = u32::from_le_bytes(length) as usize;
    if length > LIMIT {
        return Err(io::Error::other("Desktop pipe frame exceeds limit"));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framed_pipe_preserves_unicode_and_message_boundaries_and_detects_truncation() {
        let event = UiEvent::Intent {
            revision: 37,
            intent: Intent::ProfileName,
            a: "德国节点\n\"draft\"".into(),
            b: String::new(),
            view: ViewState::default(),
        };
        let bytes = encode(&event).unwrap();
        let mut stream = Vec::new();
        write_frame(&mut stream, &bytes).unwrap();
        write_frame(&mut stream, b"{}").unwrap();
        let mut input = stream.as_slice();
        let UiEvent::Intent { revision, a, .. } =
            decode(&read_frame(&mut input).unwrap().unwrap()).unwrap()
        else {
            panic!("wrong packet")
        };
        assert_eq!(revision, 37);
        assert_eq!(a, "德国节点\n\"draft\"");
        assert_eq!(read_frame(&mut input).unwrap(), Some(b"{}".to_vec()));
        assert_eq!(read_frame(&mut input).unwrap(), None);
        for mut truncated in [&stream[..2], &stream[..bytes.len() + 3]] {
            assert_eq!(
                read_frame(&mut truncated).unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
        }
        let length = ((LIMIT + 1) as u32).to_le_bytes();
        assert!(read_frame(&mut length.as_slice()).is_err());
        assert!(write_frame(&mut Vec::new(), &vec![0; LIMIT + 1]).is_err());
    }
    #[test]
    fn production_contract_round_trip_carries_lists_flags_and_drafts() {
        let data = UiData::default();
        let models = models::ViewModels::default();
        models.attach(&data);
        data.set_edit_name("未保存订阅".into());
        data.set_rule_payload("google.com".into());
        models.logs.set_vec(vec![LogItem {
            message: "完整日志".into(),
            ..Default::default()
        }]);
        data.set_site_exit_flag("DE".into());
        let packet = HostPacket::Frame {
            data: data.snapshot(),
            view_ready: true,
            accepted: 8,
        };
        let HostPacket::Frame {
            data: decoded,
            accepted,
            ..
        } = decode(&encode(&packet).unwrap()).unwrap()
        else {
            panic!("wrong packet")
        };
        assert_eq!(accepted, 8);
        assert_eq!(decoded.edit_name, "未保存订阅");
        assert_eq!(decoded.rule_payload, "google.com");
        assert_eq!(decoded.site_exit_flag, "DE");
        assert_eq!(decoded.logs.row_data(0).unwrap().message, "完整日志");
    }
    #[test]
    fn binary_codec_rejects_truncation_trailing_data_and_forged_allocations() {
        let packet = encode(&UiEvent::Focused).unwrap();
        let mut trailing = packet.clone();
        trailing.push(0);
        assert!(decode::<UiEvent>(&trailing).is_err());
        for length in 0..packet.len() {
            assert!(decode::<UiEvent>(&packet[..length]).is_err());
        }
        // A declared list length cannot request unbounded allocations even
        // when the framed input itself fits in the pipe's byte limit.
        let huge = bincode::serde::encode_to_vec(u64::MAX, bincode::config::standard()).unwrap();
        assert!(decode::<Vec<u64>>(&huge).is_err());
        assert!(decode::<Vec<u8>>(&vec![0; LIMIT + 1]).is_err());
        assert!(encode(&"x".repeat(LIMIT + 1)).is_err());
    }
}
