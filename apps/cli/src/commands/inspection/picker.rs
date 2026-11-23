use anyhow::{Context, Result, ensure};
use clash_client_core::config::Language;
use std::io::{BufRead, IsTerminal, Write};

pub(super) fn require_terminal(
    interactive: bool,
    machine_output: bool,
    language: Language,
) -> Result<()> {
    ensure!(
        interactive && !machine_output,
        "{}",
        language.text(
            "请提供 GROUP NODE；交互选择需要终端，JSON/脚本请使用 cor node select GROUP NODE",
            "Pass GROUP NODE; interactive selection requires a terminal. For JSON or scripts, use cor node select GROUP NODE",
        )
    );
    Ok(())
}

pub(super) async fn choose(
    title: &str,
    candidates: Vec<String>,
    language: Language,
) -> Result<String> {
    require_terminal(std::io::stdin().is_terminal(), false, language)?;
    ensure!(
        !candidates.is_empty(),
        "{}",
        match language {
            Language::SimplifiedChinese => format!("没有可选择的{title}"),
            _ => format!("No {title} available"),
        }
    );
    let title = title.to_owned();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("cor-node-picker".into())
        .spawn(move || {
            let result = pick(
                &mut std::io::stdin().lock(),
                &mut std::io::stderr(),
                &title,
                &candidates,
                language,
            );
            let _ = sender.send(result);
        })?;
    receiver
        .await
        .context(language.text("交互选择已结束", "Interactive selection ended"))?
}

fn clean(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| character.escape_debug())
        .collect()
}

fn pick(
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    title: &str,
    candidates: &[String],
    language: Language,
) -> Result<String> {
    let mut query = String::new();
    loop {
        let matches: Vec<_> = candidates
            .iter()
            .filter(|name| name.to_lowercase().contains(&query))
            .collect();
        match language {
            Language::SimplifiedChinese => writeln!(output, "{title}（{} 项）", matches.len())?,
            _ => writeln!(output, "{title} ({} results)", matches.len())?,
        }
        for (index, name) in matches.iter().take(50).enumerate() {
            writeln!(output, "  {}. {}", index + 1, clean(name))?;
        }
        writeln!(
            output,
            "{}",
            language.text(
                "输入编号选择，输入文字筛选，/ 清除筛选，q 取消（最多显示 50 项）",
                "Enter a number to select, text to filter, / to clear, or q to cancel (up to 50 shown)",
            )
        )?;
        output.flush()?;
        let mut answer = String::new();
        ensure!(
            input.read_line(&mut answer)? > 0,
            "{}",
            language.text("操作已取消", "Operation cancelled")
        );
        let answer = answer.trim();
        ensure!(
            answer != "q",
            "{}",
            language.text("操作已取消", "Operation cancelled")
        );
        if let Ok(number) = answer.parse::<usize>() {
            if let Some(name) = number
                .checked_sub(1)
                .filter(|index| *index < 50)
                .and_then(|index| matches.get(index))
            {
                return Ok((*name).clone());
            }
            writeln!(
                output,
                "{}",
                language.text("编号不在当前列表中", "Number is not in the current list")
            )?;
        } else if answer == "/" {
            query.clear();
        } else if !answer.is_empty() {
            query = answer.to_lowercase();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn filters_unicode_and_returns_the_original_name() {
        let names = vec!["🇺🇸 美国 01".into(), "🚀 香港 /2".into(), "DIRECT".into()];
        let mut output = Vec::new();
        assert_eq!(
            pick(
                &mut Cursor::new("香港\n1\n"),
                &mut output,
                "节点",
                &names,
                Language::SimplifiedChinese
            )
            .unwrap(),
            names[1]
        );
        assert!(String::from_utf8(output).unwrap().contains("香港 /2"));
    }

    #[test]
    fn invalid_numbers_reset_cancellation_and_eof_are_safe() {
        let names = vec!["one".into(), "two".into()];
        assert_eq!(
            pick(
                &mut Cursor::new("99\n0\nnothing\n/\n2\n"),
                &mut Vec::new(),
                "节点",
                &names,
                Language::SimplifiedChinese
            )
            .unwrap(),
            "two"
        );
        for input in ["q\n", ""] {
            assert!(
                pick(
                    &mut Cursor::new(input),
                    &mut Vec::new(),
                    "节点",
                    &names,
                    Language::SimplifiedChinese
                )
                .is_err()
            );
        }
        assert!(require_terminal(false, false, Language::SimplifiedChinese).is_err());
        assert!(require_terminal(true, true, Language::SimplifiedChinese).is_err());
        assert!(require_terminal(true, false, Language::SimplifiedChinese).is_ok());
    }

    #[test]
    fn english_picker_preserves_user_names_and_localizes_guidance() {
        let names = vec!["🚀 香港 /2".into(), "DIRECT".into()];
        let mut output = Vec::new();
        let selected = pick(
            &mut Cursor::new("99\n香港\n1\n"),
            &mut output,
            "Node",
            &names,
            Language::English,
        )
        .unwrap();
        assert_eq!(selected, names[0]);
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("Node (2 results)"), "{text}");
        assert!(text.contains("Enter a number to select"), "{text}");
        assert!(text.contains("Number is not in the current list"), "{text}");
        assert!(text.contains("🚀 香港 /2"), "{text}");
        assert!(
            require_terminal(false, false, Language::English)
                .unwrap_err()
                .to_string()
                .contains("interactive selection requires a terminal")
        );
    }

    #[test]
    fn terminal_control_characters_are_escaped() {
        assert_eq!(clean("bad\u{1b}[31m\n"), "bad\\u{1b}[31m\\n");
    }
}
