//! Resource-scoped canonical error types for analytics handlers.
//!
//! Each unit struct binds a GTS resource namespace and exposes builder-style
//! constructors (`not_found`, `invalid_argument`, …) from the
//! `toolkit-canonical-errors` crate. The resulting `CanonicalError` serializes
//! to an RFC 9457 `application/problem+json` envelope via the crate's
//! `IntoResponse` impl.
//!
//! See the committed contract at `docs/components/backend/analytics/openapi.json`
//! and DNA `REST/API.md §7` for the platform-wide contract.

use toolkit_canonical_errors::resource_error;

#[resource_error("gts.cf.insight.analytics_api.metric.v1~")]
pub struct MetricError;

/// Resource namespace for `/v1/queries*` (saved-query CRUD + run, #1965).
#[resource_error("gts.cf.insight.analytics_api.saved_query.v1~")]
pub struct SavedQueryError;

/// Resource namespace for `/v1/usage*` (adoption events + the admin read model).
#[resource_error("gts.cf.insight.analytics_api.usage.v1~")]
pub struct UsageError;

/// Resource namespace for `/v1/metrics*` (custom-metric CRUD + export/import).
#[resource_error("gts.cf.insight.analytics_api.custom_metric.v1~")]
pub struct CustomMetricError;

/// Resource namespace for tenant-resolution failures
/// (`cpt-metric-cat-constraint-tenant-default`). The middleware surfaces an
/// `invalid_argument` envelope with `field_violations[{field: "tenant_id",
/// reason: "TENANT_UNRESOLVED"}]` when neither a session tenant nor a
/// configured default is present.
// Tenant namespace retained for re-enabling auth; no unresolved path under auth_disabled.
#[allow(dead_code)]
#[resource_error("gts.cf.insight.analytics_api.tenant.v1~")]
pub struct TenantError;

#[cfg(test)]
mod tests {
    //! Wire-shape contract for analytics error responses.
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
    //! the `IntoResponse` wiring end-to-end; here we verify the analytics gear's
    //! namespaces and field shapes.

    use axum::body::to_bytes;
    use axum::http::header::CONTENT_TYPE;
    use axum::response::IntoResponse;
    use toolkit_canonical_errors::{CanonicalError, Problem};

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
    fn saved_query_invalid_sql_envelope() -> Result<(), Box<dyn std::error::Error>> {
        // POST/PUT /v1/queries with SQL the single-SELECT gate rejects: a 400
        // pinning the `sql` field under the saved-query resource namespace.
        let err = SavedQueryError::invalid_argument()
            .with_resource("q-123")
            .with_field_violation("sql", "query must be a single SELECT statement", "INVALID")
            .create();
        let p = problem(err)?;
        assert_eq!(p["status"], 400);
        assert_eq!(
            p["context"]["resource_type"],
            "gts.cf.insight.analytics_api.saved_query.v1~"
        );
        let violations = p["context"]["field_violations"]
            .as_array()
            .ok_or("field_violations must be an array")?;
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0]["field"], "sql");
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
