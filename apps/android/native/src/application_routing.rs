use super::{AndroidData, AndroidIntent, AndroidWindow, ApplicationItem, MobileData, MobileView};
use crate::application_routing_mode::selection_after_mode_change;
use anyhow::{Context, Result, bail};
use clash_mobile_runtime::NativeHost;
use serde::Deserialize;
use serde_json::json;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

#[derive(Default, Deserialize)]
struct Application {
    name: String,
    package: String,
    system: bool,
    launchable: bool,
    #[serde(default)]
    icon: String,
}

#[derive(Default, Deserialize)]
struct Catalog {
    applications: Vec<Application>,
    mode: String,
    selected: BTreeSet<String>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    visibility_limited: bool,
    #[serde(default)]
    query_permission: bool,
    #[serde(default)]
    icon_failures: usize,
}

#[derive(Default)]
pub(super) struct Editor {
    catalog: Catalog,
    query: String,
    show_system: bool,
    filter: usize,
    saved_mode: String,
    saved_selected: BTreeSet<String>,
    generation: u64,
    loaded: bool,
}

impl Editor {
    fn visible(&self, item: &Application) -> bool {
        let query = self.query.trim().to_lowercase();
        let selected = self.catalog.selected.contains(&item.package);
        (self.filter != 1 || selected)
            && (self.filter != 2 || !selected)
            && (self.show_system || item.launchable || !item.system || selected)
            && (query.is_empty()
                || item.name.to_lowercase().contains(&query)
                || item.package.to_lowercase().contains(&query))
    }

    fn present(&self, ui: &AndroidWindow) {
        let data = ui.global::<AndroidData>();
        let items = self
            .catalog
            .applications
            .iter()
            .filter(|item| self.catalog.mode != "all" && self.visible(item))
            .map(|item| {
                let icon = if item.icon.is_empty() {
                    None
                } else {
                    slint::Image::load_from_path(std::path::Path::new(&item.icon)).ok()
                };
                ApplicationItem {
                    name: item.name.as_str().into(),
                    package: item.package.as_str().into(),
                    selected: self.catalog.selected.contains(&item.package),
                    has_icon: icon.is_some(),
                    icon: icon.unwrap_or_default(),
                }
            })
            .collect::<Vec<_>>();
        data.set_applications(ModelRc::new(VecModel::from(items)));
        data.set_applications_loaded(self.loaded);
        data.set_applications_total(self.catalog.applications.len() as i32);
        data.set_applications_selected(self.catalog.selected.len() as i32);
        data.set_applications_saved_summary(self.catalog.summary.as_str().into());
        data.set_applications_visibility_limited(self.catalog.visibility_limited);
        data.set_applications_query_permission(self.catalog.query_permission);
        data.set_applications_icon_failures(self.catalog.icon_failures as i32);
        data.set_applications_mode(match self.catalog.mode.as_str() {
            "only" => 1,
            "exclude" => 2,
            _ => 0,
        });
        data.set_applications_query(self.query.as_str().into());
        data.set_applications_system(self.show_system);
        data.set_applications_filter(self.filter as i32);
        if self.loaded && ui.global::<MobileView>().get_detail() == 18 {
            ui.global::<MobileView>().set_draft_dirty(
                self.catalog.mode != self.saved_mode
                    || self.catalog.selected != self.saved_selected,
            );
        }
    }
}

pub(super) fn handles(intent: AndroidIntent) -> bool {
    matches!(
        intent,
        AndroidIntent::ApplicationRouting
            | AndroidIntent::ApplicationQuery
            | AndroidIntent::ApplicationSystem
            | AndroidIntent::ApplicationFilter
            | AndroidIntent::ApplicationMode
            | AndroidIntent::ApplicationToggle
            | AndroidIntent::ApplicationSelect
            | AndroidIntent::ApplicationRevert
            | AndroidIntent::ApplicationSave
            | AndroidIntent::ApplicationRefresh
            | AndroidIntent::ApplicationPermissions
    )
}

