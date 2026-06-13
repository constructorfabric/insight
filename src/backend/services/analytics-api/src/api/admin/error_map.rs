//! Translate domain-level rejections into canonical RFC 9457 envelopes.
//!
//! Owns three responsibilities the handler layer would otherwise duplicate:
//!
//! 1. **CHECK → 4xx mapping.** Every named CHECK in `metric_threshold` has
//!    a dedicated arm here; a SeaORM CHECK-violation error that matches one
//!    of the names becomes the canonical `invalid_argument` /
//!    `failed_precondition` envelope DESIGN §3.2 admin-crud mandates.
//!    A bare CHECK error MUST NOT surface as a 500 + raw error string —
//!    info disclosure surface. The `mapper_covers_every_required_check_name`
//!    test pins coverage at compile time.
//! 2. **Lock-bypass envelope construction.** The canonical
//!    `permission_denied` builder gives us `reason = "threshold_locked"` +
//!    `resource_type`; DESIGN §3.3 also requires `blocking_scope` /
//!    `blocking_row_id` / `locked_at` in `context`. Built through the
//!    canonical builder, then enriched on the `Problem::context` JSON —
//!    same one-liner pattern the crate uses internally to inject
//!    `resource_type` / `resource_name`.
//! 3. **Audit-unavailable 503 envelope.** Built through the canonical
//!    `service_unavailable` builder (status / type / `retry_after_seconds`
//!    all set by the crate); we enrich the body with
//!    `reason = "audit_unavailable"` and `resource_type` and ALSO set the
//!    HTTP `Retry-After` header per DNA `REST/STATUS_CODES.md` line 81.
//!    Header + body are populated from the same constant so they can't
//!    drift.
//!
//! ## Why we sometimes set `Problem.detail` directly
//!
//! `toolkit-canonical-errors` v0.7.3's `ResourceErrorBuilder::with_detail`
//! is `pub(crate)` — only callable inside the crate. The public builder
//! pre-sets the canonical default detail string for each category
//! ("You do not have permission to perform this operation", etc.). For
//! envelopes where DESIGN §3.3 specifies a more precise detail
//! ("Threshold is locked by a broader scope.", "Audit log unavailable;
//! retry shortly."), we go through `Problem::from(err)` and assign
//! `problem.detail` directly — the same `pub` field the crate itself
//! mutates inside `into_response`'s fallback path. Same pattern
//! `canonical_json.rs` uses for the 415 case the builder can't model.

use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use sea_orm::DbErr;
use serde_json::json;
use toolkit_canonical_errors::{CanonicalError, Problem};
use uuid::Uuid;

use crate::api::error::ThresholdAdminError;
use crate::domain::admin_threshold::lock_enforcer::BlockingLock;

/// `Retry-After` seconds for the `audit_unavailable` 503. Sized so a
/// transient MariaDB blip (connection reset, brief stall) has time to
/// recover before the client's retry lands. Used for BOTH the HTTP
/// header AND the body's `retry_after_seconds` — keep them in lockstep.
pub const AUDIT_RETRY_AFTER_SECONDS: u64 = 5;

// ── CHECK names (mirror migration/m20260522_000002_metric_threshold::REQUIRED_CHECKS) ──

pub const CHK_LOCK_REASON_WHEN_LOCKED: &str = "chk_metric_threshold_lock_reason_when_locked";
pub const CHK_LOCK_SCOPE_V1: &str = "chk_metric_threshold_lock_scope_v1";
pub const CHK_LOCK_REASON_LENGTH: &str = "chk_metric_threshold_lock_reason_length";
pub const CHK_ROLE_SLUG_SHAPE: &str = "chk_metric_threshold_role_slug_shape";
pub const CHK_TEAM_ID_SHAPE: &str = "chk_metric_threshold_team_id_shape";

// ── Top-level envelope builders ────────────────────────────────────

