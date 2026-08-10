//! Custom-metric CRUD + export/import handlers — `/v1/metrics*`.
//!
//! Every request is tenant-scoped from the session `SecurityContext` and only
//! ever touches `origin='custom'` rows, so a builtin (`registry.yaml`-owned) key
//! is invisible here and can never be mutated. The observation SQL is validated
//! two ways on write: the single-SELECT gate (pure) and a `LIMIT 0` column
//! probe against ClickHouse that fails the write if the SQL does not emit the
//! observation contract. Export strips tenant/origin; import re-homes each graph
//! to the caller's tenant, keys on `metric_key`, and skips one that already
//! exists (idempotent).

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use sea_orm::DbErr;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::AppState;
use super::error::CustomMetricError;
use crate::domain::metric_crud::{
    CustomMetric, CustomMetricListResponse, ExportCustomMetricsResponse, GraphViolation,
    ImportCustomMetricsRequest, ImportCustomMetricsResponse, MAX_IMPORT_METRICS, ReplaceOutcome,
    WriteOutcome, create_custom_metric, delete_custom_metric, export_custom_metrics,
    fetch_custom_metric, import_custom_metrics, list_custom_metrics as list_custom_metrics_repo,
    normalize_observation_sql, replace_custom_metric, validate_graph,
};

/// The observation-contract columns a custom source's SQL must emit. Probed as
/// a `LIMIT 0` projection so a malformed source fails on write, not at render.
const OBSERVATION_CONTRACT_COLUMNS: &str = "tenant_id, source_key, entity_type, entity_id, \
    metric_date, measure_key, observed_at, value, subject_key, dimensions";

/// ClickHouse-side execution cap for the observation probe (seconds).
const PROBE_MAX_EXECUTION_SECS: u32 = 5;
/// Outer wall-clock deadline for the observation probe.
const PROBE_DEADLINE: Duration = Duration::from_secs(10);

pub async fn list_custom_metrics(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    let items = list_custom_metrics_repo(&state.db, ctx.subject_tenant_id())
        .await
        .map_err(db_error)?;
    Ok(Json(CustomMetricListResponse { items }))
}

pub async fn get_custom_metric(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(metric_key): Path<String>,
) -> Result<impl IntoResponse, CanonicalError> {
    let metric = fetch_custom_metric(&state.db, ctx.subject_tenant_id(), &metric_key)
        .await
        .map_err(db_error)?
        .ok_or_else(|| not_found(&metric_key))?;
    Ok(Json(metric))
}

pub async fn create_custom_metric_handler(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(mut graph): Json<CustomMetric>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant_id = ctx.subject_tenant_id();
    graph.origin = None;
    graph.observation_sql = normalize_observation_sql(&graph.observation_sql);
    validate_graph(&graph).map_err(invalid_graph)?;
    probe_observation_sql(&state.ch, &graph.observation_sql).await?;

    match create_custom_metric(&state.db, tenant_id, &graph)
        .await
        .map_err(db_error)?
    {
        WriteOutcome::Created => {
            let created = reload(&state, tenant_id, &graph.metric_key).await?;
            Ok((StatusCode::CREATED, Json(created)))
        }
        WriteOutcome::AlreadyExists => Err(conflict(&graph.metric_key)),
    }
}

pub async fn update_custom_metric_handler(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(metric_key): Path<String>,
    Json(mut graph): Json<CustomMetric>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant_id = ctx.subject_tenant_id();
    // The path key is authoritative — a custom metric's identity never changes
    // under an update.
    graph.metric_key = metric_key.clone();
    graph.origin = None;
    graph.observation_sql = normalize_observation_sql(&graph.observation_sql);
    validate_graph(&graph).map_err(invalid_graph)?;
    probe_observation_sql(&state.ch, &graph.observation_sql).await?;

    match replace_custom_metric(&state.db, tenant_id, &metric_key, &graph)
        .await
        .map_err(db_error)?
    {
        ReplaceOutcome::Replaced => {
            let updated = reload(&state, tenant_id, &metric_key).await?;
            Ok(Json(updated))
        }
        ReplaceOutcome::NotFound => Err(not_found(&metric_key)),
        ReplaceOutcome::SourceConflict => Err(source_conflict(&graph.source_key)),
    }
}

