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
    EVIDENCE_QUERY_TIMEOUT_SECS, EvidencePlan, EvidenceQueryRow, MAX_EXPORT_BYTES, MAX_EXPORT_ROWS,
    MetricDrilldownColumn, MetricDrilldownEntity, MetricDrilldownExportFormat,
    MetricDrilldownExportRequest, MetricDrilldownRequest, MetricDrilldownResponse,
    MetricDrilldownRow, MetricDrilldownSelection, ValidatedDrilldown, ValidatedMetricDrilldown,
    build_export, build_response, compile_query, decode_evidence_rows, export_filename,
    export_internal, export_limit, parse_person_entity, parse_person_ids, presentation,
    presents_person, validate_export_request, validate_request, verify_evidence_snapshot,
    with_evidence_query_limits,
};
use crate::domain::not_ready::NotReadyReason;
use crate::domain::person_visibility::authorize_person_ids;
use crate::infra::metrics::{self, ErrorClass, QueryKind, QueryOutcome};
use crate::infra::not_ready;

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

    let mut req = match validate_request(&state.db, &state.ch, ctx.subject_tenant_id(), req).await?
    {
        ValidatedDrilldown::Ready(req) => *req,
        ValidatedDrilldown::NotReady(page) => {
            return Ok(Json(MetricDrilldownResponse::not_ready(
                page.selection,
                page.columns,
                page.reason,
            )));
        }
    };

    // Ahead of the read: the reader searches `Who` by name, and the names are
    // what turn that needle into identities the query can compare.
    let names = roster_names(&state, &req).await?;
    req.search_person_ids = people_named_like(&names, req.selection.search.as_deref());

    let log_comment = format!("metric-drilldown:page:{}", req.plan.definition.key());
    let rows = match fetch_rows(&state, &req, QueryKind::DrilldownPage, &log_comment).await? {
        EvidenceRead::Rows(rows) => rows,
        EvidenceRead::NotReady(reason) => {
            return Ok(Json(not_ready_page(&req, &names, reason)?));
        }
    };
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

    let validated = validate_export_request(
        &state.db,
        &state.ch,
        ctx.subject_tenant_id(),
        &req,
        MAX_EXPORT_ROWS + 1,
    )
    .await?;
    // A relation nothing has built yet exports as the file it will be: the
    // columns the plan declares, and no records.
    let mut validated = match validated {
        ValidatedDrilldown::Ready(validated) => *validated,
        ValidatedDrilldown::NotReady(page) => {
            return empty_export(
                permit,
                req.format,
                page.columns,
                &page.plan,
                &page.selection,
                deadline,
            )
            .await;
        }
    };
    let names = roster_names(&state, &validated).await?;
    validated.search_person_ids = people_named_like(&names, validated.selection.search.as_deref());

    let evidence = match collect_export_rows(&state, &validated, deadline).await? {
        EvidenceRead::Rows(evidence) => evidence,
        EvidenceRead::NotReady(_) => {
            return empty_export(
                permit,
                req.format,
                presented_columns(&validated, &names)?,
                &validated.plan,
                &validated.selection,
                deadline,
            )
            .await;
        }
    };
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

    attachment_response(
        body,
        content_type,
        &export_name(&validated.plan, &validated.selection, extension),
    )
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
) -> Result<EvidenceRead, CanonicalError> {
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
    let rows = match fetched? {
        EvidenceRead::Rows(rows) => rows,
        not_ready @ EvidenceRead::NotReady(_) => return Ok(not_ready),
    };
    verify_evidence_snapshot(&state.ch, &validated.plan.relation, &validated.snapshot_id).await?;

    enforce_export_row_limit(rows.len())?;
    Ok(EvidenceRead::Rows(rows))
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

fn export_name(
    plan: &EvidencePlan,
    selection: &MetricDrilldownSelection,
    extension: &str,
) -> String {
    export_filename(
        &plan.definition.base.label,
        &selection.metric_key,
        &selection.period.from,
        &selection.period.to,
        selection
            .filters
            .iter()
            .any(|filter| !filter.values.is_empty())
            || selection.search.is_some(),
        extension,
    )
}

/// The file a not-ready relation downloads as: its columns, and no records.
async fn empty_export(
    permit: tokio::sync::SemaphorePermit<'static>,
    format: MetricDrilldownExportFormat,
    columns: Vec<MetricDrilldownColumn>,
    plan: &EvidencePlan,
    selection: &MetricDrilldownSelection,
    deadline: tokio::time::Instant,
) -> Result<Response<Body>, CanonicalError> {
    let (body, content_type, extension) =
        serialize_export(permit, format, columns, Vec::new(), deadline).await?;

    attachment_response(body, content_type, &export_name(plan, selection, extension))
}

/// The page a relation answers before anything has built it: what the reader
/// asked for, the columns it would present, and no rows.
fn not_ready_page(
    req: &ValidatedMetricDrilldown,
    names: &BTreeMap<String, String>,
    reason: NotReadyReason,
) -> Result<MetricDrilldownResponse, CanonicalError> {
    Ok(MetricDrilldownResponse::not_ready(
        req.selection.clone(),
        presented_columns(req, names)?,
        reason,
    ))
}

/// The columns a selection presents, read off the plan rather than off rows —
/// what a not-ready answer still has to carry.
fn presented_columns(
    req: &ValidatedMetricDrilldown,
    names: &BTreeMap<String, String>,
) -> Result<Vec<MetricDrilldownColumn>, CanonicalError> {
    let (columns, _) = presentation(
        &[],
        &req.plan,
        &req.selection.filters,
        &req.selection.display_dimensions,
        &req.selection.entity,
        names,
    )?;

    Ok(columns)
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

/// What an evidence read answered: the records, or that the relation behind
/// them has not been built yet.
enum EvidenceRead {
    Rows(Vec<EvidenceQueryRow>),
    NotReady(NotReadyReason),
}

async fn fetch_rows(
    state: &Arc<AppState>,
    req: &crate::domain::metric_drilldown::ValidatedMetricDrilldown,
    kind: QueryKind,
    log_comment: &str,
) -> Result<EvidenceRead, CanonicalError> {
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
) -> Result<EvidenceRead, CanonicalError> {
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
    let mut cursor = match query.fetch_bytes("JSONEachRow") {
        Ok(cursor) => cursor,
        Err(error) => return read_outcome(&error.to_string(), kind, log_comment),
    };
    let collected = tokio::time::timeout(QUERY_TIMEOUT, cursor.collect())
        .await
        .map_err(|_| {
            tracing::error!("metric evidence query exceeded the execution time limit");
            metrics::record_clickhouse_error(kind, ErrorClass::Timeout);
            query_limit_error()
        })?;
    let bytes = match collected {
        Ok(bytes) => bytes,
        Err(error) => return read_outcome(&error.to_string(), kind, log_comment),
    };

    decode_evidence_rows(&bytes)
        .map(EvidenceRead::Rows)
        .map_err(|error| {
            tracing::error!(error = %error, "metric drilldown row decoding failed");
            metrics::record_clickhouse_error(kind, ErrorClass::ParseFailed);
            CanonicalError::internal("failed to decode metric evidence").create()
        })
}

/// What a read that produced no rows answers.
///
/// INVARIANT: the classification shares the one classifier the metric label
/// records through, so a message can never map to two different classes. Only
/// an absent relation is an answer; everything else, `UNKNOWN_IDENTIFIER`
/// included, is the failure it is.
fn read_outcome(
    message: &str,
    kind: QueryKind,
    log_comment: &str,
) -> Result<EvidenceRead, CanonicalError> {
    let class = ErrorClass::classify(message);
    metrics::record_clickhouse_error(kind, class);

    match class {
        ErrorClass::RelationMissing => {
            not_ready::warn_throttled(message, log_comment);
            Ok(EvidenceRead::NotReady(
                NotReadyReason::SourceRelationMissing,
            ))
        }
        ErrorClass::ResourceExhausted => Err(query_limit_error()),
        ErrorClass::Timeout | ErrorClass::ParseFailed | ErrorClass::QueryFailed => {
            tracing::error!(error = %message, comment = log_comment, "ClickHouse metric drilldown read failed");
            Err(CanonicalError::internal("metric evidence query failed").create())
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

    fn outcome_status(message: &str) -> Option<u16> {
        match read_outcome(message, QueryKind::DrilldownPage, "test") {
            Ok(EvidenceRead::NotReady(_)) => None,
            Ok(EvidenceRead::Rows(_)) => panic!("a failed read cannot produce rows"),
            Err(error) => Some(error.status_code()),
        }
    }

    /// An absent table (60) or database (81) is the answer "nothing here yet",
    /// so the read carries no status at all.
    #[test]
    fn an_absent_relation_reads_as_not_ready() {
        for message in [
            "Code: 60. DB::Exception: Table x does not exist. (UNKNOWN_TABLE)",
            "Code: 60. Table x does not exist",
            "Code: 81. DB::Exception: Database insight does not exist. (UNKNOWN_DATABASE)",
        ] {
            assert_eq!(outcome_status(message), None, "should answer: {message}");
        }
    }

    /// Code 47 means the relation is there and its columns disagree with the
    /// deployed code — the one thing an empty answer would hide.
    #[test]
    fn a_column_the_relation_does_not_have_still_fails() {
        use axum::http::StatusCode;

        assert_eq!(
            outcome_status(
                "Code: 47. DB::Exception: Unknown expression identifier (UNKNOWN_IDENTIFIER)"
            ),
            Some(StatusCode::INTERNAL_SERVER_ERROR.as_u16())
        );
    }

    #[test]
    fn every_other_failure_keeps_its_status() {
        use axum::http::StatusCode;

        let cases = [
            // A failure that merely quotes an absent relation is still a
            // failure: the code it reports is its own, not the nested one.
            (
                "Code: 279. All connection tries failed: Code: 60. (UNKNOWN_TABLE)",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
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
                outcome_status(message),
                Some(expected.as_u16()),
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

    /// The page a read answers when the relation went missing under it: the
    /// columns come off the plan, since there are no rows to read them from.
    #[test]
    fn a_not_ready_page_carries_the_columns_the_plan_declares_and_no_rows() {
        use crate::domain::metric_definitions::ComputationSpec;
        use crate::domain::metric_definitions::definition::MetricInputRole;
        use crate::domain::metric_drilldown::test_support::{commit_input, input, plan, validated};

        let req = validated(plan(
            ComputationSpec::Sum {
                value: input(MetricInputRole::Value, "commit_count"),
            },
            vec![commit_input(MetricInputRole::Value, "commit_count")],
        ));
        let names = BTreeMap::new();

        let page = not_ready_page(&req, &names, NotReadyReason::SourceRelationMissing)
            .unwrap_or_else(|error| panic!("a not-ready page must build: {error}"));

        assert_eq!(page.not_ready, Some(NotReadyReason::SourceRelationMissing));
        assert!(page.rows.is_empty(), "a not-ready page carries no rows");
        assert_eq!(page.next_cursor, None, "there is no next page of nothing");
        assert_eq!(page.selection.metric_key, req.selection.metric_key);
        let keys: Vec<&str> = page
            .columns
            .iter()
            .map(|column| column.key.as_str())
            .collect();
        assert!(
            keys.contains(&"ref") && keys.contains(&"date"),
            "the columns are the ones the plan declares, got {keys:?}"
        );
    }
}
