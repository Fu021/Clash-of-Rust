use anyhow::{Context, Result, ensure};
use clash_mobile_runtime::core::config::Language;
use std::sync::{
    Arc,
    atomic::{AtomicI32, Ordering},
};

static NEXT_ID: AtomicI32 = AtomicI32::new(1);

fn next_id() -> Option<i32> {
    NEXT_ID
        .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            (current > 0).then(|| current.checked_add(1)).flatten()
        })
        .ok()
}

#[derive(Clone, Default)]
pub(crate) struct Fields {
    pub name: String,
    pub source: String,
    pub dirty: bool,
}

pub(crate) enum Outcome {
    Selected { content: Arc<str> },
    Cancelled,
    Failed(String),
}

pub(crate) enum Settlement {
    Selected,
    Cancelled,
    Failed(String),
    Blocked,
}

struct Selection {
    request: i32,
    outcome: Option<Outcome>,
}

pub(crate) struct Draft {
    pub id: i32,
    pub fields: Fields,
    pub content: Option<Arc<str>>,
    selection: Option<Selection>,
}

#[derive(Default)]
pub(crate) struct ProfilePicker {
    draft: Option<Draft>,
}

impl ProfilePicker {
    pub fn open(&mut self, source: String) {
        let id = next_id().expect("profile picker sequence");
        self.draft = Some(Draft {
            id,
            fields: Fields {
                source,
                ..Fields::default()
            },
            content: None,
            selection: None,
        });
    }

    pub fn draft(&self) -> Option<&Draft> {
        self.draft.as_ref()
    }

    pub fn edit(&mut self, fields: Fields) {
        if let Some(draft) = &mut self.draft {
            draft.fields = fields;
        }
    }

    pub fn discard(&mut self) {
        self.draft = None;
    }

    pub fn clear_file(&mut self) {
        if let Some(draft) = &mut self.draft
            && draft.selection.is_none()
        {
            draft.content = None;
        }
    }

    pub fn save_content(&self, source: &str, language: Language) -> Result<Option<&str>> {
        let draft = self.draft().context(language.text(
            "请先打开添加订阅表单",
            "Open the add subscription form first",
        ))?;
        ensure!(
            draft.selection.is_none(),
            "{}",
            language.text("请等待文件选择完成", "Wait for file selection to finish")
        );
        let content = draft.content.as_deref();
        ensure!(
            content.is_none() || source.trim().is_empty(),
            "{}",
            language.text(
                "请选择本地文件或输入订阅链接，两者只能选一个",
                "Choose either a local file or a subscription URL"
            )
        );
        Ok(content)
    }

    pub fn saved(&mut self, id: i32) {
        if self.draft().is_some_and(|draft| draft.id == id) {
            self.discard();
        }
    }

    pub fn begin(&mut self) -> Option<i32> {
        let draft = self.draft.as_mut()?;
        if draft.selection.is_some() {
            return None;
        }
        let request = next_id()?;
        draft.selection = Some(Selection {
            request,
            outcome: None,
        });
        Some(request)
    }

    pub fn pending(&self) -> Option<i32> {
        Some(self.draft()?.selection.as_ref()?.request)
    }

    pub fn complete(&mut self, request: i32, outcome: Outcome) -> bool {
        if let Some(selection) = self
            .draft
            .as_mut()
            .and_then(|draft| draft.selection.as_mut())
            && selection.request == request
            && selection.outcome.is_none()
        {
            selection.outcome = Some(outcome);
            return true;
        }
        false
    }

    pub fn result(&self) -> Option<(i32, &Outcome)> {
        let selection = self.draft()?.selection.as_ref()?;
        Some((selection.request, selection.outcome.as_ref()?))
    }

