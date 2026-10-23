//! Human-readable presentation only; command results and JSON schemas stay intact.
mod fields;
mod lists;
mod table;
use crate::args::Args;
use anyhow::Result;
use fields::{clean, label, scalar, text};
use serde_json::Value;
use std::io::Write;

pub(super) fn write(out: &mut dyn Write, args: &Args, value: &Value) -> Result<()> {
    let first = args.positional.first().map(String::as_str).unwrap_or("");
    match args.path.as_str() {
        "node list" => lists::nodes(out, args, value),
        "node test" => lists::delays(out, value),
        "node select" => {
            writeln!(
                out,
                "已切换节点：{} → {}",
                text(value, "group"),
                text(value, "selected")
            )?;
            Ok(())
        }
        "status" => status(out, value),
        "start" | "restart" => {
            writeln!(
                out,
                "{}",
                if args.path == "start" {
                    "内核已启动"
                } else {
                    "内核已重启"
                }
            )?;
            let settings = &value["settings"];
            if let Some(profile) = value["profiles"].as_array().and_then(|rows| {
                rows.iter()
                    .find(|row| row["id"] == settings["active_profile"])
            }) {
                field(out, "当前订阅", &text(profile, "name"))?;
            }
            for key in ["run_mode", "proxy_mode", "mixed_port"] {
                field(out, label(key), &text(settings, key))?;
            }
            Ok(())
        }
        "stop" => message(out, "内核已停止，网络设置已恢复"),
        "version" => {
            writeln!(
                out,
                "Clash of Rust {}（{} / {}）",
                text(value, "version"),
                text(value, "os"),
                text(value, "arch")
            )?;
            Ok(())
        }
        "mode" => field(out, "运行模式", &text(value, "mode")),
        "proxy" => field(out, "代理方式", &text(value, "proxy")),
        "profile list" => lists::profiles(out, value),
        "profile add" | "profile import" => field(out, "已添加订阅", &text(value, "imported")),
        "profile use" => field(out, "当前订阅", &text(value, "name")),
        "profile edit" | "profile remove" => field(
            out,
            if args.path == "profile edit" {
                "已修改订阅"
            } else {
                "已删除订阅"
            },
            &clean(first),
        ),
        "profile update" => table::records(
            out,
            rows(value),
            &[
                ("订阅", "name"),
                ("更新成功", "updated"),
                ("失败原因", "error"),
            ],
        ),
        "rule list" => lists::rules(out, value),
        "rule shortcut" if value.is_array() => table::records(
            out,
            rows(value),
            &[
                ("名称", "name"),
                ("启用状态", "enabled"),
                ("订阅已包含", "already_in_subscription"),
                ("规则", "rule"),
            ],
        ),
        path if path.starts_with("rule ") => message(
            out,
            match path {
                "rule add" => "规则已添加",
                "rule edit" => "规则已修改",
                "rule remove" => "规则已删除",
                "rule move" => "规则顺序已调整",
                "rule enable" => "规则启用状态已保存",
                _ => "快捷规则设置已保存",
            },
        ),
        "connection list" | "connection show" => lists::connections(out, value),
        "connection close" => message(
            out,
            if args.flag("all") {
                "已关闭全部现有连接"
            } else {
                "连接已关闭"
            },
        ),
        "logs clear" => message(out, "日志已清空"),
        "config check" => message(out, "配置校验通过"),
        "config set" => {
            writeln!(out, "已保存设置：{}（{}）", label(first), clean(first))?;
            Ok(())
        }
        "config get" => {
            if args.positional.is_empty()
                && let Some(fields) = value.as_object()
            {
                for (key, value) in fields {
                    let name = format!("{}（{}）", label(key), clean(key));
                    match key.as_str() {
                        "rule_profiles" => field(
                            out,
                            &name,
                            &format!(
                                "已保存 {} 个订阅的规则设置；详见 cor rule list",
                                value.as_object().map_or(0, |m| m.len())
                            ),
                        )?,
                        "rule_overrides" => {
                            let count = rows(&value["enabled"])
                                .iter()
                                .filter(|v| v.as_bool() == Some(true))
                                .count();
                            field(
                                out,
                                &name,
                                &format!("{count} / 5 项已开启；详见 cor rule shortcut"),
                            )?;
                        }
                        "claude_protection" => field(
                            out,
                            &name,
                            if value.is_null() {
                                "已关闭"
                            } else {
                                "已开启"
                            },
                        )?,
                        _ => field(out, &name, &scalar(key, value))?,
                    }
                }
                Ok(())
            } else {
                tree(out, value, 0, true)
            }
        }
        "geo status" | "geo update" => {
            if args.path == "geo update" {
                message(out, "Geo 数据已更新并通过校验")?;
            }
            for key in ["version", "updated"] {
                field(out, label(key), &text(value, key))?;
            }
            table::records(
                out,
                rows(&value["files"]),
                &[("文件", "name"), ("大小", "size")],
            )
        }
        "autostart" => field(out, "桌面程序登录自启", &text(value, "enabled")),
        "service install" | "service remove" | "service status" => {
            let installed = value["installed"]
                .as_bool()
                .or(value["enabled"].as_bool())
                .unwrap_or(false);
            field(
                out,
                "CLI 后台登录自启",
                if installed { "已开启" } else { "已关闭" },
            )?;
            if value.get("path").is_some() {
                field(out, "配置文件", &text(value, "path"))?;
            }
            if installed && value["matches_current"] == false {
                message(out, "现有自启配置与当前程序或数据目录不一致")?;
            }
            if value.get("effective").is_some() {
                field(out, "生效时间", &text(value, "effective"))?;
            }
            Ok(())
        }
        "notify" => message(out, "系统通知已发送"),
        "update check" => {
            if value["available"] == true {
                field(
                    out,
                    if value["preview"] == true {
                        "可安装预发布版"
                    } else {
                        "可安装正式版"
                    },
                    &text(value, "version"),
                )?;
                if !value["package"].is_null() {
                    field(out, "安装包", &text(value, "package"))?;
                }
                field(out, "发布页面", &text(value, "url"))
            } else {
                message(out, "当前更新通道没有可安装的版本")
            }
        }
        "update install" => message(
            out,
            "已交给系统安装，请完成系统授权；安装完成后可用 cor start 启动后台",
        ),
        "ip-check list" => table::records(
            out,
            rows(value),
            &[("ID", "id"), ("检测项目", "name"), ("分类", "category")],
        ),
        "ip-check" => {
            writeln!(
                out,
                "IP 检测完成：共 {} 项，确认可用 {} 项，可用或部分可用 {} 项",
                text(value, "total"),
                text(value, "confirmed_available"),
                text(value, "non_red")
            )?;
            lists::ip_results(out, value)
        }
        "claude protect" => {
            field(out, "Claude 保护", &text(value, "enabled"))?;
            if value["enabled"] == true {
                field(out, "锁定节点", &text(&value["protection"], "node"))?;
            }
            Ok(())
        }
        // Logs and shell strings retain their exact content; other reports get
        // readable labels and indentation rather than raw JSON key/value rows.
        _ => tree(out, value, 0, false),
    }
}

