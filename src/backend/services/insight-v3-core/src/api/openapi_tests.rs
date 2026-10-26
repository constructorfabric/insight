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
        "/v1/datasets",
        "/v1/datasets/{name}",
        "/v1/datasets/{name}/records",
        "/v1/datasets/{name}/dependents",
        "/v1/metrics",
        "/v1/metrics/{id}",
        "/v1/metrics/{id}/run",
        "/v1/metrics/{id}/drilldown",
        "/v1/metrics/{id}/dependents",
        "/v1/widgets",
        "/v1/dashboards",
        "/v1/dashboards/{id}/duplicate",
        "/v1/dashboards/{id}/tags",
        "/v1/dashboards/{id}/grants",
        "/v1/dashboards/{id}/grants/{grantee}",
        "/v1/dashboards/{id}/transfer",
        "/v1/dashboards/{id}/widgets",
        "/v1/dashboards/{id}/widgets/{widget_id}/run",
        "/v1/dashboards/{id}/widgets/{widget_id}/drilldown",
        "/v1/folders",
        "/v1/folders/{id}",
        "/v1/folders/{id}/grants",
        "/v1/folders/{id}/grants/{grantee}",
        "/v1/folders/{id}/dashboards/{dashboard_id}",
        "/v1/folders/{id}/transfer",
        "/v1/orphaned-dashboards",
        "/v1/orphaned-folders",
        "/v1/pins/{dashboard_id}",
        "/v1/chat",
    ] {
        assert!(
            paths.contains(&expected),
            "{expected} missing from {paths:?}"
        );
    }

    assert!(
        !paths.iter().any(|path| path.ends_with("/rename")),
        "no definition is renamed by route: {paths:?}"
    );
    for gone in ["/v1/dashboards/{id}/folder", "/v1/dashboards/{id}/shares"] {
        assert!(!paths.contains(&gone), "{gone} is still served");
    }
    assert_eq!(document.info.title, "Insight v3 Core API");
}
