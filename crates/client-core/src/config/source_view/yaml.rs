use super::{MASK, SourceText};
use crate::config::YAML_INDENT;
use anyhow::{Result, ensure};
use libyaml_rs as yaml;
use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, c_char},
    marker::PhantomData,
    mem::MaybeUninit,
    ops::Range,
    slice,
};

struct Parser<'text> {
    inner: Box<yaml::yaml_parser_t>,
    input: PhantomData<&'text str>,
}

impl<'text> Parser<'text> {
    fn new(text: &'text str) -> Result<Self> {
        let mut parser = Box::new_uninit();
        // SAFETY: parser is stable, aligned output storage. Only success
        // initializes it, and this Parser cannot outlive the input slice
        // retained by libyaml until parser deletion.
        unsafe {
            ensure!(
                yaml::yaml_parser_initialize(parser.as_mut_ptr()).ok,
                "无法初始化 YAML 源码查看"
            );
            let mut inner = parser.assume_init();
            yaml::yaml_parser_set_encoding(&mut *inner, yaml::YAML_UTF8_ENCODING);
            yaml::yaml_parser_set_input_string(&mut *inner, text.as_ptr(), text.len() as u64);
            Ok(Self {
                inner,
                input: PhantomData,
            })
        }
    }

    fn next(&mut self) -> Result<Event> {
        let mut event = MaybeUninit::uninit();
        // SAFETY: the parser is initialized and exclusively borrowed. libyaml
        // initializes an owned event only on success.
        unsafe {
            ensure!(
                yaml::yaml_parser_parse(&mut *self.inner, event.as_mut_ptr()).ok,
                "无法安全遮盖此 YAML 源码"
            );
            Ok(Event(event.assume_init()))
        }
    }
}

impl Drop for Parser<'_> {
    fn drop(&mut self) {
        // SAFETY: this is the unique owner of an initialized parser.
        unsafe { yaml::yaml_parser_delete(&mut *self.inner) }
    }
}

struct Event(yaml::yaml_event_t);

impl Event {
    fn span(&self) -> Range<usize> {
        self.0.start_mark.index as usize..self.0.end_mark.index as usize
    }

    fn scalar(&self) -> Option<&str> {
        if self.0.type_ != yaml::YAML_SCALAR_EVENT {
            return None;
        }
        // SAFETY: a scalar event initializes the scalar field and owns its
        // UTF-8 buffer of the recorded length for the lifetime of this borrow.
        let value = unsafe {
            let scalar = self.0.data.scalar;
            slice::from_raw_parts(scalar.value, scalar.length as usize)
        };
        std::str::from_utf8(value).ok()
    }

    fn anchor(&self) -> Option<&[u8]> {
        // SAFETY: the event tag selects the initialized union field. The
        // non-null pointer refers to an event-owned NUL-terminated name.
        unsafe {
            let anchor = match self.0.type_ {
                yaml::YAML_SCALAR_EVENT => self.0.data.scalar.anchor,
                yaml::YAML_ALIAS_EVENT => self.0.data.alias.anchor,
                yaml::YAML_SEQUENCE_START_EVENT => self.0.data.sequence_start.anchor,
                yaml::YAML_MAPPING_START_EVENT => self.0.data.mapping_start.anchor,
                _ => return None,
            };
            (!anchor.is_null()).then(|| CStr::from_ptr(anchor.cast::<c_char>()).to_bytes())
        }
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: libyaml transferred this initialized event to its unique owner.
        unsafe { yaml::yaml_event_delete(&mut self.0) }
    }
}

struct Container {
    next_key: bool,
    mapping: bool,
    hide: bool,
    hide_next: bool,
    anchor: Option<Vec<u8>>,
    start: usize,
}

