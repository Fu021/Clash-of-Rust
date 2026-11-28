mod mask;
mod text;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};
pub use text::SourceText;

const PAGE_BYTES: usize = 48 * 1024;
const PAGE_LINES: usize = 512;
const SEARCH_BYTES: usize = 512;
const MASK: &str = "\"[redacted]\"";

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum View {
    Original,
    OriginalRevealed,
    Runtime,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Position {
    From(usize),
    Before(usize),
    Line(usize),
    Find { query: String, from: usize },
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Request {
    pub profile: String,
    pub view: View,
    pub position: Position,
    pub revision: Option<String>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Request")
            .field("profile", &self.profile)
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize, Serialize)]
pub struct Page {
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub total: usize,
    pub first_line: usize,
    pub revision: String,
    pub applied: bool,
    pub selected: Option<(usize, usize)>,
    pub match_missing: bool,
}

pub struct Input {
    file: File,
    limit: usize,
    length: usize,
}

impl Input {
    pub fn open(path: &Path, applied: bool) -> Result<Self> {
        let file = File::open(path).context("无法打开已保存的 YAML")?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "已保存的 YAML 不是普通文件");
        let limit = if applied {
            super::RUNTIME_LIMIT
        } else {
            super::PROFILE_LIMIT
        };
        ensure!(metadata.len() <= limit as u64, "已保存的 YAML 超过大小限制");
        Ok(Self {
            file,
            limit,
            length: metadata.len() as usize,
        })
    }

    pub fn is_large(&self) -> bool {
        self.length >= SourceText::MAPPING_THRESHOLD
    }

    pub fn read(self, cancelled: impl Fn() -> bool) -> Result<SourceText> {
        ensure!(!cancelled(), "源码请求已取消");
        let mut reader = self.file.take(self.limit as u64 + 1);
        let mut chunk = [0; 32 * 1024];
        let mut bytes = SourceText::new(self.length)?;
        loop {
            ensure!(!cancelled(), "源码请求已取消");
            let length = reader.read(&mut chunk)?;
            if length == 0 {
                break;
            }
            bytes.append(&chunk[..length])?;
        }
        ensure!(bytes.len() <= self.limit, "已保存的 YAML 超过大小限制");
        Ok(bytes)
    }
}

pub fn build_page(
    raw: &str,
    controller_secret: &str,
    request: &Request,
    applied: bool,
    cancelled: impl Fn() -> bool,
) -> Result<Page> {
    ensure!(!cancelled(), "源码请求已取消");
    ensure!(raw.len() <= super::RUNTIME_LIMIT, "运行 YAML 超过大小限制");
    let redacted = if matches!(request.view, View::OriginalRevealed) {
        mask::redact_controller(raw, controller_secret, &cancelled)?
    } else {
        mask::redact(raw, controller_secret, &cancelled)?
    };
    let text = redacted
        .as_ref()
        .map(SourceText::as_str)
        .transpose()?
        .unwrap_or(raw);
    ensure!(!cancelled(), "源码请求已取消");
    let revision = crate::file_hash::hex(Sha256::digest(text.as_bytes()));
    ensure!(
        request
            .revision
            .as_ref()
            .is_none_or(|saved| saved == &revision),
        "源码已变化，请重新打开"
    );
    let (start, end, match_start, match_missing) = bounds(text, &request.position)?;
    let selected = match_start.map(|(first, last)| (first - start, last - start));
    let first_line = text[..start].bytes().filter(|byte| *byte == b'\n').count() + 1;
    Ok(Page {
        text: text[start..end].to_owned(),
        start,
        end,
        total: text.len(),
        first_line,
        revision,
        applied,
        selected,
        match_missing,
    })
}

type Bounds = (usize, usize, Option<(usize, usize)>, bool);

