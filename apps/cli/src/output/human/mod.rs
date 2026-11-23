//! Human-readable presentation only; command results and JSON schemas stay intact.
#[cfg(test)]
mod diagnose_tests;
mod fields;
mod lists;
mod table;
use crate::args::Args;
use anyhow::Result;
use clash_client_core::claude_check::{
    Group, Report, group_routes, order_summary, route_group_value_in, summary_order,
};
use clash_client_core::config::Language;
use fields::{clean, label, label_in, scalar_in, text, text_in};
use serde::Deserialize;
use serde_json::Value;
use std::io::Write;

mod service_status;

pub(super) fn write(out: &mut dyn Write, args: &Args, value: &Value) -> Result<()> {
    let first = args.positional.first().map(String::as_str).unwrap_or("");
    let language = args.language();
    match args.path.as_str() {
        "node list" => lists::nodes(out, args, value),
        "node test" => lists::delays(out, args, value),
        "node select" => {
            field_in(
                out,
                language.text("已切换节点", "Selected node"),
                &format!(
                    "{} → {}",
                    text_in(value, "group", language),
                    text_in(value, "selected", language)
                ),
                language,
            )
        }
        "status" => status(out, value, language),
        "restart" if args.flag("network-only") => message(
            out,
            language.text(
                "已刷新当前内核的出站与 DNS，并关闭旧连接；目标服务是否可用仍需检查",
                "Refreshed core routes and DNS; closed previous connections.",
            ),
        ),
        "start" | "restart" => {
            writeln!(
                out,
                "{}",
                if args.path == "start" {
                    language.text("内核已启动", "Core started")
                } else {
                    language.text("内核已重启", "Core restarted")
                }
            )?;
            let settings = &value["settings"];
            if let Some(profile) = value["profiles"].as_array().and_then(|rows| {
                rows.iter()
                    .find(|row| row["id"] == settings["active_profile"])
            }) {
                field_in(
                    out,
                    language.text("当前订阅", "Current subscription"),
                    &text_in(profile, "name", language),
                    language,
                )?;
            }
            for key in ["run_mode", "proxy_mode", "mixed_port"] {
                field_in(
                    out,
                    label_in(key, language),
                    &text_in(settings, key, language),
                    language,
                )?;
            }
            Ok(())
        }
        "stop" => {
            message(out, language.text("内核已停止", "Core stopped"))?;
            network_notice(out, &value["host_routes"], language)
        }
        "version" => {
            let version = text_in(value, "version", language);
            let os = text_in(value, "os", language);
            let arch = text_in(value, "arch", language);
            match language {
                Language::SimplifiedChinese => {
                    writeln!(out, "Clash of Rust {version}（{os} / {arch}）")?
                }
                Language::English => writeln!(out, "Clash of Rust {version} ({os} / {arch})")?,
            }
            Ok(())
        }
        "mode" => field_in(
            out,
            language.text("运行模式", "Routing mode"),
            &text_in(value, "mode", language),
            language,
        ),
        "diagnose" => diagnose(out, value, language),
        "network check" => {
            field_in(
                out,
                language.text("目标", "Target"),
                &text_in(value, "target", language),
                language,
            )?;
            use clash_client_core::network_tools::{entry_detail_in, entry_label_in};
            for (label, key) in [
                ("系统 DNS", "system_dns"),
                ("内核 DNS", "core_dns"),
                ("本机代理", "proxy"),
                ("HTTPS", "https"),
                ("HTTP", "http"),
                ("采用协议", "response"),
                ("实际线路", "route"),
            ] {
                let check = &value[key];
                let code = match check["success"].as_bool() {
                    Some(true) => "ok",
                    Some(false) => "probe_failed",
                    None => "not_observed",
                };
                let detail = check["detail"]
                    .as_str()
                    .filter(|detail| !detail.is_empty())
                    .unwrap_or("—");
                field_in(
                    out,
                    &entry_label_in(label, language),
                    &clean(&entry_detail_in(label, detail, code, language)),
                    language,
                )?;
            }
            Ok(())
        }
        path if path.starts_with("network ") => {
            let entries = value["entries"].as_array();
            let target = if value["target"] == "未检查"
                && entries.is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        matches!(entry["code"].as_str(), Some("invalid_target" | "invalid_options"))
                    })
                })
            {
                language.text("未检查", "Not checked").into()
            } else {
                text_in(value, "target", language)
            };
            field_in(
                out,
                language.text("目标", "Target"),
                &target,
                language,
            )?;
            if let Some(entries) = entries {
                for entry in entries {
                    let label = entry["label"].as_str().filter(|value| !value.is_empty()).unwrap_or("—");
                    let detail = entry["detail"].as_str().filter(|value| !value.is_empty()).unwrap_or("—");
                    let code = entry["code"].as_str().unwrap_or("");
                    field_in(
                        out,
                        &clean(&clash_client_core::network_tools::entry_label_in(label, language)),
                        &clean(&clash_client_core::network_tools::entry_detail_in(
                            label, detail, code, language,
                        )),
                        language,
                    )?;
                }
            }
            Ok(())
        }
        "proxy" => {
            field_in(
                out,
                language.text("代理", "Proxy"),
                &text_in(value, "proxy", language),
                language,
            )?;
            network_notice(out, &value["host_routes"], language)
        }
        "profile list" => lists::profiles(out, value, language),
        "profile add" | "profile import" => field_in(
            out,
            language.text("已添加订阅", "Added subscription"),
            &text_in(value, "imported", language),
            language,
        ),
        "profile use" => field_in(
            out,
            language.text("当前订阅", "Current subscription"),
            &text_in(value, "name", language),
            language,
        ),
        "profile edit" | "profile remove" => field_in(
            out,
            if args.path == "profile edit" {
                language.text("已修改订阅", "Updated subscription")
            } else {
                language.text("已删除订阅", "Removed subscription")
            },
            &clean(first),
            language,
        ),
        "profile update" => {
            let headers = [
                language.text("订阅", "Subscription"),
                language.text("更新成功", "Updated"),
                language.text("失败原因", "Error"),
            ];
            let rows = rows(value)
                .iter()
                .map(|row| {
                    let error = row["error"].as_str().filter(|error| !error.is_empty());
                    vec![
                        text_in(row, "name", language),
                        text_in(row, "updated", language),
                        error.map_or_else(
                            || text_in(row, "error", language),
                            |error| {
                                clean(&clash_client_core::error_message::localize(error, language))
                            },
                        ),
                    ]
                })
                .collect::<Vec<_>>();
            table::write_in(out, &headers, &rows, table::width(), language)
        }
        "rule list" => lists::rules(out, value, language),
        "rule shortcut" if value.is_array() => table::records_in(
            out,
            rows(value),
            &[
                (language.text("名称", "Name"), "name"),
                (language.text("启用状态", "Enabled"), "enabled"),
                (
                    language.text("订阅已包含", "In subscription"),
                    "already_in_subscription",
                ),
                (language.text("规则", "Rule"), "rule"),
            ],
            language,
        ),
        path if path.starts_with("rule ") => message(
            out,
            match path {
                "rule add" => language.text("规则已添加", "Rule added"),
                "rule edit" => language.text("规则已修改", "Rule updated"),
                "rule remove" => language.text("规则已删除", "Rule removed"),
                "rule move" => language.text("规则顺序已调整", "Rule order updated"),
                "rule enable" => language.text("规则启用状态已保存", "Rule status saved"),
                _ => language.text("快捷规则设置已保存", "Rule shortcuts saved"),
            },
        ),
        "connection list" | "connection show" => lists::connections(out, value, language),
        "connection close" => message(
            out,
            if args.flag("all") {
                language.text("已关闭全部现有连接", "Closed all current connections")
            } else {
                language.text("连接已关闭", "Connection closed")
            },
        ),
        "logs clear" => message(out, language.text("日志已清空", "Logs cleared")),
        "config check" => message(
            out,
            language.text("配置校验通过", "Configuration is valid"),
        ),
        "config set" => {
            match language {
                Language::SimplifiedChinese => {
                    writeln!(out, "已保存设置：{}（{}）", label(first), clean(first))?
                }
                Language::English => writeln!(
                    out,
                    "Saved setting: {} ({})",
                    label_in(first, language),
                    clean(first)
                )?,
            }
            Ok(())
        }
        "config get" => {
            if args.positional.is_empty()
                && let Some(fields) = value.as_object()
            {
                for (key, value) in fields {
                    let name = match language {
                        Language::SimplifiedChinese => format!("{}（{}）", label(key), clean(key)),
                        Language::English => {
                            format!("{} ({})", label_in(key, language), clean(key))
                        }
                    };
                    match key.as_str() {
                        "rule_profiles" => {
                            let count = value.as_object().map_or(0, |settings| settings.len());
                            let summary = match language {
                                Language::SimplifiedChinese => {
                                    format!("已保存 {count} 个订阅的规则设置；详见 cor rule list")
                                }
                                Language::English => {
                                    format!("Rule settings for {count} subscriptions; see cor rule list")
                                }
                            };
                            field_in(out, &name, &summary, language)?;
                        }
                        "rule_overrides" => {
                            let count = rows(&value["enabled"])
                                .iter()
                                .filter(|v| v.as_bool() == Some(true))
                                .count();
                            let summary = match language {
                                Language::SimplifiedChinese => {
                                    format!("{count} / 5 项已开启；详见 cor rule shortcut")
                                }
                                Language::English => {
                                    format!("{count} / 5 enabled; see cor rule shortcut")
                                }
                            };
                            field_in(out, &name, &summary, language)?;
                        }
                        "claude_protection" => field_in(
                            out,
                            &name,
                            if value.is_null() {
                                language.text("已关闭", "Off")
                            } else {
                                language.text("已开启", "On")
                            },
                            language,
                        )?,
                        _ => field_in(out, &name, &scalar_in(key, value, language), language)?,
                    }
                }
                Ok(())
            } else {
                tree(out, value, 0, true, language, None)
            }
        }
        "geo status" | "geo update" => {
            if args.path == "geo update" {
                message(
                    out,
                    language.text("Geo 数据已更新并通过校验", "Geo data updated and verified"),
                )?;
            }
            field_in(
                out,
                language.text("数据版本", "Data version"),
                &text_in(value, "version", language),
                language,
            )?;
            field_in(
                out,
                language.text("数据准备时间", "Data prepared"),
                &text_in(value, "updated", language),
                language,
            )?;
            let installed_at = if value["installed_at"].is_null() {
                language.text("暂无记录", "No record").to_owned()
            } else {
                text_in(value, "installed_at", language)
            };
            field_in(
                out,
                language.text("本机上次更新", "Last local update"),
                &installed_at,
                language,
            )?;
            table::records_in(
                out,
                rows(&value["files"]),
                &[
                    (language.text("文件", "File"), "name"),
                    (language.text("大小", "Size"), "size"),
                ],
                language,
            )
        }
        "autostart" => field_in(
            out,
            language.text("后台自启动", "Background autostart"),
            &text_in(value, "enabled", language),
            language,
        ),
        "service install" | "service remove" | "service status" => {
            let installed = value["installed"]
                .as_bool()
                .or(value["enabled"].as_bool())
                .unwrap_or(false);
            if args.path == "service status" {
                field_in(
                    out,
                    language.text("CLI 自启配置", "CLI autostart configuration"),
                    match value["installed"].as_bool() {
                        Some(true) => language.text("已安装", "Installed"),
                        Some(false) => language.text("未安装", "Not installed"),
                        None => language.text("未确认", "Unconfirmed"),
                    },
                    language,
                )?;
                field_in(
                    out,
                    language.text("登录自启", "Start at login"),
                    match value["enabled"].as_bool() {
                        Some(true) => language.text("已开启", "On"),
                        Some(false) => language.text("已关闭", "Off"),
                        None => language.text("未确认", "Unconfirmed"),
                    },
                    language,
                )?;
                field_in(
                    out,
                    language.text("服务状态", "Service status"),
                    match value["active"].as_bool() {
                        Some(true) => language.text("运行中", "Running"),
                        Some(false) => language.text("未运行", "Stopped"),
                        None => language.text("未确认", "Unconfirmed"),
                    },
                    language,
                )?;
                field_in(
                    out,
                    language.text("服务 PID", "Service PID"),
                    &text_in(value, "main_pid", language),
                    language,
                )?;
                field_in(
                    out,
                    language.text("后台 PID", "Background PID"),
                    &text_in(value, "owner_pid", language),
                    language,
                )?;
                field_in(
                    out,
                    language.text("控制方", "Owner"),
                    match value["ownership"].as_str() {
                        Some("service") => language.text("由登录服务管理", "Login service"),
                        Some("detached") => {
                            language.text("服务之外的 CLI 后台", "CLI background outside service")
                        }
                        Some("gui") => language.text("桌面客户端", "Desktop client"),
                        Some("none") => language.text("无后台", "No background process"),
                        _ => language.text("未确认", "Unconfirmed"),
                    },
                    language,
                )?;
                for name in ["config_error", "service_error", "backend_error"] {
                    if let Some(error) = value[name].as_str() {
                        field_in(
                            out,
                            language.text("查询失败", "Query failed"),
                            &clean(&service_status::error_in(name, error, language)),
                            language,
                        )?;
                    }
                }
            } else {
                field_in(
                    out,
                    language.text("CLI 后台登录自启", "CLI background start at login"),
                    if installed {
                        language.text("已开启", "On")
                    } else {
                        language.text("已关闭", "Off")
                    },
                    language,
                )?;
            }
            if value.get("path").is_some() {
                field_in(
                    out,
                    language.text("配置文件", "Configuration file"),
                    &text_in(value, "path", language),
                    language,
                )?;
            }
            if installed && value["matches_current"] == false {
                message(
                    out,
                    language.text(
                        "现有自启配置与当前程序或数据目录不一致",
                        "Autostart configuration points to a different executable or data directory",
                    ),
                )?;
            }
            if value.get("effective").is_some() {
                field_in(
                    out,
                    language.text("生效时间", "Effective"),
                    &text_in(value, "effective", language),
                    language,
                )?;
            }
            Ok(())
        }
        "notify" => message(out, language.text("系统通知已发送", "System notification sent")),
        "update check" => {
            if value["available"] == true {
                let manager = value["package_manager"].as_str();
                field_in(
                    out,
                    match (value["preview"] == true, manager.is_some()) {
                        (true, true) => language.text("发现预发布版", "Pre-release found"),
                        (false, true) => language.text("发现正式版", "Release found"),
                        (true, false) => {
                            language.text("可安装预发布版", "Pre-release available")
                        }
                        (false, false) => language.text("可安装正式版", "Release available"),
                    },
                    &text_in(value, "version", language),
                    language,
                )?;
                if let Some(manager) = manager {
                    field_in(
                        out,
                        language.text("更新方式", "Update method"),
                        &clash_client_runtime::update::package_manager_update_hint_in(
                            manager, language,
                        ),
                        language,
                    )?;
                } else if !value["package"].is_null() {
                    field_in(
                        out,
                        language.text("安装包", "Package"),
                        &text_in(value, "package", language),
                        language,
                    )?;
                }
                field_in(
                    out,
                    language.text("发布页面", "Release page"),
                    &text_in(value, "url", language),
                    language,
                )
            } else {
                message(
                    out,
                    language.text(
                        "当前更新通道没有可安装的版本",
                        "No update is available on the selected channel",
                    ),
                )
            }
        }
        "update install" if value["state"] == "succeeded" => message(
            out,
            &clash_client_runtime::update::localize_installation_message(
                &text(value, "message"),
                language,
            ),
        ),
        "update install" => message(
            out,
            language.text(
                "安装包已校验，安装助手已启动，尚未确认安装完成。\n查看结果：cor update status（沿用原账户和 --data-dir 参数）\n安装成功后可用 cor start 启动后台。",
                "Package verified and installer started; installation is not yet confirmed.\nCheck the result: cor update status (using the same account and --data-dir).\nAfter installation, use cor start to start the background process.",
            ),
        ),
        "update status" => {
            if value["state"] == "none" {
                message(
                    out,
                    language.text(
                        "没有安装记录；旧版本开始的安装可能没有记录",
                        "No installation record; installations started by older versions may not be recorded",
                    ),
                )
            } else {
                field_in(
                    out,
                    language.text("目标版本", "Target version"),
                    &text_in(value, "version", language),
                    language,
                )?;
                field_in(
                    out,
                    language.text("安装结果", "Installation result"),
                    &clash_client_runtime::update::localize_installation_message(
                        &text(value, "message"),
                        language,
                    ),
                    language,
                )?;
                if matches!(value["state"].as_str(), Some("preparing" | "installing")) {
                    message(
                        out,
                        language.text(
                            "稍后运行 cor update status 查看结果；若安装已退出，请导出诊断",
                            "Run cor update status later; export diagnostics if the installer has exited",
                        ),
                    )
                } else {
                    Ok(())
                }
            }
        }
        "ip-check list" => table::records_in(
            out,
            rows(value),
            &[
                ("ID", "id"),
                (language.text("检测项目", "Item"), "name"),
                (language.text("分类", "Category"), "category"),
            ],
            language,
        ),
        "ip-check" => lists::ip_results(out, value, language),
        "claude check" | "claude browser" if !args.flag("details") => {
            claude_summary(out, &args.path, value, language)
        }
        "claude protect" => {
            field_in(
                out,
                language.text("Claude 保护", "Claude protection"),
                &text_in(value, "enabled", language),
                language,
            )?;
            if value["enabled"] == true {
                field_in(
                    out,
                    language.text("锁定节点", "Pinned node"),
                    &text_in(&value["protection"], "node", language),
                    language,
                )?;
            }
            Ok(())
        }
        // Logs and shell strings retain their exact content; other reports get
        // readable labels and indentation rather than raw JSON key/value rows.
        _ => tree(out, value, 0, false, language, None),
    }
}

