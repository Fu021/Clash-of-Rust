use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Key {
    Char(char),
    Paste(String),
    Up,
    Down,
    Enter,
    Escape,
    Back,
    Tab,
    Interrupt,
}

#[cfg(any(windows, test))]
#[derive(Default)]
pub(super) struct Utf16Decoder {
    high: Option<u16>,
}

#[cfg(any(windows, test))]
impl Utf16Decoder {
    pub fn feed(&mut self, unit: u16) -> Option<char> {
        if (0xd800..=0xdbff).contains(&unit) {
            self.high = Some(unit);
            return None;
        }
        if (0xdc00..=0xdfff).contains(&unit) {
            return self
                .high
                .take()
                .and_then(|high| char::decode_utf16([high, unit]).next())
                .and_then(Result::ok);
        }
        self.high = None;
        char::from_u32(u32::from(unit))
    }
}

#[derive(Default)]
pub(super) struct Decoder {
    bytes: Vec<u8>,
    escape_since: Option<Instant>,
    paste: Option<Vec<u8>>,
}

impl Decoder {
    pub fn feed(&mut self, bytes: &[u8], now: Instant) -> Vec<Key> {
        self.bytes.extend(bytes.iter().copied().take(4096));
        let mut keys = Vec::new();
        while !self.bytes.is_empty() {
            if let Some(paste) = &mut self.paste {
                let end = self
                    .bytes
                    .windows(6)
                    .position(|bytes| bytes == b"\x1b[201~");
                let count = end.unwrap_or(self.bytes.len().saturating_sub(5));
                paste.extend(self.bytes[..count].iter().copied().take(1024 - paste.len()));
                self.bytes.drain(..count);
                if end.is_some() {
                    self.bytes.drain(..6);
                    keys.push(Key::Paste(
                        String::from_utf8_lossy(self.paste.take().unwrap().as_slice()).into_owned(),
                    ));
                    self.escape_since = None;
                    continue;
                }
                break;
            }
            if self.bytes.starts_with(b"\x1b[200~") {
                self.bytes.drain(..6);
                self.paste = Some(Vec::new());
                continue;
            }
            let first = self.bytes[0];
            let (count, key) = match first {
                3 | 4 => (1, Some(Key::Interrupt)),
                9 => (1, Some(Key::Tab)),
                10 | 13 => (1, Some(Key::Enter)),
                8 | 127 => (1, Some(Key::Back)),
                27 => {
                    let since = *self.escape_since.get_or_insert(now);
                    if self.bytes.len() == 1 {
                        if now.duration_since(since) < Duration::from_millis(80) {
                            break;
                        }
                        (1, Some(Key::Escape))
                    } else if matches!(self.bytes[1], b'[' | b'O') {
                        let end = self
                            .bytes
                            .iter()
                            .enumerate()
                            .skip(2)
                            .find(|(_, byte)| (0x40..=0x7e).contains(*byte))
                            .map(|(index, _)| index);
                        match end {
                            Some(end) => (
                                end + 1,
                                match self.bytes[end] {
                                    b'A' => Some(Key::Up),
                                    b'B' => Some(Key::Down),
                                    _ => None,
                                },
                            ),
                            None if self.bytes.len() < 64
                                && now.duration_since(since) < Duration::from_millis(80) =>
                            {
                                break;
                            }
                            None => (self.bytes.len(), None),
                        }
                    } else {
                        (2, None)
                    }
                }
                _ if first < 32 => (1, None),
                _ => {
                    let size = match first {
                        0..=127 => 1,
                        0xc2..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf4 => 4,
                        _ => 1,
                    };
                    if self.bytes.len() < size {
                        break;
                    }
                    let character = std::str::from_utf8(&self.bytes[..size])
                        .ok()
                        .and_then(|text| text.chars().next());
                    (size, character.map(Key::Char))
                }
            };
            self.bytes.drain(..count);
            self.escape_since = None;
            if let Some(key) = key {
                keys.push(key);
            }
        }
        keys
    }
}
