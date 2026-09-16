//! Running a compiled query, and what comes back.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::field::coerce_value;
use super::filter::FilterBind;
use super::people::People;
use super::{CompiledQuery, UndatedQuery};
use crate::domain::query::undated::UndatedCount;

const FETCH_TIMEOUT_SECS: u64 = 30;
const MAX_RESULT_BYTES: usize = 5 * 1024 * 1024;

/// The result of running a [`CompiledQuery`] against `ClickHouse`.
#[derive(Debug, Serialize)]
pub(crate) struct RunResult {
    pub(crate) columns: Vec<String>,
    pub(crate) rows: Vec<Vec<serde_json::Value>>,
    /// Which of those columns are percentages. `83.9` and `83.9%` are the
    /// same number until something says which one it is.
    pub(crate) percents: Vec<String>,
    /// How many rows the window left out because they carry no clock. Only
    /// a run that asked for a window leaves any out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) undated: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ResultMeta {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ClickHouseJsonResult {
    meta: Vec<ResultMeta>,
    data: Vec<serde_json::Map<String, serde_json::Value>>,
}

pub(crate) struct MetricRunner {
    client: insight_clickhouse::Client,
    fetch_timeout: Duration,
    people: People,
}

impl MetricRunner {
    pub(crate) fn new(client: insight_clickhouse::Client, people: People) -> Self {
        Self {
            client,
            fetch_timeout: Duration::from_secs(FETCH_TIMEOUT_SECS),
            people,
        }
    }

    /// Where the queries this runs resolve a person's name from.
    pub(crate) fn people(&self) -> &People {
        &self.people
    }

    pub(crate) async fn undated(
        &self,
        query: &UndatedQuery,
    ) -> Result<UndatedCount, MetricRunError> {
        let bytes = self.fetch(&query.sql, &query.binds).await?;

        Ok(UndatedCount::parse(&bytes)?)
    }

    pub(crate) async fn run(&self, compiled: &CompiledQuery) -> Result<RunResult, MetricRunError> {
        let bytes = self.fetch(&compiled.sql, &compiled.binds).await?;

        let parsed: ClickHouseJsonResult = serde_json::from_slice(&bytes)?;
        let columns: Vec<String> = parsed.meta.into_iter().map(|column| column.name).collect();
        let rows = parsed
            .data
            .into_iter()
            .map(|mut row| {
                columns
                    .iter()
                    .map(|name| {
                        let value = row.remove(name).unwrap_or(serde_json::Value::Null);
                        match compiled.column_types.get(name) {
                            Some(field_type) => coerce_value(value, *field_type),
                            None => value,
                        }
                    })
                    .collect()
            })
            .collect();

        Ok(RunResult {
            columns,
            rows,
            percents: compiled.percents.clone(),
            undated: None,
        })
    }

    async fn fetch(&self, sql: &str, binds: &[FilterBind]) -> Result<Vec<u8>, MetricRunError> {
        let mut query = self.client.query(sql);
        for bind in binds {
            query = bind.bind_onto(query);
        }
        let mut cursor = query.fetch_bytes("JSON")?;

        let fetch = async {
            let mut bytes = Vec::new();
            while let Some(chunk) = cursor.next().await? {
                let next_len = bytes.len().saturating_add(chunk.len());
                if next_len > MAX_RESULT_BYTES {
                    return Err(MetricRunError::ResultTooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok::<_, MetricRunError>(bytes)
        };

        tokio::time::timeout(self.fetch_timeout, fetch)
            .await
            .map_err(|_| MetricRunError::Timeout)?
    }
}

impl fmt::Debug for MetricRunner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetricRunner")
            .field("fetch_timeout", &self.fetch_timeout)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error)]
pub(crate) enum MetricRunError {
    #[error("metric query timed out")]
    Timeout,
    #[error("metric query result exceeded the size limit")]
    ResultTooLarge,
    #[error(transparent)]
    ClickHouse(clickhouse::error::Error),
    #[error(transparent)]
    InvalidResponse(#[from] serde_json::Error),
}

impl From<clickhouse::error::Error> for MetricRunError {
    fn from(error: clickhouse::error::Error) -> Self {
        match error {
            clickhouse::error::Error::TimedOut => Self::Timeout,
            error => Self::ClickHouse(error),
        }
    }
}
