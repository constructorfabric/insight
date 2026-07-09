//! Wire shape for `POST /v1/catalog/get_metrics` (DESIGN §3.3 "Catalog Read").
//!
//! Three invariants pinned by tests:
//!
//! 1. **`metric_key` IS on the wire as the transitional FE-bridge identifier**
//!    (ADR-002). The hard "never surface `metric_key`" rule was narrowed in v1.12:
//!    `id` (UUIDv7) is still the stable lookup key consumers MUST use; `metric_key`
//!    is additive and exists so the FE can align its compiled-in `BULLET_DEFS`
//!    constants to wire rows during the catalog-hydration transitional release
//!    (constructorfabric/insight-front#66). Once the FE deletes those constants,
//!    `metric_key` keeps the documented additive-field stability contract.
//! 2. **`bounded_by_lock` is a separate field from `resolved_from`.** `resolved_from`
//!    names the row that won the walk; `bounded_by_lock` is `true` iff the walk
//!    halted on a locked broader-scope row before reaching the most-specific
//!    candidate. The two signals together let admin tooling explain "why was
//!    this team-scope override ignored" without a second request.
//! 3. **`links` exposes the `metric_query_catalog` junction (ADR-003).** Each
//!    `(query_id, catalog_metric_ids)` pair tells a consumer which catalog rows
//!    a `metrics.query_ref` will emit. The mapping is time- and filter-invariant
//!    — the same query emits the same set of catalog ids regardless of (period,
//!    person, org) — so consumers can cache this Layer-2 map once per session/TTL
//!    instead of recomputing it on every value request.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request body for `POST /v1/catalog/get_metrics`.
///
/// `tenant_id` is intentionally NOT accepted here — it is resolved server-side
/// from the session by `tenant_middleware` (Refs #522 auth-trait). Allowing a
/// body-supplied `tenant_id` would open a cross-tenant disclosure surface.
/// `deny_unknown_fields` enforces that defensively at the parser layer: a
/// caller that smuggles `"tenant_id": "..."` into the body gets a 400 instead
/// of a silent ignore.
#[derive(Debug, Clone, Deserialize, Default, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GetMetricsRequest {
    /// Role slug for `role` / `team+role` resolution chains. `None` and `Some("")`
    /// are semantically identical and produce the same cache key (canonical
    /// empty-string sentinel — see `cache_key` in the cache layer).
    #[serde(default)]
    pub role_slug: Option<String>,

    /// Team id for `team` / `team+role` resolution chains. Same `None` vs `Some("")`
    /// equivalence as `role_slug`.
    #[serde(default)]
    pub team_id: Option<String>,
}

/// Top-level response body. `tenant_id` is echoed for client-side cache
/// reasoning AND re-asserted on cache hydrate as defense in depth against a
/// misconfigured cache backend serving a sibling tenant's payload.
///
/// `links` carries the `metric_query_catalog` M:N mapping per ADR-003. The
/// mapping is time/filter-invariant, so consumers cache it for the same TTL as
/// the catalog itself; see [`MetricQueryLink`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct CatalogResponse {
    pub tenant_id: Uuid,
    pub generated_at: DateTime<Utc>,
    pub metrics: Vec<MetricView>,
    pub links: Vec<MetricQueryLink>,
}

/// One link row from `metric_query_catalog`. Tells a consumer which catalog
/// rows a `metrics.query_ref` emits when executed — the M:N answer ADR-001
/// added at the DB layer, surfaced here so consumers don't have to derive it
/// by joining on backend-internal `metric_key` strings.
///
/// `catalog_metric_ids` is the set of `metric_catalog.id` UUIDs the query
/// produces. The set is empty only when the linked catalog rows are all
/// `is_enabled = false` (filtered out of the `metrics` array) — consumers
/// degrade gracefully on empty.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct MetricQueryLink {
    /// `metrics.id` — the ClickHouse `query_ref` row this link is FROM.
    pub query_id: Uuid,
    /// `metric_catalog.id` UUIDs this query emits. Sorted ascending so the
    /// wire payload is byte-stable for cache + diff tooling.
    pub catalog_metric_ids: Vec<Uuid>,
}

