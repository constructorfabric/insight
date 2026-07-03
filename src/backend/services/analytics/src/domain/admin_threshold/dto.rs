//! Request / response shapes for `/v1/admin/metric-thresholds/*` (Refs #525).
//!
//! Three invariants pinned by `deny_unknown_fields` on the inbound shapes:
//!
//! 1. **No body-supplied `tenant_id`.** The list / get / create / update / delete
//!    handlers all derive `tenant_id` from `SecurityContext` (resolved by
//!    `tenant_middleware`). A smuggled body field would be a cross-tenant
//!    disclosure surface — `deny_unknown_fields` rejects it as a canonical 400
//!    at the serde layer.
//! 2. **No body-supplied `id` / `locked_by` / `locked_at` / `created_at` /
//!    `updated_at`.** Server-owned columns; the same `deny_unknown_fields`
//!    rejects callers that try to spoof them.
//! 3. **No query-string `tenant_id` on the list endpoint.** [`ListFilters`]
//!    is `deny_unknown_fields`, so `?tenant_id=...` surfaces as a canonical
//!    400. Spec mandate: DESIGN §3.3 ("`tenant_id` is NOT a filter parameter
//!    — derived strictly from the session to prevent cross-tenant
//!    disclosure").

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Canonical scope values for `metric_threshold.scope`. Mirrors the DB-side
/// ENUM declared in `migration/m20260522_000002_metric_threshold.rs` line
/// 102–106 and the resolver's `Scope` (kept as a separate type because the
/// resolver's enum is private to that module).
///
/// Wire form is the dash-keyed string the DB stores — deserializing via
/// `serde(rename_all = "kebab-case")` would NOT produce the right value for
/// `team+role` (kebab would yield `team-role`), so we spell each variant
/// explicitly with `#[serde(rename = ...)]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    #[serde(rename = "product-default")]
    ProductDefault,
    #[serde(rename = "tenant")]
    Tenant,
    #[serde(rename = "role")]
    Role,
    #[serde(rename = "team")]
    Team,
    #[serde(rename = "team+role")]
    TeamRole,
}

impl Scope {
    /// Wire / DB string. Always exactly the value `metric_threshold.scope`
    /// stores — keep this in lockstep with the ENUM declaration.
    #[must_use]
    pub const fn as_db_str(self) -> &'static str {
        match self {
            Self::ProductDefault => "product-default",
            Self::Tenant => "tenant",
            Self::Role => "role",
            Self::Team => "team",
            Self::TeamRole => "team+role",
        }
    }

    /// Parse from the wire / DB string. Returns `None` for unknown values —
    /// callers (the row-decode path; the lock-enforcer's broader-scope
    /// list) treat that as a DB-shape regression and degrade gracefully.
    #[must_use]
    pub fn from_db_str(s: &str) -> Option<Self> {
        Some(match s {
            "product-default" => Self::ProductDefault,
            "tenant" => Self::Tenant,
            "role" => Self::Role,
            "team" => Self::Team,
            "team+role" => Self::TeamRole,
            _ => return None,
        })
    }

    /// Broadness rank — broad→narrow per DESIGN §3.6 lock-bypass walk.
    /// `0` is broadest; greater rank is narrower. Used today only by
    /// [`Scope::is_broader_than`]; kept `pub` because the lock-enforcer's
    /// `ORDER BY CASE scope` mirrors this exact ordering and a future
    /// re-use is the natural next consumer.
    #[allow(dead_code)] // public invariant; consumed by is_broader_than + tests
    #[must_use]
    pub const fn broadness_rank(self) -> u8 {
        match self {
            Self::ProductDefault => 0,
            Self::Tenant => 1,
            Self::Role => 2,
            Self::Team => 3,
            Self::TeamRole => 4,
        }
    }

    /// True iff `self` is strictly broader than `other` — i.e. `self`
    /// appears in `other.broader_scopes()` and therefore counts as a
    /// blocking-lock candidate when a write at `other`'s scope is checked.
    ///
    /// Note: `role` and `team` are PEERS, not each other's ancestor —
    /// the resolver halts on broader-scope locks during the walk, and
    /// neither `role` nor `team` is broader than the other. Using
    /// [`broadness_rank`] for this would falsely return `true` for
    /// `Role.is_broader_than(Team)`.
    #[allow(dead_code)] // exercised by tests; documents the type's invariant
    #[must_use]
    pub fn is_broader_than(self, other: Self) -> bool {
        other.broader_scopes().contains(&self)
    }

    /// Scopes strictly broader than `self`. Drives the `IN (...)` list in
    /// the lock-enforcer's SQL. `product-default` has no broader scope, so
    /// the slice is empty there and the lock-enforcer short-circuits.
    #[must_use]
    pub fn broader_scopes(self) -> &'static [Scope] {
        const PRODUCT_DEFAULT: &[Scope] = &[];
        const TENANT: &[Scope] = &[Scope::ProductDefault];
        const ROLE: &[Scope] = &[Scope::ProductDefault, Scope::Tenant];
        const TEAM: &[Scope] = &[Scope::ProductDefault, Scope::Tenant];
        const TEAM_ROLE: &[Scope] = &[
            Scope::ProductDefault,
            Scope::Tenant,
            Scope::Role,
            Scope::Team,
        ];
        match self {
            Self::ProductDefault => PRODUCT_DEFAULT,
            Self::Tenant => TENANT,
            // `role` and `team` are peers — neither is broader than the
            // other. A locked `role` row does NOT shadow a `team` write
            // (different chain ancestry) and vice versa, but BOTH are
            // shadowed by `tenant` / `product-default`. The DESIGN §3.6
            // walk halts on the FIRST broader-scope locked row in
            // broad→narrow order; `role` / `team` siblings are surfaced
            // only when the write target is `team+role` (their common
            // descendant).
            Self::Role => ROLE,
            Self::Team => TEAM,
            Self::TeamRole => TEAM_ROLE,
        }
    }
}