fn bounds(text: &str, position: &Position) -> Result<Bounds> {
    let offset = |value| {
        ensure!(text.is_char_boundary(value), "源码位置无效");
        Ok::<_, anyhow::Error>(value)
    };
    let (start, selected, missing) = match position {
        Position::From(start) => (offset(*start)?, None, false),
        Position::Before(end) => {
            let end = offset(*end)?;
            let minimum = text.ceil_char_boundary(end.saturating_sub(PAGE_BYTES));
            let start = text[minimum..end]
                .rmatch_indices('\n')
                .filter(|(index, _)| minimum + index + 1 != end)
                .nth(PAGE_LINES - 1)
                .or_else(|| {
                    (minimum > 0)
                        .then(|| text[minimum..end].match_indices('\n').next())
                        .flatten()
                        .filter(|(index, _)| minimum + index + 1 != end)
                })
                .map_or(minimum, |(index, _)| minimum + index + 1);
            return Ok((start, end, None, false));
        }
        Position::Line(line) => {
            ensure!(*line > 0, "行号必须大于零");
            let start = if *line == 1 {
                0
            } else {
                text.match_indices('\n')
                    .nth(line - 2)
                    .map(|(index, _)| index + 1)
                    .context("行号超出源码范围")?
            };
            (start, None, false)
        }
        Position::Find { query, from } => {
            ensure!(
                !query.is_empty() && query.len() <= SEARCH_BYTES && !query.contains(['\r', '\n']),
                "请输入不超过 512 字节的单行搜索词"
            );
            let from = offset(*from)?;
            match text[from..]
                .find(query)
                .map(|index| from + index)
                .or_else(|| text.find(query))
            {
                Some(first) => {
                    let minimum = text.ceil_char_boundary(first.saturating_sub(PAGE_BYTES / 2));
                    let start = text[minimum..first]
                        .rfind('\n')
                        .map_or(minimum, |index| minimum + index + 1);
                    (start, Some((first, first + query.len())), false)
                }
                None => (from, None, true),
            }
        }
    };
    let maximum = text.floor_char_boundary((start + PAGE_BYTES).min(text.len()));
    let available = &text[start..maximum];
    let line_limit = available.match_indices('\n').nth(PAGE_LINES - 1);
    let end = if let Some((index, _)) = line_limit {
        start + index + 1
    } else if maximum < text.len() {
        available
            .rfind('\n')
            .map_or(maximum, |index| start + index + 1)
    } else {
        maximum
    };
    let end = end.max(selected.map_or(0, |(_, last)| last));
    Ok((start, end, selected, missing))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(view: View, position: Position) -> Request {
        Request {
            profile: "fixture".into(),
            view,
            position,
            revision: None,
        }
    }

    #[test]
    fn default_preserves_original_comments_and_hides_nested_and_aliased_credentials() {
        let raw = "\
# 中文🙂
anchors: &fixture
  embedded: &nested 'never expose'
unrelated: { name: safe, uuid: 'private-id' } # keep
password: *fixture
ordinary: {header: {Authorization: \"bearer xyz\"}, url: 'https://fixture.invalid/?key=abc'}
secret: >-
  multiline
  sensitive
rules: [MATCH,DIRECT]
# control-generated-secret
";
        let page = build_page(
            raw,
            "control-generated-secret",
            &request(View::Original, Position::From(0)),
            false,
            || false,
        )
        .unwrap();
        for hidden in [
            "never expose",
            "private-id",
            "bearer xyz",
            "key=abc",
            "multiline",
            "sensitive",
            "control-generated-secret",
        ] {
            assert!(!page.text.contains(hidden), "{hidden}");
        }
        for visible in ["# 中文🙂", "name: safe", "# keep", "rules: [MATCH,DIRECT]"] {
            assert!(page.text.contains(visible), "{visible}");
        }
        let original = build_page(
            raw,
            "control-generated-secret",
            &request(View::OriginalRevealed, Position::From(0)),
            false,
            || false,
        )
        .unwrap();
        assert_eq!(original.text, raw.replace("control-generated-secret", MASK));
    }

    #[test]
    fn pages_long_unicode_lines_and_selects_search_results_without_retaining_source() {
        let raw = format!(
            "# header\nproxies: []\n# {}\nrules: []\n",
            "中文🙂".repeat(PAGE_BYTES)
        );
        let first = build_page(
            &raw,
            "",
            &request(View::Original, Position::From(0)),
            false,
            || false,
        )
        .unwrap();
        let following = build_page(
            &raw,
            "",
            &request(View::Original, Position::From(first.end)),
            false,
            || false,
        )
        .unwrap();
        assert!(following.text.len() <= PAGE_BYTES);
        assert_eq!(following.first_line, 3);
        let mut search = request(
            View::Original,
            Position::Find {
                query: "rules".into(),
                from: following.end,
            },
        );
        search.revision = Some(first.revision);
        let found = build_page(&raw, "", &search, false, || false).unwrap();
        let (start, end) = found.selected.unwrap();
        assert_eq!(&found.text[start..end], "rules");
        assert!(build_page("proxies: []", "", &search, false, || false).is_err());
        assert!(build_page(&raw, "", &search, false, || true).is_err());
    }

    #[test]
    fn pages_near_limit_scalar_and_masks_following_credentials() {
        let long_value = "a".repeat(super::super::PROFILE_LIMIT - 256);
        let raw = format!("label: {long_value}\npassword: private-test-value\n");
        for view in [View::Original, View::Runtime] {
            let secret = "private-test-value";
            let first = build_page(
                &raw,
                secret,
                &request(view, Position::From(0)),
                true,
                || false,
            )
            .unwrap();
            assert_eq!(first.end, PAGE_BYTES);
            assert_eq!(first.text, raw[..PAGE_BYTES]);
            let mut last_request = request(view, Position::Line(2));
            last_request.revision = Some(first.revision);
            let last = build_page(&raw, secret, &last_request, true, || false).unwrap();
            assert_eq!(last.text, "password: \"[redacted]\"\n");
            assert_eq!(last.end, last.total);
        }
    }
}
