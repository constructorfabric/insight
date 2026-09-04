use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;

use super::metrics::{self, ErrorClass, QueryKind, QueryOutcome};

const QUERY_FETCH_TIMEOUT: Duration = Duration::from_mins(1);

#[derive(Debug, thiserror::Error)]
pub(crate) enum QueryFetchError {
    #[error("query submission failed: {0}")]
    Submit(String),
    #[error("query fetch timed out")]
    Timeout,
    #[error("query fetch failed: {0}")]
    Fetch(String),
    #[error("query result parsing failed: {0}")]
    Parse(String),
}

impl QueryFetchError {
    fn class(&self) -> ErrorClass {
        match self {
            Self::Submit(message) | Self::Fetch(message) => ErrorClass::classify(message),
            Self::Timeout => ErrorClass::Timeout,
            Self::Parse(_) => ErrorClass::ParseFailed,
        }
    }
}

// WORKAROUND: ClickHouse quotes every 64-bit integer by default, which would
// make an answer's number column arrive as a string.
pub(crate) async fn fetch_bound_rows(
    client: &insight_clickhouse::Client,
    sql: &str,
    params: &[serde_json::Value],
    kind: QueryKind,
    log_comment: &str,
) -> Result<Vec<serde_json::Value>, QueryFetchError> {
    let started = Instant::now();
    let result = fetch_bound_rows_inner(client, sql, params, log_comment).await;

    match &result {
        Ok(_) => metrics::record_query(kind, QueryOutcome::Success, started.elapsed()),
        Err(error) => {
            metrics::record_query(kind, QueryOutcome::Error, started.elapsed());
            metrics::record_clickhouse_error(kind, error.class());
        }
    }
    result
}

async fn fetch_bound_rows_inner(
    client: &insight_clickhouse::Client,
    sql: &str,
    params: &[serde_json::Value],
    log_comment: &str,
) -> Result<Vec<serde_json::Value>, QueryFetchError> {
    let mut query = client
        .query(sql)
        .with_setting("log_comment", log_comment)
        .with_setting("output_format_json_quote_64bit_integers", "0");
    for param in params {
        query = query.bind(param);
    }

    let mut cursor = query.fetch_bytes("JSONEachRow").map_err(|error| {
        tracing::error!(error = %error, comment = log_comment, sql, "ClickHouse query failed");
        QueryFetchError::Submit(error.to_string())
    })?;
    let raw_bytes = tokio::time::timeout(QUERY_FETCH_TIMEOUT, cursor.collect())
        .await
        .map_err(|_| {
            tracing::error!(comment = log_comment, sql, "ClickHouse query fetch timed out");
            QueryFetchError::Timeout
        })?
        .map_err(|error| {
            tracing::error!(error = %error, comment = log_comment, sql, "ClickHouse query fetch failed");
            QueryFetchError::Fetch(error.to_string())
        })?;

    raw_bytes
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(serde_json::from_slice)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            tracing::error!(error = %error, comment = log_comment, sql, "failed to parse ClickHouse query rows");
            QueryFetchError::Parse(error.to_string())
        })
}

pub(crate) async fn fetch_json_rows<T>(
    client: &insight_clickhouse::Client,
    sql: &str,
    params: &[String],
    kind: QueryKind,
    log_comment: &str,
) -> Result<Vec<T>, QueryFetchError>
where
    T: DeserializeOwned,
{
    let started = Instant::now();
    let result = fetch_json_rows_inner(client, sql, params, log_comment).await;

    match &result {
        Ok(_) => metrics::record_query(kind, QueryOutcome::Success, started.elapsed()),
        Err(error) => {
            metrics::record_query(kind, QueryOutcome::Error, started.elapsed());
            metrics::record_clickhouse_error(kind, error.class());
        }
    }
    result
}

async fn fetch_json_rows_inner<T>(
    client: &insight_clickhouse::Client,
    sql: &str,
    params: &[String],
    log_comment: &str,
) -> Result<Vec<T>, QueryFetchError>
where
    T: DeserializeOwned,
{
    let mut query = client.query(sql).with_setting("log_comment", log_comment);
    for param in params {
        query = query.bind(param.as_str());
    }

    let mut cursor = query.fetch_bytes("JSONEachRow").map_err(|error| {
        tracing::error!(error = %error, comment = log_comment, sql, "ClickHouse query failed");
        QueryFetchError::Submit(error.to_string())
    })?;
    let raw_bytes = tokio::time::timeout(QUERY_FETCH_TIMEOUT, cursor.collect())
        .await
        .map_err(|_| {
            tracing::error!(comment = log_comment, sql, "ClickHouse query fetch timed out");
            QueryFetchError::Timeout
        })?
        .map_err(|error| {
            tracing::error!(error = %error, comment = log_comment, sql, "ClickHouse query fetch failed");
            QueryFetchError::Fetch(error.to_string())
        })?;
    if raw_bytes.is_empty() {
        return Ok(Vec::new());
    }

    raw_bytes
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(serde_json::from_slice)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            tracing::error!(error = %error, comment = log_comment, sql, "failed to parse ClickHouse query rows");
            QueryFetchError::Parse(error.to_string())
        })
}