/// One catalog metric on the wire. `metric_key` is surfaced per ADR-002 as the
/// transitional FE-bridge identifier; consumers MUST still key lookups by `id`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct MetricView {
    pub id: Uuid,
    /// Backend's `<table_name>.<column_name>` identifier. Surfaced per ADR-002
    /// so the FE can align compiled-in `BULLET_DEFS` constants to wire rows
    /// during the catalog-hydration transitional release; the stable lookup
    /// key remains `id`.
    pub metric_key: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sublabel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    pub higher_is_better: bool,
    pub is_member_scale: bool,
    pub source_tags: Vec<String>,
    /// `"ok" | "error" | "unchecked"` — sourced from `metric_catalog.schema_status`.
    /// Consumers render `"unchecked"` the same as `"ok"` (validator hasn't run
    /// yet); only `"error"` triggers the broken-metric indicator.
    pub schema_status: String,
    /// Canonical code from `{ table_not_found, column_not_found,
    /// clickhouse_unreachable, unknown }`, only present when `schema_status = "error"`.
    /// Raw ClickHouse error text NEVER reaches consumers per DESIGN §3.3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_error_code: Option<String>,
    pub thresholds: ThresholdView,
}

/// Resolved threshold for one metric.
///
/// `good` / `warn` are `f64` on the wire — DECIMAL(20,6) in the DB rounds-trips
/// through DOUBLE for every seed value (integers and one-decimal floats). If
/// future seed entries need full-precision decimals, this is the place to switch
/// to a string serializer; the FE byte-for-byte comparison gate (PRD §12) is
/// the regression detector.
///
/// The OpenAPI component is named `CatalogThresholdView` (via `#[schema(as)]`)
/// to disambiguate from the admin-CRUD `ThresholdView`
/// (`domain::admin_threshold::dto::ThresholdView`), which is a different wire
/// shape registered under `AdminMetricThresholdView`. `#[schema(as)]` renames
/// only the OpenAPI component — it does NOT affect serde / the wire format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
#[schema(as = CatalogThresholdView)]
pub struct ThresholdView {
    pub good: f64,
    pub warn: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert_trigger: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert_bad: Option<f64>,
    /// One of `"team+role" | "team" | "role" | "tenant" | "product-default"`.
    /// Names the row that won the walk.
    pub resolved_from: String,
    /// `true` iff the walk halted on a locked broader-scope row before reaching
    /// the most-specific candidate. Separate signal from `resolved_from`, which
    /// always names the row that won.
    pub bounded_by_lock: bool,
}

// Marker traits — `GetMetricsRequest` is the `POST /v1/catalog/get_metrics`
// request body; `CatalogResponse` is its response. `MetricView` /
// `MetricQueryLink` / `ThresholdView` are nested inside `CatalogResponse` and
// need only `ToSchema` (above).
impl toolkit::api::api_dto::RequestApiDto for GetMetricsRequest {}
impl toolkit::api::api_dto::ResponseApiDto for CatalogResponse {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_metric() -> MetricView {
        MetricView {
            id: Uuid::nil(),
            metric_key: "ic_kpis.tasks_closed".to_owned(),
            label: "Tasks Closed".to_owned(),
            sublabel: Some("Jira".to_owned()),
            description: None,
            unit: None,
            format: Some("integer".to_owned()),
            higher_is_better: true,
            is_member_scale: false,
            source_tags: vec!["jira".to_owned()],
            schema_status: "ok".to_owned(),
            schema_error_code: None,
            thresholds: ThresholdView {
                good: 5.0,
                warn: 3.0,
                alert_trigger: None,
                alert_bad: None,
                resolved_from: "product-default".to_owned(),
                bounded_by_lock: false,
            },
        }
    }

    #[test]
    fn metric_view_serializes_metric_key_for_fe_bridge() -> Result<(), serde_json::Error> {
        // ADR-002 narrowed the original "never surface metric_key" rule:
        // `metric_key` IS on the wire as the transitional FE-bridge
        // identifier so dashboards can align their compiled-in `BULLET_DEFS`
        // constants to wire rows during the catalog-hydration release. `id`
        // remains the stable lookup key. If a future refactor drops the
        // field, this test catches it before the FE breaks.
        let m = sample_metric();
        let v: serde_json::Value = serde_json::to_value(&m)?;
        assert_eq!(
            v.get("metric_key").and_then(serde_json::Value::as_str),
            Some("ic_kpis.tasks_closed"),
            "metric_key MUST appear on the wire per ADR-002"
        );
        assert!(
            v.get("id").is_some(),
            "id remains the stable lookup key alongside metric_key"
        );
        Ok(())
    }