/// 403 `permission_denied` envelope for a `threshold_locked` rejection
/// (DESIGN §3.3). Built through the canonical builder; `context`
/// enriched with the three §3.3 extras the crate doesn't model
/// (`permission_denied`'s built-in ctx is `{ reason: String }` only).
#[must_use]
pub fn threshold_locked_response(blocking: &BlockingLock) -> Response {
    let err = ThresholdAdminError::permission_denied()
        .with_reason("threshold_locked")
        .create();
    let mut problem = Problem::from(err);
    "Threshold is locked by a broader scope.".clone_into(&mut problem.detail);
    problem.context["blocking_scope"] = json!(blocking.scope.as_db_str());
    problem.context["blocking_row_id"] = json!(blocking.id.to_string());
    if let Some(locked_at) = blocking.locked_at {
        problem.context["locked_at"] = json!(locked_at.to_rfc3339());
    }
    problem.into_response()
}

/// 503 `service_unavailable` envelope for an audit-primary-sink failure
/// (DESIGN §3.6 — "no silent bypass"). Sets BOTH the body's
/// `retry_after_seconds` field (via the canonical builder) AND the HTTP
/// `Retry-After` header (DNA `REST/STATUS_CODES.md` L81). Both values
/// come from [`AUDIT_RETRY_AFTER_SECONDS`].
#[must_use]
pub fn audit_unavailable_response() -> Response {
    let err = CanonicalError::service_unavailable()
        .with_retry_after_seconds(AUDIT_RETRY_AFTER_SECONDS)
        .create();
    let mut problem = Problem::from(err);
    "Audit log unavailable; retry shortly.".clone_into(&mut problem.detail);
    problem.context["resource_type"] = json!("gts.cf.insight.metric_catalog.threshold.v1~");
    problem.context["reason"] = json!("audit_unavailable");
    let mut response = problem.into_response();
    response.headers_mut().insert(
        header::RETRY_AFTER,
        HeaderValue::from(AUDIT_RETRY_AFTER_SECONDS),
    );
    response
}

/// Translate a SeaORM `DbErr` into the canonical envelope. Looks for the
/// named CHECK constraints; an unmatched error is alarmed-on + surfaced
/// as a generic 500 (the alarm is the point — see DESIGN §3.2's "CHECK
/// violation never surfaces as a 500" invariant: if we land in the
/// fallback path, schema and mapper have drifted).
#[must_use]
pub fn map_db_err(err: &DbErr, target_id_for_log: Option<Uuid>) -> Response {
    let msg = err.to_string();
    if msg.contains(CHK_LOCK_REASON_WHEN_LOCKED) {
        return lock_reason_required_response(target_id_for_log);
    }
    if msg.contains(CHK_LOCK_SCOPE_V1) {
        return lock_scope_invalid_response(target_id_for_log);
    }
    if msg.contains(CHK_LOCK_REASON_LENGTH) {
        return lock_reason_length_response(target_id_for_log);
    }
    if msg.contains(CHK_ROLE_SLUG_SHAPE) {
        return role_slug_shape_response(target_id_for_log);
    }
    if msg.contains(CHK_TEAM_ID_SHAPE) {
        return team_id_shape_response(target_id_for_log);
    }
    tracing::error!(
        error = %err,
        "admin-crud: DbErr with no matching CHECK mapper — schema/mapper drift"
    );
    CanonicalError::internal(err.to_string())
        .create()
        .into_response()
}

// ── Per-CHECK envelope builders ────────────────────────────────────
//
// These are used BOTH by `map_db_err` (CHECK violation surfaces from the
// DB) AND by the gauntlet in `service.rs` (request-time validation that
// fires before the DB write). DESIGN §2.1 `cpt-metric-cat-principle-dual-validate`
// mandates the same envelope shape from either layer; making these
// `pub(crate)` keeps the wire shape canonical no matter which layer
// caught the rejection.

pub(crate) fn lock_reason_required_response(rid: Option<Uuid>) -> Response {
    let builder = ThresholdAdminError::failed_precondition().with_precondition_violation(
        "lock_reason",
        "lock_reason must be set when is_locked = true",
        "lock_reason_required",
    );
    let err = match rid {
        Some(id) => builder.with_resource(id.to_string()).create(),
        None => builder.create(),
    };
    err.into_response()
}

