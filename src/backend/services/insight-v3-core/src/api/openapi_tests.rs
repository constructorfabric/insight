use super::openapi_document;

/// The document CI diffs against `docs/components/backend/insight-v3-core/`.
/// It is built offline from the same registrations that serve the routes, so
/// a route added without a spec, or a state that starts dialing at build
/// time, fails here rather than in the drift gate.
#[test]
fn the_document_covers_every_route_this_service_serves() {
    let document = openapi_document().unwrap_or_else(|error| panic!("builds: {error}"));

    let paths: Vec<&str> = document.paths.paths.keys().map(String::as_str).collect();
    for expected in [
        "/v1/raw-data",
        "/v1/tables/{table}",
        "/v1/metrics",
        "/v1/metrics/{name}",
        "/v1/metrics/{name}/run",
        "/v1/metrics/{name}/rename",
        "/v1/widgets",
        "/v1/dashboards",
        "/v1/chat",
    ] {
        assert!(
            paths.contains(&expected),
            "{expected} missing from {paths:?}"
        );
    }

    assert_eq!(document.info.title, "Insight v3 Core API");
}