/// Query-string filters for `GET /v1/admin/metric-thresholds`.
///
/// `tenant_id` is intentionally absent and would be rejected by
/// `deny_unknown_fields` if a caller smuggled `?tenant_id=...` into the URL
/// — DESIGN §3.3 cross-tenant disclosure protection.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListFilters {
    /// UUIDv7 of a `metric_catalog` row — narrows the result to threshold
    /// rows whose `metric_key` matches that catalog row's `metric_key`.
    pub metric_id: Option<Uuid>,
    pub scope: Option<Scope>,
    pub role_slug: Option<String>,
    pub team_id: Option<String>,
}

/// `POST /v1/admin/metric-thresholds` body — create a new threshold row.
///
/// `tenant_id` / `id` / `locked_by` / `locked_at` / `created_at` /
/// `updated_at` are NOT accepted from the body. `deny_unknown_fields`
/// enforces that at the serde layer.
///
/// `role_slug` / `team_id` use `Option<String>` — `None` is the canonical
/// empty-string sentinel (DESIGN §3.7 + `infra/cache/catalog_cache.rs::cache_field`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub metric_id: Uuid,
    pub scope: Scope,
    #[serde(default)]
    pub role_slug: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    pub good: f64,
    pub warn: f64,
    #[serde(default)]
    pub alert_trigger: Option<f64>,
    #[serde(default)]
    pub alert_bad: Option<f64>,
    #[serde(default)]
    pub is_locked: bool,
    #[serde(default)]
    pub lock_reason: Option<String>,
}

/// `PUT /v1/admin/metric-thresholds/{id}` body — update an existing row.
///
/// `scope` / `role_slug` / `team_id` are intentionally accepted here even
/// though they're immutable post-create: when present, the gauntlet
/// compares the value to the row's current value and rejects with
/// `failed_precondition` + `type: "immutable_field"` if they differ. Re-
/// scoping requires DELETE + POST per DESIGN §3.7 line 1034.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRequest {
    /// Echoed by the caller as a sanity check; the gauntlet validates it
    /// against the row's current value.
    #[serde(default)]
    pub scope: Option<Scope>,
    #[serde(default)]
    pub role_slug: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    pub good: f64,
    pub warn: f64,
    #[serde(default)]
    pub alert_trigger: Option<f64>,
    #[serde(default)]
    pub alert_bad: Option<f64>,
    #[serde(default)]
    pub is_locked: bool,
    #[serde(default)]
    pub lock_reason: Option<String>,
}

/// On-wire shape of one `metric_threshold` row in list / get responses.
///
/// `metric_key` is NOT serialized — same backend-internal opacity rule the
/// read endpoint follows (`domain/catalog/response.rs::MetricView`).
/// Consumers identify a metric by `metric_id`.
#[derive(Debug, Clone, Serialize)]
pub struct ThresholdView {
    pub id: Uuid,
    /// `Some(_)` for tenant-scoped rows, `None` for `product-default`.
    pub tenant_id: Option<Uuid>,
    /// UUIDv7 of the corresponding `metric_catalog` row.
    pub metric_id: Uuid,
    pub scope: Scope,
    /// Empty-string sentinel collapsed to `None` on the wire so the JSON
    /// shape is `null` instead of `""` (the latter would confuse FE
    /// "is this set?" predicates).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    pub good: f64,
    pub warn: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert_trigger: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert_bad: Option<f64>,
    pub is_locked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_reason: Option<String>,
    /// One of `ok | error | unchecked`, joined from `metric_catalog.schema_status`
    /// (DESIGN §3.3 "Schema status surface"). Lets the admin UI flag a
    /// broken metric before the operator submits a write.
    pub schema_status: String,
    /// Canonical error code (`table_not_found | column_not_found |
    /// clickhouse_unreachable | unknown`) when `schema_status = "error"`,
    /// otherwise omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_error_code: Option<String>,
}