pub(crate) fn lock_scope_invalid_response(rid: Option<Uuid>) -> Response {
    let builder = ThresholdAdminError::failed_precondition().with_precondition_violation(
        "scope",
        "is_locked = true is permitted only on scope product-default or tenant in v1",
        "lock_scope_invalid",
    );
    let err = match rid {
        Some(id) => builder.with_resource(id.to_string()).create(),
        None => builder.create(),
    };
    err.into_response()
}

pub(crate) fn lock_reason_length_response(rid: Option<Uuid>) -> Response {
    let builder = ThresholdAdminError::invalid_argument().with_field_violation(
        "lock_reason",
        "lock_reason must be ≤ 512 chars",
        "OUT_OF_RANGE",
    );
    let err = match rid {
        Some(id) => builder.with_resource(id.to_string()).create(),
        None => builder.create(),
    };
    err.into_response()
}

fn role_slug_shape_response(rid: Option<Uuid>) -> Response {
    let builder = ThresholdAdminError::invalid_argument().with_field_violation(
        "role_slug",
        "role_slug must be non-empty for scope role / team+role and empty otherwise",
        "INVALID",
    );
    let err = match rid {
        Some(id) => builder.with_resource(id.to_string()).create(),
        None => builder.create(),
    };
    err.into_response()
}

fn team_id_shape_response(rid: Option<Uuid>) -> Response {
    let builder = ThresholdAdminError::invalid_argument().with_field_violation(
        "team_id",
        "team_id must be non-empty for scope team / team+role and empty otherwise",
        "INVALID",
    );
    let err = match rid {
        Some(id) => builder.with_resource(id.to_string()).create(),
        None => builder.create(),
    };
    err.into_response()
}

// ── Gauntlet-time helpers (pre-DB rejections from `service.rs`) ────

/// Immutable-field PUT — emitted when the request payload's `scope` /
/// `role_slug` / `team_id` differs from the row's stored value (DESIGN
/// §3.7 line 1034). Re-scoping is DELETE+POST.
#[must_use]
pub fn immutable_field_response(row_id: Uuid, field: &'static str) -> Response {
    ThresholdAdminError::failed_precondition()
        .with_precondition_violation(
            field,
            format!("{field} is immutable post-create; re-scoping requires DELETE + POST"),
            "immutable_field",
        )
        .with_resource(row_id.to_string())
        .create()
        .into_response()
}

/// Caller is not a tenant-admin for the target tenant, or the addressed
/// row's `tenant_id` doesn't match the caller's session tenant
/// (cross-tenant write). Both converge on the same `reason` per DESIGN
/// §3.3. `permission_denied` is `ResourceAbsent` in the crate's
/// builder type-state — it does NOT carry `resource_name`.
#[must_use]
pub fn not_tenant_admin_response() -> Response {
    ThresholdAdminError::permission_denied()
        .with_reason("not_tenant_admin")
        .create()
        .into_response()
}

/// Unknown / disabled `metric_id` at create-time. The DB enforces the
/// metric existence indirectly (no FK; CRUD validates), so the
/// gauntlet's pre-write referential-integrity check is the only place
/// this fires.
#[must_use]
pub fn unknown_or_disabled_metric_response() -> Response {
    ThresholdAdminError::invalid_argument()
        .with_field_violation(
            "metric_id",
            "metric_id does not resolve to an enabled metric_catalog row",
            "UNKNOWN_OR_DISABLED",
        )
        .create()
        .into_response()
}

/// Sanity-bound rejection — `warn` crosses `good` in the wrong direction
/// relative to the metric's `higher_is_better` flag.
#[must_use]
pub fn sanity_bound_response() -> Response {
    ThresholdAdminError::invalid_argument()
        .with_field_violation(
            "warn",
            "warn must not cross good in the wrong direction relative to higher_is_better",
            "INVALID",
        )
        .create()
        .into_response()
}

