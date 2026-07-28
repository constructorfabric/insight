use std::io::{Cursor, Seek, SeekFrom, Write};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Body;
use axum::extract::Extension;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderValue, Response};
use rust_xlsxwriter::{ExcelDateTime, Format, Workbook};
use tokio::sync::Semaphore;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::AppState;
use crate::api::error::MetricError;
use crate::domain::metric_drilldown::{
    EvidenceQueryRow, MAX_EXPORT_ROWS, MetricDrilldownColumn, MetricDrilldownExportFormat,
    MetricDrilldownExportRequest, MetricDrilldownRequest, MetricDrilldownResponse,
    MetricDrilldownRow, build_response, compile_query, presentation, validate_export_request,
    validate_request, verify_evidence_snapshot,
};

const QUERY_TIMEOUT: Duration = Duration::from_secs(45);
const EXPORT_TIMEOUT: Duration = Duration::from_mins(1);
const EXPORT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_EXPORT_BYTES: usize = 25 * 1024 * 1024;
const MAX_CELL_BYTES: usize = 32 * 1024;
const MAX_QUERY_RESULT_BYTES: usize = 32 * 1024 * 1024;
const MAX_QUERY_MEMORY_BYTES: usize = 256 * 1024 * 1024;
const MAX_QUERY_READ_BYTES: usize = 512 * 1024 * 1024;
const MAX_CONCURRENT_EXPORTS: usize = 2;
const MAX_CONCURRENT_QUERIES: usize = 8;
static EXPORT_SEMAPHORE: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(MAX_CONCURRENT_EXPORTS));
static QUERY_SEMAPHORE: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(MAX_CONCURRENT_QUERIES));

pub async fn query_metric_drilldown(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<MetricDrilldownRequest>,
) -> Result<Json<MetricDrilldownResponse>, CanonicalError> {
    let started = Instant::now();
    let req = validate_request(&state.db, &state.ch, ctx.subject_tenant_id(), req).await?;
    let log_comment = format!("metric-drilldown:page:{}", req.plan.definition.key());
    let rows = fetch_rows(&state, &req, &log_comment).await?;
    verify_evidence_snapshot(&state.ch, &req.plan.relation, &req.snapshot_id).await?;
    let fetched_rows = rows.len();
    let response = build_response(&req, rows)?;
    tracing::info!(
        duration_ms = started.elapsed().as_millis(),
        rows = response.rows.len(),
        fetched_rows,
        limit = req.limit,
        has_next_page = response.next_cursor.is_some(),
        "metric drilldown page completed"
    );
    Ok(Json(response))
}

pub async fn export_metric_drilldown(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<MetricDrilldownExportRequest>,
) -> Result<Response<Body>, CanonicalError> {
    let started = Instant::now();
    let permit = tokio::time::timeout(EXPORT_ACQUIRE_TIMEOUT, EXPORT_SEMAPHORE.acquire())
        .await
        .map_err(|_| {
            tracing::warn!(
                capacity = MAX_CONCURRENT_EXPORTS,
                available = EXPORT_SEMAPHORE.available_permits(),
                "metric drilldown export capacity exhausted"
            );
            export_busy()
        })?
        .map_err(|_| export_busy())?;
    let deadline = tokio::time::Instant::now() + EXPORT_TIMEOUT;
    let validated = validate_export_request(
        &state.db,
        &state.ch,
        ctx.subject_tenant_id(),
        &req,
        MAX_EXPORT_ROWS + 1,
    )
    .await?;
    let log_comment = format!(
        "metric-drilldown:export:{}",
        validated.plan.definition.key()
    );
    let result = tokio::time::timeout_at(deadline, fetch_rows(&state, &validated, &log_comment))
        .await
        .map_err(|_| export_limit("Export exceeded the execution time limit."))??;
    verify_evidence_snapshot(&state.ch, &validated.plan.relation, &validated.snapshot_id).await?;
    if result.len() > MAX_EXPORT_ROWS {
        return Err(export_limit(format!(
            "Export exceeds the {MAX_EXPORT_ROWS} row limit."
        )));
    }
    let exported_rows = result.len();
    let (columns, rows) = presentation(
        &result,
        &validated.plan,
        &validated.selection.filters,
        &validated.selection.display_dimensions,
    )?;
    drop(result);
    let export_format = req.format;
    let export = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        build_export(export_format, &columns, &rows)
    });
    let (body, content_type, extension) = tokio::time::timeout_at(deadline, export)
        .await
        .map_err(|_| export_limit("Export exceeded the execution time limit."))?
        .map_err(|_| export_internal())??;
    tracing::info!(
        duration_ms = started.elapsed().as_millis(),
        rows = exported_rows,
        bytes = body.len(),
        format = export_format.as_str(),
        row_limit = MAX_EXPORT_ROWS,
        byte_limit = MAX_EXPORT_BYTES,
        capacity = MAX_CONCURRENT_EXPORTS,
        "metric drilldown export completed"
    );
    let filename = format!("{}-evidence.{extension}", safe_filename(&req.metric_key));
    Response::builder()
        .header(CONTENT_TYPE, HeaderValue::from_static(content_type))
        .header(
            CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
                .map_err(|_| export_internal())?,
        )
        .body(Body::from(body))
        .map_err(|_| export_internal())
}

