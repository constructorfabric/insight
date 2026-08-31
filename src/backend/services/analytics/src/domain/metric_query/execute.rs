//! Running a compiled statement and decoding what it answered.
//!
//! SAFETY: the statement carries its own values; execution binds them in order
//! and never interpolates one.

use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::domain::compiler::sql::{CompiledMeasureQuery, QueryParam};

use super::error::QueryError;

/// Client-side bound on one read, covering the transport path the
/// `ClickHouse` client's own server-side cap cannot reach.
const FETCH_TIMEOUT: Duration = Duration::from_mins(1);

pub(super) async fn fetch<T>(
    clickhouse: &insight_clickhouse::Client,
    query: &CompiledMeasureQuery,
    log_comment: &str,
) -> Result<Vec<T>, QueryError>
where
    T: DeserializeOwned,
{
    let mut request = clickhouse
        .query(&query.sql)
        .with_setting("log_comment", log_comment);
    for param in &query.params {
        request = match param {
            QueryParam::Text(value) => request.bind(value.as_str()),
            QueryParam::Int(value) => request.bind(*value),
            QueryParam::UInt(value) => request.bind(*value),
            QueryParam::Float(value) => request.bind(*value),
            QueryParam::Bool(value) => request.bind(*value),
        };
    }

    let mut cursor = request.fetch_bytes("JSONEachRow").map_err(|error| {
        tracing::error!(error = %error, comment = log_comment, sql = %query.sql, "metric values query failed");
        QueryError::ReadFailed
    })?;

    let raw_bytes = tokio::time::timeout(FETCH_TIMEOUT, cursor.collect())
        .await
        .map_err(|_| {
            tracing::error!(comment = log_comment, sql = %query.sql, "metric values fetch timed out");
            QueryError::ReadFailed
        })?
        .map_err(|error| {
            tracing::error!(error = %error, comment = log_comment, sql = %query.sql, "metric values fetch failed");
            QueryError::ReadFailed
        })?;

    if raw_bytes.is_empty() {
        return Ok(Vec::new());
    }

    raw_bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(serde_json::from_slice)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            tracing::error!(error = %error, comment = log_comment, sql = %query.sql, "metric values rows did not decode");
            QueryError::RowsUndecodable
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::fixtures::offline_clickhouse;
    use super::*;

    #[tokio::test]
    async fn a_read_that_cannot_reach_the_server_answers_nothing() {
        let query = CompiledMeasureQuery {
            sql: "SELECT 1".to_owned(),
            params: Vec::new(),
        };

        let outcome = fetch::<serde_json::Value>(&offline_clickhouse(), &query, "test").await;

        assert!(matches!(
            outcome.expect_err("a closed port cannot answer"),
            QueryError::ReadFailed
        ));
    }
}
