use super::{AndroidData, AndroidIntent, AndroidWindow, ApplicationItem, MobileData, MobileView};
use anyhow::{Context, Result, bail};
use clash_mobile_runtime::NativeHost;
use serde::Deserialize;
use serde_json::{Value, json};
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
}

#[derive(Default, Deserialize)]
struct Catalog {
    applications: Vec<Application>,
    mode: String,
    selected: BTreeSet<String>,
}

#[derive(Default)]
pub(super) struct Editor {
    catalog: Catalog,
    query: String,
    show_system: bool,
    generation: u64,
    loaded: bool,
}

impl Editor {
    fn visible(&self, item: &Application) -> bool {
        let query = self.query.trim().to_lowercase();
        (self.show_system
            || item.launchable
            || !item.system
            || self.catalog.selected.contains(&item.package))
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
            .filter(|item| self.visible(item))
            .map(|item| ApplicationItem {
                name: item.name.as_str().into(),
                package: item.package.as_str().into(),
                selected: self.catalog.selected.contains(&item.package),
            })
            .collect::<Vec<_>>();
        data.set_applications(ModelRc::new(VecModel::from(items)));
        data.set_applications_loaded(self.loaded);
        data.set_applications_total(self.catalog.applications.len() as i32);
        data.set_applications_selected(self.catalog.selected.len() as i32);
        data.set_applications_mode(match self.catalog.mode.as_str() {
            "only" => 1,
            "exclude" => 2,
            _ => 0,
        });
        data.set_applications_query(self.query.as_str().into());
        data.set_applications_system(self.show_system);
    }
}

pub(super) fn handles(intent: AndroidIntent) -> bool {
    matches!(
        intent,
        AndroidIntent::ApplicationRouting
            | AndroidIntent::ApplicationQuery
            | AndroidIntent::ApplicationSystem
            | AndroidIntent::ApplicationMode
            | AndroidIntent::ApplicationToggle
            | AndroidIntent::ApplicationSelect
            | AndroidIntent::ApplicationSave
            | AndroidIntent::ApplicationRefresh
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
    if ui.global::<MobileData>().get_protected()
        || ui.global::<MobileData>().get_protection_pending()
    {
        bail!("请先关闭 Claude 保护，再修改应用分流");
    }
    if data.get_applications_loading() || data.get_applications_saving() {
        return Ok(());
    }
    let mut state = editor.lock().map_err(|_| anyhow::anyhow!("应用列表忙碌"))?;
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
                            if !state.loaded {
                                state.catalog = catalog;
                            } else {
                                state.catalog.applications = catalog.applications;
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
                    let Ok(state) = editor.lock() else {
                        return;
                    };
                    if state.generation != generation {
                        return;
                    }
                    let data = ui.global::<AndroidData>();
                    data.set_applications_saving(false);
                    match result {
                        Ok(result) => {
                            data.set_application_scope(
                                result["summary"].as_str().unwrap_or("已保存").into(),
                            );
                            let view = ui.global::<MobileView>();
                            view.set_draft_dirty(false);
                            if !view.get_notice_error() || view.get_notice().is_empty() {
                                view.set_notice_error(false);
                                view.set_notice_critical(false);
                                ui.invoke_notify(
                                    if result["connected"].as_bool().unwrap_or(false) {
                                        "应用分流已保存，重新连接 VPN 后生效"
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
        AndroidIntent::ApplicationQuery => {
            state.query = value.into();
            false
        }
        AndroidIntent::ApplicationSystem => {
            state.show_system = value == "true";
            false
        }
        AndroidIntent::ApplicationMode => {
            state.catalog.mode = ["all", "only", "exclude"]
                .get(value.parse::<usize>()?)
                .context("无法识别应用分流模式")?
                .to_string();
            true
        }
        AndroidIntent::ApplicationToggle => {
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