fn build_export(
    format: MetricDrilldownExportFormat,
    columns: &[MetricDrilldownColumn],
    rows: &[MetricDrilldownRow],
) -> Result<(Vec<u8>, &'static str, &'static str), CanonicalError> {
    ensure_export_input_bound(columns, rows)?;
    match format {
        MetricDrilldownExportFormat::Csv => {
            Ok((build_csv(columns, rows)?, "text/csv; charset=utf-8", "csv"))
        }
        MetricDrilldownExportFormat::Xlsx => Ok((
            build_xlsx(columns, rows)?,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "xlsx",
        )),
    }
}

async fn fetch_rows(
    state: &Arc<AppState>,
    req: &crate::domain::metric_drilldown::ValidatedMetricDrilldown,
    log_comment: &str,
) -> Result<Vec<EvidenceQueryRow>, CanonicalError> {
    let _permit = tokio::time::timeout(EXPORT_ACQUIRE_TIMEOUT, QUERY_SEMAPHORE.acquire())
        .await
        .map_err(|_| query_busy())?
        .map_err(|_| query_busy())?;
    let (sql, params) = compile_query(req)?;
    let mut query = state
        .ch
        .query(&sql)
        .with_option("log_comment", log_comment)
        .with_option("max_execution_time", QUERY_TIMEOUT.as_secs().to_string())
        .with_option("max_threads", "2")
        .with_option("max_memory_usage", MAX_QUERY_MEMORY_BYTES.to_string())
        .with_option("max_bytes_to_read", MAX_QUERY_READ_BYTES.to_string())
        .with_option("max_result_bytes", MAX_QUERY_RESULT_BYTES.to_string());
    for param in params {
        query = query.bind(param);
    }
    let mut cursor = query.fetch_bytes("JSONEachRow").map_err(|error| {
        tracing::error!(error = %error, "ClickHouse metric drilldown query failed");
        query_error(&error.to_string())
    })?;
    let bytes = tokio::time::timeout(QUERY_TIMEOUT, cursor.collect())
        .await
        .map_err(|_| CanonicalError::internal("metric evidence query timed out").create())?
        .map_err(|error| {
            tracing::error!(error = %error, "ClickHouse metric drilldown fetch failed");
            query_error(&error.to_string())
        })?;
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(serde_json::from_slice)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            tracing::error!(error = %error, "metric drilldown row decoding failed");
            CanonicalError::internal("failed to decode metric evidence").create()
        })
}

