use super::{Connection, Connections, TextScope};
use serde::Deserialize;

pub const CONNECTION_PREVIEW_LIMIT: usize = 4;

pub struct ConnectionPreview {
    pub connections: Connections,
    pub count: usize,
}

fn sample<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<(usize, Vec<Connection>), D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = (usize, Vec<Connection>);
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a connection array or null")
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok((0, Vec::new()))
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut count = 0;
            let mut rows = Vec::with_capacity(CONNECTION_PREVIEW_LIMIT);
            while rows.len() < CONNECTION_PREVIEW_LIMIT {
                let Some(row) = sequence.next_element::<Connection>()? else {
                    return Ok((count, rows));
                };
                rows.push(row);
                count += 1;
            }
            while sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                count += 1;
            }
            Ok((count, rows))
        }
    }
    deserializer.deserialize_any(Visitor)
}

impl<'de> Deserialize<'de> for ConnectionPreview {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let _scope = TextScope::new();
        #[derive(Deserialize)]
        struct Document {
            #[serde(rename = "uploadTotal", default)]
            upload: u64,
            #[serde(rename = "downloadTotal", default)]
            download: u64,
            #[serde(rename = "connections", default, deserialize_with = "sample")]
            sample: (usize, Vec<Connection>),
        }
        let document = Document::deserialize(deserializer)?;
        Ok(Self {
            connections: Connections {
                upload_total: document.upload,
                download_total: document.download,
                connections: document.sample.1,
            },
            count: document.sample.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_retains_four_rows_and_counts_the_rest_without_decoding_metadata() {
        let mut rows: Vec<_> = (0..CONNECTION_PREVIEW_LIMIT)
            .map(|index| serde_json::json!({"id":index.to_string(),"metadata":{"host":"sample.test"}}))
            .collect();
        rows.extend((0..512).map(|_| serde_json::json!({"metadata":"ignored tail metadata"})));
        let preview: ConnectionPreview = serde_json::from_value(serde_json::json!({
            "uploadTotal":42,"downloadTotal":73,"connections":rows
        }))
        .unwrap();
        assert_eq!(preview.count, 512 + CONNECTION_PREVIEW_LIMIT);
        assert_eq!(
            preview.connections.connections.len(),
            CONNECTION_PREVIEW_LIMIT
        );
        assert_eq!(preview.connections.upload_total, 42);
        assert_eq!(preview.connections.download_total, 73);
        assert_eq!(preview.connections.connections[0].id.as_ref(), "0");
    }

    #[test]
    fn idle_core_accepts_missing_null_or_empty_connections() {
        for text in ["{}", r#"{"connections":null}"#, r#"{"connections":[]}"#] {
            let preview: ConnectionPreview = serde_json::from_str(text).unwrap();
            assert_eq!(preview.count, 0);
            assert!(preview.connections.connections.is_empty());
        }
    }
}