pub async fn delete_custom_metric_handler(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(metric_key): Path<String>,
) -> Result<impl IntoResponse, CanonicalError> {
    let deleted = delete_custom_metric(&state.db, ctx.subject_tenant_id(), &metric_key)
        .await
        .map_err(db_error)?;
    if !deleted {
        return Err(not_found(&metric_key));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn export_custom_metrics_handler(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    let metrics = export_custom_metrics(&state.db, ctx.subject_tenant_id())
        .await
        .map_err(db_error)?;
    Ok(Json(ExportCustomMetricsResponse { metrics }))
}

pub async fn import_custom_metrics_handler(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<ImportCustomMetricsRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    if req.metrics.len() > MAX_IMPORT_METRICS {
        return Err(invalid_graph(GraphViolation {
            field: "metrics",
            reason: format!("at most {MAX_IMPORT_METRICS} metrics per import"),
        }));
    }

    // Validate and probe the whole batch before writing any row, so a malformed
    // metric fails the import cleanly instead of after earlier ones have landed.
    let tenant_id = ctx.subject_tenant_id();
    let mut graphs = req.metrics;
    for graph in &mut graphs {
        graph.origin = None;
        graph.observation_sql = normalize_observation_sql(&graph.observation_sql);
        validate_graph(graph).map_err(invalid_graph)?;
        probe_observation_sql(&state.ch, &graph.observation_sql).await?;
    }

    let skipped = import_custom_metrics(&state.db, tenant_id, &graphs)
        .await
        .map_err(db_error)?;
    let imported = graphs.len() - skipped.len();

    Ok(Json(ImportCustomMetricsResponse { imported, skipped }))
}

// ── Helpers ─────────────────────────────────────────────────

/// Author-time observation-contract probe: wrap the (already single-SELECT
/// gated) SQL exactly as the compiler does and project the contract columns
/// with `LIMIT 0`, so a source that omits a column or does not parse fails the
/// write. The raw engine message is logged, never returned, so ClickHouse
/// internals do not leak to the client.
async fn probe_observation_sql(
    ch: &insight_clickhouse::Client,
    sql: &str,
) -> Result<(), CanonicalError> {
    // The probe runs author-supplied SQL, so it is bounded twice: ClickHouse
    // `SETTINGS` cap the work the engine will do, and an outer wall-clock
    // deadline caps a stalled connection — neither the engine nor the request
    // thread can be held open by an expensive source.
    //
    // INVARIANT: no `max_rows_to_read` here — ClickHouse enforces it against
    // the planner's storage-scan estimate before `LIMIT 0` applies, so any cap
    // rejects every real (multi-row) source while `LIMIT 0` already keeps the
    // read at zero rows.
    let probe = format!(
        "SELECT {OBSERVATION_CONTRACT_COLUMNS} FROM ({sql}) LIMIT 0 \
         SETTINGS max_execution_time = {PROBE_MAX_EXECUTION_SECS}, max_result_rows = 0, \
         timeout_overflow_mode = 'throw'"
    );

    let run = async {
        let mut cursor = ch.query(&probe).fetch_bytes("JSONEachRow")?;
        cursor.collect().await.map(|_| ())
    };

    let outcome = match tokio::time::timeout(PROBE_DEADLINE, run).await {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(_elapsed) => Err(format!("probe exceeded {PROBE_DEADLINE:?}")),
    };

    outcome.map_err(|error| {
        tracing::warn!(error = %error, "custom observation SQL failed the column probe");
        invalid_observation()
    })
}

async fn reload(
    state: &AppState,
    tenant_id: uuid::Uuid,
    metric_key: &str,
) -> Result<CustomMetric, CanonicalError> {
    fetch_custom_metric(&state.db, tenant_id, metric_key)
        .await
        .map_err(db_error)?
        .ok_or_else(|| {
            tracing::error!(%metric_key, "custom metric vanished immediately after write");
            CanonicalError::internal("custom metric operation failed").create()
        })
}

fn invalid_graph(violation: GraphViolation) -> CanonicalError {
    CustomMetricError::invalid_argument()
        .with_field_violation(violation.field, violation.reason, "INVALID")
        .create()
}

fn invalid_observation() -> CanonicalError {
    CustomMetricError::invalid_argument()
        .with_field_violation(
            "observation_sql",
            "must emit the observation contract columns as a single read",
            "INVALID",
        )
        .create()
}

fn not_found(metric_key: &str) -> CanonicalError {
    CustomMetricError::not_found("custom metric not found")
        .with_resource(metric_key.to_owned())
        .create()
}

fn conflict(metric_key: &str) -> CanonicalError {
    CustomMetricError::already_exists("custom metric already exists")
        .with_resource(metric_key.to_owned())
        .create()
}

fn source_conflict(source_key: &str) -> CanonicalError {
    CustomMetricError::already_exists("source_key already belongs to another custom metric")
        .with_resource(source_key.to_owned())
        .create()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "used as `.map_err(db_error)`, which hands over an owned DbErr"
)]
fn db_error(error: DbErr) -> CanonicalError {
    tracing::error!(error = %error, "custom metric database operation failed");
    CanonicalError::internal("custom metric operation failed").create()
}

/// Live ClickHouse tests for the observation probe. `#[ignore]`d and skip
/// silently when `INTEGRATION_TESTS_CLICKHOUSE_URL` is unset (same convention
/// as the MariaDB `live_tests`); optional auth via
/// `INTEGRATION_TESTS_CLICKHOUSE_USER` / `INTEGRATION_TESTS_CLICKHOUSE_PASSWORD`.
#[cfg(test)]
mod probe_live_tests {
    use uuid::Uuid;

    use super::probe_observation_sql;

    const URL_VAR: &str = "INTEGRATION_TESTS_CLICKHOUSE_URL";

    fn client_or_skip() -> Option<insight_clickhouse::Client> {
        let Ok(url) = std::env::var(URL_VAR) else {
            eprintln!("skipping: {URL_VAR} not set");
            return None;
        };
        let mut config = insight_clickhouse::Config::new(url, "default");
        if let (Ok(user), Ok(password)) = (
            std::env::var("INTEGRATION_TESTS_CLICKHOUSE_USER"),
            std::env::var("INTEGRATION_TESTS_CLICKHOUSE_PASSWORD"),
        ) {
            config = config.with_auth(user, password);
        }
        Some(insight_clickhouse::Client::new(config))
    }

    async fn create_contract_table(
        ch: &insight_clickhouse::Client,
        rows: u32,
    ) -> Result<String, insight_clickhouse::Error> {
        let table = format!("probe_live_{}", Uuid::now_v7().simple());
        ch.query(&format!(
            "CREATE TABLE {table} (tenant_id UUID, source_key String, entity_type String, \
             entity_id UUID, metric_date Date, measure_key String, observed_at DateTime64(3), \
             value Float64, subject_key Nullable(String), \
             dimensions Array(Tuple(key String, value String, label Nullable(String)))) \
             ENGINE = MergeTree ORDER BY (tenant_id, metric_date)"
        ))
        .execute()
        .await?;
        ch.query(&format!(
            "INSERT INTO {table} SELECT generateUUIDv4(), 'probe', 'person', generateUUIDv4(), \
             toDate('2026-01-01') + number, 'measure', now64(3), number, NULL, [] \
             FROM numbers({rows})"
        ))
        .execute()
        .await?;
        Ok(table)
    }

    async fn drop_table(ch: &insight_clickhouse::Client, table: &str) {
        let _ = ch
            .query(&format!("DROP TABLE IF EXISTS {table}"))
            .execute()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires live ClickHouse; set INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
    async fn probe_accepts_a_source_larger_than_one_row() -> anyhow::Result<()> {
        let Some(ch) = client_or_skip() else {
            return Ok(());
        };
        let table = create_contract_table(&ch, 1000).await?;

        let result = probe_observation_sql(&ch, &format!("SELECT * FROM {table}")).await;

        drop_table(&ch, &table).await;
        anyhow::ensure!(
            result.is_ok(),
            "the probe must validate columns, not source size (cf/insight#2337)"
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires live ClickHouse; set INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
    async fn probe_rejects_a_source_missing_contract_columns() -> anyhow::Result<()> {
        let Some(ch) = client_or_skip() else {
            return Ok(());
        };

        let result = probe_observation_sql(&ch, "SELECT 1 AS tenant_id").await;

        anyhow::ensure!(
            result.is_err(),
            "a source without the contract columns must fail the probe"
        );
        Ok(())
    }
}