/// Scope-shape rejection — `role_slug` / `team_id` sentinels don't
/// match the declared `scope`. Builds a single envelope carrying one
/// or two field violations depending on which side(s) failed —
/// the type-state of the builder collapses `NeedsFieldViolation` →
/// `HasFieldViolations` after the first call, so we branch up-front.
#[must_use]
pub fn scope_shape_response(role_slug_bad: bool, team_id_bad: bool) -> Response {
    const ROLE: (&str, &str, &str) = (
        "role_slug",
        "role_slug must be non-empty for scope role / team+role and empty otherwise",
        "INVALID",
    );
    const TEAM: (&str, &str, &str) = (
        "team_id",
        "team_id must be non-empty for scope team / team+role and empty otherwise",
        "INVALID",
    );
    let b = ThresholdAdminError::invalid_argument();
    let err = match (role_slug_bad, team_id_bad) {
        (true, false) => b.with_field_violation(ROLE.0, ROLE.1, ROLE.2).create(),
        (false, true) => b.with_field_violation(TEAM.0, TEAM.1, TEAM.2).create(),
        (true, true) => b
            .with_field_violation(ROLE.0, ROLE.1, ROLE.2)
            .with_field_violation(TEAM.0, TEAM.1, TEAM.2)
            .create(),
        (false, false) => {
            // Caller bug — service.rs only invokes this when at least
            // one side is bad. Surface as a 500 with a deliberate detail
            // so the log line and the wire envelope correlate at
            // `metric_catalog.scope_shape_internal_inconsistency`.
            tracing::error!(
                event = "metric_catalog.scope_shape_internal_inconsistency",
                "scope_shape_response called with no violations; coding error in service.rs"
            );
            return CanonicalError::internal("scope_shape_internal_inconsistency")
                .create()
                .into_response();
        }
    };
    err.into_response()
}

/// 404 `not_found` envelope — threshold row by `id` is absent.
#[must_use]
pub fn threshold_not_found_response(id: Uuid) -> Response {
    ThresholdAdminError::not_found("threshold not found")
        .with_resource(id.to_string())
        .create()
        .into_response()
}