pub(super) fn request(
    editor: &Arc<Mutex<Editor>>,
    host: Arc<dyn NativeHost>,
    ui: &AndroidWindow,
    intent: AndroidIntent,
    value: &str,
) -> Result<()> {
    let data = ui.global::<AndroidData>();
    if (ui.global::<MobileData>().get_protected()
        || ui.global::<MobileData>().get_protection_pending())
        && !matches!(
            intent,
            AndroidIntent::ApplicationRouting
                | AndroidIntent::ApplicationRefresh
                | AndroidIntent::ApplicationQuery
                | AndroidIntent::ApplicationSystem
                | AndroidIntent::ApplicationFilter
                | AndroidIntent::ApplicationPermissions
                | AndroidIntent::ApplicationRevert
        )
    {
        bail!("请先关闭 Claude 保护，再修改应用分流");
    }
    if data.get_applications_loading() || data.get_applications_saving() {
        return Ok(());
    }
    let mut state = editor.lock().map_err(|_| anyhow::anyhow!("应用列表忙碌"))?;
    if intent == AndroidIntent::ApplicationPermissions {
        host.request(json!({"op":"applications-permissions"}))?;
        return Ok(());
    }
    let opening = intent == AndroidIntent::ApplicationRouting;
    if opening || intent == AndroidIntent::ApplicationRefresh {
        if opening {
            *state = Editor {
                generation: state.generation,
                ..Default::default()
            };
            ui.global::<MobileView>().set_detail(18);
            ui.global::<MobileView>().set_draft_dirty(false);
        }
        state.generation += 1;
        let generation = state.generation;
        state.present(ui);
        data.set_applications_error("".into());
        data.set_applications_loading(true);
        let weak = ui.as_weak();
        let editor = editor.clone();
        std::thread::Builder::new()
            .name("application-catalog".into())
            .spawn(move || {
                let result = host
                    .request(json!({"op":"applications-list"}))
                    .and_then(|value| Ok(serde_json::from_value::<Catalog>(value)?));
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else {
                        return;
                    };
                    let Ok(mut state) = editor.lock() else {
                        return;
                    };
                    if state.generation != generation {
                        return;
                    }
                    let data = ui.global::<AndroidData>();
                    data.set_applications_loading(false);
                    match result {
                        Ok(catalog) => {
                            state.saved_mode = catalog.mode.clone();
                            state.saved_selected = catalog.selected.clone();
                            if !state.loaded {
                                state.catalog = catalog;
                            } else {
                                state.catalog.applications = catalog.applications;
                                state.catalog.summary = catalog.summary;
                                state.catalog.visibility_limited = catalog.visibility_limited;
                                state.catalog.query_permission = catalog.query_permission;
                                state.catalog.icon_failures = catalog.icon_failures;
                            }
                            state.loaded = true;
                        }
                        Err(error) => data.set_applications_error(
                            format!("读取应用列表失败：{error:#}。可重试或导出诊断").into(),
                        ),
                    }
                    state.present(&ui);
                });
            })
            .inspect_err(|_| data.set_applications_loading(false))?;
        return Ok(());
    }
    if !state.loaded {
        return Ok(());
    }
    if intent == AndroidIntent::ApplicationSave {
        if state.catalog.mode == "only" && state.catalog.selected.is_empty() {
            bail!("请至少选择一个应用");
        }
        let input = json!({
            "op":"applications-save", "mode":state.catalog.mode,
            "selected":state.catalog.selected
        });
        let generation = state.generation;
        data.set_applications_error("".into());
        data.set_applications_saving(true);
        let weak = ui.as_weak();
        let editor = editor.clone();
        std::thread::Builder::new()
            .name("application-routing-save".into())
            .spawn(move || {
                let result = host.request(input);
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else {
                        return;
                    };
                    let Ok(mut state) = editor.lock() else {
                        return;
                    };
                    if state.generation != generation {
                        return;
                    }
                    let data = ui.global::<AndroidData>();
                    data.set_applications_saving(false);
                    match result {
                        Ok(result) => {
                            state.catalog.summary =
                                result["summary"].as_str().unwrap_or("已保存").to_string();
                            state.saved_mode = state.catalog.mode.clone();
                            state.saved_selected = state.catalog.selected.clone();
                            state.present(&ui);
                            data.set_application_scope(
                                result["summary"].as_str().unwrap_or("已保存").into(),
                            );
                            let view = ui.global::<MobileView>();
                            if let Some(error) = result["reconnect_error"].as_str().filter(|value| !value.is_empty()) {
                                data.set_applications_error(
                                    format!("应用分流已保存，但 VPN 重连失败：{error}。请重新连接或导出诊断").into(),
                                );
                                return;
                            }
                            if !view.get_notice_error() || view.get_notice().is_empty() {
                                view.set_notice_error(false);
                                view.set_notice_critical(false);
                                ui.invoke_notify(
                                    if result["reconnecting"].as_bool().unwrap_or(false) {
                                        "应用分流已保存，正在重新连接 VPN"
                                    } else {
                                        "应用分流已保存，下次连接时生效"
                                    }
                                    .into(),
                                );
                            }
                        }
                        Err(error) => data.set_applications_error(
                            format!("保存失败：{error:#}。可重试或导出诊断").into(),
                        ),
                    }
                });
            })
            .inspect_err(|_| data.set_applications_saving(false))?;
        return Ok(());
    }
    let changes_selection = match intent {
        AndroidIntent::ApplicationRevert => {
            state.catalog.mode = state.saved_mode.clone();
            state.catalog.selected = state.saved_selected.clone();
            if state.catalog.mode == "all" {
                state.filter = 0;
            }
            true
        }
        AndroidIntent::ApplicationQuery => {
            state.query = value.into();
            false
        }
        AndroidIntent::ApplicationSystem => {
            state.show_system = value == "true";
            false
        }
        AndroidIntent::ApplicationFilter => {
            let filter = value.parse::<usize>()?;
            if filter > 2 {
                bail!("无法识别应用筛选条件");
            }
            state.filter = filter;
            false
        }
        AndroidIntent::ApplicationMode => {
            let next = *["all", "only", "exclude"]
                .get(value.parse::<usize>()?)
                .context("无法识别应用分流模式")?;
            let current = state.catalog.mode.as_str();
            if current == next {
                return Ok(());
            }
            let needs_complete_catalog =
                next == "only" || matches!((current, next), ("only", "exclude"));
            if needs_complete_catalog
                && (state.catalog.visibility_limited || !state.catalog.query_permission)
            {
                bail!("应用列表不完整，无法自动反选；请检查应用信息中的权限或设备限制，再刷新列表");
            }
            let available = state
                .catalog
                .applications
                .iter()
                .map(|item| item.package.clone())
                .collect();
            state.catalog.selected =
                selection_after_mode_change(current, next, &state.catalog.selected, &available);
            state.catalog.mode = next.to_string();
            state.filter = 0;
            true
        }
        AndroidIntent::ApplicationToggle => {
            if state.catalog.mode == "all" {
                bail!("全部应用模式无需勾选；请先选择仅选中或排除选中");
            }
            if !state
                .catalog
                .applications
                .iter()
                .any(|item| item.package == value)
            {
                bail!("应用不在当前列表，请刷新后重试");
            }
            if !state.catalog.selected.remove(value) {
                state.catalog.selected.insert(value.into());
            }
            true
        }
        AndroidIntent::ApplicationSelect => {
            if state.catalog.mode == "all" {
                bail!("全部应用模式无需勾选；请先选择仅选中或排除选中");
            }
            if !["all", "invert", "clear"].contains(&value) {
                bail!("无法识别应用选择操作");
            }
            if value == "clear" {
                state.catalog.selected.clear();
            } else {
                let visible = state
                    .catalog
                    .applications
                    .iter()
                    .filter(|item| state.visible(item))
                    .map(|item| item.package.clone())
                    .collect::<Vec<_>>();
                for package in visible {
                    if value == "all" || !state.catalog.selected.remove(&package) {
                        state.catalog.selected.insert(package);
                    }
                }
            }
            true
        }
        _ => false,
    };
    if changes_selection {
        ui.global::<MobileView>().set_draft_dirty(true);
    }
    data.set_applications_error("".into());
    state.present(ui);
    Ok(())
}
