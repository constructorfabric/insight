use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode};
use clickhouse::test::Mock;
use serde_json::{Value, json};
use toolkit::api::OpenApiRegistryImpl;
use tower::ServiceExt as _;

use super::*;
use crate::api::AppState;
use crate::chat::ChatClient;
use crate::domain::alerts::{Destinations, Limits};
use crate::domain::definition::DefinitionKind;
use crate::domain::folders::DefinitionStore;
use crate::store::alert_schedule::memory::MemorySchedule;
use crate::store::alerts::memory::MemoryAlerts;
use crate::store::definitions::memory::MemoryDefinitions;

type R = Result<(), Box<dyn std::error::Error>>;

struct TestHarness {
    _clickhouse: Mock,
    router: Router,
    schedule: Arc<MemorySchedule>,
}

impl TestHarness {
    fn new() -> Self {
        Self::build(true, true)
    }

    fn build(is_admin: bool, alerts_on: bool) -> Self {
        Self::build_with(is_admin, alerts_on, MemoryAlerts::new())
    }

    fn build_with(is_admin: bool, alerts_on: bool, alerts: MemoryAlerts) -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let openapi = OpenApiRegistryImpl::new();
        let client =
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(mock.url(), "insight"));
        let definitions: Arc<dyn DefinitionStore> = Arc::new(MemoryDefinitions::new());
        futures::executor::block_on(async {
            let name = crate::domain::definition::DefinitionName::parse("prs-open")
                .unwrap_or_else(|error| panic!("{error}"));
            definitions
                .put(
                    DefinitionKind::Metric,
                    &name,
                    &json!({"table": "gold.prs", "fields": [{"agg": "count", "type": "int", "as_name": "total"}]}),
                )
                .await
                .unwrap_or_else(|error| panic!("the metric is stored: {error}"));
        });
        let schedule = Arc::new(MemorySchedule::new());
        let mut state = AppState::new(
            crate::domain::query::metric_query::MetricRunner::new(
                client,
                crate::domain::query::metric_query::People::new("identity"),
            ),
            definitions,
            ChatClient::keyless(),
            crate::store::identity::IdentityClient::fixed(is_admin),
            crate::api::Datasets::offline(mock.url()),
            crate::store::catalog::Catalog::fixed(Vec::new()),
        );
        if alerts_on {
            state = state.with_alerts(crate::api::Alerts {
                store: Arc::new(alerts),
                schedule: schedule.clone(),
                limits: Limits {
                    max_rules: 2,
                    ..Limits::default()
                },
                destinations: Destinations::new(BTreeMap::from([(
                    "ops".to_owned(),
                    "discord".to_owned(),
                )])),
            });
        }
        let router = register_routes(Router::new(), &openapi, &Arc::new(state));

        Self {
            _clickhouse: mock,
            router,
            schedule,
        }
    }

    async fn send(&self, method: &str, uri: &str, body: Option<Value>) -> TestResponse {
        let builder = Request::builder().method(method).uri(uri);
        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string())),
            None => builder.body(Body::empty()),
        }
        .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .unwrap_or_else(|error| panic!("router must respond: {error}"));

        TestResponse::from_response(response).await
    }
}

struct TestResponse {
    status: StatusCode,
    body: Bytes,
}

impl TestResponse {
    async fn from_response(response: axum::response::Response) -> Self {
        let status = response.status();
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap_or_else(|error| panic!("response body must be readable: {error}"));

        Self { status, body }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|error| panic!("response body must be JSON: {error}"))
    }
}

fn rule(edit: impl FnOnce(&mut Value)) -> Value {
    let mut body = json!({
        "metric": "prs-open",
        "column": "total",
        "operator": ">",
        "threshold": 10,
        "interval_secs": 300,
        "destination": "ops"
    });
    edit(&mut body);

    body
}

