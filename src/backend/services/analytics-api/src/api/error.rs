//! Resource-scoped canonical error types for analytics-api handlers.
//!
//! Each unit struct binds a GTS resource namespace and exposes builder-style
//! constructors (`not_found`, `invalid_argument`, …) from the
//! `modkit-canonical-errors` crate. The resulting `CanonicalError` serializes
//! to an RFC 9457 `application/problem+json` envelope via the crate's
//! `IntoResponse` impl.
//!
//! See `docs/domain/metric-catalog/specs/DESIGN.md` §3.3 (Error Envelope)
//! and DNA `REST/API.md §7` for the platform-wide contract.

use modkit_canonical_errors::resource_error;

#[resource_error("gts.cf.insight.analytics_api.metric.v1~")]
pub struct MetricError;

#[resource_error("gts.cf.insight.analytics_api.threshold.v1~")]
pub struct ThresholdError;

#[resource_error("gts.cf.insight.analytics_api.person.v1~")]
pub struct PersonError;

/// Resource namespace for the metric-catalog domain (Refs #524, #525).
///
/// Distinct from [`MetricError`] (which scopes the legacy `/v1/metrics` CRUD
/// surface) so catalog endpoints see the catalog's own GTS namespace per
/// DESIGN §3.3 ("Resource GTS namespaces introduced for the catalog …
/// `gts.cf.insight.metric_catalog.metric.v1~`").
///
/// `POST /catalog/get_metrics` doesn't currently use this — its body-parse
/// errors flow through [`super::canonical_json::CanonicalJson`], which emits a
/// resource-less envelope because body parse failures fire before the
/// request's target resource is known. Admin-crud (#525) will use this for
/// `not_found` / `permission_denied` / `failed_precondition` on threshold
/// rows, where the resource IS knowable.
#[allow(dead_code)] // first consumer lands with admin-crud (#525)
#[resource_error("gts.cf.insight.metric_catalog.metric.v1~")]
pub struct MetricCatalogError;

/// Resource namespace for tenant-resolution failures
/// (`cpt-metric-cat-constraint-tenant-default`). The middleware surfaces an
/// `invalid_argument` envelope with `field_violations[{field: "tenant_id",
/// reason: "TENANT_UNRESOLVED"}]` when neither a session tenant nor a
/// configured default is present.
#[resource_error("gts.cf.insight.analytics_api.tenant.v1~")]
pub struct TenantError;

#[cfg(test)]
mod tests {
    //! Wire-shape contract for analytics-api error responses.
    //!
    //! These tests pin the §3.3 / RFC 9457 envelope: status code,
    //! `Content-Type: application/problem+json`, `type` GTS URI, and
    //! `context.resource_type` / `context.resource_name` /
    //! `context.field_violations` per category. They prevent silent regressions
    //! in the contract the FE and downstream services depend on.
    //!
    //! Tests assert against `Problem` JSON rather than spinning up an Axum
    //! router so they stay free of the production `AppState` (which needs
    //! MariaDB + `ClickHouse` + `IdentityClient`). The crate's own tests cover
    //! the `IntoResponse` wiring end-to-end; here we verify analytics-api's
    //! namespaces and field shapes.

    use axum::body::to_bytes;
    use axum::http::header::CONTENT_TYPE;
    use axum::response::IntoResponse;
    use modkit_canonical_errors::{CanonicalError, Problem};

    use super::*;

