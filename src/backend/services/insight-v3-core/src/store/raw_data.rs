use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::store::tables::{TableError, TableName};

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
    tables: crate::store::tables::TableStore,
    timeouts: InsertTimeouts,
}

impl RawDataStore {
    pub(crate) fn new(client: insight_clickhouse::Client) -> Self {
        Self {
            tables: crate::store::tables::TableStore::new(client.clone()),
            client,
            timeouts: InsertTimeouts::production(),
        }
    }

    #[cfg(test)]
    fn with_timeouts(client: insight_clickhouse::Client, timeouts: InsertTimeouts) -> Self {
        Self {
            tables: crate::store::tables::TableStore::new(client.clone()),
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

        let name =
            crate::store::tables::TableName::parse(&table).map_err(|_| StoreError::NoTable)?;
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
    Create(#[from] crate::store::tables::TableStoreError),
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
mod tests;