pub fn indent_sequences(raw: &str, cancelled: &dyn Fn() -> bool) -> Result<Option<SourceText>> {
    if !raw.lines().any(|line| {
        line.trim_start_matches(' ')
            .strip_prefix('-')
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
    }) {
        return Ok(None);
    }
    let mut parser = Parser::new(raw)?;
    let mut containers = Vec::new();
    let mut indentation = 0;
    let mut padding = Vec::new();
    let mut remaining = super::super::RUNTIME_LIMIT - raw.len();
    let mut output = None;
    let mut copied = 0;
    loop {
        ensure!(!cancelled(), "源码请求已取消");
        let Ok(event) = parser.next() else {
            return Ok(None);
        };
        let span = event.span();
        let previous_indent = indentation;
        let boundary = match event.0.type_ {
            yaml::YAML_STREAM_END_EVENT => break,
            yaml::YAML_MAPPING_START_EVENT => {
                containers.push(false);
                continue;
            }
            yaml::YAML_SEQUENCE_START_EVENT => {
                let indentless =
                    !span.is_empty() && raw.as_bytes().get(span.end - 1) == Some(&b'-');
                containers.push(indentless);
                if !indentless {
                    continue;
                }
                indentation += YAML_INDENT.len();
                span.end - 1
            }
            yaml::YAML_MAPPING_END_EVENT | yaml::YAML_SEQUENCE_END_EVENT => {
                if !containers.pop().expect("matched YAML container") {
                    continue;
                }
                indentation -= YAML_INDENT.len();
                span.start
            }
            _ => continue,
        };
        let boundary = if boundary == raw.len() {
            boundary
        } else {
            raw[..boundary].rfind('\n').map_or(0, |newline| newline + 1)
        };
        let output = match &mut output {
            Some(output) => output,
            slot @ None => slot.insert(SourceText::new(raw.len())?),
        };
        let segment = &raw[copied..boundary];
        if previous_indent == 0 {
            output.append(segment.as_bytes())?;
        } else {
            if previous_indent > padding.len() {
                padding.resize(previous_indent, b' ');
            }
            for line in segment.split_inclusive('\n') {
                ensure!(!cancelled(), "源码请求已取消");
                if !line.starts_with(['\r', '\n']) {
                    let Some(space) = remaining.checked_sub(previous_indent) else {
                        return Ok(None);
                    };
                    remaining = space;
                    output.append(&padding[..previous_indent])?;
                }
                output.append(line.as_bytes())?;
            }
        }
        copied = boundary;
    }
    if let Some(output) = &mut output {
        output.append(&raw.as_bytes()[copied..])?;
    }
    Ok(output)
}

#[derive(Default)]
struct Anchor {
    spans: Vec<Range<usize>>,
    sensitive_key: bool,
}

fn sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace('-', "_");
    matches!(
        normalized.as_str(),
        "url" | "source" | "user" | "username" | "cookie" | "header" | "headers" | "authentication"
    ) || crate::support::sensitive_field(&normalized)
}

