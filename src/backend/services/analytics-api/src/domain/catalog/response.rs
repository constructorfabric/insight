//! Wire shape for `POST /catalog/get_metrics` (DESIGN §3.3 "Catalog Read").
//!
//! Two invariants pinned by tests:
//!
//! 1. **`metric_key` is NEVER in the response.** It is `<table_name>.<column_name>`
//!    backend form; surfacing it would couple consumers to internal source-schema
//!    naming. Consumers identify metrics by `id` (UUIDv7) and render `label`.
//! 2. **`bounded_by_lock` is a separate field from `resolved_from`.** `resolved_from`
//!    names the row that won the walk; `bounded_by_lock` is `true` iff the walk
//!    halted on a locked broader-scope row before reaching the most-specific
//!    candidate. The two signals together let admin tooling explain "why was
//!    this team-scope override ignored" without a second request.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request body for `POST /catalog/get_metrics`.
///
/// `tenant_id` is intentionally NOT accepted here — it is resolved server-side
/// from the session by `tenant_middleware` (Refs #522 auth-trait). Allowing a
/// body-supplied `tenant_id` would open a cross-tenant disclosure surface.
/// `deny_unknown_fields` enforces that defensively at the parser layer: a
/// caller that smuggles `"tenant_id": "..."` into the body gets a 400 instead
/// of a silent ignore.
#[derive(Debug, Clone, Deserialize, Default)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CatalogResponse {
    pub tenant_id: Uuid,
    pub generated_at: DateTime<Utc>,
    pub metrics: Vec<MetricView>,
}

/// One catalog metric on the wire. `metric_key` is deliberately absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricView {
    pub id: Uuid,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_metric() -> MetricView {
        MetricView {
            id: Uuid::nil(),
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
    fn metric_view_does_not_serialize_metric_key() -> Result<(), serde_json::Error> {
        // Wire-shape regression guard: `metric_key` MUST stay backend-internal.
        // If a future refactor adds a `metric_key` field to `MetricView` without
        // a `#[serde(skip_serializing)]`, this test catches it before consumers
        // start coupling to source-schema names.
        let m = sample_metric();
        let s = serde_json::to_string(&m)?;
        assert!(
            !s.contains("metric_key"),
            "metric_key MUST NOT appear in the wire response; got: {s}"
        );
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
