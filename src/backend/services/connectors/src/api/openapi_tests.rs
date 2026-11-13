use super::openapi_document;

#[test]
fn openapi_document_preserves_the_moved_operations() -> anyhow::Result<()> {
    let json = serde_json::to_value(openapi_document()?)?;
    assert_eq!(json["info"]["title"], "Connectors API");
    assert_eq!(json["info"]["version"], "1.0.0");

    let paths = json["paths"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("paths object missing"))?;
    assert_eq!(paths.len(), 3);
    for (path, operation_id) in [
        (
            "/v1/connector-health",
            "analytics_api.connector_health.summary",
        ),
        (
            "/v1/connector-health/{connector}/syncs",
            "analytics_api.connector_health.syncs",
        ),
        (
            "/v1/ingestion/intensity",
            "analytics_api.ingestion.intensity",
        ),
    ] {
        assert_eq!(paths[path]["get"]["operationId"], operation_id, "{path}");
        assert_eq!(
            paths[path]["get"]["security"],
            serde_json::json!([{ "bearerAuth": [] }]),
            "{path}"
        );
    }

    let schemas = json["components"]["schemas"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("component schemas missing"))?;
    for expected in [
        "AppliedSchedule",
        "ConnectorHealth",
        "ConnectorHealthResponse",
        "Grain",
        "IngestionIntensityResponse",
        "IngestionPoint",
        "ScheduleOrigin",
        "Series",
        "SyncFact",
        "SyncHistoryResponse",
    ] {
        assert!(schemas.contains_key(expected), "missing schema {expected}");
    }
    Ok(())
}
