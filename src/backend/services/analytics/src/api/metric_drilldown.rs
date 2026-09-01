use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Body;
use axum::extract::Extension;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, Response};
use tokio::sync::Semaphore;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::AppState;
use crate::api::error::MetricError;
use crate::domain::metric_access::authorize_tenant_metrics;
use crate::domain::metric_drilldown::{
    EVIDENCE_QUERY_TIMEOUT_SECS, EvidenceQueryRow, MAX_EXPORT_BYTES, MAX_EXPORT_ROWS,
    MetricDrilldownColumn, MetricDrilldownEntity, MetricDrilldownExportFormat,
    MetricDrilldownExportRequest, MetricDrilldownRequest, MetricDrilldownResponse,
    MetricDrilldownRow, ValidatedMetricDrilldown, build_export, build_response, compile_query,
    decode_evidence_rows, evidence_unavailable, export_filename, export_internal, export_limit,
    parse_person_entity, parse_person_ids, presentation, presents_person, validate_export_request,
    validate_request, verify_evidence_snapshot, with_evidence_query_limits,
};
use crate::domain::person_visibility::authorize_person_ids;
use crate::infra::metrics::{self, ErrorClass, QueryKind, QueryOutcome};

use super::person_names;

const QUERY_TIMEOUT: Duration = Duration::from_secs(EVIDENCE_QUERY_TIMEOUT_SECS);
const EXPORT_TIMEOUT: Duration = Duration::from_mins(1);
const EXPORT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);
const QUERY_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CONCURRENT_EXPORTS: usize = 2;
const MAX_CONCURRENT_QUERIES: usize = 8;
static EXPORT_SEMAPHORE: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(MAX_CONCURRENT_EXPORTS));
static QUERY_SEMAPHORE: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(MAX_CONCURRENT_QUERIES));

pub async fn query_metric_drilldown(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Json(req): Json<MetricDrilldownRequest>,
) -> Result<Json<MetricDrilldownResponse>, CanonicalError> {
    let started = Instant::now();
    authorize_metric_entity(&state, &ctx, &headers, &req.entity).await?;

    let mut req = validate_request(&state.db, &state.ch, ctx.subject_tenant_id(), req).await?;
    req.enforce_tenant_scope = state.config.metric_catalog.enforce_tenant_scope;

    // Ahead of the read: the reader searches `Who` by name, and the names are
    // what turn that needle into identities the query can compare.
    let names = roster_names(&state, &req).await?;
    req.search_person_ids = people_named_like(&names, req.selection.search.as_deref());

    let log_comment = format!("metric-drilldown:page:{}", req.plan.definition.key());
    let rows = fetch_rows(&state, &req, QueryKind::DrilldownPage, &log_comment).await?;
    verify_evidence_snapshot(&state.ch, &req.plan.relation, &req.snapshot_id).await?;
    let fetched_rows = rows.len();
    let response = build_response(&req, rows, &names, &state.external_links)?;
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
    headers: HeaderMap,
    Json(req): Json<MetricDrilldownExportRequest>,
) -> Result<Response<Body>, CanonicalError> {
    let started = Instant::now();
    // Ahead of the permit: a caller who may not see this person must not occupy
    // one of MAX_CONCURRENT_EXPORTS slots.
    authorize_metric_entity(&state, &ctx, &headers, &req.entity).await?;

    let permit = acquire_export_permit().await?;
    let deadline = tokio::time::Instant::now() + EXPORT_TIMEOUT;

    let mut validated = validate_export_request(
        &state.db,
        &state.ch,
        ctx.subject_tenant_id(),
        &req,
        MAX_EXPORT_ROWS + 1,
    )
    .await?;
    validated.enforce_tenant_scope = state.config.metric_catalog.enforce_tenant_scope;
    let names = roster_names(&state, &validated).await?;
    validated.search_person_ids = people_named_like(&names, validated.selection.search.as_deref());

    let evidence = collect_export_rows(&state, &validated, deadline).await?;
    let exported_rows = evidence.len();

    let (columns, rows) = presentation(
        &evidence,
        &validated.plan,
        &validated.selection.filters,
        &validated.selection.display_dimensions,
        &validated.selection.entity,
        &names,
    )?;
    drop(evidence);

    let format = req.format;
    let (body, content_type, extension) =
        serialize_export(permit, format, columns, rows, deadline).await?;

    tracing::info!(
        duration_ms = started.elapsed().as_millis(),
        rows = exported_rows,
        bytes = body.len(),
        format = format.as_str(),
        row_limit = MAX_EXPORT_ROWS,
        byte_limit = MAX_EXPORT_BYTES,
        capacity = MAX_CONCURRENT_EXPORTS,
        "metric drilldown export completed"
    );

    attachment_response(body, content_type, &export_name(&validated, extension))
}