fn claude_summary(
    out: &mut dyn Write,
    command: &str,
    value: &Value,
    language: Language,
) -> Result<()> {
    let report = Report::deserialize(value.get("report").unwrap_or(value))?;
    let (_, risk, _) = report.counts();
    match language {
        Language::SimplifiedChinese => writeln!(out, "Claude 检测 · {risk} 项需关注")?,
        Language::English => writeln!(out, "Claude check · Attention: {risk}")?,
    }
    let mut entries: Vec<_> = report
        .signals
        .iter()
        .filter_map(|signal| {
            let risk = report.differs(signal);
            (risk || signal.group == Group::Quality).then_some((signal, risk))
        })
        .collect();
    order_summary(&mut entries, |(signal, risk)| {
        summary_order(Some(signal), *risk)
    });
    for (signal, risk) in entries {
        let title = clean(&signal.display_title_in(language));
        field_in(
            out,
            &if risk {
                format!("{} · {title}", language.text("需关注", "Attention"))
            } else {
                title
            },
            &clean(&signal.display_value_in(language)),
            language,
        )?;
    }
    let mut route_groups = group_routes(&report.routes);
    route_groups.sort_by_key(|routes| !routes[0].risky());
    for routes in route_groups {
        let route = routes[0];
        if route.risky() {
            let outcome = route.display_outcome_in(language);
            let value = if route.chain.trim().is_empty() {
                outcome
            } else {
                format!(
                    "{outcome} · {}{}",
                    language.text("线路：", "Route: "),
                    route.chain.trim()
                )
            };
            field_in(
                out,
                &format!(
                    "{} · {}",
                    language.text("选路异常", "Unexpected route"),
                    clean(route.display_title_in(language))
                ),
                &clean(&value),
                language,
            )?;
        } else if routes.len() > 1 {
            field_in(
                out,
                language.text("Claude 域名选路", "Claude domain routes"),
                &clean(&route_group_value_in(&routes, language)),
                language,
            )?;
        } else {
            field_in(
                out,
                &format!(
                    "{} · {}",
                    language.text("Claude 选路", "Claude route"),
                    clean(route.display_title_in(language))
                ),
                &clean(&route.display_outcome_in(language)),
                language,
            )?;
        }
    }
    writeln!(
        out,
        "{}cor {command} --details",
        language.text("查看完整报告：", "Full report: ")
    )?;
    Ok(())
}

