use std::io::{Cursor, Seek, SeekFrom, Write};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Body;
use axum::extract::Extension;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderValue, Response};
use rust_xlsxwriter::{ExcelDateTime, Format, Table, TableStyle, Workbook};
use tokio::sync::Semaphore;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::AppState;
use crate::api::error::MetricError;
use crate::domain::metric_drilldown::{
    EVIDENCE_QUERY_MEMORY_BYTES, EVIDENCE_QUERY_READ_BYTES, EVIDENCE_QUERY_RESULT_BYTES,
    EVIDENCE_QUERY_TIMEOUT_SECS, EvidenceQueryRow, MAX_EXPORT_ROWS, MetricDrilldownColumn,
    MetricDrilldownExportFormat, MetricDrilldownExportRequest, MetricDrilldownRequest,
    MetricDrilldownResponse, MetricDrilldownRow, build_response, compile_query, presentation,
    validate_export_request, validate_request, verify_evidence_snapshot,
};

const QUERY_TIMEOUT: Duration = Duration::from_secs(EVIDENCE_QUERY_TIMEOUT_SECS);
const EXPORT_TIMEOUT: Duration = Duration::from_mins(1);
const EXPORT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_EXPORT_BYTES: usize = 25 * 1024 * 1024;
const MAX_CELL_BYTES: usize = 32 * 1024;
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
    let filename = export_filename(
        &validated.plan.definition.base.label,
        &validated.selection.metric_key,
        &validated.selection.period.from,
        &validated.selection.period.to,
        validated
            .selection
            .filters
            .iter()
            .any(|filter| !filter.values.is_empty()),
        extension,
    );
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
    let formatted_rows = rows
        .iter()
        .map(|row| export_values(columns, row))
        .collect::<Result<Vec<_>, _>>()?;
    ensure_export_input_bound(columns, &formatted_rows)?;
    match format {
        MetricDrilldownExportFormat::Csv => Ok((
            build_csv(columns, formatted_rows)?,
            "text/csv; charset=utf-8",
            "csv",
        )),
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
        .with_option("max_memory_usage", EVIDENCE_QUERY_MEMORY_BYTES.to_string())
        .with_option("max_bytes_to_read", EVIDENCE_QUERY_READ_BYTES.to_string())
        .with_option("max_result_bytes", EVIDENCE_QUERY_RESULT_BYTES.to_string());
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
    rows: Vec<Vec<String>>,
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
        let values = row.into_iter().map(csv_safe_cell).collect::<Vec<_>>();
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
                    if let Some(value) = value.as_f64() {
                        worksheet
                            .write_number(row_index, column_index, value)
                            .map_err(|_| export_internal())?
                    } else {
                        worksheet
                            .write_string(row_index, column_index, value.to_string())
                            .map_err(|_| export_internal())?
                    }
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
    if !rows.is_empty() && !columns.is_empty() {
        let last_row = u32::try_from(rows.len()).map_err(|_| export_internal())?;
        let last_column = u16::try_from(columns.len() - 1).map_err(|_| export_internal())?;
        let table = Table::new()
            .set_style(TableStyle::None)
            .set_autofilter(false)
            .set_banded_rows(false);
        worksheet
            .add_table(0, 0, last_row, last_column, &table)
            .map_err(|_| export_internal())?;
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
    rows: &[Vec<String>],
) -> Result<(), CanonicalError> {
    let mut bytes = columns
        .iter()
        .try_fold(0usize, |total, column| {
            total.checked_add(column.label.len() + 1)
        })
        .ok_or_else(|| export_limit("Export input exceeds the byte limit."))?;
    for row in rows {
        for value in row {
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

fn export_filename(
    metric_label: &str,
    metric_key: &str,
    from: &str,
    to: &str,
    filtered: bool,
    extension: &str,
) -> String {
    let metric = filename_slug(metric_label);
    let metric = if metric.is_empty() {
        filename_slug(metric_key)
    } else {
        metric
    };
    let suffix = if filtered { "_filtered" } else { "" };
    format!("{metric}_{from}_{to}{suffix}.{extension}")
}

fn filename_slug(value: &str) -> String {
    const MAX_BYTES: usize = 80;

    let mut slug = String::with_capacity(value.len().min(MAX_BYTES));
    let mut separated = true;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if slug.len() == MAX_BYTES {
                break;
            }
            slug.push(character.to_ascii_lowercase());
            separated = false;
        } else if !separated && slug.len() < MAX_BYTES {
            slug.push('-');
            separated = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_drilldown::MetricDrilldownColumnType;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn columns() -> Vec<MetricDrilldownColumn> {
        vec![
            MetricDrilldownColumn {
                key: "ref".to_owned(),
                label: "Ref".to_owned(),
                r#type: MetricDrilldownColumnType::String,
            },
            MetricDrilldownColumn {
                key: "date".to_owned(),
                label: "Date".to_owned(),
                r#type: MetricDrilldownColumnType::Date,
            },
            MetricDrilldownColumn {
                key: "value".to_owned(),
                label: "Value".to_owned(),
                r#type: MetricDrilldownColumnType::Number,
            },
            MetricDrilldownColumn {
                key: "active".to_owned(),
                label: "Active".to_owned(),
                r#type: MetricDrilldownColumnType::String,
            },
        ]
    }

    fn row() -> MetricDrilldownRow {
        MetricDrilldownRow {
            values: BTreeMap::from([
                ("ref".to_owned(), json!("=formula")),
                ("date".to_owned(), json!("2026-07-28")),
                ("value".to_owned(), json!(12.5)),
                ("active".to_owned(), json!(true)),
            ]),
        }
    }

    #[test]
    fn csv_export_is_bounded_and_formula_safe() {
        let (bytes, content_type, extension) =
            build_export(MetricDrilldownExportFormat::Csv, &columns(), &[row()])
                .unwrap_or_else(|error| panic!("CSV export must succeed: {error}"));
        let csv = String::from_utf8(bytes)
            .unwrap_or_else(|error| panic!("CSV export must be UTF-8: {error}"));
        assert_eq!(content_type, "text/csv; charset=utf-8");
        assert_eq!(extension, "csv");
        assert!(csv.contains("'=formula"));
        assert!(csv.contains("2026-07-28"));
        assert!(csv.contains("12.5"));
    }

    #[test]
    fn xlsx_export_contains_typed_cells() {
        let (bytes, content_type, extension) =
            build_export(MetricDrilldownExportFormat::Xlsx, &columns(), &[row()])
                .unwrap_or_else(|error| panic!("XLSX export must succeed: {error}"));
        assert_eq!(
            content_type,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
        assert_eq!(extension, "xlsx");
        assert!(bytes.starts_with(b"PK"));
        assert!(bytes.len() > 1_000);
    }

    #[test]
    fn export_values_serialize_supported_json_values() {
        let columns = vec![
            MetricDrilldownColumn {
                key: "missing".to_owned(),
                label: "Missing".to_owned(),
                r#type: MetricDrilldownColumnType::String,
            },
            MetricDrilldownColumn {
                key: "object".to_owned(),
                label: "Object".to_owned(),
                r#type: MetricDrilldownColumnType::String,
            },
        ];
        let row = MetricDrilldownRow {
            values: BTreeMap::from([("object".to_owned(), json!({"key": "value"}))]),
        };
        assert_eq!(
            export_values(&columns, &row)
                .unwrap_or_else(|error| panic!("export values must serialize: {error}")),
            ["", r#"{"key":"value"}"#]
        );
    }

    #[test]
    fn oversized_export_cells_are_rejected() {
        let columns = vec![MetricDrilldownColumn {
            key: "value".to_owned(),
            label: "Value".to_owned(),
            r#type: MetricDrilldownColumnType::String,
        }];
        let row = MetricDrilldownRow {
            values: BTreeMap::from([("value".to_owned(), json!("x".repeat(MAX_CELL_BYTES + 1)))]),
        };
        assert!(export_values(&columns, &row).is_err());
    }

    #[test]
    fn limited_buffer_enforces_write_and_seek_bounds() {
        let mut buffer = LimitedBuffer::new(4);
        assert_eq!(
            buffer
                .write(b"1234")
                .unwrap_or_else(|error| panic!("bounded write must succeed: {error}")),
            4
        );
        assert!(buffer.write(b"5").is_err());
        assert!(buffer.seek(SeekFrom::Start(5)).is_err());
        assert_eq!(buffer.into_inner(), b"1234");
    }

    #[test]
    fn filenames_are_human_readable_and_bounded() {
        assert_eq!(
            export_filename(
                "Tasks closed",
                "tasks.closed",
                "2025-07-28",
                "2026-07-27",
                true,
                "xlsx"
            ),
            "tasks-closed_2025-07-28_2026-07-27_filtered.xlsx"
        );
        assert_eq!(filename_slug("***"), "");
        assert!(filename_slug(&"a".repeat(100)).len() <= 80);
    }

    #[test]
    fn query_errors_are_classified() {
        assert!(is_clickhouse_resource_limit("MEMORY_LIMIT_EXCEEDED"));
        assert!(is_clickhouse_resource_limit("Code: 241"));
        assert!(!is_clickhouse_resource_limit("syntax error"));
        let missing = query_error("UNKNOWN_TABLE");
        let limited = query_error("QUOTA_EXCEEDED");
        let internal = query_error("syntax error");
        assert_eq!(missing.status_code(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(
            limited.status_code(),
            axum::http::StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            internal.status_code(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn export_format_strings_are_stable() {
        assert_eq!(MetricDrilldownExportFormat::Csv.as_str(), "csv");
        assert_eq!(MetricDrilldownExportFormat::Xlsx.as_str(), "xlsx");
    }
}
