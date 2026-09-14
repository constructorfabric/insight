use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::tables::{TableError, TableName};

const INSERT_SEND_TIMEOUT_SECS: u64 = 10;
const INSERT_END_TIMEOUT_SECS: u64 = 30;
const INSERT_TOTAL_TIMEOUT_SECS: u64 = 35;

#[derive(Debug)]
pub(crate) struct RawDataRecord {
    table_name: TableName,
    raw_data: String,
}
impl RawDataRecord {
    pub(crate) fn parse(
        table_name: &str,
        raw_data: &serde_json::Value,
    ) -> Result<Self, RawDataError> {
        let table_name = TableName::parse(table_name)?;
        let raw_data = serde_json::to_string(raw_data)?;

        Ok(Self {
            table_name,
            raw_data,
        })
    }

    #[cfg(test)]
    fn table_name(&self) -> &str {
        self.table_name.as_str()
    }

    fn into_parts(self) -> (String, RawDataRow) {
        let physical_table = self.table_name.as_str().to_owned();
        let row = RawDataRow {
            id: Uuid::now_v7(),
            table_name: self.table_name.into_string(),
            raw_data: self.raw_data,
            received_at: Utc::now(),
        };

        (physical_table, row)
    }
}

pub(crate) struct RawDataStore {
    client: insight_clickhouse::Client,
    tables: crate::tables::TableStore,
    timeouts: InsertTimeouts,
}

impl RawDataStore {
    pub(crate) fn new(client: insight_clickhouse::Client) -> Self {
        Self {
            tables: crate::tables::TableStore::new(client.clone()),
            client,
            timeouts: InsertTimeouts::production(),
        }
    }

    #[cfg(test)]
    fn with_timeouts(client: insight_clickhouse::Client, timeouts: InsertTimeouts) -> Self {
        Self {
            tables: crate::tables::TableStore::new(client.clone()),
            client,
            timeouts,
        }
    }

    /// A stream's first write creates the table it lands in.
    ///
    /// The shape is ours and fixed and the name was already constrained to
    /// `^[A-Za-z0-9_]{1,128}$`, so there is nothing for the caller to describe
    /// and nothing unsafe to interpolate. Creating it here rather than only
    /// behind the admin-only route is what lets a connector write with its
    /// ingest token alone.
    pub(crate) async fn insert(&self, record: RawDataRecord) -> Result<(), StoreError> {
        let (table, row) = record.into_parts();

        match self.insert_once(&table, &row).await {
            Err(StoreError::NoTable) => {}
            other => return other,
        }

        let name = crate::tables::TableName::parse(&table).map_err(|_| StoreError::NoTable)?;
        self.tables.create(&name).await?;

        self.insert_once(&table, &row).await
    }

    async fn insert_once(&self, table: &str, row: &RawDataRow) -> Result<(), StoreError> {
        tokio::time::timeout(self.timeouts.total, self.insert_with_timeouts(table, row))
            .await
            .map_err(|_| StoreError::Timeout)?
    }

    async fn insert_with_timeouts(&self, table: &str, row: &RawDataRow) -> Result<(), StoreError> {
        let mut insert = self
            .client
            .inner()
            .insert::<RawDataRow>(table)
            .await?
            .with_timeouts(Some(self.timeouts.send), Some(self.timeouts.end));
        insert.write(row).await?;
        insert.end().await?;

        Ok(())
    }
}

impl fmt::Debug for RawDataStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RawDataStore")
            .field("timeouts", &self.timeouts)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy)]
struct InsertTimeouts {
    send: Duration,
    end: Duration,
    total: Duration,
}