fn status(out: &mut dyn Write, value: &Value, language: Language) -> Result<()> {
    if let Some(error) = value["backend_error"].as_str() {
        field_in(
            out,
            language.text("内核", "Core"),
            language.text("状态读取失败", "Status unavailable"),
            language,
        )?;
        field_in(
            out,
            language.text("失败原因", "Error"),
            &clean(error),
            language,
        )?;
    } else {
        let mut state = vec![text_in(value, "running", language)];
        for key in ["owner", "version"] {
            if !value[key].is_null() {
                state.push(text_in(value, key, language));
            }
        }
        field_in(
            out,
            language.text("内核", "Core"),
            &state.join(" · "),
            language,
        )?;
    }
    if value["running"] != true {
        return network_notice(out, &value["host_routes"], language);
    }
    let proxy = text_in(value, "proxy", language);
    let mode = text_in(value, "mode", language);
    let proxy_status = match language {
        Language::SimplifiedChinese => format!("{proxy} · {mode}模式"),
        Language::English => format!("{proxy} · {mode} mode"),
    };
    field_in(out, language.text("代理", "Proxy"), &proxy_status, language)?;
    if !value["profile"].is_null() {
        field_in(
            out,
            label_in("profile", language),
            &text_in(value, "profile", language),
            language,
        )?;
    }
    let route = value["route"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(clean)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty());
    if let Some(route) = route {
        let name = if value["protected"] == true {
            language.text("保护节点", "Protected node")
        } else if value["mode"] == "rule" {
            language.text("规则参考线路", "Reference route")
        } else {
            language.text("当前线路", "Current route")
        };
        field_in(out, name, &route.join(" → "), language)?;
        if value["mode"] == "rule" && value["protected"] != true {
            message(
                out,
                language.text(
                    "实际连接可能使用其他规则；可用 cor connection list 查看",
                    "Other rules may route connections differently; run cor connection list",
                ),
            )?;
        }
    } else if !value["node"].is_null() {
        field_in(
            out,
            label_in("node", language),
            &text_in(value, "node", language),
            language,
        )?;
    }
    if !value["protected"].is_null() {
        field_in(
            out,
            language.text("Claude 保护", "Claude protection"),
            &text_in(value, "protected", language),
            language,
        )?;
    }
    let traffic = match language {
        Language::SimplifiedChinese => format!(
            "下载 {} · 上传 {} · {} 个连接",
            text_in(value, "download_rate", language),
            text_in(value, "upload_rate", language),
            text_in(value, "connections", language)
        ),
        Language::English => format!(
            "Download {} · Upload {} · {} {}",
            text_in(value, "download_rate", language),
            text_in(value, "upload_rate", language),
            text_in(value, "connections", language),
            if value["connections"].as_u64() == Some(1) {
                "connection"
            } else {
                "connections"
            },
        ),
    };
    field_in(out, language.text("流量", "Traffic"), &traffic, language)?;
    network_notice(out, &value["host_routes"], language)
}