    pub fn settle(&mut self, request: i32, selected_name: Option<String>) -> Option<Settlement> {
        if self.result()?.0 != request {
            return None;
        }
        let draft = self.draft.as_mut()?;
        let outcome = draft.selection.take()?.outcome?;
        Some(match outcome {
            Outcome::Selected { content } => match selected_name {
                Some(name) => {
                    draft.content = Some(content);
                    draft.fields = Fields {
                        name,
                        source: String::new(),
                        dirty: true,
                    };
                    Settlement::Selected
                }
                None => Settlement::Blocked,
            },
            Outcome::Cancelled => Settlement::Cancelled,
            Outcome::Failed(error) => Settlement::Failed(error),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(content: &str) -> Outcome {
        Outcome::Selected {
            content: content.into(),
        }
    }

    #[test]
    fn an_abandoned_picker_cannot_complete_a_new_draft() {
        let mut picker = ProfilePicker::default();
        picker.open("https://example.com/old".into());
        let first = picker.begin().unwrap();
        assert!(picker.begin().is_none());
        picker.discard();
        assert!(!picker.complete(first, file("stale")));

        picker.open("https://example.com/new".into());
        let second = picker.begin().unwrap();
        assert_ne!(first, second);
        assert!(!picker.complete(first, file("stale")));
        assert!(picker.complete(second, file("current")));
        assert!(!picker.complete(second, Outcome::Cancelled));
        assert!(picker.settle(first, Some("old.yaml".into())).is_none());
        assert_eq!(picker.pending(), Some(second));
        assert!(matches!(
            picker.settle(second, Some("new.yaml".into())),
            Some(Settlement::Selected)
        ));
        let draft = picker.draft().unwrap();
        assert_eq!(draft.fields.name, "new.yaml");
        assert_eq!(draft.content.as_deref(), Some("current"));
    }

    #[test]
    fn cancel_and_failure_preserve_the_selected_content_and_edited_fields() {
        let mut picker = ProfilePicker::default();
        picker.open(String::new());
        let initial = picker.begin().unwrap();
        assert!(picker.complete(initial, file("initial")));
        picker.settle(initial, Some("initial.yaml".into()));
        picker.edit(Fields {
            name: "renamed".into(),
            source: "https://example.com/draft".into(),
            dirty: true,
        });
        for outcome in [Outcome::Cancelled, Outcome::Failed("read failed".into())] {
            let request = picker.begin().unwrap();
            assert!(picker.complete(request, outcome));
            assert!(picker.settle(request, None).is_some());
            assert!(picker.pending().is_none());
            let draft = picker.draft().unwrap();
            assert_eq!(draft.fields.name, "renamed");
            assert_eq!(draft.fields.source, "https://example.com/draft");
            assert_eq!(draft.content.as_deref(), Some("initial"));
            assert!(draft.fields.dirty);
        }
    }

    #[test]
    fn recreation_can_read_the_draft_and_deliver_a_result_once() {
        let mut picker = ProfilePicker::default();
        picker.open("https://example.com/source".into());
        let request = picker.begin().unwrap();
        assert!(picker.complete(request, file("yaml")));
        let restored = picker.draft().unwrap();
        assert_eq!(restored.fields.source, "https://example.com/source");
        assert_eq!(picker.result().unwrap().0, request);
        let id = restored.id;
        picker.settle(request, Some("received (2).yaml".into()));
        assert_eq!(picker.draft().unwrap().fields.name, "received (2).yaml");
        assert!(picker.result().is_none());
        assert!(!picker.complete(request, file("duplicate")));
        picker.saved(id);
        assert!(picker.draft().is_none());
    }

    #[test]
    fn a_new_picker_never_reuses_an_abandoned_request() {
        let mut first = ProfilePicker::default();
        first.open(String::new());
        let abandoned = first.begin().unwrap();
        first.discard();

        let mut replacement = ProfilePicker::default();
        replacement.open(String::new());
        let current = replacement.begin().unwrap();

        assert!(current > abandoned);
        assert!(!replacement.complete(abandoned, file("stale")));
        assert!(replacement.complete(current, file("current")));
    }

    #[test]
    fn import_cannot_save_a_url_with_selected_file_bytes() {
        let mut picker = ProfilePicker::default();
        let url = "https://example.test/subscription";
        let language = Language::English;
        assert!(picker.save_content(url, language).is_err());
        picker.open(url.into());
        assert_eq!(picker.save_content(url, language).unwrap(), None);

        let request = picker.begin().unwrap();
        assert!(picker.save_content(url, language).is_err());
        assert!(picker.complete(request, file("local yaml")));
        picker.settle(request, Some("local.yaml".into()));
        assert!(picker.save_content(url, language).is_err());
        assert_eq!(
            picker.save_content(" \t", language).unwrap(),
            Some("local yaml")
        );

        picker.clear_file();
        assert_eq!(picker.save_content(url, language).unwrap(), None);
        assert!(!picker.complete(request, file("late file")));
        assert_eq!(picker.draft().unwrap().fields.name, "local.yaml");
    }
}