impl InsertTimeouts {
    fn production() -> Self {
        Self {
            send: Duration::from_secs(INSERT_SEND_TIMEOUT_SECS),
            end: Duration::from_secs(INSERT_END_TIMEOUT_SECS),
            total: Duration::from_secs(INSERT_TOTAL_TIMEOUT_SECS),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum RawDataError {
    #[error(transparent)]
    Table(#[from] TableError),
    #[error("raw_data could not be serialized")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub(crate) enum StoreError {
    #[error("failed to insert raw data")]
    ClickHouse(#[source] clickhouse::error::Error),
    #[error("raw data insert timed out")]
    Timeout,
    #[error("there is no table for this stream yet")]
    NoTable,
    #[error("the stream's table could not be created")]
    Create(#[from] crate::tables::TableStoreError),
}

/// `ClickHouse` reports a missing relation as error 60 inside a message rather
/// than as a variant, and only the code is stable across a real server's text
/// and a header-only reply.
const UNKNOWN_TABLE_CODE: &str = "Code: 60";

impl From<clickhouse::error::Error> for StoreError {
    fn from(error: clickhouse::error::Error) -> Self {
        match error {
            clickhouse::error::Error::TimedOut => Self::Timeout,
            clickhouse::error::Error::BadResponse(ref message)
                if message.contains(UNKNOWN_TABLE_CODE) =>
            {
                Self::NoTable
            }
            error => Self::ClickHouse(error),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, clickhouse::Row)]
struct RawDataRow {
    #[serde(with = "clickhouse::serde::uuid")]
    id: Uuid,
    table_name: String,
    raw_data: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    received_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use clickhouse::test::{Mock, handlers};
    use serde_json::json;
    use tokio::net::TcpListener;

    use super::*;

    #[test]
    fn physical_table_name_is_preserved() {
        let record = RawDataRecord::parse("synthetic_events", &json!({"value": 1}))
            .unwrap_or_else(|error| panic!("record must parse: {error}"));

        assert_eq!(record.table_name(), "synthetic_events");
    }

    #[test]
    fn arbitrary_json_values_are_accepted() {
        for value in [
            serde_json::Value::Null,
            json!(true),
            json!(42),
            json!("text"),
            json!([1, {"nested": false}]),
            json!({"nested": [1, 2, 3]}),
        ] {
            assert!(
                RawDataRecord::parse("synthetic_events", &value).is_ok(),
                "every JSON shape must be accepted"
            );
        }
    }

    #[test]
    fn unsafe_table_names_are_rejected() {
        for table in [
            "",
            "   ",
            "synthetic.events",
            &"x".repeat(crate::tables::MAX_TABLE_NAME_CHARS + 1),
        ] {
            assert!(
                RawDataRecord::parse(table, &json!(null)).is_err(),
                "must reject physical table name: {table:?}"
            );
        }
    }

    #[test]
    fn clickhouse_timeout_is_normalized_to_store_timeout() {
        let error = StoreError::from(clickhouse::error::Error::TimedOut);

        assert!(matches!(error, StoreError::Timeout));
    }

    #[tokio::test]
    async fn insert_writes_the_selected_table_row() {
        let mock = Mock::new();
        let recording = mock.add(handlers::record::<RawDataRow>());
        let client =
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));
        let store = RawDataStore::new(client);
        let record = RawDataRecord::parse("synthetic_events", &json!({"nested": [1, true, null]}))
            .unwrap_or_else(|error| panic!("record must parse: {error}"));

        store
            .insert(record)
            .await
            .unwrap_or_else(|error| panic!("insert must succeed: {error}"));
        let rows: Vec<RawDataRow> = recording.collect().await;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].table_name, "synthetic_events");
        assert_eq!(rows[0].raw_data, r#"{"nested":[1,true,null]}"#);
        assert_ne!(rows[0].id, uuid::Uuid::nil());
    }

    #[tokio::test]
    async fn a_streams_first_write_creates_the_table_it_lands_in() {
        let mock = Mock::new();
        mock.add(handlers::exception(60));
        let ddl = mock.add(handlers::record_ddl());
        mock.add(handlers::record::<RawDataRow>());
        let store = RawDataStore::new(insight_clickhouse::Client::new(
            insight_clickhouse::Config::new(mock.url(), "insight"),
        ));
        let record = RawDataRecord::parse("first_write_stream", &json!({"a": 1}))
            .unwrap_or_else(|error| panic!("record must parse: {error}"));

        store.insert(record).await.unwrap_or_else(|error| {
            panic!("the write should land after the table exists: {error}")
        });
        let statement = ddl.query().await;

        assert!(
            statement.contains("CREATE TABLE IF NOT EXISTS `first_write_stream`"),
            "the stream's own table was not created: {statement}"
        );
    }

    #[test]
    fn a_write_to_a_stream_with_no_table_is_the_callers_to_fix() {
        let refusal = StoreError::from(clickhouse::error::Error::BadResponse(
            "Code: 60. DB::Exception: Table insight.absent_stream does not exist. (UNKNOWN_TABLE)"
                .to_owned(),
        ));

        assert!(
            matches!(refusal, StoreError::NoTable),
            "a missing table answered 500 and leaked the database's own text: {refusal:?}"
        );
    }

    #[test]
    fn any_other_bad_response_stays_a_server_error() {
        let refusal = StoreError::from(clickhouse::error::Error::BadResponse(
            "Code: 241. DB::Exception: Memory limit exceeded".to_owned(),
        ));

        assert!(matches!(refusal, StoreError::ClickHouse(_)));
    }

    #[tokio::test]
    async fn hung_clickhouse_insert_is_time_bounded() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|error| panic!("test listener must bind: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("test listener must have an address: {error}"));
        let server = tokio::spawn(async move {
            let _connection = listener
                .accept()
                .await
                .unwrap_or_else(|error| panic!("test server must accept: {error}"));
            futures::future::pending::<()>().await;
        });
        let client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            format!("http://{address}"),
            "insight",
        ));
        let timeout = Duration::from_millis(25);
        let store = RawDataStore::with_timeouts(
            client,
            InsertTimeouts {
                send: timeout,
                end: timeout,
                total: timeout,
            },
        );
        let record = RawDataRecord::parse("synthetic_events", &json!(1))
            .unwrap_or_else(|error| panic!("record must parse: {error}"));

        let result = tokio::time::timeout(Duration::from_secs(1), store.insert(record)).await;
        server.abort();

        assert!(matches!(result, Ok(Err(StoreError::Timeout))));
    }
}