fn network_notice(out: &mut dyn Write, value: &Value, language: Language) -> Result<()> {
    if let Some(notice) = crate::host_network::notice_in(value, language) {
        writeln!(out)?;
        field_in(
            out,
            language.text("网络提醒", "Network notice"),
            &clean(&notice),
            language,
        )?;
        return field_in(
            out,
            language.text("下一步", "Next"),
            language.text(
                "导出报告：cor diagnose --output cor-diagnose.json",
                "Export diagnostics: cor diagnose --output cor-diagnose.json",
            ),
            language,
        );
    }
    if let Some(summary) = crate::host_network::holder_summary_in(value, language) {
        field_in(
            out,
            language.text("TUN 持有进程", "TUN holders"),
            &clean(&summary),
            language,
        )?;
    }
    Ok(())
}

fn diagnose(out: &mut dyn Write, value: &Value, language: Language) -> Result<()> {
    use clash_client_core::error_message::localize_diagnostic;

    if value["offline"] == true {
        for (chinese, english, key) in [
            ("客户端版本", "Client version", "version"),
            ("采集时间", "Captured", "captured_at"),
            ("程序路径", "Executable", "executable"),
            ("数据目录", "Data directory", "data_directory"),
        ] {
            field_in(
                out,
                language.text(chinese, english),
                &text_in(value, key, language),
                language,
            )?;
        }
        let log = &value["local_evidence"]["daemon_log"];
        let log_state = match log["state"].as_str() {
            Some("collected") => language.text("已采集", "Collected").to_owned(),
            Some("missing") => language.text("未找到", "Not found").to_owned(),
            Some("withheld") => language.text("已隐藏", "Withheld").to_owned(),
            Some("unavailable") => language.text("不可用", "Unavailable").to_owned(),
            _ => text_in(log, "state", language),
        };
        field_in(
            out,
            language.text("后台日志采集", "Background log capture"),
            &log_state,
            language,
        )?;
        network_notice(out, &value["host_routes"], language)?;
        return message(
            out,
            language.text(
                "仅收集本地现场，未检测网络或连接后台；用 --output cor-diagnose.json 保存完整报告。",
                "Local evidence only; no network tests or background connection. Save the full report with --output cor-diagnose.json.",
            ),
        );
    }
    field_in(
        out,
        language.text("DNS 检测域名", "DNS test domain"),
        &text_in(value, "dns_host", language),
        language,
    )?;
    for (chinese, english, key) in [
        ("系统 DNS", "System DNS", "system_dns"),
        ("当前内核 DNS", "Current core DNS", "core_dns"),
    ] {
        let result = &value[key];
        let description = if let Some(answers) = result["Ok"].as_array() {
            answers
                .iter()
                .filter_map(Value::as_str)
                .map(clean)
                .collect::<Vec<_>>()
                .join(language.text("、", ", "))
        } else if let Some(error) = result["Err"].as_str() {
            format!(
                "{}{}",
                language.text("失败：", "Failed: "),
                clean(&localize_diagnostic(error, language))
            )
        } else {
            language.text("未检测", "Not checked").to_owned()
        };
        field_in(out, language.text(chinese, english), &description, language)?;
    }
    for (chinese, english, key) in [
        (
            "系统路由 HTTP（不使用代理环境变量）",
            "System route HTTP (without proxy environment variables)",
            "system_route_http",
        ),
        ("本地代理 HTTP", "Local proxy HTTP", "http"),
        ("代理 IPv4 出口", "Proxy IPv4 exit", "ipv4"),
        ("代理 IPv6 出口", "Proxy IPv6 exit", "ipv6"),
    ] {
        let probe = &value[key];
        let status = &probe["status"];
        let description = if let Some(error) = probe["error"].as_str() {
            format!(
                "{}{}",
                language.text("失败：", "Failed: "),
                clean(&localize_diagnostic(error, language))
            )
        } else if status.is_number() {
            let detail = text_in(probe, "detail", language);
            let detail = if language == Language::English
                && detail == format!("HTTP {status} · 响应头耗时，不是 ICMP 延迟")
            {
                format!("HTTP {status} · Response header time, not ICMP latency")
            } else {
                detail
            };
            format!(
                "{} · {detail} · {}",
                text_in(probe, "url", language),
                text_in(probe, "millis", language)
            )
        } else {
            language.text("未检测", "Not checked").to_owned()
        };
        field_in(out, language.text(chinese, english), &description, language)?;
    }
    if let Some(error) = value["backend_error"].as_str() {
        field_in(
            out,
            language.text("后台", "Background"),
            &clean(&localize_diagnostic(error, language)),
            language,
        )?;
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
    let configured = configured.join(language.text("、", ", "));
    field_in(
        out,
        language.text("终端代理环境变量", "Terminal proxy environment variables"),
        if configured.is_empty() {
            language.text("未检测到常见代理变量", "None detected")
        } else {
            &configured
        },
        language,
    )?;
    if value["tun_configuration"]["state"] == "observed" {
        field_in(
            out,
            language.text("TUN 配置", "TUN configuration"),
            &tun_configuration(&value["tun_configuration"], language),
            language,
        )?;
    }
    network_notice(out, &value["host_routes"], language)
}

fn tun_configuration(value: &Value, language: Language) -> String {
    let mut parts = Vec::new();
    for (chinese, english, key) in [
        ("自动路由", "Auto route", "auto_route"),
        ("严格路由", "Strict route", "strict_route"),
        ("DNS 劫持", "DNS hijack", "dns_hijack"),
        ("IPv4 路由段", "IPv4 route ranges", "inet4_route_address"),
        ("IPv6 路由段", "IPv6 route ranges", "inet6_route_address"),
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
                .join(language.text("、", ", ")),
            Value::String(value) => clean(value),
            _ => value.to_string(),
        };
        if !text.is_empty() {
            parts.push(format!("{}={text}", language.text(chinese, english)));
        }
    }
    if parts.is_empty() {
        language
            .text("已读取，但未声明覆盖限制", "Read; no route limits declared")
            .into()
    } else {
        parts.join(" · ")
    }
}

