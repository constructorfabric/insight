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
        "/v1/metrics",
        "/v1/metrics/queries",
        "/v1/catalog/get_metrics",
        "/v1/metric-results",
    ] {
        assert!(paths.contains_key(expected), "missing path {expected}");
    }

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
    assert!(schemas.contains_key("Metric"), "Metric schema missing");
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
