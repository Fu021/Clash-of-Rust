mod text;
mod yaml;

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
    Find {
        query: String,
        from: usize,
        #[serde(default)]
        previous: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct Matches {
    pub current: usize,
    pub total: usize,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Request {
    pub profile: String,
    pub view: View,
    #[serde(default)]
    pub show_credentials: bool,
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
    pub matches: Option<Matches>,
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
    let indented = if matches!(request.view, View::Runtime) {
        yaml::indent_sequences(raw, &cancelled)?
    } else {
        None
    };
    let raw = indented
        .as_ref()
        .map(SourceText::as_str)
        .transpose()?
        .unwrap_or(raw);
    let redacted = if request.show_credentials || matches!(request.view, View::OriginalRevealed) {
        yaml::redact_controller(raw, controller_secret, &cancelled)?
    } else {
        yaml::redact(raw, controller_secret, &cancelled)?
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
    let (start, end, match_start, matches) = bounds(text, &request.position, &cancelled)?;
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
        match_missing: matches.is_some_and(|result| result.total == 0),
        matches,
    })
}

pub fn validate_query(query: &str) -> Result<()> {
    ensure!(
        !query.is_empty() && query.len() <= SEARCH_BYTES && !query.contains(['\r', '\n']),
        "请输入不超过 512 字节的单行搜索词"
    );
    Ok(())
}

pub fn find_match(
    text: &str,
    query: &str,
    from: usize,
    previous: bool,
    cancelled: impl Fn() -> bool,
) -> Result<(Option<(usize, usize)>, Matches)> {
    validate_query(query)?;
    let mut summary = Matches::default();
    let mut first = None;
    let mut last = None;
    let mut nearest = None;
    for (start, matched) in text.match_indices(query) {
        if summary.total.is_multiple_of(4096) {
            ensure!(!cancelled(), "源码请求已取消");
        }
        summary.total += 1;
        let result = ((start, start + matched.len()), summary.total);
        first = first.or(Some(result));
        last = Some(result);
        if previous && start < from || !previous && start >= from && nearest.is_none() {
            nearest = Some(result);
        }
    }
    ensure!(!cancelled(), "源码请求已取消");
    let selected = nearest.or(if previous { last } else { first });
    if let Some((range, current)) = selected {
        summary.current = current;
        Ok((Some(range), summary))
    } else {
        Ok((None, summary))
    }
}

type Bounds = (usize, usize, Option<(usize, usize)>, Option<Matches>);

fn bounds(text: &str, position: &Position, cancelled: impl Fn() -> bool) -> Result<Bounds> {
    let offset = |value| {
        ensure!(text.is_char_boundary(value), "源码位置无效");
        Ok::<_, anyhow::Error>(value)
    };
    let (start, selected, matches) = match position {
        Position::From(start) => (offset(*start)?, None, None),
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
            return Ok((start, end, None, None));
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
            (start, None, None)
        }
        Position::Find {
            query,
            from,
            previous,
        } => {
            let from = offset(*from)?;
            let (selected, matches) = find_match(text, query, from, *previous, cancelled)?;
            match selected {
                Some((first, last)) => {
                    let minimum = text.ceil_char_boundary(first.saturating_sub(PAGE_BYTES / 2));
                    let start = text[minimum..first]
                        .rmatch_indices('\n')
                        .take(7)
                        .last()
                        .map_or(minimum, |(index, _)| minimum + index + 1);
                    (start, Some((first, last)), Some(matches))
                }
                None => return Ok((from, from, None, Some(matches))),
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
    Ok((start, end, selected, matches))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(view: View, position: Position) -> Request {
        Request {
            profile: "fixture".into(),
            view,
            show_credentials: false,
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
        for view in [View::OriginalRevealed, View::Runtime] {
            let mut revealed_request = request(view, Position::From(0));
            revealed_request.show_credentials = matches!(view, View::Runtime);
            let revealed = build_page(
                raw,
                "control-generated-secret",
                &revealed_request,
                matches!(view, View::Runtime),
                || false,
            )
            .unwrap();
            assert_eq!(revealed.text, raw.replace("control-generated-secret", MASK));
        }
    }

    #[test]
    fn runtime_preview_indents_yaml_sequences_without_changing_the_values() {
        let raw = "\
proxies:
- name: 中文🙂
  password: example-password
  description: |-
    - this dash is part of the value
    second line
proxy-groups:
- name: Group
  proxies:
  - 中文🙂
  - DIRECT
  already-indented:
    - nested
rules: &entries
- MATCH,DIRECT
alias: *entries
flow: [one, two]";
        let mut runtime_request = request(View::Runtime, Position::From(0));
        runtime_request.show_credentials = true;
        for applied in [false, true] {
            let page = build_page(raw, "", &runtime_request, applied, || false).unwrap();
            assert_eq!(page.applied, applied);
            assert!(page.text.contains("proxies:\n    - name: 中文🙂"));
            assert!(
                page.text
                    .contains("      proxies:\n          - 中文🙂\n          - DIRECT\n")
            );
            assert!(
                page.text
                    .contains("      already-indented:\n        - nested\n")
            );
            assert!(
                page.text
                    .contains("rules: &entries\n    - MATCH,DIRECT\nalias: *entries\n")
            );
            assert_eq!(
                yaml_serde::from_str::<yaml_serde::Value>(&page.text).unwrap(),
                yaml_serde::from_str::<yaml_serde::Value>(raw).unwrap()
            );
        }
        let original = build_page(
            raw,
            "",
            &request(View::OriginalRevealed, Position::From(0)),
            false,
            || false,
        )
        .unwrap();
        assert_eq!(original.text, raw);
        runtime_request.show_credentials = false;
        runtime_request.position = Position::Find {
            query: "          - DIRECT".into(),
            from: 0,
            previous: false,
        };
        let page = build_page(raw, "example-password", &runtime_request, true, || false).unwrap();
        let (start, end) = page.selected.unwrap();
        assert_eq!(&page.text[start..end], "          - DIRECT");
        assert!(!page.text.contains("example-password"));
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
                previous: false,
            },
        );
        search.revision = Some(first.revision);
        let found = build_page(&raw, "", &search, false, || false).unwrap();
        let (start, end) = found.selected.unwrap();
        assert_eq!(&found.text[start..end], "rules");
        assert_eq!(
            found.matches,
            Some(Matches {
                current: 1,
                total: 1
            })
        );
        assert!(build_page("proxies: []", "", &search, false, || false).is_err());
        assert!(build_page(&raw, "", &search, false, || true).is_err());
    }

    #[test]
    fn find_counts_utf8_matches_and_wraps_in_both_directions() {
        let text = "中文🙂\n中文🙂\n中文🙂";
        for (from, previous, current) in [
            (0, false, 1),
            (10, false, 2),
            (11, true, 1),
            (0, true, 3),
            (32, false, 1),
        ] {
            let (selected, count) = find_match(text, "中文🙂", from, previous, || false).unwrap();
            assert_eq!(count, Matches { current, total: 3 });
            assert_eq!(
                selected,
                Some(((current - 1) * 11, (current - 1) * 11 + 10))
            );
        }
        assert_eq!(
            find_match(text, "不存在", 0, false, || false).unwrap(),
            (None, Matches::default())
        );
        assert!(find_match(text, "\n", 0, false, || false).is_err());
        assert!(find_match(text, "中文", 0, false, || true).is_err());
    }

    #[test]
    fn source_find_counts_the_full_document_without_exposing_masked_matches() {
        let text = format!(
            "name: fixture\n{}name: fixture\npassword: fixture\n",
            "# padding\n".repeat(PAGE_LINES + 5)
        );
        let find = |query: &str, from, previous| Position::Find {
            query: query.into(),
            from,
            previous,
        };
        let page = |position| {
            build_page(&text, "", &request(View::Original, position), false, || {
                false
            })
            .unwrap()
        };
        let last = page(find("fixture", 0, true));
        assert_eq!(
            last.matches,
            Some(Matches {
                current: 2,
                total: 2
            })
        );
        assert!(last.first_line >= PAGE_LINES);
        let (start, end) = last.selected.unwrap();
        assert_eq!(&last.text[start..end], "fixture");
        let wrapped = page(find("fixture", last.start + end, false));
        assert_eq!(wrapped.first_line, 1);
        assert_eq!(
            wrapped.matches,
            Some(Matches {
                current: 1,
                total: 2
            })
        );
        let miss = page(find("password: fixture", 0, false));
        assert_eq!(miss.matches, Some(Matches::default()));
        assert!(miss.text.is_empty());
    }

    #[test]
    fn source_visibility_uses_a_new_revision_and_old_requests_remain_masked() {
        let mut request: Request = serde_json::from_value(serde_json::json!({
            "profile": "fixture",
            "view": "runtime",
            "position": {"find": {"query": "fictional", "from": 0}},
            "revision": null
        }))
        .unwrap();
        let raw = "password: fictional\nsecret: control-generated-secret\n";
        let build = |request: &Request| {
            build_page(raw, "control-generated-secret", request, true, || false)
        };
        assert!(!request.show_credentials);
        let hidden = build(&request).unwrap();
        assert!(hidden.match_missing);
        request.show_credentials = true;
        request.revision = Some(hidden.revision);
        assert!(build(&request).is_err());
        request.revision = None;
        let shown = build(&request).unwrap();
        assert!(!shown.match_missing);
        assert!(!shown.text.contains("control-generated-secret"));
        assert!(shown.applied);
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