    #[test]
    fn response_carries_links_for_query_to_catalog_mapping() -> Result<(), serde_json::Error> {
        // ADR-003: the M:N `metric_query_catalog` mapping is surfaced as a
        // top-level `links` array. Field must exist even when empty so
        // consumers can rely on its shape without defensive null checks.
        let r = CatalogResponse {
            tenant_id: Uuid::nil(),
            generated_at: chrono::Utc::now(),
            metrics: vec![],
            links: vec![MetricQueryLink {
                query_id: Uuid::from_u128(0x11),
                catalog_metric_ids: vec![Uuid::from_u128(0xaa), Uuid::from_u128(0xbb)],
            }],
        };
        let v: serde_json::Value = serde_json::to_value(&r)?;
        let links = v
            .get("links")
            .and_then(serde_json::Value::as_array)
            .unwrap_or_else(|| panic!("links must serialize as an array; got: {v}"));
        assert_eq!(links.len(), 1, "expected exactly one link row");
        assert!(
            links[0].get("query_id").is_some(),
            "link row must expose query_id"
        );
        let ids = links[0]
            .get("catalog_metric_ids")
            .and_then(serde_json::Value::as_array)
            .unwrap_or_else(|| panic!("catalog_metric_ids must serialize as an array"));
        assert_eq!(ids.len(), 2, "expected the two ids we constructed");
        Ok(())
    }

    #[test]
    fn response_links_array_is_empty_not_omitted_when_no_mappings() -> Result<(), serde_json::Error>
    {
        // Defensive: `links` is a real field, not `skip_serializing_if`.
        // Consumers degrade on empty array; null/absent forces a defensive
        // shape check FE-side and breaks the byte-stable wire contract.
        let r = CatalogResponse {
            tenant_id: Uuid::nil(),
            generated_at: chrono::Utc::now(),
            metrics: vec![],
            links: vec![],
        };
        let v: serde_json::Value = serde_json::to_value(&r)?;
        let links = v
            .get("links")
            .and_then(serde_json::Value::as_array)
            .unwrap_or_else(|| {
                panic!("links must be an array even when empty (not null / absent); got: {v}")
            });
        assert_eq!(links.len(), 0, "empty array, not null");
        Ok(())
    }

    #[test]
    fn response_carries_tenant_id_for_cache_reassert() -> Result<(), serde_json::Error> {
        // The cache layer re-asserts `tenant_id` on hydrate. If a future
        // refactor accidentally drops the field from the on-wire envelope, the
        // cache's cross-tenant defense-in-depth check silently degrades.
        let r = CatalogResponse {
            tenant_id: Uuid::nil(),
            generated_at: chrono::Utc::now(),
            metrics: vec![],
            links: vec![],
        };
        let v: serde_json::Value = serde_json::to_value(&r)?;
        assert!(v.get("tenant_id").is_some(), "tenant_id must serialize");
        assert!(
            v.get("generated_at").is_some(),
            "generated_at must serialize"
        );
        assert!(v.get("metrics").is_some(), "metrics must serialize");
        Ok(())
    }

    #[test]
    fn threshold_view_keeps_bounded_by_lock_separate_from_resolved_from()
    -> Result<(), serde_json::Error> {
        // DESIGN §3.3 pins these as two distinct fields: `resolved_from` names
        // the winning row, `bounded_by_lock` indicates whether a narrower
        // candidate was shadowed by a broader lock. Collapsing the two would
        // break the "team override ignored because of a tenant lock" admin
        // explanation surface.
        let t = ThresholdView {
            good: 1.0,
            warn: 0.0,
            alert_trigger: None,
            alert_bad: None,
            resolved_from: "tenant".to_owned(),
            bounded_by_lock: true,
        };
        let v: serde_json::Value = serde_json::to_value(&t)?;
        assert_eq!(v["resolved_from"], "tenant");
        assert_eq!(v["bounded_by_lock"], true);
        Ok(())
    }

    #[test]
    fn request_rejects_body_tenant_id() {
        // `tenant_id` is never accepted from the body — it's a cross-tenant
        // disclosure surface. `deny_unknown_fields` enforces that at the
        // serde layer so a misbehaving / malicious caller gets a 400 instead
        // of a silent ignore. The Axum handler also relies on this: it does
        // not re-check the field, so this serde-level rejection is the only
        // gate.
        let err = serde_json::from_str::<GetMetricsRequest>(
            r#"{"tenant_id": "11111111-1111-1111-1111-111111111111"}"#,
        );
        assert!(err.is_err(), "body-supplied tenant_id must be rejected");
    }

    #[test]
    fn request_accepts_empty_body() -> Result<(), serde_json::Error> {
        // Empty `{}` must resolve at the tenant / product-default chain only —
        // a generic catalog hydrator without role/team context is a legitimate
        // first-class caller (admin audit UI, etc.).
        let r: GetMetricsRequest = serde_json::from_str("{}")?;
        assert!(r.role_slug.is_none());
        assert!(r.team_id.is_none());
        Ok(())
    }
}
