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
        "node test" => lists::delays(out, args, value),
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
        "restart" if args.flag("network-only") => message(
            out,
            "已刷新当前内核的出站与 DNS，并关闭旧连接；目标服务是否可用仍需检查",
        ),
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
        "stop" => {
            message(out, "内核已停止")?;
            network_notice(out, &value["host_routes"])
        }
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
        "diagnose" => diagnose(out, value),
        "proxy" => {
            field(out, "代理", &text(value, "proxy"))?;
            network_notice(out, &value["host_routes"])
        }
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
            field(out, "数据版本", &text(value, "version"))?;
            field(out, "数据准备时间", &text(value, "updated"))?;
            let installed_at = if value["installed_at"].is_null() {
                "暂无记录".to_owned()
            } else {
                text(value, "installed_at")
            };
            field(out, "本机上次更新", &installed_at)?;
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
            "安装包已校验，安装助手已启动，尚未确认安装完成。\n查看结果：cor update status（沿用原账户和 --data-dir 参数）\n安装成功后可用 cor start 启动后台。",
        ),
        "update status" => {
            if value["state"] == "none" {
                message(out, "没有安装记录；旧版本开始的安装可能没有记录")
            } else {
                field(out, "目标版本", &text(value, "version"))?;
                field(out, "安装结果", &text(value, "message"))?;
                if matches!(value["state"].as_str(), Some("preparing" | "installing")) {
                    message(
                        out,
                        "稍后运行 cor update status 查看结果；若安装已退出，请导出诊断",
                    )
                } else {
                    Ok(())
                }
            }
        }
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
    if let Some(error) = value["backend_error"].as_str() {
        field(out, "内核", "状态读取失败")?;
        field(out, "失败原因", &clean(error))?;
    } else {
        let mut state = vec![text(value, "running")];
        for key in ["owner", "version"] {
            if !value[key].is_null() {
                state.push(text(value, key));
            }
        }
        field(out, "内核", &state.join(" · "))?;
    }
    if value["running"] != true {
        return network_notice(out, &value["host_routes"]);
    }
    field(
        out,
        "代理",
        &format!("{} · {}模式", text(value, "proxy"), text(value, "mode")),
    )?;
    for key in ["profile", "node"] {
        if !value[key].is_null() {
            field(out, label(key), &text(value, key))?;
        }
    }
    if !value["protected"].is_null() {
        field(out, "Claude 保护", &text(value, "protected"))?;
    }
    field(
        out,
        "流量",
        &format!(
            "下载 {} · 上传 {} · {} 个连接",
            text(value, "download_rate"),
            text(value, "upload_rate"),
            text(value, "connections")
        ),
    )?;
    network_notice(out, &value["host_routes"])
}

fn network_notice(out: &mut dyn Write, value: &Value) -> Result<()> {
    if let Some(notice) = crate::host_network::notice(value) {
        writeln!(out)?;
        field(out, "网络提醒", &clean(&notice))?;
        return field(
            out,
            "下一步",
            "导出报告：cor diagnose --output cor-diagnose.json",
        );
    }
    if let Some(summary) = crate::host_network::holder_summary(value) {
        field(out, "TUN 持有进程", &clean(&summary))?;
    }
    Ok(())
}

fn diagnose(out: &mut dyn Write, value: &Value) -> Result<()> {
    if value["offline"] == true {
        for (name, key) in [
            ("客户端版本", "version"),
            ("采集时间", "captured_at"),
            ("程序路径", "executable"),
            ("数据目录", "data_directory"),
        ] {
            field(out, name, &text(value, key))?;
        }
        field(
            out,
            "后台日志采集",
            &text(&value["local_evidence"]["daemon_log"], "state"),
        )?;
        network_notice(out, &value["host_routes"])?;
        return message(
            out,
            "仅收集本地现场，未检测网络或连接后台；用 --output cor-diagnose.json 保存完整报告。",
        );
    }
    field(out, "DNS 检测域名", &text(value, "dns_host"))?;
    for (name, key) in [("系统 DNS", "system_dns"), ("当前内核 DNS", "core_dns")] {
        let result = &value[key];
        let description = if let Some(answers) = result["Ok"].as_array() {
            answers
                .iter()
                .filter_map(Value::as_str)
                .map(clean)
                .collect::<Vec<_>>()
                .join("、")
        } else if let Some(error) = result["Err"].as_str() {
            format!("失败：{}", clean(error))
        } else {
            "未检测".to_owned()
        };
        field(out, name, &description)?;
    }
    for (name, key) in [
        ("系统路由 HTTP（不使用代理环境变量）", "system_route_http"),
        ("本地代理 HTTP", "http"),
        ("代理 IPv4 出口", "ipv4"),
        ("代理 IPv6 出口", "ipv6"),
    ] {
        let probe = &value[key];
        let description = if let Some(error) = probe["error"].as_str() {
            format!("失败：{}", clean(error))
        } else if probe["status"].is_number() {
            format!(
                "{} · {} · {}",
                text(probe, "url"),
                text(probe, "detail"),
                text(probe, "millis")
            )
        } else {
            "未检测".to_owned()
        };
        field(out, name, &description)?;
    }
    if let Some(error) = value["backend_error"].as_str() {
        field(out, "后台", &clean(error))?;
    }
    let environment = &value["proxy_environment"];
    let configured: Vec<_> = [
        ("http", "HTTP_PROXY"),
        ("https", "HTTPS_PROXY"),
        ("all", "ALL_PROXY"),
        ("no_proxy", "NO_PROXY"),
    ]
    .into_iter()
    .filter_map(|(key, name)| (environment[key] == true).then_some(name))
    .collect();
    let configured = configured.join("、");
    field(
        out,
        "终端代理环境变量",
        if configured.is_empty() {
            "未检测到常见代理变量"
        } else {
            &configured
        },
    )?;
    if value["tun_configuration"]["state"] == "observed" {
        field(
            out,
            "TUN 配置",
            &tun_configuration(&value["tun_configuration"]),
        )?;
    }
    network_notice(out, &value["host_routes"])
}

fn tun_configuration(value: &Value) -> String {
    let mut parts = Vec::new();
    for (label, key) in [
        ("自动路由", "auto_route"),
        ("严格路由", "strict_route"),
        ("DNS 劫持", "dns_hijack"),
        ("IPv4 路由段", "inet4_route_address"),
        ("IPv6 路由段", "inet6_route_address"),
    ] {
        let value = &value[key];
        if value.is_null() {
            continue;
        }
        let text = match value {
            Value::Array(values) => values
                .iter()
                .map(|value| match value {
                    Value::String(value) => clean(value),
                    _ => value.to_string(),
                })
                .collect::<Vec<_>>()
                .join("、"),
            Value::String(value) => clean(value),
            _ => value.to_string(),
        };
        if !text.is_empty() {
            parts.push(format!("{label}={text}"));
        }
    }
    if parts.is_empty() {
        "已读取，但未声明覆盖限制".into()
    } else {
        parts.join(" · ")
    }
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
