use clash_mobile_runtime::core::{
    api::Connection,
    config::Language,
    connection::{detail_values_in, hostname},
};
use tokio::{sync::watch, task::JoinHandle};

pub(crate) const CONNECTIONS_PAGE: i32 = 3;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Selection {
    pub(crate) id: String,
    pub(crate) request: i32,
}

impl Selection {
    pub(crate) fn matches(&self, id: &str, request: i32) -> bool {
        self.id == id && self.request == request
    }
}

pub(crate) fn dismiss(visible: &watch::Sender<Option<Selection>>, id: &str, request: i32) {
    visible.send_if_modified(|current| {
        if current
            .as_ref()
            .is_some_and(|current| current.matches(id, request))
        {
            *current = None;
            true
        } else {
            false
        }
    });
}

pub(crate) fn discard_hidden(
    detail: &mut Option<Detail>,
    address_job: &mut Option<JoinHandle<()>>,
    visible: Option<&Selection>,
) {
    if detail
        .as_ref()
        .is_some_and(|detail| Some(&detail.selection) != visible)
    {
        *detail = None;
        if let Some(job) = address_job.take() {
            job.abort();
        }
    }
}

pub(crate) fn accept_address(
    detail: &mut Option<Detail>,
    address_job: &mut Option<JoinHandle<()>>,
    visible: Option<&Selection>,
    selection: &Selection,
    request: &AddressRequest,
    value: String,
) -> bool {
    if Some(selection) != visible
        || !detail
            .as_mut()
            .is_some_and(|detail| detail.record_address(selection, request, value))
    {
        return false;
    }
    *address_job = None;
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Unavailable,
    Loading,
    Available,
    Ended,
}

enum Observation {
    Unavailable,
    Loading,
    Available(Box<Connection>),
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AddressRequest {
    pub(crate) host: String,
    generation: u64,
}

struct Address {
    request: AddressRequest,
    value: String,
}

pub(crate) struct Detail {
    pub(crate) selection: Selection,
    observation: Observation,
    address: Option<Address>,
    address_generation: u64,
}

impl Detail {
    pub(crate) fn new(selection: Selection) -> Self {
        Self {
            selection,
            observation: Observation::Loading,
            address: None,
            address_generation: 0,
        }
    }

    pub(crate) fn status(&self) -> Status {
        match self.observation {
            Observation::Unavailable => Status::Unavailable,
            Observation::Loading => Status::Loading,
            Observation::Available(_) => Status::Available,
            Observation::Ended => Status::Ended,
        }
    }

    pub(crate) fn selection_for_refresh(&self, page: i32) -> Option<Selection> {
        (page == CONNECTIONS_PAGE && self.status() != Status::Ended).then(|| self.selection.clone())
    }

    pub(crate) fn confirm(&mut self, selection: &Selection, connections: &[Connection]) -> bool {
        if self.selection != *selection || self.status() == Status::Ended {
            return false;
        }
        self.observation = connections
            .iter()
            .find(|connection| connection.id.as_ref() == selection.id)
            .map_or(Observation::Ended, |connection| {
                Observation::Available(Box::new(connection.clone()))
            });
        if self.address_host().is_none() {
            self.address = None;
        }
        true
    }

    pub(crate) fn refresh_failed(&mut self, selection: &Selection) {
        if self.selection == *selection && self.status() == Status::Loading {
            self.observation = Observation::Unavailable;
        }
    }

    pub(crate) fn core_stopped(&mut self) {
        if self.status() != Status::Ended {
            self.observation = Observation::Unavailable;
            self.address = None;
        }
    }

    pub(crate) fn wait_for_close_confirmation(&mut self, id: &str) {
        if self.selection.id == id {
            self.wait_for_all_close_confirmation();
        }
    }

    pub(crate) fn wait_for_all_close_confirmation(&mut self) {
        if self.status() == Status::Available {
            self.observation = Observation::Loading;
            self.address = None;
        }
    }

