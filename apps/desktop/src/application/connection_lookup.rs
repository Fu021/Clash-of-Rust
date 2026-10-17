//! Resolve only an open connection's missing address, with cancellation and no OS DNS.
use super::*;
use clash_of_rust::api::Connection;

#[derive(Default)]
pub(super) struct Lookup {
    generation: u64,
    id: String,
    host: String,
    value: String,
    task: Option<AbortHandle>,
}
impl Lookup {
    pub(super) fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.generation = self.generation.wrapping_add(1);
        self.id.clear();
        self.host.clear();
        self.value.clear();
    }
    pub(super) fn addresses(&self, connection: &Connection) -> &str {
        if connection.id.as_ref() == self.id
            && hostname(connection).trim_end_matches('.') == self.host
        {
            &self.value
        } else {
            ""
        }
    }
}
impl Drop for Lookup {
    fn drop(&mut self) {
        self.cancel();
    }
}
pub(super) fn hostname(connection: &Connection) -> &str {
    if connection.metadata.sniff_host.is_empty() {
        connection.metadata.host.as_ref()
    } else {
        connection.metadata.sniff_host.as_ref()
    }
}
impl App {
    pub(super) fn clear_connection_detail(&mut self) {
        self.connection_detail = None;
        self.connection_lookup.cancel();
    }
    pub(super) fn open_connection_detail(&mut self, id: &str) -> Task<Message> {
        self.clear_connection_detail();
        self.connection_detail = Some(id.into());
        let Some(connection) = self
            .snapshot
            .connections
            .connections
            .iter()
            .find(|connection| connection.id.as_ref() == id)
        else {
            return Task::none();
        };
        let host = hostname(connection).trim_end_matches('.');
        if !connection.metadata.destination_ip.is_empty() || host.is_empty() {
            return Task::none();
        }
        self.connection_lookup.id = id.into();
        self.connection_lookup.host = host.into();
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            self.connection_lookup.value = ip.to_string();
            return Task::none();
        }
        let Some(engine) = self.engine.clone() else {
            return Task::none();
        };
        let host = host.to_owned();
        let port = self.settings.mixed_port;
        let generation = self.connection_lookup.generation;
        let (task, handle) = Task::perform(
            async move {
                let api = engine.lock().await.api.clone();
                clash_client_core::connection::resolve_target(&api, &host, port)
                    .await
                    .map_err(|_| "未查到目标地址".to_owned())
            },
            move |result| Message::ConnectionResolved(generation, result),
        )
        .abortable();
        self.connection_lookup.task = Some(handle);
        task
    }
    pub(super) fn accept_connection_addresses(
        &mut self,
        generation: u64,
        result: &Result<String, String>,
    ) {
        if generation != self.connection_lookup.generation
            || self.connection_detail.as_deref() != Some(self.connection_lookup.id.as_str())
        {
            return;
        }
        if self
            .snapshot
            .connections
            .connections
            .iter()
            .any(|connection| {
                connection.id.as_ref() == self.connection_lookup.id
                    && hostname(connection).trim_end_matches('.') == self.connection_lookup.host
            })
        {
            self.connection_lookup.value = result.as_deref().unwrap_or("").into();
        }
        self.connection_lookup.task = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_or_switching_details_rejects_late_dns_and_releases_the_request() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("isolated")), None);
        app.snapshot.connections = serde_json::from_str(r#"{"connections":[{"id":"1","metadata":{"host":"one.test"}},{"id":"2","metadata":{"host":"two.test"}}]}"#).unwrap();
        let _ = app.open_connection_detail("1");
        let first = app.connection_lookup.generation;
        app.accept_connection_addresses(first, &Ok("203.0.113.1".into()));
        assert_eq!(
            app.connection_lookup
                .addresses(&app.snapshot.connections.connections[0]),
            "203.0.113.1"
        );
        let (handle, _) = AbortHandle::new_pair();
        app.connection_lookup.task = Some(handle.clone());
        let _ = app.open_connection_detail("2");
        assert!(handle.is_aborted());
        app.accept_connection_addresses(first, &Ok("203.0.113.1".into()));
        assert!(
            app.connection_lookup
                .addresses(&app.snapshot.connections.connections[1])
                .is_empty()
        );
        let second = app.connection_lookup.generation;
        app.clear_connection_detail();
        app.accept_connection_addresses(second, &Ok("203.0.113.2".into()));
        assert!(app.connection_lookup.value.is_empty());
    }
}
