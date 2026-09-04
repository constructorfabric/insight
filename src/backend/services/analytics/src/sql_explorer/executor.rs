use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::sync::Semaphore;

use crate::domain::query_gate::validate_mcp_sql;

pub(crate) const MAX_REQUEST_BODY_BYTES: usize = 128 * 1024;
pub(crate) const MAX_SQL_BYTES: usize = 64 * 1024;
pub(crate) const MAX_RESULT_BYTES: usize = 5 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(35);

#[derive(Debug, Deserialize, Serialize)]
struct ResultColumn {
    name: String,
    #[serde(rename = "type")]
    data_type: String,
}

#[derive(Debug, Deserialize)]
struct ClickHouseJsonResult {
    meta: Vec<ResultColumn>,
    data: Vec<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct QuerySqlResult {
    columns: Vec<ResultColumn>,
    rows: Vec<serde_json::Map<String, serde_json::Value>>,
    row_count: usize,
    truncated: bool,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum QueryFailure {
    #[error("{0}")]
    InvalidSql(String),
    #[error("SQL exceeds the 65536-byte request limit")]
    SqlTooLarge,
    #[error("query capacity is exhausted")]
    Busy,
    #[error("query result exceeded the response limit")]
    ResultTooLarge,
    #[error("query timed out")]
    Timeout,
    #[error("ClickHouse query failed: {0}")]
    ClickHouse(#[from] clickhouse::error::Error),
    #[error("ClickHouse returned an invalid JSON response: {0}")]
    InvalidResponse(#[from] serde_json::Error),
    #[error("query result processing task failed: {0}")]
    ProcessingTask(#[from] tokio::task::JoinError),
}

impl QueryFailure {
    pub(crate) fn public_message(&self) -> &str {
        match self {
            Self::InvalidSql(reason) => reason,
            Self::SqlTooLarge => "SQL exceeds the 65536-byte request limit",
            Self::Busy => "the SQL explorer is busy; retry shortly",
            Self::ResultTooLarge => "query result exceeded the response limit",
            Self::Timeout => "query timed out",
            Self::ClickHouse(_) | Self::InvalidResponse(_) | Self::ProcessingTask(_) => {
                "query execution failed"
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct QueryExecutor {
    client: insight_clickhouse::Client,
    query_slots: Arc<Semaphore>,
}

impl std::fmt::Debug for QueryExecutor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QueryExecutor")
            .field("query_slots", &self.query_slots)
            .finish_non_exhaustive()
    }
}

impl QueryExecutor {
    pub(crate) fn new(client: insight_clickhouse::Client, max_concurrent_queries: usize) -> Self {
        Self {
            client,
            query_slots: Arc::new(Semaphore::new(max_concurrent_queries)),
        }
    }

    #[cfg(test)]
    pub(crate) async fn execute(&self, sql: &str) -> Result<QuerySqlResult, QueryFailure> {
        let permit = self
            .query_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| QueryFailure::Busy)?;
        // INVARIANT: the permit bounds database work and result decoding.
        let result = self.execute_admitted(sql).await;
        drop(permit);
        result
    }

    async fn execute_admitted(&self, sql: &str) -> Result<QuerySqlResult, QueryFailure> {
        let mut query = self.client.query(sql).fetch_bytes("JSON")?;

        let fetch = async {
            let mut bytes = Vec::new();
            while let Some(chunk) = query.next().await? {
                let next_len = bytes.len().saturating_add(chunk.len());
                if next_len > MAX_RESULT_BYTES {
                    return Err(QueryFailure::ResultTooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok::<_, QueryFailure>(bytes)
        };

        let bytes = tokio::time::timeout(FETCH_TIMEOUT, fetch)
            .await
            .map_err(|_| QueryFailure::Timeout)??;
        // INVARIANT: query capacity also bounds CPU-heavy result decoding.
        let result = tokio::task::spawn_blocking(move || {
            serde_json::from_slice::<ClickHouseJsonResult>(&bytes)
        })
        .await??;

        let row_count = result.data.len();
        Ok(QuerySqlResult {
            columns: result.meta,
            rows: result.data,
            row_count,
            truncated: false,
        })
    }

    pub(crate) async fn query<T, F>(&self, sql: String, encode: F) -> Result<T, QueryFailure>
    where
        T: Send + 'static,
        F: FnOnce(serde_json::Value) -> Result<T, serde_json::Error> + Send + 'static,
    {
        if sql.len() > MAX_SQL_BYTES {
            return Err(QueryFailure::SqlTooLarge);
        }
        // INVARIANT: capacity bounds SQL parsing, database work, and result conversion together.
        let _permit = self
            .query_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| QueryFailure::Busy)?;
        let sql = tokio::task::spawn_blocking(move || {
            validate_mcp_sql(&sql).map_err(QueryFailure::InvalidSql)?;
            Ok::<_, QueryFailure>(sql)
        })
        .await??;

        let started = Instant::now();
        let query_hash = hex::encode(Sha256::digest(sql.as_bytes()));
        let result = self.execute_admitted(&sql).await;
        match result {
            Ok(result) => {
                tracing::info!(
                    query_hash,
                    duration_ms = started.elapsed().as_millis(),
                    rows = result.row_count,
                    outcome = "success",
                    "SQL explorer query completed"
                );
                let value =
                    tokio::task::spawn_blocking(move || encode(serde_json::to_value(result)?))
                        .await;
                match value {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(error)) => {
                        tracing::error!(query_hash, %error, "SQL result serialization failed");
                        Err(QueryFailure::InvalidResponse(error))
                    }
                    Err(error) => {
                        tracing::error!(query_hash, %error, "SQL result processing failed");
                        Err(QueryFailure::ProcessingTask(error))
                    }
                }
            }
            Err(error) => {
                tracing::warn!(query_hash, duration_ms = started.elapsed().as_millis(),
                    %error, outcome = "error", "SQL explorer query failed");
                Err(error)
            }
        }
    }
}