    fn address_host(&self) -> Option<&str> {
        let Observation::Available(connection) = &self.observation else {
            return None;
        };
        let host = hostname(connection).trim_end_matches('.');
        (connection.metadata.destination_ip.is_empty() && !host.is_empty()).then_some(host)
    }

    pub(crate) fn needs_address(&self) -> bool {
        self.address_host().is_some()
    }

    pub(crate) fn begin_address(&mut self) -> Option<AddressRequest> {
        let host = self.address_host()?;
        if self
            .address
            .as_ref()
            .is_some_and(|address| address.request.host == host)
        {
            return None;
        }
        let host = host.to_owned();
        self.address_generation = self.address_generation.wrapping_add(1);
        let request = AddressRequest {
            host,
            generation: self.address_generation,
        };
        self.address = Some(Address {
            request: request.clone(),
            value: String::new(),
        });
        Some(request)
    }

    pub(crate) fn record_address(
        &mut self,
        selection: &Selection,
        request: &AddressRequest,
        value: String,
    ) -> bool {
        if self.selection != *selection
            || self.address_host() != Some(request.host.as_str())
            || !self
                .address
                .as_ref()
                .is_some_and(|address| address.request == *request)
        {
            return false;
        }
        self.address = Some(Address {
            request: request.clone(),
            value,
        });
        true
    }

    pub(crate) fn values(&self, language: Language) -> Option<Vec<(&'static str, String)>> {
        let Observation::Available(connection) = &self.observation else {
            return None;
        };
        let resolved = self
            .address
            .as_ref()
            .filter(|address| self.address_host() == Some(address.request.host.as_str()))
            .map_or("", |address| address.value.as_str());
        Some(detail_values_in(connection, resolved, language))
    }
}

#[cfg(any(target_os = "android", all(test, feature = "mobile-ui-tests")))]
pub(crate) mod ui {
    #[cfg(target_os = "android")]
    use crate::android::{
        AndroidWindow, ConnectionDetailSnapshot, ConnectionDetailStatus, MobileData,
    };
    #[cfg(not(target_os = "android"))]
    use crate::test_ui::{
        AndroidWindow, ConnectionDetailSnapshot, ConnectionDetailStatus, MobileData,
    };
    use slint::{ComponentHandle, SharedString};