#[tokio::test]
async fn a_created_rule_reads_back_at_revision_one_and_is_scheduled() -> R {
    let harness = TestHarness::new();

    let created = harness
        .send("PUT", "/v1/alerts/too-many-prs", Some(rule(|_| {})))
        .await;
    assert_eq!(created.status, StatusCode::OK, "{:?}", created.body);
    let shown = created.json();
    assert_eq!(shown["revision"], json!(1));
    assert_eq!(shown["enabled"], json!(true));
    assert_eq!(shown["threshold"], json!(10));
    assert_eq!(shown["state"], json!({}));

    let read = harness.send("GET", "/v1/alerts/too-many-prs", None).await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.json()["name"], json!("too-many-prs"));

    let scheduled = harness.schedule.entries();
    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0].every_secs, 300);
    assert_eq!(scheduled[0].job.revision, 1);

    let listed = harness.send("GET", "/v1/alerts?q=prs", None).await;
    assert_eq!(listed.json()["names"], json!(["too-many-prs"]));

    Ok(())
}

#[tokio::test]
async fn replacing_needs_the_revision_it_replaces() -> R {
    let harness = TestHarness::new();
    harness
        .send("PUT", "/v1/alerts/a", Some(rule(|_| {})))
        .await;

    let without = harness
        .send(
            "PUT",
            "/v1/alerts/a",
            Some(rule(|body| body["threshold"] = json!(20))),
        )
        .await;
    assert_eq!(without.status, StatusCode::CONFLICT, "{:?}", without.body);

    let stale = harness
        .send(
            "PUT",
            "/v1/alerts/a",
            Some(rule(|body| {
                body["threshold"] = json!(20);
                body["expected_revision"] = json!(7);
            })),
        )
        .await;
    assert_eq!(stale.status, StatusCode::CONFLICT, "{:?}", stale.body);

    let replaced = harness
        .send(
            "PUT",
            "/v1/alerts/a",
            Some(rule(|body| {
                body["threshold"] = json!(20);
                body["interval_secs"] = json!(600);
                body["expected_revision"] = json!(1);
            })),
        )
        .await;
    assert_eq!(replaced.status, StatusCode::OK, "{:?}", replaced.body);
    assert_eq!(replaced.json()["revision"], json!(2));
    let scheduled = harness.schedule.entries();
    assert_eq!(
        (scheduled[0].job.revision, scheduled[0].every_secs),
        (2, 600)
    );

    let absent = harness
        .send(
            "PUT",
            "/v1/alerts/b",
            Some(rule(|body| body["expected_revision"] = json!(1))),
        )
        .await;
    assert_eq!(absent.status, StatusCode::NOT_FOUND, "{:?}", absent.body);

    Ok(())
}

#[tokio::test]
async fn a_rule_is_refused_for_what_it_gets_wrong() -> R {
    let harness = TestHarness::new();
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "absent metric",
            rule(|body| body["metric"] = json!("nothing-here")),
            "metric",
        ),
        (
            "unknown destination",
            rule(|body| body["destination"] = json!("nowhere")),
            "destination",
        ),
        (
            "interval below the floor",
            rule(|body| body["interval_secs"] = json!(5)),
            "interval_secs",
        ),
        (
            "text threshold",
            rule(|body| body["threshold"] = json!("10")),
            "threshold",
        ),
        (
            "unknown field",
            rule(|body| body["colour"] = json!("red")),
            "body",
        ),
    ];

    for (case, body, field) in cases {
        let refused = harness.send("PUT", "/v1/alerts/x", Some(body)).await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "should refuse: {case}"
        );
        let shown = refused.json();
        assert!(
            shown.to_string().contains(field),
            "should name `{field}` for {case}: {shown}"
        );
    }
    assert!(harness.schedule.entries().is_empty());

    Ok(())
}

#[tokio::test]
async fn the_rule_count_is_bounded() -> R {
    let harness = TestHarness::new();
    for name in ["a", "b"] {
        let created = harness
            .send("PUT", &format!("/v1/alerts/{name}"), Some(rule(|_| {})))
            .await;
        assert_eq!(created.status, StatusCode::OK);
    }

    let third = harness
        .send("PUT", "/v1/alerts/c", Some(rule(|_| {})))
        .await;
    assert_eq!(third.status, StatusCode::CONFLICT, "{:?}", third.body);

    Ok(())
}