    fn problem(err: CanonicalError) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(Problem::from(err))
    }

    #[test]
    fn metric_not_found_envelope() -> Result<(), Box<dyn std::error::Error>> {
        let err = MetricError::not_found("metric not found or disabled")
            .with_resource("abc-123")
            .create();
        let p = problem(err)?;
        assert_eq!(p["status"], 404);
        assert_eq!(p["title"], "Not Found");
        assert_eq!(p["detail"], "metric not found or disabled");
        assert_eq!(
            p["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.not_found.v1~"
        );
        assert_eq!(
            p["context"]["resource_type"],
            "gts.cf.insight.analytics_api.metric.v1~"
        );
        assert_eq!(p["context"]["resource_name"], "abc-123");
        Ok(())
    }

    #[test]
    fn metric_invalid_argument_envelope() -> Result<(), Box<dyn std::error::Error>> {
        let err = MetricError::invalid_argument()
            .with_field_violation("query_ref", "query_ref must contain SELECT", "INVALID")
            .create();
        let p = problem(err)?;
        assert_eq!(p["status"], 400);
        assert_eq!(
            p["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.invalid_argument.v1~"
        );
        assert_eq!(
            p["context"]["resource_type"],
            "gts.cf.insight.analytics_api.metric.v1~"
        );
        let violations = p["context"]["field_violations"]
            .as_array()
            .ok_or("field_violations must be an array")?;
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0]["field"], "query_ref");
        assert_eq!(violations[0]["reason"], "INVALID");
        Ok(())
    }

    #[test]
    fn threshold_update_invalid_operator_only() -> Result<(), Box<dyn std::error::Error>> {
        // PUT /v1/metrics/{id}/thresholds/{tid} with a bad operator. The
        // handler attaches the threshold id as the resource and reports a
        // single field violation against `operator` — NOT a double violation.
        let err = ThresholdError::invalid_argument()
            .with_resource("tid-xyz")
            .with_field_violation("operator", "invalid operator", "INVALID")
            .create();
        let p = problem(err)?;
        assert_eq!(p["status"], 400);
        assert_eq!(
            p["context"]["resource_type"],
            "gts.cf.insight.analytics_api.threshold.v1~"
        );
        assert_eq!(p["context"]["resource_name"], "tid-xyz");
        let violations = p["context"]["field_violations"]
            .as_array()
            .ok_or("field_violations must be an array")?;
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0]["field"], "operator");
        Ok(())
    }

    #[test]
    fn threshold_create_both_fields_invalid() -> Result<(), Box<dyn std::error::Error>> {
        // POST /v1/metrics/{id}/thresholds with both operator AND level bad.
        // Envelope MUST carry one entry per offending field — no duplication,
        // no cross-talk where the operator's diagnostic gets pinned to `level`.
        let err = ThresholdError::invalid_argument()
            .with_field_violation(
                "operator",
                "operator must be one of: gt, ge, lt, le, eq",
                "INVALID",
            )
            .with_field_violation(
                "level",
                "level must be one of: good, warning, critical",
                "INVALID",
            )
            .create();
        let p = problem(err)?;
        let violations = p["context"]["field_violations"]
            .as_array()
            .ok_or("field_violations must be an array")?;
        assert_eq!(violations.len(), 2);
        assert_eq!(violations[0]["field"], "operator");
        assert_eq!(violations[1]["field"], "level");
        // Each diagnostic names its own field — the operator message MUST
        // NOT appear under the `level` violation (the previous buggy code
        // duplicated the same message under both fields).
        assert!(
            violations[1]["description"]
                .as_str()
                .is_some_and(|s| s.starts_with("level must be one of")),
            "level violation must carry the level-specific message",
        );
        Ok(())
    }

    #[test]
    fn person_not_found_envelope() -> Result<(), Box<dyn std::error::Error>> {
        let err = PersonError::not_found("person not found")
            .with_resource("alice@example.com")
            .create();
        let p = problem(err)?;
        assert_eq!(p["status"], 404);
        assert_eq!(
            p["context"]["resource_type"],
            "gts.cf.insight.analytics_api.person.v1~"
        );
        assert_eq!(p["context"]["resource_name"], "alice@example.com");
        Ok(())
    }

    #[test]
    fn internal_envelope_carries_no_diagnostic() -> Result<(), Box<dyn std::error::Error>> {
        // Internal errors MUST NOT leak the raw error string to the client.
        // The diagnostic (`description`) is serde-skipped on the wire and only
        // surfaces through `CanonicalError::diagnostic()` for server-side logs.
        let err = CanonicalError::internal("DB connection refused: 127.0.0.1:5432").create();
        let p = problem(err)?;
        assert_eq!(p["status"], 500);
        assert_eq!(
            p["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.internal.v1~"
        );
        // The default Internal detail message is used; the raw diagnostic
        // never appears in `detail` or `context`.
        assert_eq!(
            p["detail"],
            "An internal error occurred. Please retry later."
        );
        assert_eq!(p["context"], serde_json::json!({}));
        // No keys leak the diagnostic string.
        let body = serde_json::to_string(&p)?;
        assert!(
            !body.contains("DB connection refused"),
            "raw diagnostic leaked to wire: {body}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn into_response_sets_problem_json_content_type() -> Result<(), Box<dyn std::error::Error>>
    {
        // End-to-end: a CanonicalError flows through axum's IntoResponse and
        // lands on the wire with the RFC 9457 content type and correct status.
        let err = MetricError::not_found("metric not found or disabled")
            .with_resource("abc-123")
            .create();
        let resp = err.into_response();
        assert_eq!(resp.status(), 404);
        assert_eq!(
            resp.headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/problem+json"),
        );
        let body_bytes = to_bytes(resp.into_body(), 16 * 1024).await?;
        let body: serde_json::Value = serde_json::from_slice(&body_bytes)?;
        assert_eq!(
            body["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.not_found.v1~"
        );
        assert_eq!(
            body["context"]["resource_type"],
            "gts.cf.insight.analytics_api.metric.v1~"
        );
        Ok(())
    }
}
