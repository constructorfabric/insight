//! Persons-seed HTTP surface + background worker.
//!
//! `POST /v1/persons-seed` enqueues an `operations` row and a job on an
//! in-process channel, returning 202; a worker (spawned once in the gear init)
//! drains the channel, runs the seed via [`run_seed`], and marks the operation
//! completed/failed. The GETs poll status. Ported from the .NET
//! `PersonsSeedEndpoints` + `PersonsSeedQueue`.
//!
//! Admin-gated like the .NET `CallerAdminCheck`: the caller is the gateway-JWT
//! subject (`SecurityContext::subject_id`, verified by the host authn pipeline —
//! `NGINX_BFF` R1) and must hold an active `admin` role in the tenant; it is
//! recorded as the seed author.

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Extension, Path, Query};
use axum::http::StatusCode;
use axum::http::header::LOCATION;
use axum::response::IntoResponse;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::canonical_json::CanonicalJson;
use super::error::PersonsSeedError;
use crate::config::GearConfig;
use crate::domain::seed_service::run_seed;
use crate::infra::db::ops_repo::{self, Operation, OperationStatus};
use crate::infra::db::roles_repo;
use crate::infra::db::seed_repo::MariaDbSeedStore;
use crate::infra::identity_inputs::ClickHouseIdentityInputsReader;

const LINK_BY_EMAIL_MODE: &str = "link-by-email";

/// Default page size / cap for the list endpoint (parity with the .NET
/// `PageRequest.DefaultLimit` / `MaxLimit`).
const LIST_DEFAULT_LIMIT: u64 = 50;
const LIST_MAX_LIMIT: u64 = 500;

/// Upper bound on one seed run in the serial worker; a stall past this fails the
/// job rather than wedging the whole queue.
const SEED_TIMEOUT: Duration = Duration::from_mins(10);
const PERSONS_SEED_OP: &str = "persons-seed";

/// A queued persons-seed job handed from the POST handler to the worker.
#[allow(clippy::struct_field_names)] // all three fields are ids by nature
#[derive(Debug, Clone, Copy)]
pub struct PersonsSeedJob {
    pub operation_id: Uuid,
    pub tenant_id: Uuid,
    pub author_person_id: Uuid,
}

/// Body of `POST /v1/persons-seed`. `mode` defaults to `link-by-email`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PersonsSeedRequest {
    #[serde(default)]
    pub mode: Option<String>,
}
impl toolkit::api::api_dto::RequestApiDto for PersonsSeedRequest {}

/// One operation's status (POST returns the queued row; GETs return current).
/// Wire shape mirrors the .NET `PersonsSeedOperationResponse`: `request` and
/// `summary` are surfaced as parsed JSON (not double-encoded strings), the
/// tenant/author ids are included, timestamps are ISO-8601, and null fields are
/// emitted (the .NET serializer does not drop nulls).
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonsSeedOperationResponse {
    pub operation_id: Uuid,
    pub operation_type: String,
    pub status: String,
    pub insight_tenant_id: Uuid,
    pub author_person_id: Uuid,
    #[schema(value_type = Option<Object>)]
    pub request: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub summary: Option<serde_json::Value>,
    pub error_message: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for PersonsSeedOperationResponse {}

impl PersonsSeedOperationResponse {
    /// The just-enqueued shape for the `202 Accepted` body, built from the
    /// fields the POST handler already holds — avoids a second round-trip to
    /// re-read the row, and (unlike a re-read) always reports `queued` even if
    /// the worker has already picked the job up. Mirrors the .NET `Queued(...)`.
    fn queued(
        operation_id: Uuid,
        tenant_id: Uuid,
        author_person_id: Uuid,
        request_json: Option<&str>,
        started_at: sea_orm::prelude::DateTime,
    ) -> Self {
        Self {
            operation_id,
            operation_type: PERSONS_SEED_OP.to_owned(),
            status: OperationStatus::Queued.as_db().to_owned(),
            insight_tenant_id: tenant_id,
            author_person_id,
            request: parse_or_null(request_json),
            summary: None,
            error_message: None,
            started_at: fmt_ts(started_at),
            completed_at: None,
        }
    }
}

impl From<Operation> for PersonsSeedOperationResponse {
    fn from(op: Operation) -> Self {
        Self {
            operation_id: op.operation_id,
            operation_type: op.operation_type,
            status: op.status.as_db().to_owned(),
            insight_tenant_id: op.insight_tenant_id,
            author_person_id: op.author_person_id,
            request: parse_or_null(op.request_json.as_deref()),
            summary: parse_or_null(op.summary_json.as_deref()),
            error_message: op.error_message,
            started_at: fmt_ts(op.started_at),
            completed_at: op.completed_at.map(fmt_ts),
        }
    }
}

/// Surface a stored JSON column as a parsed value (not a double-encoded string);
/// `None` for absent/empty/unparseable. Mirrors the .NET `ParseOrNull`.
fn parse_or_null(json: Option<&str>) -> Option<serde_json::Value> {
    let s = json?;
    if s.is_empty() {
        return None;
    }
    serde_json::from_str(s).ok()
}

/// Format a DB `DateTime` (naive) as ISO-8601 with a `T` separator, matching the
/// .NET `System.Text.Json` `DateTime` output (`NaiveDateTime::to_string` uses a
/// space, which breaks ISO-8601 parsers).
fn fmt_ts(dt: sea_orm::prelude::DateTime) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
}