#[tokio::test]
async fn disabling_takes_the_rule_off_the_schedule_and_enabling_puts_it_back() -> R {
    let harness = TestHarness::new();
    harness
        .send("PUT", "/v1/alerts/a", Some(rule(|_| {})))
        .await;

    let off = harness
        .send(
            "POST",
            "/v1/alerts/a/disable",
            Some(json!({"expected_revision": 1})),
        )
        .await;
    assert_eq!(off.status, StatusCode::OK, "{:?}", off.body);
    assert_eq!(off.json()["enabled"], json!(false));
    assert_eq!(off.json()["revision"], json!(2));
    assert!(harness.schedule.entries().is_empty());

    let stale = harness
        .send(
            "POST",
            "/v1/alerts/a/enable",
            Some(json!({"expected_revision": 1})),
        )
        .await;
    assert_eq!(stale.status, StatusCode::CONFLICT);

    let on = harness
        .send(
            "POST",
            "/v1/alerts/a/enable",
            Some(json!({"expected_revision": 2})),
        )
        .await;
    assert_eq!(on.status, StatusCode::OK, "{:?}", on.body);
    assert_eq!(harness.schedule.entries()[0].job.revision, 3);

    Ok(())
}

#[tokio::test]
async fn deleting_removes_the_rule_and_its_schedule() -> R {
    let harness = TestHarness::new();
    harness
        .send("PUT", "/v1/alerts/a", Some(rule(|_| {})))
        .await;

    let deleted = harness.send("DELETE", "/v1/alerts/a", None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert!(harness.schedule.entries().is_empty());
    assert_eq!(
        harness.send("GET", "/v1/alerts/a", None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        harness.send("DELETE", "/v1/alerts/a", None).await.status,
        StatusCode::NOT_FOUND
    );

    Ok(())
}

#[tokio::test]
async fn notifications_and_destinations_are_listed() -> R {
    let harness = TestHarness::new();
    harness
        .send("PUT", "/v1/alerts/a", Some(rule(|_| {})))
        .await;

    let none = harness
        .send("GET", "/v1/alerts/a/notifications", None)
        .await;
    assert_eq!(none.status, StatusCode::OK);
    assert_eq!(none.json()["notifications"], json!([]));
    assert_eq!(
        harness
            .send("GET", "/v1/alerts/b/notifications", None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    let destinations = harness.send("GET", "/v1/alert-destinations", None).await;
    assert_eq!(
        destinations.json(),
        json!({"destinations": [{"name": "ops", "provider": "discord"}]})
    );

    Ok(())
}

#[tokio::test]
async fn every_route_refuses_a_caller_without_the_admin_role() -> R {
    let harness = TestHarness::build(false, true);
    let calls = [
        ("GET", "/v1/alerts", None),
        ("PUT", "/v1/alerts/a", Some(rule(|_| {}))),
        ("GET", "/v1/alerts/a", None),
        ("DELETE", "/v1/alerts/a", None),
        (
            "POST",
            "/v1/alerts/a/enable",
            Some(json!({"expected_revision": 1})),
        ),
        (
            "POST",
            "/v1/alerts/a/disable",
            Some(json!({"expected_revision": 1})),
        ),
        ("GET", "/v1/alerts/a/notifications", None),
        ("GET", "/v1/alert-destinations", None),
    ];

    for (method, uri, body) in calls {
        let refused = harness.send(method, uri, body).await;
        assert_eq!(
            refused.status,
            StatusCode::FORBIDDEN,
            "should refuse: {method} {uri}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn a_store_that_is_down_is_a_server_error_not_a_refusal() -> R {
    let harness = TestHarness::build_with(true, true, MemoryAlerts::refusing());

    let answered = harness.send("GET", "/v1/alerts/a", None).await;
    assert_eq!(
        answered.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{:?}",
        answered.body
    );
    assert!(!String::from_utf8_lossy(&answered.body).contains("store is down"));

    Ok(())
}

#[tokio::test]
async fn an_installation_without_alerts_says_so() -> R {
    let harness = TestHarness::build(true, false);

    let answered = harness.send("GET", "/v1/alerts", None).await;
    assert_eq!(
        answered.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        answered.body
    );
    assert!(String::from_utf8_lossy(&answered.body).contains("not enabled"));

    Ok(())
}