pub fn redact(raw: &str, secret: &str, cancelled: &dyn Fn() -> bool) -> Result<Option<SourceText>> {
    let mut parser = Parser::new(raw)?;
    let mut stack: Vec<Container> = Vec::new();
    let mut spans = Vec::new();
    let mut anchors = HashMap::<Vec<u8>, Anchor>::new();
    let mut aliases = Vec::<(usize, Vec<u8>)>::new();
    loop {
        ensure!(!cancelled(), "源码请求已取消");
        let event = parser.next()?;
        let kind = event.0.type_;
        let span = event.span();
        match kind {
            yaml::YAML_STREAM_END_EVENT => break,
            yaml::YAML_MAPPING_END_EVENT | yaml::YAML_SEQUENCE_END_EVENT => {
                let container = stack.pop().expect("matched YAML container");
                let span = container.start..span.end;
                if let Some(anchor) = container.anchor {
                    anchors.entry(anchor).or_default().spans.push(span.clone());
                }
                if container.hide {
                    spans.push(span);
                }
                continue;
            }
            yaml::YAML_SCALAR_EVENT
            | yaml::YAML_ALIAS_EVENT
            | yaml::YAML_MAPPING_START_EVENT
            | yaml::YAML_SEQUENCE_START_EVENT => {}
            _ => continue,
        }
        let anchor = event.anchor();
        let is_alias = kind == yaml::YAML_ALIAS_EVENT;
        let is_container = matches!(
            kind,
            yaml::YAML_MAPPING_START_EVENT | yaml::YAML_SEQUENCE_START_EVENT
        );
        let is_key = stack
            .last()
            .is_some_and(|parent| parent.mapping && parent.next_key);
        let key_is_sensitive =
            (is_key || anchor.is_some()) && event.scalar().is_some_and(sensitive_key);
        let mut hide = false;
        if let Some(parent) = stack.last_mut()
            && parent.mapping
        {
            if parent.next_key {
                ensure!(!is_container, "无法安全遮盖含复杂键的 YAML 源码");
                parent.hide_next = key_is_sensitive
                    || (is_alias
                        && anchor
                            .and_then(|name| anchors.get(name))
                            .is_none_or(|value| value.sensitive_key));
            } else {
                hide = parent.hide_next;
            }
            parent.next_key = !parent.next_key;
        }
        if is_container {
            let mapping = kind == yaml::YAML_MAPPING_START_EVENT;
            stack.push(Container {
                next_key: mapping,
                mapping,
                hide,
                hide_next: false,
                anchor: anchor.map(<[u8]>::to_vec),
                start: span.start,
            });
            continue;
        }
        if let Some(anchor) = anchor {
            if is_alias {
                aliases.push((span.start, anchor.to_vec()));
            } else {
                let anchor = anchors.entry(anchor.to_vec()).or_default();
                anchor.sensitive_key |= key_is_sensitive;
                anchor.spans.push(span.clone());
            }
        }
        if hide {
            spans.push(span);
        }
    }
    aliases.sort_unstable_by_key(|(start, _)| *start);
    let mut checked = HashSet::new();
    let mut cursor = 0;
    while cursor < spans.len() {
        ensure!(!cancelled(), "源码请求已取消");
        let span = spans[cursor].clone();
        let first = aliases.partition_point(|(position, _)| *position < span.start);
        for (position, name) in &aliases[first..] {
            if *position >= span.end {
                break;
            }
            if checked.insert(name)
                && let Some(anchor) = anchors.get(name)
            {
                spans.extend(anchor.spans.iter().cloned());
            }
        }
        cursor += 1;
    }
    if !secret.is_empty() {
        spans.extend(
            raw.match_indices(secret)
                .map(|(index, _)| index..index + secret.len()),
        );
    }
    if spans.is_empty() {
        return Ok(None);
    }
    spans.sort_unstable_by_key(|span| (span.start, span.end));
    let mut result = SourceText::new(raw.len())?;
    let mut copied = 0;
    let mut spans = spans.into_iter().peekable();
    while let Some(mut span) = spans.next() {
        ensure!(!cancelled(), "源码请求已取消");
        while spans.peek().is_some_and(|next| next.start < span.end) {
            span.end = span.end.max(spans.next().unwrap().end);
        }
        ensure!(
            raw.is_char_boundary(span.start) && raw.is_char_boundary(span.end),
            "无法安全遮盖此 YAML 源码"
        );
        if span.is_empty() {
            continue;
        }
        result.append(&raw.as_bytes()[copied..span.start])?;
        result.append(MASK.as_bytes())?;
        let replaced = &raw[span.clone()];
        for byte in replaced
            .bytes()
            .filter(|byte| matches!(byte, b'\r' | b'\n'))
        {
            result.append(&[byte])?;
        }
        if let Some((_, indent)) = replaced.rsplit_once('\n')
            && indent.bytes().all(|byte| matches!(byte, b' ' | b'\t'))
        {
            result.append(indent.as_bytes())?;
        }
        copied = span.end;
    }
    result.append(&raw.as_bytes()[copied..])?;
    Ok(Some(result))
}

pub fn redact_controller(
    raw: &str,
    secret: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<SourceText>> {
    if secret.is_empty() {
        return Ok(None);
    }
    let mut hits = raw.match_indices(secret).peekable();
    if hits.peek().is_none() {
        return Ok(None);
    }
    let mut result = SourceText::new(raw.len())?;
    let mut copied = 0;
    for (start, _) in hits {
        ensure!(!cancelled(), "源码请求已取消");
        result.append(&raw.as_bytes()[copied..start])?;
        result.append(MASK.as_bytes())?;
        copied = start + secret.len();
    }
    result.append(&raw.as_bytes()[copied..])?;
    Ok(Some(result))
}
