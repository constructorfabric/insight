use toolkit_canonical_errors::resource_error;

#[resource_error("gts.cf.insight.analytics_api.metric.v1~")]
pub struct MetricError;

#[resource_error("gts.cf.insight.analytics_api.report.v1~")]
pub struct ReportError;

#[resource_error("gts.cf.insight.analytics_api.saved_query.v1~")]
pub struct SavedQueryError;

#[resource_error("gts.cf.insight.analytics_api.usage.v1~")]
pub struct UsageError;

#[resource_error("gts.cf.insight.analytics_api.connector_health.v1~")]
pub struct ConnectorHealthError;

#[resource_error("gts.cf.insight.analytics_api.feedback.v1~")]
pub struct FeedbackError;

#[resource_error("gts.cf.insight.analytics_api.ai_assist.v1~")]
pub struct AiError;

#[resource_error("gts.cf.insight.analytics_api.ingestion.v1~")]
pub struct IngestionError;

#[resource_error("gts.cf.insight.analytics_api.custom_metric.v1~")]
pub struct CustomMetricError;

#[allow(dead_code)]
#[resource_error("gts.cf.insight.analytics_api.tenant.v1~")]
pub struct TenantError;

#[cfg(test)]
mod tests {

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
        let err = CanonicalError::internal("DB connection refused: 127.0.0.1:5432").create();
        let p = problem(err)?;
        assert_eq!(p["status"], 500);
        assert_eq!(
            p["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.internal.v1~"
        );
        assert_eq!(
            p["detail"],
            "An internal error occurred. Please retry later."
        );
        assert_eq!(p["context"], serde_json::json!({}));
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
