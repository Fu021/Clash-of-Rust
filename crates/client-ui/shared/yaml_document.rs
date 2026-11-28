use super::yaml_core::config::{
    Language,
    documents::{Action, Reply},
    overrides::{Change, EDIT_LIMIT},
    source_view::{Position, Request, View},
};
use anyhow::{Context, Result, ensure};
use slint::{ModelRc, SharedString, VecModel};

use super::{YamlDocument, YamlHighlight};

fn highlight(line: &str) -> YamlHighlight {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') && line.len() <= 512 {
        return YamlHighlight {
            prefix: line.into(),
            tone: 1,
        };
    }
    let key = trimmed.strip_prefix("- ").unwrap_or(trimmed);
    let Some((name, value)) = key.split_once(':') else {
        return YamlHighlight::default();
    };
    if name.is_empty()
        || line.len() - value.len() > 192
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || !value.is_empty() && !value.starts_with(char::is_whitespace)
    {
        return YamlHighlight::default();
    }
    YamlHighlight {
        prefix: line[..line.len() - value.len()].into(),
        tone: 2,
    }
}

pub fn load(document: &YamlDocument<'_>, kind: &str, value: &str) -> Result<Action> {
    let view = match document.get_mode() {
        1 => return Ok(Action::OpenOverride),
        2 => View::Original,
        3 => View::OriginalRevealed,
        4 => View::Runtime,
        _ => anyhow::bail!("YAML 视图已关闭"),
    };
    let offset = || value.parse::<usize>().context("请输入有效的行号或位置");
    let position = match kind {
        "from" => Position::From(offset()?),
        "before" => Position::Before(offset()?),
        "line" => Position::Line(offset()?),
        "find" => Position::Find {
            query: value.into(),
            from: if document.get_cursor() >= 0 {
                (document.get_start() + document.get_cursor()) as usize
            } else {
                document.get_start() as usize
            },
        },
        _ => anyhow::bail!("未知的源码位置"),
    };
    let revision = document.get_revision();
    Ok(Action::Source(Request {
        profile: document.get_profile().to_string(),
        view,
        position,
        revision: (!revision.is_empty()).then(|| revision.to_string()),
    }))
}

pub fn write(document: &YamlDocument<'_>, apply: bool) -> Result<Action> {
    ensure!(
        document.get_editable() && !document.get_writing() && !document.get_external_open(),
        "当前不可编辑"
    );
    let text = document.get_text();
    ensure!(text.len() <= EDIT_LIMIT, "全局覆写不能超过 128 KiB");
    let change = Change {
        text: text.to_string(),
        enabled: document.get_enabled(),
        revision: document.get_revision().to_string(),
        expected_enabled: document.get_saved_enabled(),
    };
    document.set_writing(true);
    document.set_error(Default::default());
    document.set_status(Default::default());
    Ok(if apply {
        Action::Apply(change)
    } else {
        Action::Check(change)
    })
}

fn set_text(document: &YamlDocument<'_>, text: String) {
    document.set_line_count(line_count(&text));
    if document.get_mode() >= 2 {
        let rows = text
            .split('\n')
            .map(|line| highlight(line.trim_end_matches('\r')))
            .collect::<Vec<_>>();
        document.set_highlights(ModelRc::new(VecModel::from(rows)));
    }
    document.set_text(SharedString::from(text));
    locate(document, 0);
}

fn line_count(text: &str) -> i32 {
    let breaks = text.bytes().filter(|byte| *byte == b'\n').count() as i32;
    breaks + 1
}

fn select(document: &YamlDocument<'_>, selection: Option<(usize, usize)>) {
    let (anchor, cursor) = selection.map_or((-1, -1), |(a, b)| (a as i32, b as i32));
    document.set_anchor(anchor);
    document.set_cursor(cursor);
    document.set_selection_revision(document.get_selection_revision().wrapping_add(1));
}

pub fn show_error(document: &YamlDocument<'_>, error: &str, language: Language) {
    let detail = error
        .split(": ")
        .map(|part| super::yaml_core::error_message::localize(part, language))
        .collect::<Vec<_>>()
        .join(": ");
    document.set_status(Default::default());
    document.set_error(detail.into());
}

