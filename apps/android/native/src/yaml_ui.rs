use super::{AndroidWindow, Session, Update, YamlDocument, YamlHighlight, fonts, language_ui};
use clash_mobile_runtime::core as yaml_core;
use clash_mobile_runtime::core::config::{
    documents::{Action, Failure, Operation, Reply},
    overrides,
};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};
use tokio::sync::{Semaphore, mpsc, watch};

static SOURCE_SLOT: Semaphore = Semaphore::const_new(1);

#[path = "../../../../crates/client-ui/shared/yaml_document.rs"]
mod document;

#[derive(Default)]
struct Pending(RefCell<Option<watch::Sender<bool>>>);

impl Pending {
    fn cancel(&self) {
        if let Some(cancel) = self.0.borrow_mut().take() {
            cancel.send_replace(true);
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(super) struct Request {
    id: i32,
    action: Action,
    cancel: watch::Receiver<bool>,
}

pub(super) struct Outcome {
    id: i32,
    operation: Operation,
    cancel: watch::Receiver<bool>,
    result: anyhow::Result<Reply>,
}

impl Outcome {
    pub(super) fn finish(
        self,
        weak: &slint::Weak<AndroidWindow>,
    ) -> (bool, Option<(Failure, watch::Receiver<bool>)>) {
        let Self {
            id,
            operation,
            cancel,
            result,
        } = self;
        let saved = matches!(&result, Ok(Reply::Saved(_)));
        let failure = result
            .as_ref()
            .err()
            .map(|error| Failure::new(operation, error));
        if !*cancel.borrow() {
            let pending = cancel.clone();
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if !*pending.borrow() {
                    document::respond(
                        &ui.global::<YamlDocument>(),
                        id,
                        result.map_err(|error| format!("{error:#}")),
                        language_ui::current(&ui),
                    );
                }
            });
        }
        let notice = failure
            .filter(|failure| failure.critical() || !*cancel.borrow())
            .map(|failure| (failure, cancel));
        (saved, notice)
    }
}

fn submit(
    ui: &AndroidWindow,
    pending: &Pending,
    sender: &mpsc::Sender<Request>,
    action: anyhow::Result<Action>,
) {
    let data = ui.global::<YamlDocument>();
    let id = data.get_request_id();
    pending.cancel();
    let result = action.and_then(|action| {
        let (cancel, cancelled) = watch::channel(false);
        sender
            .try_send(Request {
                id,
                action,
                cancel: cancelled,
            })
            .map_err(|error| {
                anyhow::anyhow!(match error {
                    mpsc::error::TrySendError::Full(_) => "操作过于频繁，请稍后再试",
                    mpsc::error::TrySendError::Closed(_) => "客户端后台未就绪",
                })
            })?;
        pending.0.replace(Some(cancel));
        Ok(())
    });
    if let Err(error) = result {
        document::respond(&data, id, Err(error.to_string()), language_ui::current(ui));
    }
}

pub(super) fn bind(ui: &AndroidWindow) -> mpsc::Receiver<Request> {
    let (sender, receive) = mpsc::channel(4);
    let pending = Rc::new(Pending::default());
    let data = ui.global::<YamlDocument>();
    data.set_font_family(fonts::editor_font());
    data.on_highlight_lines(document::highlight_lines);
    data.on_valid_query(|query| yaml_core::config::source_view::validate_query(&query).is_ok());
    let weak = ui.as_weak();
    let load_sender = sender.clone();
    let load_pending = pending.clone();
    data.on_load(move |kind, value| {
        if let Some(ui) = weak.upgrade() {
            let action = document::load(&ui.global::<YamlDocument>(), &kind, &value);
            submit(&ui, &load_pending, &load_sender, action);
        }
    });
    let weak = ui.as_weak();
    let write_pending = pending.clone();
    data.on_write(move |apply| {
        if let Some(ui) = weak.upgrade() {
            let action = document::write(&ui.global::<YamlDocument>(), apply);
            submit(&ui, &write_pending, &sender, action);
        }
    });
    data.on_cancel(move || pending.cancel());
    let weak = ui.as_weak();
    data.on_edited(move || {
        if let Some(ui) = weak.upgrade() {
            document::edited(&ui.global::<YamlDocument>());
        }
    });
    let weak = ui.as_weak();
    data.on_locate(move |offset| {
        if let Some(ui) = weak.upgrade() {
            document::locate(&ui.global::<YamlDocument>(), offset);
        }
    });
    let weak = ui.as_weak();
    data.on_edit_position(move |kind, argument, anchor, cursor| {
        if let Some(ui) = weak.upgrade() {
            let data = ui.global::<YamlDocument>();
            if let Err(error) = document::edit_position(&data, &kind, &argument, anchor, cursor) {
                document::show_error(&data, &error.to_string(), language_ui::current(&ui));
            }
        }
    });
    let weak = ui.as_weak();
    data.on_indent(move || {
        if let Some(ui) = weak.upgrade() {
            document::indent(ui.window());
        }
    });
    receive
}

pub(super) fn run(
    session: &mut Session,
    request: Request,
    protection_pending: bool,
    updates: mpsc::Sender<Update>,
) {
    let Request { id, action, cancel } = request;
    if *cancel.borrow() {
        return;
    }
    let operation = action.operation();
    let mut source_cancel = cancel.clone();
    let deliver = move |result| async move {
        let _ = updates
            .send(Update::YamlFinished(Outcome {
                id,
                operation,
                cancel,
                result,
            }))
            .await;
    };
    let result = match action {
        Action::Source(request) => {
            let source = match session.subscription_source(request) {
                Ok(source) => source,
                Err(error) => {
                    tokio::spawn(deliver(Err(error)));
                    return;
                }
            };
            tokio::spawn(async move {
                let slot = tokio::select! {
                    biased;
                    _ = source_cancel.wait_for(|cancelled| *cancelled) => return,
                    slot = SOURCE_SLOT.acquire() => slot.expect("source semaphore is never closed"),
                };
                let result = tokio::task::spawn_blocking(move || {
                    let _slot = slot;
                    source.read(|| *source_cancel.borrow()).map(Reply::Source)
                })
                .await
                .unwrap_or_else(|error| Err(error.into()));
                deliver(result).await;
            });
            return;
        }
        Action::OpenOverride => session.global_override_document().map(Reply::Override),
        Action::Apply(_) | Action::Check(_) if protection_pending => {
            Err(anyhow::anyhow!("Claude 保护正在切换"))
        }
        Action::Check(change) => session.check_global_override(change).map(|()| Reply::Valid),
        Action::Apply(change) => {
            let revision = overrides::revision(&change.text);
            session
                .save_global_override(change)
                .map(|()| Reply::Saved(revision))
        }
        Action::OpenExternal(_) | Action::PollExternal(_) | Action::WriteExternal { .. } => {
            Err(anyhow::anyhow!("当前平台不支持外部编辑器"))
        }
    };
    tokio::spawn(deliver(result));
}
