use crate::args::Args;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::io::{IsTerminal, Write};

pub fn emit(args: &Args, value: Value) -> Result<()> {
    let value = if args.flag("show-secrets") {
        value
    } else {
        redact(value)
    };
    if let Some(path) = args.get("output") {
        let bytes =
            serde_json::to_vec_pretty(&json!({"schema":1,"command":args.path,"data":value}))?;
        clash_client_core::config::atomic_write(&crate::backend::absolute(path)?, &bytes)?;
        if !args.flag("quiet") {
            eprintln!("已保存：{path}");
        }
        return Ok(());
    }
    let mut stdout = std::io::stdout().lock();
    if args.flag("json") || args.flag("jsonl") {
        serde_json::to_writer(
            &mut stdout,
            &json!({"schema":1,"command":args.path,"data":value}),
        )?;
        writeln!(stdout)?;
    } else {
        print_value(&mut stdout, &value)?;
    }
    Ok(())
}

fn print_value(out: &mut impl Write, value: &Value) -> Result<()> {
    match value {
        Value::String(text) => writeln!(out, "{text}")?,
        Value::Array(rows) => {
            for row in rows {
                if let Some(fields) = row.as_object() {
                    let mut first = true;
                    for (key, value) in fields {
                        if !first {
                            write!(out, "  ")?;
                        }
                        first = false;
                        write!(out, "{key}={}", scalar(value))?;
                    }
                    writeln!(out)?;
                } else {
                    print_value(out, row)?;
                }
            }
            if rows.is_empty() {
                writeln!(out, "无记录")?;
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                if value.is_array() || value.is_object() {
                    writeln!(out, "{key}:")?;
                    print_value(out, value)?;
                } else {
                    writeln!(out, "{key}: {}", scalar(value))?;
                }
            }
        }
        _ => writeln!(out, "{value}")?,
    }
    Ok(())
}
fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}

pub fn redact(mut value: Value) -> Value {
    match &mut value {
        Value::Object(map) => {
            for (key, field) in map {
                let normalized = key.to_ascii_lowercase().replace('-', "_");
                if [
                    "password",
                    "secret",
                    "token",
                    "uuid",
                    "private_key",
                    "ip_reputation_key",
                    "authorization",
                    "auth",
                    "psk",
                    "preshared_key",
                    "api_key",
                ]
                .iter()
                .any(|s| normalized == *s || normalized.ends_with(&format!("_{s}")))
                {
                    if !field.is_null() {
                        *field = Value::String("[已隐藏]".into());
                    }
                } else if matches!(normalized.as_str(), "source" | "url")
                    && field
                        .as_str()
                        .is_some_and(|s| s.starts_with("http://") || s.starts_with("https://"))
                {
                    *field = Value::String(clash_client_runtime::claude_check::system::redact_url(
                        field.as_str().unwrap(),
                    ));
                } else {
                    *field = redact(std::mem::take(field));
                }
            }
        }
        Value::Array(items) => {
            for field in items {
                *field = redact(std::mem::take(field));
            }
        }
        _ => {}
    }
    value
}

pub async fn confirm(args: &Args, message: &str) -> Result<()> {
    if args.flag("yes") {
        return Ok(());
    }
    ensure!(
        std::io::stdin().is_terminal(),
        "{message}；非交互模式请明确传入 --yes"
    );
    eprint!("{message} [y/N] ");
    std::io::stderr().flush()?;
    // Tokio stdin uses a blocking-pool read that cannot be cancelled. A plain
    // reader thread keeps Ctrl+C responsive and never delays runtime shutdown.
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("cor-confirm".into())
        .spawn(move || {
            let mut answer = String::new();
            let result = std::io::stdin().read_line(&mut answer).map(|_| answer);
            let _ = sender.send(result);
        })?;
    let answer = receiver.await??;
    ensure!(matches!(answer.trim(), "y" | "Y" | "yes"), "操作已取消");
    Ok(())
}

pub fn on_off(value: &str) -> Result<bool> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => anyhow::bail!("参数应为 on 或 off"),
    }
}
pub fn page(args: &Args, rows: Vec<Value>) -> Result<Value> {
    let limit = args.number("limit", 100, 10_000)?;
    let offset = args.number("offset", 0, usize::MAX)?;
    let total = rows.len();
    Ok(
        json!({"total":total, "offset":offset, "items":rows.into_iter().skip(offset).take(limit).collect::<Vec<_>>() }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_configuration_credentials_are_removed_without_hiding_results() {
        let value = redact(
            json!({"secret":"controller","proxies":[{"name":"node","password":"pw","uuid":"key","private-key":"private"}],"source":"https://example.com/sub?token=key","latency":20}),
        );
        let text = value.to_string();
        for sensitive in ["controller", "\"pw\"", "\"key\"", "?token", "\"private\""] {
            assert!(!text.contains(sensitive), "{text}");
        }
        assert_eq!(value["latency"], 20);
    }
}