fn status(out: &mut dyn Write, value: &Value) -> Result<()> {
    field(out, "内核", &text(value, "running"))?;
    if !value["owner"].is_null() {
        field(out, "控制方", &text(value, "owner"))?;
    }
    if value["running"] != true {
        return Ok(());
    }
    for key in [
        "version",
        "profile",
        "node",
        "mode",
        "proxy",
        "protected",
        "connections",
        "upload_rate",
        "download_rate",
    ] {
        field(out, label(key), &text(value, key))?;
    }
    Ok(())
}

fn message(out: &mut dyn Write, message: &str) -> Result<()> {
    writeln!(out, "{message}")?;
    Ok(())
}
fn field(out: &mut dyn Write, key: &str, value: &str) -> Result<()> {
    writeln!(out, "{key}：{value}")?;
    Ok(())
}
fn rows(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

pub(super) fn download_progress(progress: &clash_client_core::update::Progress) -> String {
    format!(
        "下载：{} / {} · 速度：{}/s · 线路：{}",
        fields::bytes(progress.received),
        fields::bytes(progress.total),
        fields::bytes(progress.bytes_per_second),
        clean(&progress.routing())
    )
}

fn tree(out: &mut dyn Write, value: &Value, depth: usize, settings: bool) -> Result<()> {
    let indent = "  ".repeat(depth);
    match value {
        Value::Object(fields) => {
            if fields.is_empty() {
                writeln!(out, "{indent}无记录")?;
            }
            for (key, value) in fields {
                if key == "schema" || key == "event" {
                    continue;
                }
                let name = if settings {
                    format!("{}（{}）", label(key), clean(key))
                } else {
                    clean(label(key))
                };
                if value.is_object() || value.is_array() {
                    writeln!(out, "{indent}{name}：")?;
                    tree(out, value, depth + 1, false)?;
                } else {
                    writeln!(out, "{indent}{name}：{}", scalar(key, value))?;
                }
            }
        }
        Value::Array(values) => {
            if values.is_empty() {
                writeln!(out, "{indent}无记录")?;
            }
            for (index, value) in values.iter().enumerate() {
                if value.is_object() {
                    writeln!(out, "{indent}第 {} 项", index + 1)?;
                    tree(out, value, depth + 1, false)?;
                } else {
                    writeln!(out, "{indent}- {}", scalar("", value))?;
                }
            }
        }
        Value::String(s) if depth == 0 => writeln!(out, "{s}")?,
        _ => writeln!(out, "{indent}{}", scalar("", value))?,
    }
    Ok(())
}