fn build_csv(
    columns: &[MetricDrilldownColumn],
    rows: &[MetricDrilldownRow],
) -> Result<Vec<u8>, CanonicalError> {
    let mut writer = csv::Writer::from_writer(LimitedBuffer::new(MAX_EXPORT_BYTES));
    let headers = columns
        .iter()
        .map(|column| column.label.as_str())
        .collect::<Vec<_>>();
    writer
        .write_record(&headers)
        .map_err(|_| export_limit("CSV export exceeds the byte limit."))?;
    for row in rows {
        let values = export_values(columns, row)?
            .into_iter()
            .map(csv_safe_cell)
            .collect::<Vec<_>>();
        writer
            .write_record(values)
            .map_err(|_| export_limit("CSV export exceeds the byte limit."))?;
    }
    writer
        .into_inner()
        .map(LimitedBuffer::into_inner)
        .map_err(|_| export_limit("CSV export exceeds the byte limit."))
}

fn build_xlsx(
    columns: &[MetricDrilldownColumn],
    rows: &[MetricDrilldownRow],
) -> Result<Vec<u8>, CanonicalError> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    let date_format = Format::new().set_num_format("yyyy-mm-dd");
    for (column, header) in columns.iter().enumerate() {
        worksheet
            .write_string(
                0,
                u16::try_from(column).map_err(|_| export_internal())?,
                &header.label,
            )
            .map_err(|_| export_internal())?;
    }
    for (row_index, row) in rows.iter().enumerate() {
        let row_index = u32::try_from(row_index + 1).map_err(|_| export_internal())?;
        for (column_index, column) in columns.iter().enumerate() {
            let column_index = u16::try_from(column_index).map_err(|_| export_internal())?;
            let value = row
                .values
                .get(&column.key)
                .unwrap_or(&serde_json::Value::Null);
            match (column.r#type, value) {
                (_, serde_json::Value::Null) => worksheet
                    .write_blank(row_index, column_index, &Format::new())
                    .map_err(|_| export_internal())?,
                (crate::domain::metric_drilldown::MetricDrilldownColumnType::Number, value) => {
                    worksheet
                        .write_number(
                            row_index,
                            column_index,
                            value.as_f64().ok_or_else(export_internal)?,
                        )
                        .map_err(|_| export_internal())?
                }
                (
                    crate::domain::metric_drilldown::MetricDrilldownColumnType::Date,
                    serde_json::Value::String(value),
                ) => {
                    let date =
                        ExcelDateTime::parse_from_str(value).map_err(|_| export_internal())?;
                    worksheet
                        .write_datetime_with_format(row_index, column_index, &date, &date_format)
                        .map_err(|_| export_internal())?
                }
                (_, serde_json::Value::String(value)) => worksheet
                    .write_string(row_index, column_index, value)
                    .map_err(|_| export_internal())?,
                (_, serde_json::Value::Bool(value)) => worksheet
                    .write_boolean(row_index, column_index, *value)
                    .map_err(|_| export_internal())?,
                (_, value) => worksheet
                    .write_string(
                        row_index,
                        column_index,
                        serde_json::to_string(value).map_err(|_| export_internal())?,
                    )
                    .map_err(|_| export_internal())?,
            };
        }
    }
    let mut output = LimitedBuffer::new(MAX_EXPORT_BYTES);
    workbook.save_to_writer(&mut output).map_err(|error| {
        tracing::warn!(error = %error, "metric drilldown XLSX generation failed");
        export_limit("XLSX export exceeds the byte limit.")
    })?;
    Ok(output.into_inner())
}

