//! Unit test for the offline OpenAPI generation (`analytics openapi` /
//! [`super::openapi_document`]).
//!
//! Builds the spec from the same stateless `build_operations` route table the
//! gear serves — no DB, no HTTP listener, no `AppState`. The live
//! `/openapi.json` route is owned by the gears-rust host (the api-gateway
//! system gear), so it is not exercised here; this guards that the committed
//! contract the drift gate diffs is buildable and carries the typed schemas.

use super::openapi_document;

#[test]
fn openapi_document_covers_the_route_table() -> anyhow::Result<()> {
    // Build offline (no DB / listener) and inspect the serialized form — the
    // same JSON `print_openapi` emits and the drift gate diffs.
    let doc = openapi_document()?;
    let json = serde_json::to_value(&doc)?;

    // Stable API-contract identity from `openapi_info` (deliberately not the
    // crate version — see the drift-gate rationale).
    assert_eq!(json["info"]["title"], "Analytics API");
    assert_eq!(json["info"]["version"], "1.0.0");

    // Registered operations show up as paths. `/health` is host-owned, so it is
    // intentionally absent from the analytics contract.
    let paths = json["paths"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("paths object missing"))?;
    for expected in [
        "/v1/ai/config",
        "/v1/ai/context",
        "/v1/ai/context/{id}",
        "/v1/ai/credentials",
        "/v1/ai/explain",
        "/v1/ai/settings",
        "/v1/connector-health",
        "/v1/connector-health/{connector}/syncs",
        "/v1/feedback",
        "/v1/gear-roadmap",
        "/v1/gear-roadmap/boards",
        "/v1/ingestion/intensity",
        "/v1/metric-definitions",
        "/v1/metric-drilldown",
        "/v1/metric-drilldown/export",
        "/v1/metric-results",
        "/v1/reports/preview",
        "/v1/reports/export",
        "/v1/metrics",
        "/v1/metrics/export",
        "/v1/metrics/import",
        "/v1/metrics/{metric_key}",
        "/v1/queries",
        "/v1/queries/{id}",
        "/v1/queries/{id}/run",
        "/v1/usage/config",
        "/v1/usage/events",
        "/v1/usage/summary",
    ] {
        assert!(paths.contains_key(expected), "missing path {expected}");
    }
    assert_eq!(
        paths.len(),
        28,
        "the contract must carry exactly the surviving paths, got {:?}",
        paths.keys().collect::<Vec<_>>()
    );

    // Typed request/response bodies register real component schemas instead of
    // the pre-migration generic `object`.
    let schemas = json["components"]["schemas"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("component schemas missing"))?;
    assert!(
        schemas.len() >= 20,
        "expected the typed contract to register many schemas, got {}",
        schemas.len()
    );
    assert!(
        schemas.contains_key("SavedQuery"),
        "SavedQuery schema missing"
    );
    assert!(
        schemas.contains_key("MetricResultsRequest"),
        "MetricResultsRequest schema missing"
    );
    assert!(
        schemas.contains_key("MetricGroupLimitRequest"),
        "MetricGroupLimitRequest schema missing"
    );
    assert!(
        schemas.contains_key("TimeseriesDto"),
        "TimeseriesDto schema missing"
    );
    Ok(())
}

#[test]
fn report_export_request_is_strict_and_caps_metric_keys() -> anyhow::Result<()> {
    let json = serde_json::to_value(openapi_document()?)?;
    let request = &json["components"]["schemas"]["ReportExportRequest"];

    assert_eq!(request["type"], "object");
    assert_eq!(request["additionalProperties"], false);
    assert_eq!(request["properties"]["metric_keys"]["maxItems"], 100);
    assert_eq!(
        request["required"],
        serde_json::json!(["subject", "period", "granularity", "metric_keys", "format"])
    );
    Ok(())
}

#[test]
fn the_gear_roadmap_publishes_the_order_it_accepts() -> anyhow::Result<()> {
    let json = serde_json::to_value(openapi_document()?)?;
    let params = json["paths"]["/v1/gear-roadmap"]["get"]["parameters"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("gear-roadmap declares no query parameters"))?
        .iter()
        .map(|param| param["name"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();

    assert!(params.contains(&"sort".to_owned()), "got {params:?}");
    assert!(params.contains(&"direction".to_owned()), "got {params:?}");
    Ok(())
}

#[test]
fn the_gear_roadmap_takes_the_board_from_the_caller() -> anyhow::Result<()> {
    let json = serde_json::to_value(openapi_document()?)?;
    let params = json["paths"]["/v1/gear-roadmap"]["get"]["parameters"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("gear-roadmap declares no query parameters"))?;

    let project = params
        .iter()
        .find(|param| param["name"] == "project")
        .ok_or_else(|| anyhow::anyhow!("gear-roadmap does not take a project"))?;

    assert_eq!(project["required"], true, "the board is not optional");
    Ok(())
}

#[test]
fn the_boards_a_caller_may_name_are_published() -> anyhow::Result<()> {
    let json = serde_json::to_value(openapi_document()?)?;

    assert!(
        json["paths"]["/v1/gear-roadmap/boards"]["get"].is_object(),
        "no endpoint lists the boards the project parameter accepts"
    );
    Ok(())
}