/// Who each row belongs to, in the words the reader knows them by.
///
/// Gated on the column actually being presented, not on the shape of the
/// entity: a roster read of a ratio metric shows no `Who`, and would otherwise
/// pay for a full identity scan per page for a map nothing reads.
///
/// The identity rows failing to answer names nobody rather than failing the
/// read — an empty column, not an error. Being unable to ASK is different, and
/// answers 429 like any other read this endpoint cannot find capacity for.
async fn roster_names(
    state: &AppState,
    validated: &ValidatedMetricDrilldown,
) -> Result<BTreeMap<String, String>, CanonicalError> {
    if !presents_person(validated) {
        return Ok(BTreeMap::new());
    }
    let Ok(ids) = parse_person_ids(&validated.selection.entity) else {
        return Ok(BTreeMap::new());
    };

    // INVARIANT: the permit is held across the awaited lookup below — the hold
    // is this endpoint's share of MAX_CONCURRENT_QUERIES, which the identity
    // scan belongs to as much as the evidence read does.
    let _permit = acquire_query_permit().await?;
    let names = tokio::time::timeout(
        QUERY_TIMEOUT,
        person_names::lookup_bounded(
            &state.ch,
            validated.tenant_id,
            &ids,
            with_evidence_query_limits,
        ),
    )
    .await
    .unwrap_or_else(|_| {
        tracing::warn!(
            people = ids.len(),
            "naming a roster exceeded the execution time limit"
        );
        std::collections::HashMap::new()
    });

    Ok(names
        .into_iter()
        .filter_map(|(id, name)| Some((id.to_string(), person_name(&name)?)))
        .collect())
}

/// The people the needle picks out of the `Who` column, as the ids the query
/// compares. Folded the same way the SQL search folds — case-insensitively,
/// on a substring.
fn people_named_like(names: &BTreeMap<String, String>, search: Option<&str>) -> Vec<String> {
    let Some(needle) = search.map(str::to_lowercase) else {
        return Vec::new();
    };
    names
        .iter()
        .filter(|(_, name)| name.to_lowercase().contains(&needle))
        .map(|(id, _)| id.clone())
        .collect()
}

fn person_name(name: &person_names::PersonName) -> Option<String> {
    [name.display_name.trim(), name.username.trim()]
        .into_iter()
        .find(|candidate| !candidate.is_empty())
        .map(str::to_owned)
}

// INVARIANT: both drilldown routes authorize the typed entity before validation
// touches MariaDB or ClickHouse, preventing person IDORs and tenant-gate bypasses.
async fn authorize_metric_entity(
    state: &AppState,
    ctx: &SecurityContext,
    headers: &HeaderMap,
    entity: &MetricDrilldownEntity,
) -> Result<(), CanonicalError> {
    match entity {
        MetricDrilldownEntity::Tenant {} => {
            authorize_tenant_metrics(state.config.metric_catalog.tenant_metrics_enabled)
        }
        MetricDrilldownEntity::Unknown => Err(MetricError::invalid_argument()
            .with_field_violation("entity.type", "unsupported entity type", "INVALID")
            .create()),
        MetricDrilldownEntity::Person { .. } => {
            let (_, person_id) = parse_person_entity(entity)?;

            authorize_person_ids(
                &state.identity,
                ctx,
                super::forwarded_authorization(headers),
                &[person_id],
            )
            .await
        }
        // Every person in the list, checked one by one: a rollup card is a
        // convenience over the same records, never a way around the gate that
        // decides which people this caller may read.
        MetricDrilldownEntity::Persons { .. } => {
            let person_ids = parse_person_ids(entity)?;

            authorize_person_ids(
                &state.identity,
                ctx,
                super::forwarded_authorization(headers),
                &person_ids,
            )
            .await
        }
    }
}