/// List response wrapper (typed for OpenAPI).
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonsSeedListResponse {
    pub items: Vec<PersonsSeedOperationResponse>,
}
impl toolkit::api::api_dto::ResponseApiDto for PersonsSeedListResponse {}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub status: Option<String>,
    pub limit: Option<u64>,
}

/// `POST /v1/persons-seed` — enqueue an async persons-seed run.
pub async fn create_persons_seed(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<PersonsSeedRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    // Admin gate first (parity with .NET: the caller/admin check precedes mode
    // validation, so an unauthenticated/non-admin caller gets 401/403, not 400).
    // The resolved caller is recorded as the author of the job + observations.
    let author = require_admin(&state.db, &ctx).await?;

    let mode = req
        .mode
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(LINK_BY_EMAIL_MODE);
    if mode != LINK_BY_EMAIL_MODE {
        return Err(PersonsSeedError::invalid_argument()
            .with_field_violation(
                "mode",
                "unsupported mode; only 'link-by-email' is available",
                "INVALID",
            )
            .create());
    }

    let operation_id = Uuid::now_v7();
    let started_at = chrono::Utc::now().naive_utc();
    let request_json = serde_json::json!({ "mode": mode }).to_string();

    ops_repo::enqueue(
        &state.db,
        operation_id,
        PERSONS_SEED_OP,
        tenant,
        author,
        Some(&request_json),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "enqueue operation failed");
        CanonicalError::internal("failed to enqueue seed").create()
    })?;

    let job = PersonsSeedJob {
        operation_id,
        tenant_id: tenant,
        author_person_id: author,
    };
    if state.seed_tx.try_send(job).is_err() {
        // Channel full/closed — fail the row so it isn't a zombie, and tell the
        // caller to retry later (503, not 500 — parity with the .NET queue-full).
        let _ = ops_repo::fail(&state.db, operation_id, "seed queue full; retry later").await;
        return Err(CanonicalError::service_unavailable()
            .with_detail("seed queue is full; retry later")
            .create());
    }

    // Audit the enqueue (parity with the .NET `persons_seed.enqueue` audit).
    tracing::info!(
        %operation_id,
        %mode,
        author_person_id = %author,
        "persons_seed.enqueue"
    );

    // Build the 202 body from the in-memory snapshot (always `queued`) and set
    // Location to the status URL — no re-read of the just-inserted row.
    let body = PersonsSeedOperationResponse::queued(
        operation_id,
        tenant,
        author,
        Some(&request_json),
        started_at,
    );
    let location = format!("/v1/persons-seed/{operation_id}");
    Ok((StatusCode::ACCEPTED, [(LOCATION, location)], Json(body)))
}

/// `GET /v1/persons-seed/{id}` — poll one operation.
pub async fn get_persons_seed(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    // Same admin gate as POST (parity — the .NET service gates all three routes).
    require_admin(&state.db, &ctx).await?;
    let op = ops_repo::get_by_id(&state.db, tenant, id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "get operation failed");
            CanonicalError::internal("failed to read operation").create()
        })?
        .filter(|o| o.operation_type == PERSONS_SEED_OP)
        .ok_or_else(|| {
            PersonsSeedError::not_found("operation not found")
                .with_resource(id.to_string())
                .create()
        })?;
    Ok(Json(PersonsSeedOperationResponse::from(op)))
}

/// `GET /v1/persons-seed` — list persons-seed operations. Optional `?status=`
/// (unknown values ignored) and `?limit=` (default 50, capped 500).
pub async fn list_persons_seed(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    // Same admin gate as POST (parity — the .NET service gates all three routes).
    require_admin(&state.db, &ctx).await?;
    let status = status_filter(params.status.as_deref());
    let limit = params
        .limit
        .unwrap_or(LIST_DEFAULT_LIMIT)
        .clamp(1, LIST_MAX_LIMIT);
    let ops = ops_repo::list(&state.db, tenant, Some(PERSONS_SEED_OP), status, limit)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "list operations failed");
            CanonicalError::internal("failed to list operations").create()
        })?;
    let items = ops
        .into_iter()
        .map(PersonsSeedOperationResponse::from)
        .collect();
    Ok(Json(PersonsSeedListResponse { items }))
}