fn message(out: &mut dyn Write, message: &str) -> Result<()> {
    writeln!(out, "{message}")?;
    Ok(())
}
fn field_in(out: &mut dyn Write, key: &str, value: &str, language: Language) -> Result<()> {
    writeln!(out, "{key}{}{value}", language.text("：", ": "))?;
    Ok(())
}
fn rows(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

pub(super) fn download_progress(
    progress: &clash_client_core::update::Progress,
    language: Language,
) -> String {
    let route = clean(&progress.routing_in(language));
    match language {
        Language::SimplifiedChinese => format!(
            "下载：{} / {} · 速度：{}/s · 线路：{route}",
            fields::bytes(progress.received),
            fields::bytes(progress.total),
            fields::bytes(progress.bytes_per_second)
        ),
        Language::English => format!(
            "Download: {} / {} · Speed: {}/s · Route: {route}",
            fields::bytes(progress.received),
            fields::bytes(progress.total),
            fields::bytes(progress.bytes_per_second)
        ),
    }
}

fn tree(
    out: &mut dyn Write,
    value: &Value,
    depth: usize,
    settings: bool,
    language: Language,
    context: Option<&str>,
) -> Result<()> {
    let indent = "  ".repeat(depth);
    match value {
        Value::Object(fields) => {
            if fields.is_empty() {
                writeln!(out, "{indent}{}", language.text("无记录", "No records"))?;
            }
            let signal = (language == Language::English && context == Some("signals"))
                .then(|| {
                    serde_json::from_value::<clash_client_core::claude_check::Signal>(value.clone())
                        .ok()
                })
                .flatten();
            let route = (language == Language::English && context == Some("routes"))
                .then(|| {
                    serde_json::from_value::<clash_client_core::claude_check::Route>(value.clone())
                        .ok()
                })
                .flatten();
            for (key, value) in fields {
                if key == "schema" || key == "event" {
                    continue;
                }
                let name = if settings {
                    match language {
                        Language::SimplifiedChinese => format!("{}（{}）", label(key), clean(key)),
                        Language::English => {
                            format!("{} ({})", label_in(key, language), clean(key))
                        }
                    }
                } else {
                    clean(label_in(key, language))
                };
                if value.is_object() || value.is_array() {
                    writeln!(out, "{indent}{name}{}", language.text("：", ":"))?;
                    tree(out, value, depth + 1, false, language, Some(key))?;
                } else {
                    let shown = match (&signal, &route, key.as_str()) {
                        (Some(signal), _, "title") => signal.display_title_in(language),
                        (Some(signal), _, "value") => signal.display_value_in(language),
                        (Some(signal), _, "source") => signal.source_in(language),
                        (Some(signal), _, "detail") => signal.detail_in(language),
                        (Some(signal), _, "advice") => signal.advice_in(language).to_owned(),
                        (_, Some(route), "outcome")
                            if value
                                .as_str()
                                .is_some_and(|outcome| !outcome.trim().is_empty()) =>
                        {
                            route.recorded_outcome_in(language).to_owned()
                        }
                        _ => scalar_in(key, value, language),
                    };
                    writeln!(
                        out,
                        "{indent}{name}{}{}",
                        language.text("：", ": "),
                        clean(&shown)
                    )?;
                }
            }
        }
        Value::Array(values) => {
            if values.is_empty() {
                writeln!(out, "{indent}{}", language.text("无记录", "No records"))?;
            }
            for (index, value) in values.iter().enumerate() {
                if value.is_object() {
                    match language {
                        Language::SimplifiedChinese => {
                            writeln!(out, "{indent}第 {} 项", index + 1)?
                        }
                        Language::English => writeln!(out, "{indent}Item {}", index + 1)?,
                    }
                    tree(out, value, depth + 1, false, language, context)?;
                } else {
                    writeln!(out, "{indent}- {}", scalar_in("", value, language))?;
                }
            }
        }
        Value::String(s) if depth == 0 => writeln!(out, "{s}")?,
        _ => writeln!(out, "{indent}{}", scalar_in("", value, language))?,
    }
    Ok(())
}
