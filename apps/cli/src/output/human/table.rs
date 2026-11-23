use super::fields::scalar_in;
use crate::messages;
use anyhow::Result;
use clash_client_core::config::Language;
use serde_json::Value;
use std::io::Write;
use unicode_width::UnicodeWidthStr;

pub(super) fn width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(80)
        .clamp(40, 240)
}

#[cfg(test)]
pub(super) fn write(
    out: &mut dyn Write,
    headers: &[&str],
    rows: &[Vec<String>],
    available: usize,
) -> Result<()> {
    write_in(out, headers, rows, available, Language::SimplifiedChinese)
}

pub(super) fn write_in(
    out: &mut dyn Write,
    headers: &[&str],
    rows: &[Vec<String>],
    available: usize,
    language: Language,
) -> Result<()> {
    if rows.is_empty() {
        writeln!(out, "{}", messages::output_no_records(language))?;
        return Ok(());
    }
    let mut widths: Vec<_> = headers.iter().map(|s| s.width()).collect();
    for row in rows {
        for (i, value) in row.iter().enumerate().take(widths.len()) {
            widths[i] = widths[i].max(value.width());
        }
    }
    if widths.iter().sum::<usize>() + headers.len().saturating_sub(1) * 2 > available {
        // Keep complete names and actionable IDs on narrow SSH terminals.
        // Truncated names cannot safely be copied into select/show commands.
        for (i, row) in rows.iter().enumerate() {
            if i > 0 {
                writeln!(out)?;
            }
            for (header, cell) in headers.iter().zip(row) {
                if cell != "—" {
                    super::field_in(out, header, cell, language)?;
                }
            }
        }
    } else {
        let header: Vec<_> = headers.iter().map(|s| (*s).to_owned()).collect();
        line(out, &header, &widths)?;
        writeln!(
            out,
            "{}",
            "-".repeat(widths.iter().sum::<usize>() + headers.len().saturating_sub(1) * 2)
        )?;
        for row in rows {
            line(out, row, &widths)?;
        }
    }
    Ok(())
}

fn line(out: &mut dyn Write, row: &[String], widths: &[usize]) -> Result<()> {
    for (i, (cell, width)) in row.iter().zip(widths).enumerate() {
        write!(out, "{cell}")?;
        if i + 1 < row.len() {
            write!(
                out,
                "{}",
                " ".repeat(width.saturating_sub(cell.width()) + 2)
            )?;
        }
    }
    writeln!(out)?;
    Ok(())
}

pub(super) fn records_in(
    out: &mut dyn Write,
    records: &[Value],
    columns: &[(&str, &str)],
    language: Language,
) -> Result<()> {
    let rows: Vec<Vec<String>> = records
        .iter()
        .map(|record| {
            columns
                .iter()
                .map(|(_, key)| scalar_in(key, &record[*key], language))
                .collect()
        })
        .collect();
    let headers: Vec<_> = columns.iter().map(|(label, _)| *label).collect();
    write_in(out, &headers, &rows, width(), language)
}

pub(super) fn page_in(out: &mut dyn Write, value: &Value, language: Language) -> Result<()> {
    let count = value["items"].as_array().map_or(0, Vec::len);
    let offset = value["offset"].as_u64().unwrap_or(0);
    let total = value["total"].as_u64().unwrap_or(count as u64);
    let footer = if count == 0 && total > 0 {
        messages::output_page_empty(language, offset, total)
    } else if offset > 0 || total > count as u64 {
        let first = offset.saturating_add(1);
        let last = offset.saturating_add(count as u64);
        messages::output_page_range(language, first, last, total)
    } else {
        messages::output_page_total(language, total)
    };
    writeln!(out, "{footer}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_and_ascii_cells_align_by_display_width() {
        let mut out = Vec::new();
        write(
            &mut out,
            &["节点", "延迟"],
            &[
                vec!["德国01".into(), "140 ms".into()],
                vec!["US".into(), "未测速".into()],
            ],
            80,
        )
        .unwrap();
        let output = String::from_utf8(out).unwrap();
        assert_eq!(
            output,
            "节点    延迟\n--------------\n德国01  140 ms\nUS      未测速\n"
        );
    }

    #[test]
    fn narrow_terminals_keep_full_names_and_actionable_ids() {
        let name = "德国01-9929-ipv6-很长的节点名称";
        let id = "f6f08dc4-bb3b-4f04-92a5-874f79a67571";
        let mut out = Vec::new();
        write(
            &mut out,
            &["节点", "ID"],
            &[vec![name.into(), id.into()]],
            40,
        )
        .unwrap();
        let output = String::from_utf8(out).unwrap();
        assert_eq!(output, format!("节点：{name}\nID：{id}\n"));
    }
}