/// The persons-seed admin gate (parity with the .NET `CallerAdminCheck`): the
/// caller is the gateway-JWT subject (`SecurityContext::subject_id`, verified by
/// the host authn pipeline), which must hold an active `admin` role in the
/// tenant. Returns the caller `person_id`, or 401 (no subject) / 403 (not admin).
async fn require_admin(
    db: &DatabaseConnection,
    ctx: &SecurityContext,
) -> Result<Uuid, CanonicalError> {
    let caller = ctx.subject_id();
    if caller.is_nil() {
        return Err(CanonicalError::unauthenticated()
            .with_reason("caller not identified: the gateway JWT carries no person subject")
            .create());
    }
    let is_admin = roles_repo::has_active_admin(db, ctx.subject_tenant_id(), caller)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "admin role check failed");
            CanonicalError::internal("failed to verify caller permissions").create()
        })?;
    if !is_admin {
        return Err(PersonsSeedError::permission_denied()
            .with_reason("admin role required for this operation")
            .create());
    }
    Ok(caller)
}

/// Map the `?status=` query to a filter. An unknown/blank value is ignored
/// (returns all statuses), matching the .NET `_ => null` — not a 400.
fn status_filter(raw: Option<&str>) -> Option<OperationStatus> {
    match raw {
        Some("queued") => Some(OperationStatus::Queued),
        Some("running") => Some(OperationStatus::Running),
        Some("completed") => Some(OperationStatus::Completed),
        Some("failed") => Some(OperationStatus::Failed),
        _ => None,
    }
}

/// How stale a `queued`/`running` row must be before the startup sweep reclaims
/// it — parity with the .NET `PersonsSeedWorker.ZombieCutoff` (1 hour).
const ZOMBIE_CUTOFF_HOURS: i64 = 1;

/// Background worker: drain the queue and run each seed to completion, updating
/// the `operations` row. Spawned once from the gear `init`; ends when the
/// channel closes (all senders dropped).
pub async fn run_worker(
    mut rx: mpsc::Receiver<PersonsSeedJob>,
    db: DatabaseConnection,
    config: GearConfig,
) {
    let reader = ClickHouseIdentityInputsReader::connect(
        &config.clickhouse_url,
        &config.clickhouse_database,
        &config.clickhouse_user,
        &config.clickhouse_password,
    );
    let store = MariaDbSeedStore::new(&db);

    // Startup sweep: a pod restart drops the in-memory queue, so any row left
    // `queued`/`running` by the previous process would otherwise never resolve.
    // Fail rows older than the cutoff (parity with .NET `SweepZombiesAsync`).
    let cutoff = chrono::Utc::now().naive_utc() - chrono::Duration::hours(ZOMBIE_CUTOFF_HOURS);
    match ops_repo::sweep_zombies(&db, cutoff).await {
        Ok(n) if n > 0 => tracing::warn!(swept = n, "persons-seed: reclaimed zombie operations"),
        Ok(_) => {}
        Err(e) => tracing::error!(error = %e, "persons-seed: zombie sweep failed"),
    }

    while let Some(job) = rx.recv().await {
        // Only the worker that wins queued→running proceeds (no double-run).
        match ops_repo::try_start(&db, job.operation_id).await {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => {
                // A transient DB blip on the queued→running transition must not
                // strand the (already-consumed) job as a zombie `queued` row —
                // mark it failed so it isn't stuck forever, like the queue-full
                // path in `create_persons_seed`.
                tracing::error!(error = %e, operation_id = %job.operation_id, "try_start failed");
                let _ =
                    ops_repo::fail(&db, job.operation_id, "try_start failed; retry later").await;
                continue;
            }
        }

        // Bound each run: the worker is single-threaded and serial, so a hung
        // ClickHouse/MariaDB call would otherwise block every subsequent job for
        // every tenant until the process restarts. Generous ceiling — a healthy
        // large-tenant seed is seconds; this only trips on a real stall.
        let seed = run_seed(
            &reader,
            &store,
            job.tenant_id,
            job.author_person_id,
            Uuid::now_v7,
        );
        let result = tokio::time::timeout(SEED_TIMEOUT, seed)
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("persons-seed timed out")));

        match result {
            Ok(summary) => {
                let summary_json =
                    serde_json::to_string(&summary).unwrap_or_else(|_| "{}".to_owned());
                if let Err(e) = ops_repo::complete(&db, job.operation_id, &summary_json).await {
                    tracing::error!(error = %e, operation_id = %job.operation_id, "complete failed");
                }
            }
            Err(e) => {
                // Log the real error server-side, but persist only a generic
                // message: `error_message` is returned verbatim by the GET/list
                // endpoints, so raw driver/anyhow text must not leak to callers.
                tracing::error!(error = %e, operation_id = %job.operation_id, "persons-seed failed");
                if let Err(e2) = ops_repo::fail(
                    &db,
                    job.operation_id,
                    "persons-seed failed; see server logs",
                )
                .await
                {
                    tracing::error!(error = %e2, operation_id = %job.operation_id, "fail update failed");
                }
            }
        }
    }
}