pub fn respond(
    document: &YamlDocument<'_>,
    id: i32,
    result: Result<Reply, String>,
    language: Language,
) {
    if id != document.get_request_id() || document.get_mode() == 0 {
        return;
    }
    document.set_busy(false);
    document.set_writing(false);
    document.set_error(Default::default());
    document.set_status(Default::default());
    match result {
        Ok(Reply::Override(saved)) => {
            document.set_revision(saved.revision.into());
            document.set_enabled(saved.enabled);
            document.set_saved_enabled(saved.enabled);
            set_text(document, saved.text);
            document.set_accepted_text(document.get_text());
            document.set_loaded(true);
            select(document, None);
        }
        Ok(Reply::Source(page)) => {
            if page.match_missing {
                document.invoke_report(3);
                return;
            }
            document.set_revision(page.revision.into());
            document.set_applied(page.applied);
            document.set_start(page.start as i32);
            document.set_end(page.end as i32);
            document.set_total(page.total as i32);
            document.set_first_line(page.first_line as i32);
            set_text(document, page.text);
            select(document, page.selected);
            document.set_loaded(true);
        }
        Ok(Reply::Valid) => document.invoke_report(1),
        Ok(Reply::Saved(revision)) => {
            document.set_revision(revision.into());
            document.set_saved_enabled(document.get_enabled());
            document.set_dirty(false);
            document.invoke_report(2);
        }
        Ok(Reply::ExternalOpened) => document.set_external_open(true),
        Ok(Reply::ExternalDraft(text)) => {
            document.set_external_open(false);
            if document.get_editable() && text != document.get_text().as_str() {
                set_text(document, text);
                edited(document);
                select(document, None);
            }
        }
        Err(error) => {
            if document.get_mode() == 1
                && let Some(location) = error.strip_prefix("全局覆写 YAML 格式错误 · ")
                && let Some((line, column)) = location.split_once(':')
                && let (Ok(line), Ok(column)) = (line.parse::<usize>(), column.parse::<usize>())
                && let Ok(start) = line_start(&document.get_text(), line)
            {
                let text = document.get_text();
                let offset = text[start..]
                    .char_indices()
                    .take_while(|(_, ch)| *ch != '\n')
                    .nth(column.saturating_sub(1))
                    .map_or(start, |(index, _)| start + index);
                select(document, Some((offset, offset)));
            }
            show_error(document, &error, language);
        }
    }
}

pub fn edited(document: &YamlDocument<'_>) {
    if !document.get_editable() {
        return;
    }
    let text = document.get_text();
    if text.len() > EDIT_LIMIT {
        document.set_text(document.get_accepted_text());
        document.invoke_oversized_edit();
        return;
    }
    document.set_line_count(line_count(&text));
    document.set_accepted_text(text);
    document.set_dirty(true);
    document.set_error(Default::default());
    document.invoke_report(0);
}

pub fn locate(document: &YamlDocument<'_>, offset: i32) {
    let text = document.get_text();
    let offset = text.floor_char_boundary((offset.max(0) as usize).min(text.len()));
    let prefix = &text[..offset];
    document.set_line(
        document.get_first_line() + prefix.bytes().filter(|byte| *byte == b'\n').count() as i32,
    );
    let tail = prefix.rsplit_once('\n').map_or(prefix, |(_, tail)| tail);
    document.set_column(tail.chars().count() as i32 + 1);
}

pub fn edit_position(
    document: &YamlDocument<'_>,
    kind: &str,
    argument: &str,
    anchor: i32,
    cursor: i32,
) -> Result<()> {
    let text = document.get_text();
    let boundary = |offset: i32| text.floor_char_boundary((offset.max(0) as usize).min(text.len()));
    let anchor = boundary(anchor);
    let cursor = boundary(cursor);
    let selection = match kind {
        "find" => {
            ensure!(
                !argument.is_empty() && argument.len() <= 512,
                "请输入不超过 512 字节的搜索词"
            );
            let from = cursor.max(anchor);
            let found = text[from..]
                .find(argument)
                .map(|start| from + start)
                .or_else(|| text.find(argument));
            if found.is_none() {
                document.invoke_report(3);
            }
            found.map(|start| (start, start + argument.len()))
        }
        "line" => {
            let line = argument
                .parse::<usize>()
                .ok()
                .filter(|line| *line > 0)
                .context("请输入有效的行号")?;
            let start = line_start(&text, line)?;
            Some((start, start))
        }
        _ => return Ok(()),
    };
    if let Some(selection) = selection {
        document.set_status(Default::default());
        select(document, Some(selection));
    }
    Ok(())
}

fn line_start(text: &str, line: usize) -> Result<usize> {
    match line {
        0 => anyhow::bail!("行号必须大于零"),
        1 => Ok(0),
        _ => text
            .match_indices('\n')
            .nth(line - 2)
            .map(|(index, _)| index + 1)
            .context("行号超出源码范围"),
    }
}

pub fn indent(window: &slint::Window) {
    use slint::platform::WindowEvent;
    window.dispatch_event(WindowEvent::KeyPressed { text: "  ".into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: "  ".into() });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_margin_and_highlights_preserve_source_boundaries() {
        assert_eq!(line_count("a\nb\r\n"), 3);
        assert_eq!(highlight("  - name: 节点🙂").prefix.as_str(), "  - name:");
        assert_eq!(highlight("  # 中文🙂").tone, 1);
        assert!(highlight("  - https://example.invalid").prefix.is_empty());
        assert!(
            highlight(&format!("# {}", "字".repeat(512)))
                .prefix
                .is_empty()
        );
    }
}