async fn acquire_export_permit() -> Result<tokio::sync::SemaphorePermit<'static>, CanonicalError> {
    // INVARIANT: the caller holds this permit across validation, fetch, and the
    // blocking serialization — the hold is the MAX_CONCURRENT_EXPORTS cap.
    tokio::time::timeout(EXPORT_ACQUIRE_TIMEOUT, EXPORT_SEMAPHORE.acquire())
        .await
        .map_err(|_| {
            tracing::warn!(
                capacity = MAX_CONCURRENT_EXPORTS,
                available = EXPORT_SEMAPHORE.available_permits(),
                "metric drilldown export capacity exhausted"
            );
            export_busy()
        })?
        .map_err(|_| export_busy())
}

async fn acquire_query_permit() -> Result<tokio::sync::SemaphorePermit<'static>, CanonicalError> {
    tokio::time::timeout(QUERY_ACQUIRE_TIMEOUT, QUERY_SEMAPHORE.acquire())
        .await
        .map_err(|_| {
            tracing::warn!(
                capacity = MAX_CONCURRENT_QUERIES,
                available = QUERY_SEMAPHORE.available_permits(),
                "metric drilldown query capacity exhausted"
            );
            query_busy()
        })?
        .map_err(|_| query_busy())
}

async fn collect_export_rows(
    state: &Arc<AppState>,
    validated: &ValidatedMetricDrilldown,
    deadline: tokio::time::Instant,
) -> Result<Vec<EvidenceQueryRow>, CanonicalError> {
    let log_comment = format!(
        "metric-drilldown:export:{}",
        validated.plan.definition.key()
    );
    // The outer deadline cancels the fetch future, so its own metric never
    // records — the timeout is this query's terminal outcome, recorded here.
    let started = Instant::now();
    let Ok(fetched) = tokio::time::timeout_at(
        deadline,
        fetch_rows(state, validated, QueryKind::DrilldownExport, &log_comment),
    )
    .await
    else {
        metrics::record_query(
            QueryKind::DrilldownExport,
            QueryOutcome::Error,
            started.elapsed(),
        );
        return Err(export_limit("Export exceeded the execution time limit."));
    };
    let rows = fetched?;
    verify_evidence_snapshot(&state.ch, &validated.plan.relation, &validated.snapshot_id).await?;

    enforce_export_row_limit(rows.len())?;
    Ok(rows)
}

fn enforce_export_row_limit(rows: usize) -> Result<(), CanonicalError> {
    if rows > MAX_EXPORT_ROWS {
        return Err(export_limit(format!(
            "Export exceeds the {MAX_EXPORT_ROWS} row limit."
        )));
    }
    Ok(())
}

async fn serialize_export(
    permit: tokio::sync::SemaphorePermit<'static>,
    format: MetricDrilldownExportFormat,
    columns: Vec<MetricDrilldownColumn>,
    rows: Vec<MetricDrilldownRow>,
    deadline: tokio::time::Instant,
) -> Result<(Vec<u8>, &'static str, &'static str), CanonicalError> {
    let blocking_deadline = deadline.into_std();
    let export = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        build_export(format, &columns, &rows, blocking_deadline)
    });
    tokio::time::timeout_at(deadline, export)
        .await
        .map_err(|_| export_limit("Export exceeded the execution time limit."))?
        .map_err(|_| export_internal())?
}