    pub(crate) fn begin_loading(ui: &AndroidWindow, id: SharedString, request: i32) {
        ui.global::<MobileData>()
            .set_connection_detail(ConnectionDetailSnapshot {
                id,
                request,
                status: ConnectionDetailStatus::Loading,
                rows: Default::default(),
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_mobile_runtime::core::api::Metadata;

    fn selection(id: &str, request: i32) -> Selection {
        Selection {
            id: id.into(),
            request,
        }
    }

    fn connection(id: &str, host: &str) -> Connection {
        Connection {
            id: id.into(),
            metadata: Metadata {
                host: host.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn only_a_complete_refresh_for_the_current_request_can_confirm_the_end() {
        let current = selection("current", 3);
        let mut detail = Detail::new(current.clone());
        assert_eq!(detail.status(), Status::Loading);
        assert!(detail.values(Language::English).is_none());
        assert!(current.matches("current", 3));
        assert!(!current.matches("previous", 3));
        assert!(!current.matches("current", 2));
        assert!(!detail.confirm(&selection("previous", 3), &[]));
        assert!(!detail.confirm(&selection("current", 2), &[]));
        assert_eq!(detail.status(), Status::Loading);

        detail.refresh_failed(&current);
        assert_eq!(detail.status(), Status::Unavailable);
        let mut full: Vec<_> = (0..55)
            .map(|index| connection(&index.to_string(), ""))
            .collect();
        full.push(connection("current", "current.test"));
        detail.confirm(&current, &full);
        assert_eq!(detail.status(), Status::Available);
        let preview = &full[..50];
        for page in [0, -1] {
            if let Some(selection) = detail.selection_for_refresh(page) {
                detail.confirm(&selection, preview);
            }
            assert_eq!(detail.status(), Status::Available);
        }
        detail.refresh_failed(&current);
        assert_eq!(detail.status(), Status::Available);
        let sampled = detail.selection_for_refresh(CONNECTIONS_PAGE).unwrap();
        assert!(detail.confirm(&sampled, &full[..55]));
        assert_eq!(detail.status(), Status::Ended);
        assert!(detail.values(Language::English).is_none());
        assert!(detail.selection_for_refresh(CONNECTIONS_PAGE).is_none());
        detail.core_stopped();
        assert_eq!(detail.status(), Status::Ended);
    }

    #[test]
    fn close_waits_for_controllers_next_full_snapshot_and_core_stop_is_unavailable() {
        let current = selection("current", 3);
        let mut detail = Detail::new(current.clone());
        let full = [connection("current", "current.test")];
        detail.confirm(&current, &full);
        detail.wait_for_close_confirmation("other");
        assert_eq!(detail.status(), Status::Available);
        detail.wait_for_close_confirmation("current");
        assert_eq!(detail.status(), Status::Loading);
        assert!(detail.values(Language::English).is_none());
        detail.confirm(&current, &full);
        assert_eq!(detail.status(), Status::Available);
        detail.core_stopped();
        assert_eq!(detail.status(), Status::Unavailable);
        detail.confirm(&current, &[]);
        assert_eq!(detail.status(), Status::Ended);
    }

    #[test]
    fn a_resolver_result_matches_the_request_id_and_current_hostname() {
        let current = selection("same", 3);
        let mut detail = Detail::new(current.clone());
        detail.confirm(&current, &[connection("same", "before.test")]);
        let before = detail.begin_address().unwrap();
        assert_eq!(before.host, "before.test");
        assert!(detail.begin_address().is_none());
        assert!(!detail.record_address(&selection("same", 2), &before, "192.0.2.1".into()));
        assert!(!detail.record_address(&selection("other", 3), &before, "192.0.2.1".into()));
        detail.confirm(&current, &[connection("same", "after.test")]);
        assert!(!detail.record_address(&current, &before, "192.0.2.1".into()));
        let after = detail.begin_address().unwrap();
        assert_eq!(after.host, "after.test");
        assert!(detail.record_address(&current, &after, "192.0.2.2".into()));
        let values = detail.values(Language::English).unwrap();
        assert!(values.contains(&("Destination IP", "192.0.2.2".into())));
        assert!(values.contains(&("Connection ID", "same".into())));
        let mut with_address = connection("same", "after.test");
        with_address.metadata.destination_ip = "192.0.2.3".into();
        assert!(detail.confirm(&current, &[with_address]));
        assert!(!detail.needs_address());
        assert!(!detail.record_address(&current, &after, "192.0.2.2".into()));
    }

    #[tokio::test]
    async fn stale_dns_for_the_same_connection_keeps_the_new_request_cancellable() {
        let current = selection("same", 3);
        let mut detail = Detail::new(current.clone());
        let full = [connection("same", "same.test")];
        detail.confirm(&current, &full);
        let old = detail.begin_address().unwrap();
        detail.wait_for_close_confirmation("same");
        assert_eq!(detail.status(), Status::Loading);
        detail.confirm(&current, &full);
        let new = detail.begin_address().unwrap();
        assert_ne!(old, new);

        let (resolver, closed) = tokio::sync::oneshot::channel::<()>();
        let mut address_job = Some(tokio::spawn(async move {
            std::future::pending::<()>().await;
            drop(resolver);
        }));
        let mut detail = Some(detail);
        assert!(!accept_address(
            &mut detail,
            &mut address_job,
            Some(&current),
            &current,
            &old,
            "192.0.2.1".into(),
        ));
        assert!(address_job.is_some());
        assert!(
            !detail
                .as_ref()
                .unwrap()
                .values(Language::English)
                .unwrap()
                .contains(&("Destination IP", "192.0.2.1".into()))
        );
        discard_hidden(&mut detail, &mut address_job, None);
        assert!(address_job.is_none());
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), closed)
                .await
                .unwrap()
                .is_err()
        );
    }

    #[test]
    fn closing_all_disables_detail_actions_until_a_full_refresh_confirms_the_result() {
        let current = selection("current", 3);
        let mut detail = Detail::new(current.clone());
        detail.confirm(&current, &[connection("current", "current.test")]);
        detail.wait_for_all_close_confirmation();
        assert_eq!(detail.status(), Status::Loading);
        assert!(detail.values(Language::English).is_none());
        detail.refresh_failed(&current);
        assert_eq!(detail.status(), Status::Unavailable);
        assert!(detail.values(Language::English).is_none());
        detail.confirm(&current, &[]);
        assert_eq!(detail.status(), Status::Ended);
        assert!(detail.values(Language::English).is_none());
    }

    #[tokio::test]
    async fn dismissing_the_visible_request_drops_its_detail_and_cancels_dns() {
        let current = selection("same", 3);
        let (shown, mut visible) = watch::channel(Some(current.clone()));
        let mut detail = Some(Detail::new(current.clone()));
        let (resolver, closed) = tokio::sync::oneshot::channel::<()>();
        let mut address_job = Some(tokio::spawn(async move {
            std::future::pending::<()>().await;
            drop(resolver);
        }));

        dismiss(&shown, "same", 2);
        dismiss(&shown, "other", 3);
        assert!(!visible.has_changed().unwrap());
        assert!(!address_job.as_ref().unwrap().is_finished());

        dismiss(&shown, "same", 3);
        visible.changed().await.unwrap();
        discard_hidden(&mut detail, &mut address_job, visible.borrow().as_ref());
        assert!(detail.is_none());
        assert!(address_job.is_none());
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), closed)
                .await
                .unwrap()
                .is_err()
        );

        let reopened = selection("same", 4);
        shown.send_replace(Some(reopened.clone()));
        dismiss(&shown, "same", 3);
        let mut detail = Some(Detail::new(reopened.clone()));
        discard_hidden(
            &mut detail,
            &mut address_job,
            visible.borrow_and_update().as_ref(),
        );
        let detail = detail.as_mut().unwrap();
        assert!(detail.selection.matches("same", 4));
        assert!(!detail.confirm(&current, &[]));
        let old = AddressRequest {
            host: "old.test".into(),
            generation: 1,
        };
        assert!(!detail.record_address(&current, &old, "192.0.2.1".into()));
    }