/// Generic 5xx for SeaORM errors that AREN'T CHECK violations. Caller
/// logs the underlying error; the wire response is the canonical opaque
/// envelope per §3.3 (`detail` is client-safe — the crate's default).
#[must_use]
pub fn internal_error_response(err: &DbErr) -> Response {
    tracing::error!(error = %err, "admin-crud: unexpected DbErr");
    CanonicalError::internal(err.to_string())
        .create()
        .into_response()
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::header::CONTENT_TYPE;
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::domain::admin_threshold::dto::Scope;
    use crate::migration::REQUIRED_CHECKS_BY_TABLE;

    /// Re-derive the `metric_threshold` CHECK list from the source of
    /// truth so we don't reach across the migration module's privacy
    /// boundary. A renamed table in the migration registry would still
    /// trip this lookup — exactly the schema-drift signal the test
    /// exists to catch.
    fn required_checks_for_metric_threshold() -> &'static [&'static str] {
        REQUIRED_CHECKS_BY_TABLE
            .iter()
            .find_map(|(table, checks)| (*table == "metric_threshold").then_some(*checks))
            .unwrap_or_else(|| {
                panic!("metric_threshold not registered in REQUIRED_CHECKS_BY_TABLE")
            })
    }

    /// Coverage gate: every CHECK name declared by the migration's
    /// `REQUIRED_CHECKS` list MUST have a corresponding mapper constant
    /// in this module. A new CHECK without a mapper would otherwise
    /// surface as a generic 500 — exactly the info-disclosure regression
    /// DESIGN §3.2 admin-crud warns against.
    #[test]
    fn mapper_covers_every_required_check_name() {
        let mapper_constants: &[&str] = &[
            CHK_LOCK_REASON_WHEN_LOCKED,
            CHK_LOCK_SCOPE_V1,
            CHK_LOCK_REASON_LENGTH,
            CHK_ROLE_SLUG_SHAPE,
            CHK_TEAM_ID_SHAPE,
        ];
        let required = required_checks_for_metric_threshold();
        for r in required {
            assert!(
                mapper_constants.contains(r),
                "no mapper for CHECK {r} — admin-crud error_map missing an arm"
            );
        }
        assert_eq!(
            mapper_constants.len(),
            required.len(),
            "mapper has more entries than REQUIRED_CHECKS — schema drift?"
        );
    }

    async fn body_json(resp: Response) -> serde_json::Value {
        let bytes = to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap_or_else(|e| panic!("read body bytes: {e}"));
        serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("decode body json: {e}"))
    }

    fn sample_blocking() -> BlockingLock {
        BlockingLock {
            id: Uuid::from_u128(0x7af0_0000_0000_0000_0000_0000_0000_0001_u128),
            scope: Scope::Tenant,
            locked_at: Some(
                DateTime::parse_from_rfc3339("2026-05-12T14:02:11Z")
                    .unwrap_or_else(|e| panic!("rfc3339 parse: {e}"))
                    .with_timezone(&Utc),
            ),
            locked_by: Some("u-alice".to_owned()),
            lock_reason: Some("TICKET-7421: compliance pin".to_owned()),
        }
    }

    #[tokio::test]
    async fn threshold_locked_envelope_carries_blocking_extras() {
        let resp = threshold_locked_response(&sample_blocking());
        assert_eq!(resp.status(), 403);
        assert_eq!(
            resp.headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/problem+json")
        );
        let body = body_json(resp).await;
        assert_eq!(body["status"], 403);
        assert_eq!(
            body["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.permission_denied.v1~"
        );
        assert_eq!(
            body["context"]["resource_type"],
            "gts.cf.insight.metric_catalog.threshold.v1~"
        );
        assert_eq!(body["context"]["reason"], "threshold_locked");
        assert_eq!(body["context"]["blocking_scope"], "tenant");
        assert_eq!(
            body["context"]["blocking_row_id"],
            "7af00000-0000-0000-0000-000000000001"
        );
        assert_eq!(body["context"]["locked_at"], "2026-05-12T14:02:11+00:00");
        assert_eq!(body["detail"], "Threshold is locked by a broader scope.");
    }

    #[tokio::test]
    async fn audit_unavailable_sets_header_and_body() {
        let resp = audit_unavailable_response();
        assert_eq!(resp.status(), 503);
        assert_eq!(
            resp.headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("5"),
            "DNA REST/STATUS_CODES.md L81 mandates Retry-After header on 503"
        );
        let body = body_json(resp).await;
        assert_eq!(body["status"], 503);
        assert_eq!(
            body["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.service_unavailable.v1~"
        );
        assert_eq!(body["context"]["reason"], "audit_unavailable");
        assert_eq!(
            body["context"]["resource_type"],
            "gts.cf.insight.metric_catalog.threshold.v1~"
        );
        assert_eq!(body["context"]["retry_after_seconds"], 5);
        assert_eq!(body["detail"], "Audit log unavailable; retry shortly.");
    }

    #[tokio::test]
    async fn immutable_field_envelope_carries_precondition_violations() {
        let row_id = Uuid::from_u128(0x0190_abe0_0000_0000_0000_0000_0000_0001_u128);
        let resp = immutable_field_response(row_id, "scope");
        assert_eq!(resp.status(), 400);
        let body = body_json(resp).await;
        assert_eq!(
            body["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.failed_precondition.v1~"
        );
        assert_eq!(body["context"]["resource_name"], row_id.to_string());
        // The cyberfabric `toolkit-canonical-errors` crate serializes the
        // `FailedPrecondition` ctx as `context.violations` (not
        // `context.precondition_violations`); DESIGN §3.3 example shows
        // the latter — cross-team alignment is a follow-up. We pin to
        // what the crate actually emits today so regressions surface
        // here, not at FE integration time.
        let vios = body["context"]["violations"]
            .as_array()
            .unwrap_or_else(|| panic!("violations array missing"));
        assert_eq!(vios.len(), 1);
        assert_eq!(vios[0]["type"], "immutable_field");
        assert_eq!(vios[0]["subject"], "scope");
    }

    #[tokio::test]
    async fn not_tenant_admin_envelope_carries_canonical_reason() {
        let resp = not_tenant_admin_response();
        assert_eq!(resp.status(), 403);
        let body = body_json(resp).await;
        assert_eq!(body["context"]["reason"], "not_tenant_admin");
        assert_eq!(
            body["context"]["resource_type"],
            "gts.cf.insight.metric_catalog.threshold.v1~"
        );
    }

    #[tokio::test]
    async fn lock_reason_required_response_maps_to_failed_precondition() {
        let resp = lock_reason_required_response(None);
        assert_eq!(resp.status(), 400);
        let body = body_json(resp).await;
        // The cyberfabric `toolkit-canonical-errors` crate serializes the
        // `FailedPrecondition` ctx as `context.violations` (not
        // `context.precondition_violations`); DESIGN §3.3 example shows
        // the latter — cross-team alignment is a follow-up. We pin to
        // what the crate actually emits today so regressions surface
        // here, not at FE integration time.
        let vios = body["context"]["violations"]
            .as_array()
            .unwrap_or_else(|| panic!("violations array missing"));
        assert_eq!(vios[0]["type"], "lock_reason_required");
    }

    #[tokio::test]
    async fn lock_scope_invalid_response_maps_to_failed_precondition() {
        let resp = lock_scope_invalid_response(None);
        assert_eq!(resp.status(), 400);
        let body = body_json(resp).await;
        // The cyberfabric `toolkit-canonical-errors` crate serializes the
        // `FailedPrecondition` ctx as `context.violations` (not
        // `context.precondition_violations`); DESIGN §3.3 example shows
        // the latter — cross-team alignment is a follow-up. We pin to
        // what the crate actually emits today so regressions surface
        // here, not at FE integration time.
        let vios = body["context"]["violations"]
            .as_array()
            .unwrap_or_else(|| panic!("violations array missing"));
        assert_eq!(vios[0]["type"], "lock_scope_invalid");
    }

    #[tokio::test]
    async fn lock_reason_length_response_maps_to_invalid_argument() {
        let resp = lock_reason_length_response(None);
        assert_eq!(resp.status(), 400);
        let body = body_json(resp).await;
        let vios = body["context"]["field_violations"]
            .as_array()
            .unwrap_or_else(|| panic!("field_violations array missing"));
        assert_eq!(vios[0]["field"], "lock_reason");
        assert_eq!(vios[0]["reason"], "OUT_OF_RANGE");
    }

    #[tokio::test]
    async fn scope_shape_can_carry_two_field_violations() {
        let resp = scope_shape_response(true, true);
        let body = body_json(resp).await;
        let vios = body["context"]["field_violations"]
            .as_array()
            .unwrap_or_else(|| panic!("field_violations array missing"));
        assert_eq!(vios.len(), 2);
        assert_eq!(vios[0]["field"], "role_slug");
        assert_eq!(vios[1]["field"], "team_id");
    }

    #[tokio::test]
    async fn scope_shape_single_violation_when_only_one_side_bad() {
        let resp = scope_shape_response(true, false);
        let body = body_json(resp).await;
        let vios = body["context"]["field_violations"]
            .as_array()
            .unwrap_or_else(|| panic!("field_violations array missing"));
        assert_eq!(vios.len(), 1);
        assert_eq!(vios[0]["field"], "role_slug");
    }

    #[tokio::test]
    async fn threshold_not_found_envelope() {
        let id = Uuid::from_u128(0x0190_abf0_0000_0000_0000_0000_0000_0001_u128);
        let resp = threshold_not_found_response(id);
        assert_eq!(resp.status(), 404);
        let body = body_json(resp).await;
        assert_eq!(
            body["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.not_found.v1~"
        );
        assert_eq!(body["context"]["resource_name"], id.to_string());
    }
}