fn export_name(validated: &ValidatedMetricDrilldown, extension: &str) -> String {
    export_filename(
        &validated.plan.definition.base.label,
        &validated.selection.metric_key,
        &validated.selection.period.from,
        &validated.selection.period.to,
        validated
            .selection
            .filters
            .iter()
            .any(|filter| !filter.values.is_empty())
            || validated.selection.search.is_some(),
        extension,
    )
}

fn attachment_response(
    body: Vec<u8>,
    content_type: &'static str,
    filename: &str,
) -> Result<Response<Body>, CanonicalError> {
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

async fn fetch_rows(
    state: &Arc<AppState>,
    req: &crate::domain::metric_drilldown::ValidatedMetricDrilldown,
    kind: QueryKind,
    log_comment: &str,
) -> Result<Vec<EvidenceQueryRow>, CanonicalError> {
    let started = Instant::now();
    let result = fetch_rows_inner(state, req, kind, log_comment).await;

    let outcome = match &result {
        Ok(_) => QueryOutcome::Success,
        Err(_) => QueryOutcome::Error,
    };
    metrics::record_query(kind, outcome, started.elapsed());
    result
}

async fn fetch_rows_inner(
    state: &Arc<AppState>,
    req: &crate::domain::metric_drilldown::ValidatedMetricDrilldown,
    kind: QueryKind,
    log_comment: &str,
) -> Result<Vec<EvidenceQueryRow>, CanonicalError> {
    // INVARIANT: the permit is held across the awaited ClickHouse execution and
    // byte collection below — the hold is the MAX_CONCURRENT_QUERIES cap.
    let _permit = acquire_query_permit().await?;
    let (sql, params) = compile_query(req)?;
    let base = state
        .ch
        .query(&sql)
        .with_setting("log_comment", log_comment);
    let mut query = with_evidence_query_limits(base).with_setting("max_threads", "2");
    for param in params {
        query = query.bind(param);
    }
    let mut cursor = query.fetch_bytes("JSONEachRow").map_err(|error| {
        tracing::error!(error = %error, "ClickHouse metric drilldown query failed");
        let message = error.to_string();
        metrics::record_clickhouse_error(kind, ErrorClass::classify(&message));
        query_error(&message)
    })?;
    let bytes = tokio::time::timeout(QUERY_TIMEOUT, cursor.collect())
        .await
        .map_err(|_| {
            tracing::error!("metric evidence query exceeded the execution time limit");
            metrics::record_clickhouse_error(kind, ErrorClass::Timeout);
            query_limit_error()
        })?
        .map_err(|error| {
            tracing::error!(error = %error, "ClickHouse metric drilldown fetch failed");
            let message = error.to_string();
            metrics::record_clickhouse_error(kind, ErrorClass::classify(&message));
            query_error(&message)
        })?;

    decode_evidence_rows(&bytes).map_err(|error| {
        tracing::error!(error = %error, "metric drilldown row decoding failed");
        metrics::record_clickhouse_error(kind, ErrorClass::ParseFailed);
        CanonicalError::internal("failed to decode metric evidence").create()
    })
}

// INVARIANT: the response classification shares the one classifier the metric
// label records through, so a message can never map to two different classes.
fn query_error(message: &str) -> CanonicalError {
    match ErrorClass::classify(message) {
        ErrorClass::RelationMissing => evidence_unavailable(),
        ErrorClass::ResourceExhausted => query_limit_error(),
        ErrorClass::Timeout | ErrorClass::ParseFailed | ErrorClass::QueryFailed => {
            CanonicalError::internal("metric evidence query failed").create()
        }
    }
}

fn query_limit_error() -> CanonicalError {
    MetricError::resource_exhausted("Metric evidence query exceeded resource limits.")
        .with_quota_violation("metric evidence query", "ClickHouse resource limit reached")
        .create()
}

fn export_busy() -> CanonicalError {
    MetricError::resource_exhausted("Metric evidence export capacity is busy.")
        .with_quota_violation("metric evidence exports", "concurrency limit reached")
        .with_quota_violation_retry_after_seconds(EXPORT_ACQUIRE_TIMEOUT.as_secs())
        .create()
}

fn query_busy() -> CanonicalError {
    MetricError::resource_exhausted("Metric evidence query capacity is busy.")
        .with_quota_violation("metric evidence queries", "concurrency limit reached")
        .with_quota_violation_retry_after_seconds(QUERY_ACQUIRE_TIMEOUT.as_secs())
        .create()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_needle_picks_the_people_it_names_case_insensitively() {
        let names = BTreeMap::from([
            ("id-a".to_owned(), "Ada Example".to_owned()),
            ("id-b".to_owned(), "Grace Park".to_owned()),
        ]);

        assert_eq!(people_named_like(&names, Some("ada")), ["id-a"]);
        assert_eq!(people_named_like(&names, Some("PARK")), ["id-b"]);
        assert!(people_named_like(&names, Some("nobody")).is_empty());
        assert!(people_named_like(&names, None).is_empty());
    }

    #[test]
    fn a_person_is_named_by_whichever_name_identity_holds() {
        assert_eq!(
            person_name(&person_names::PersonName::named("Ada Example", "ada")).as_deref(),
            Some("Ada Example")
        );
        assert_eq!(
            person_name(&person_names::PersonName::named("  ", "ada")).as_deref(),
            Some("ada")
        );
        assert_eq!(person_name(&person_names::PersonName::named("", "")), None);
    }

    #[test]
    fn query_errors_are_classified() {
        use axum::http::StatusCode;

        let cases = [
            ("UNKNOWN_TABLE", StatusCode::BAD_REQUEST),
            ("Code: 60. Table x does not exist", StatusCode::BAD_REQUEST),
            ("QUOTA_EXCEEDED", StatusCode::TOO_MANY_REQUESTS),
            (
                "Code: 241. Memory limit exceeded",
                StatusCode::TOO_MANY_REQUESTS,
            ),
            ("syntax error", StatusCode::INTERNAL_SERVER_ERROR),
            // A longer code sharing the prefix must not read as missing-relation 60.
            ("Code: 600. novel", StatusCode::INTERNAL_SERVER_ERROR),
            ("Code: 60 no period", StatusCode::INTERNAL_SERVER_ERROR),
        ];
        for (message, expected) in cases {
            assert_eq!(
                query_error(message).status_code(),
                expected,
                "wrong classification: {message}"
            );
        }
    }

    #[test]
    fn the_export_row_limit_refuses_only_past_the_cap() {
        assert!(enforce_export_row_limit(MAX_EXPORT_ROWS).is_ok());
        let refused = enforce_export_row_limit(MAX_EXPORT_ROWS + 1)
            .err()
            .map(|error| error.status_code());
        assert_eq!(
            refused,
            Some(axum::http::StatusCode::TOO_MANY_REQUESTS.as_u16())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn export_permits_refuse_past_the_concurrency_cap() {
        let mut held = Vec::new();
        for _ in 0..MAX_CONCURRENT_EXPORTS {
            held.extend(acquire_export_permit().await.ok());
        }
        assert_eq!(held.len(), MAX_CONCURRENT_EXPORTS);
        let refused = acquire_export_permit()
            .await
            .err()
            .map(|error| error.status_code());
        assert_eq!(
            refused,
            Some(axum::http::StatusCode::TOO_MANY_REQUESTS.as_u16())
        );
        drop(held);
        assert!(acquire_export_permit().await.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn query_permits_refuse_past_the_concurrency_cap() {
        let mut held = Vec::new();
        for _ in 0..MAX_CONCURRENT_QUERIES {
            held.extend(acquire_query_permit().await.ok());
        }
        assert_eq!(held.len(), MAX_CONCURRENT_QUERIES);
        let refused = acquire_query_permit()
            .await
            .err()
            .map(|error| error.status_code());
        assert_eq!(
            refused,
            Some(axum::http::StatusCode::TOO_MANY_REQUESTS.as_u16())
        );
        drop(held);
        assert!(acquire_query_permit().await.is_ok());
    }
}