    #[cfg(all(not(target_os = "android"), feature = "mobile-ui-tests"))]
    #[test]
    fn opening_from_home_publishes_loading_before_slint_opens_the_detail() {
        use crate::test_ui::{
            AndroidWindow, ConnectionDetailStatus, MobileActions, MobileData, MobileIntent,
            MobileView,
        };
        use slint::{
            ComponentHandle,
            platform::{
                Platform, PlatformError, WindowAdapter,
                software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
            },
        };
        use std::rc::Rc;

        struct Headless;
        impl Platform for Headless {
            fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
                Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
            }
        }

        slint::platform::set_platform(Box::new(Headless)).unwrap();
        let window = AndroidWindow::new().unwrap();
        let view = window.global::<MobileView>();
        let weak = window.as_weak();
        window
            .global::<MobileActions>()
            .on_request(move |intent, id, sequence| {
                if intent == MobileIntent::ConnectionDetail {
                    let window = weak.upgrade().unwrap();
                    assert_eq!(window.get_current_detail(), 0);
                    assert_eq!(window.get_current_page(), CONNECTIONS_PAGE);
                    ui::begin_loading(&window, id, sequence.as_str().parse().unwrap());
                }
            });
        assert_eq!(window.get_current_page(), 0);
        view.invoke_navigate(CONNECTIONS_PAGE);
        view.invoke_open_connection("home-row".into());
        assert_eq!(window.get_current_detail(), 5);
        let snapshot = window.global::<MobileData>().get_connection_detail();
        assert_eq!(snapshot.status, ConnectionDetailStatus::Loading);
        assert_eq!(snapshot.id.as_str(), "home-row");
        assert_eq!(snapshot.request, view.get_connection_detail_request());
    }
}