/// `GET /v1/admin/metric-thresholds` response envelope.
///
/// Wraps `items` in an object (instead of a bare array) so future
/// additions (pagination cursor, count, generated-at) are additive and
/// non-breaking. Mirrors the catalog read endpoint's envelope shape.
#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub items: Vec<ThresholdView>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_wire_form_matches_db_enum() {
        // Cross-component pin: a refactor that renames `team+role` (a
        // gotcha — kebab-case would mangle it) would silently desync the
        // wire and the DB ENUM.
        assert_eq!(Scope::ProductDefault.as_db_str(), "product-default");
        assert_eq!(Scope::Tenant.as_db_str(), "tenant");
        assert_eq!(Scope::Role.as_db_str(), "role");
        assert_eq!(Scope::Team.as_db_str(), "team");
        assert_eq!(Scope::TeamRole.as_db_str(), "team+role");
    }

    #[test]
    fn scope_round_trips_through_db_str() {
        for s in [
            Scope::ProductDefault,
            Scope::Tenant,
            Scope::Role,
            Scope::Team,
            Scope::TeamRole,
        ] {
            assert_eq!(Scope::from_db_str(s.as_db_str()), Some(s));
        }
    }

    #[test]
    fn scope_serde_uses_db_form() -> Result<(), serde_json::Error> {
        // The kebab-case gotcha — `team+role` MUST serialize as `team+role`,
        // not `team-role`.
        let s = serde_json::to_string(&Scope::TeamRole)?;
        assert_eq!(s, "\"team+role\"");
        let back: Scope = serde_json::from_str("\"team+role\"")?;
        assert_eq!(back, Scope::TeamRole);
        Ok(())
    }

    #[test]
    fn broader_scopes_for_product_default_is_empty() {
        // `product-default` has no broader ancestor — the lock-enforcer
        // short-circuits.
        assert!(Scope::ProductDefault.broader_scopes().is_empty());
    }

    #[test]
    fn broader_scopes_for_tenant_is_product_default_only() {
        assert_eq!(Scope::Tenant.broader_scopes(), &[Scope::ProductDefault]);
    }

    #[test]
    fn broader_scopes_for_team_role_includes_role_and_team_siblings() {
        // `team+role` is the only scope whose ancestor chain pulls in
        // `role` AND `team` — both shadow it during resolution. A regression
        // that dropped one of the siblings would let a `team+role` write
        // sneak past a sibling-locked row.
        assert_eq!(
            Scope::TeamRole.broader_scopes(),
            &[
                Scope::ProductDefault,
                Scope::Tenant,
                Scope::Role,
                Scope::Team
            ]
        );
    }

    #[test]
    fn role_and_team_are_peers_not_each_others_ancestor() {
        // A locked `role` row does NOT shadow a `team` write (and vice
        // versa). Both are children of `tenant` / `product-default`; they
        // diverge from there. Pinning this so a future "add role to team's
        // broader_scopes" mistake is caught.
        assert!(!Scope::Role.is_broader_than(Scope::Team));
        assert!(!Scope::Team.is_broader_than(Scope::Role));
        assert!(Scope::Tenant.is_broader_than(Scope::Role));
        assert!(Scope::Tenant.is_broader_than(Scope::Team));
        assert!(Scope::Tenant.is_broader_than(Scope::TeamRole));
        assert!(Scope::ProductDefault.is_broader_than(Scope::Tenant));
    }

    // `ListFilters` rejection of `?tenant_id=...` is verified end-to-end at
    // the handler layer (`api::admin::handlers` integration tests). serde's
    // `deny_unknown_fields` works the same way against url-encoded inputs as
    // against JSON, but driving the actual `axum::extract::Query` extractor
    // there pins both the parser AND the canonical-envelope response path
    // (which is what the FE actually observes).

    #[test]
    fn create_request_rejects_body_tenant_id() {
        let err = serde_json::from_str::<CreateRequest>(
            r#"{"metric_id":"01900000-0000-7000-8000-000000000000","scope":"tenant","good":1,"warn":0,"tenant_id":"sneaky"}"#,
        );
        assert!(err.is_err(), "body-supplied tenant_id MUST be rejected");
    }

    #[test]
    fn create_request_rejects_server_owned_fields() {
        // `id` / `locked_by` / `locked_at` are server-owned. A caller that
        // tries to set them is either confused or hostile; either way they
        // must get a canonical 400.
        for smuggled in ["id", "locked_by", "locked_at", "created_at", "updated_at"] {
            let body = format!(
                r#"{{"metric_id":"01900000-0000-7000-8000-000000000000","scope":"tenant","good":1,"warn":0,"{smuggled}":"x"}}"#
            );
            assert!(
                serde_json::from_str::<CreateRequest>(&body).is_err(),
                "body-supplied {smuggled} MUST be rejected (deny_unknown_fields gate)"
            );
        }
    }

    #[test]
    fn update_request_rejects_body_tenant_id() {
        let err =
            serde_json::from_str::<UpdateRequest>(r#"{"good":1,"warn":0,"tenant_id":"sneaky"}"#);
        assert!(err.is_err());
    }
}