fn export_values(
    columns: &[MetricDrilldownColumn],
    row: &MetricDrilldownRow,
) -> Result<Vec<String>, CanonicalError> {
    let values = columns
        .iter()
        .map(|column| {
            let value = row
                .values
                .get(&column.key)
                .unwrap_or(&serde_json::Value::Null);
            match value {
                serde_json::Value::Null => Ok(String::new()),
                serde_json::Value::String(value) => Ok(value.clone()),
                serde_json::Value::Bool(value) => Ok(value.to_string()),
                serde_json::Value::Number(value) => Ok(value.to_string()),
                value => serde_json::to_string(value).map_err(|_| export_internal()),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.iter().any(|value| value.len() > MAX_CELL_BYTES) {
        return Err(export_limit(format!(
            "Export contains a value exceeding the {MAX_CELL_BYTES} byte limit."
        )));
    }
    Ok(values)
}

fn ensure_export_input_bound(
    columns: &[MetricDrilldownColumn],
    rows: &[MetricDrilldownRow],
) -> Result<(), CanonicalError> {
    let mut bytes = columns
        .iter()
        .try_fold(0usize, |total, column| {
            total.checked_add(column.label.len() + 1)
        })
        .ok_or_else(|| export_limit("Export input exceeds the byte limit."))?;
    for row in rows {
        for value in export_values(columns, row)? {
            bytes = bytes
                .checked_add(value.len() + 1)
                .ok_or_else(|| export_limit("Export input exceeds the byte limit."))?;
            if bytes > MAX_EXPORT_BYTES {
                return Err(export_limit("Export input exceeds the byte limit."));
            }
        }
    }
    Ok(())
}

fn csv_safe_cell(value: String) -> String {
    if value.as_bytes().first().is_some_and(|first| {
        matches!(
            first,
            b'=' | b'+' | b'-' | b'@' | b'\t' | b'\r' | b'\n' | b' '
        )
    }) {
        format!("'{value}")
    } else {
        value
    }
}

struct LimitedBuffer {
    inner: Cursor<Vec<u8>>,
    limit: usize,
}

impl LimitedBuffer {
    fn new(limit: usize) -> Self {
        Self {
            inner: Cursor::new(Vec::new()),
            limit,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.inner.into_inner()
    }
}

impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let end = self
            .inner
            .position()
            .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            .ok_or_else(|| std::io::Error::other("export byte limit exceeded"))?;
        if end > self.limit as u64 {
            return Err(std::io::Error::other("export byte limit exceeded"));
        }
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for LimitedBuffer {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let offset = self.inner.seek(position)?;
        if offset > self.limit as u64 {
            return Err(std::io::Error::other("export byte limit exceeded"));
        }
        Ok(offset)
    }
}

fn safe_filename(metric_key: &str) -> String {
    metric_key
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn query_error(message: &str) -> CanonicalError {
    if message.contains("UNKNOWN_TABLE") || message.contains("Code: 60") {
        return MetricError::failed_precondition()
            .with_precondition_violation(
                "metric evidence relation",
                "The evidence relation backing this metric is unavailable.",
                "EVIDENCE_RELATION_MISSING",
            )
            .create();
    }
    if is_clickhouse_resource_limit(message) {
        return MetricError::resource_exhausted("Metric evidence query exceeded resource limits.")
            .with_quota_violation("metric evidence query", "ClickHouse resource limit reached")
            .create();
    }
    CanonicalError::internal("metric evidence query failed").create()
}

fn is_clickhouse_resource_limit(message: &str) -> bool {
    [
        "MEMORY_LIMIT_EXCEEDED",
        "TOO_MANY_SIMULTANEOUS_QUERIES",
        "TOO_MANY_ROWS_OR_BYTES",
        "QUOTA_EXCEEDED",
        "LIMIT_EXCEEDED",
        "Code: 198",
        "Code: 201",
        "Code: 202",
        "Code: 241",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

fn export_busy() -> CanonicalError {
    MetricError::resource_exhausted("Metric evidence export capacity is busy.")
        .with_quota_violation("metric evidence exports", "concurrency limit reached")
        .with_quota_violation_retry_after_seconds(2)
        .create()
}

fn query_busy() -> CanonicalError {
    MetricError::resource_exhausted("Metric evidence query capacity is busy.")
        .with_quota_violation("metric evidence queries", "concurrency limit reached")
        .with_quota_violation_retry_after_seconds(2)
        .create()
}

fn export_limit(description: impl Into<String>) -> CanonicalError {
    MetricError::resource_exhausted("Metric evidence export exceeded resource limits.")
        .with_quota_violation("metric evidence export", description.into())
        .create()
}

impl MetricDrilldownExportFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Xlsx => "xlsx",
        }
    }
}

fn export_internal() -> CanonicalError {
    CanonicalError::internal("failed to build metric evidence export").create()
}
