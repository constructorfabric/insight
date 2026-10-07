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
use crate::store::alert_schedule::memory::{MemoryDeliveries, MemorySchedule};
use crate::store::alerts::memory::MemoryAlerts;
use crate::store::definitions::memory::MemoryDefinitions;

type R = Result<(), Box<dyn std::error::Error>>;

struct TestHarness {
    _clickhouse: Mock,
    router: Router,
    schedule: Arc<MemorySchedule>,
    store: Arc<MemoryAlerts>,
}

impl TestHarness {
    fn new() -> Self {
        Self::build(true, true)
    }

    fn build(is_admin: bool, alerts_on: bool) -> Self {
        Self::build_with(
            is_admin,
            alerts_on,
            MemoryAlerts::new(),
            MemorySchedule::new(),
        )
    }

    fn build_with(
        is_admin: bool,
        alerts_on: bool,
        alerts: MemoryAlerts,
        schedule: MemorySchedule,
    ) -> Self {
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
        let schedule = Arc::new(schedule);
        let store = Arc::new(alerts);
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
                store: store.clone(),
                schedule: schedule.clone(),
                deliveries: Arc::new(MemoryDeliveries::new()),
                providers: BTreeMap::new(),
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
            store,
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
    content_type: Option<String>,
    body: Bytes,
}

impl TestResponse {
    async fn from_response(response: axum::response::Response) -> Self {
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap_or_else(|error| panic!("response body must be readable: {error}"));

        Self {
            status,
            content_type,
            body,
        }
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|error| panic!("response body must be JSON: {error}"))
    }
}

fn rule(edit: impl FnOnce(&mut Value)) -> Value {
    let mut body = json!({
        "name": "Too many open PRs",
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

impl TestHarness {
    /// Creates a rule and answers its id.
    async fn create(&self, body: Value) -> String {
        let created = self.send("POST", "/v1/alerts", Some(body)).await;
        assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);

        created.json()["id"]
            .as_str()
            .unwrap_or_else(|| panic!("a created alert has an id"))
            .to_owned()
    }
}

/// A missing alert as the document declares it: a problem body naming what
/// was asked for, whether the id is absent or not an id at all.
fn assert_not_found(response: &TestResponse, asked: &str) {
    assert_eq!(
        response.status,
        StatusCode::NOT_FOUND,
        "{:?}",
        response.body
    );
    assert!(
        response
            .content_type
            .as_deref()
            .is_some_and(|value| value.starts_with("application/problem+json")),
        "{:?}",
        response.content_type
    );
    let shown = response.json();
    assert_eq!(shown["status"], json!(404), "{shown}");
    let detail = shown["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("was not found") && detail.contains(asked),
        "{shown}"
    );
}

#[tokio::test]
async fn a_created_rule_reads_back_at_revision_one_and_is_scheduled() -> R {
    let harness = TestHarness::new();

    let created = harness.send("POST", "/v1/alerts", Some(rule(|_| {}))).await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let shown = created.json();
    let id = shown["id"].as_str().unwrap_or_default().to_owned();
    assert_eq!(id.len(), 32, "the id is shown as 32 hex characters: {id}");
    assert_eq!(shown["name"], json!("Too many open PRs"));
    assert_eq!(shown["revision"], json!(1));
    assert_eq!(shown["enabled"], json!(true));
    assert_eq!(shown["threshold"], json!(10));
    assert_eq!(shown["state"], json!({}));

    let read = harness.send("GET", &format!("/v1/alerts/{id}"), None).await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.json()["id"], json!(id));

    let scheduled = harness.schedule.entries();
    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0].every_secs, 300);
    assert_eq!(scheduled[0].job.revision, 1);

    let listed = harness.send("GET", "/v1/alerts?q=open", None).await;
    assert_eq!(
        listed.json()["alerts"],
        json!([{"id": id, "name": "Too many open PRs", "metric": "prs-open", "enabled": true}])
    );
    assert_eq!(listed.json()["total"], json!(1));
    let regardless_of_case = harness.send("GET", "/v1/alerts?q=OPEN", None).await;
    assert_eq!(regardless_of_case.json()["total"], json!(1));

    Ok(())
}

#[tokio::test]
async fn a_threshold_past_what_javascript_holds_round_trips_as_a_digit_string() -> R {
    let harness = TestHarness::new();

    let created = harness
        .send(
            "POST",
            "/v1/alerts",
            Some(rule(|body| {
                body["threshold"] = json!(9_007_199_254_740_993_u64);
            })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let shown = created.json();
    assert_eq!(shown["threshold"], json!("9007199254740993"));
    let id = shown["id"].as_str().unwrap_or_default().to_owned();

    let written_back = harness
        .send(
            "PUT",
            &format!("/v1/alerts/{id}"),
            Some(rule(|body| {
                body["threshold"] = json!("9007199254740993");
                body["expected_revision"] = json!(1);
            })),
        )
        .await;
    assert_eq!(
        written_back.status,
        StatusCode::OK,
        "{:?}",
        written_back.body
    );
    assert_eq!(written_back.json()["threshold"], json!("9007199254740993"));

    Ok(())
}

#[tokio::test]
async fn two_alerts_may_share_a_name_and_are_told_apart_by_id() -> R {
    let harness = TestHarness::new();

    let first = harness.create(rule(|_| {})).await;
    let second = harness.create(rule(|_| {})).await;
    assert_ne!(first, second);

    let listed = harness.send("GET", "/v1/alerts", None).await;
    assert_eq!(listed.json()["total"], json!(2));

    Ok(())
}

#[tokio::test]
async fn replacing_needs_the_revision_it_replaces() -> R {
    let harness = TestHarness::new();
    let id = harness.create(rule(|_| {})).await;
    let path = format!("/v1/alerts/{id}");

    let without = harness
        .send(
            "PUT",
            &path,
            Some(rule(|body| body["threshold"] = json!(20))),
        )
        .await;
    assert_eq!(
        without.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        without.body
    );

    let stale = harness
        .send(
            "PUT",
            &path,
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
            &path,
            Some(rule(|body| {
                body["name"] = json!("Far too many open PRs");
                body["threshold"] = json!(20);
                body["interval_secs"] = json!(600);
                body["expected_revision"] = json!(1);
            })),
        )
        .await;
    assert_eq!(replaced.status, StatusCode::OK, "{:?}", replaced.body);
    assert_eq!(replaced.json()["revision"], json!(2));
    assert_eq!(replaced.json()["name"], json!("Far too many open PRs"));
    let scheduled = harness.schedule.entries();
    assert_eq!(
        (scheduled[0].job.revision, scheduled[0].every_secs),
        (2, 600)
    );

    let missing = uuid::Uuid::now_v7().simple().to_string();
    let absent = harness
        .send(
            "PUT",
            &format!("/v1/alerts/{missing}"),
            Some(rule(|body| body["expected_revision"] = json!(1))),
        )
        .await;
    assert_not_found(&absent, &missing);

    let malformed = harness
        .send(
            "PUT",
            "/v1/alerts/not-an-id",
            Some(rule(|body| body["expected_revision"] = json!(1))),
        )
        .await;
    assert_not_found(&malformed, "not-an-id");

    Ok(())
}

#[tokio::test]
async fn a_replace_silent_on_enabled_leaves_a_disabled_rule_off() -> R {
    let harness = TestHarness::new();
    let id = harness.create(rule(|_| {})).await;
    let path = format!("/v1/alerts/{id}");
    harness
        .send(
            "POST",
            &format!("{path}/disable"),
            Some(json!({"expected_revision": 1})),
        )
        .await;

    let silent = harness
        .send(
            "PUT",
            &path,
            Some(rule(|body| {
                body["threshold"] = json!(20);
                body["expected_revision"] = json!(2);
            })),
        )
        .await;
    assert_eq!(silent.status, StatusCode::OK, "{:?}", silent.body);
    assert_eq!(silent.json()["enabled"], json!(false));
    assert_eq!(silent.json()["revision"], json!(3));
    assert!(
        harness.schedule.entries().is_empty(),
        "stays off the schedule"
    );

    let explicit = harness
        .send(
            "PUT",
            &path,
            Some(rule(|body| {
                body["enabled"] = json!(true);
                body["expected_revision"] = json!(3);
            })),
        )
        .await;
    assert_eq!(explicit.status, StatusCode::OK, "{:?}", explicit.body);
    assert_eq!(explicit.json()["enabled"], json!(true));
    assert_eq!(harness.schedule.entries()[0].job.revision, 4);

    Ok(())
}

#[tokio::test]
async fn enabling_a_rule_whose_destination_is_gone_is_refused() -> R {
    use crate::domain::alerts::rule::{AlertStore as _, RuleSpec, Write};

    let harness = TestHarness::new();
    let draft: RuleDraft =
        serde_json::from_value(rule(|body| body["destination"] = json!("retired")))?;
    let once_configured = Destinations::new(BTreeMap::from([(
        "retired".to_owned(),
        "discord".to_owned(),
    )]));
    let spec = RuleSpec::parse(&draft, Limits::default(), &once_configured)?;
    let stored = harness
        .store
        .create(
            Write {
                spec,
                enabled: false,
                actor: None,
            },
            u64::MAX,
        )
        .await?;

    let refused = harness
        .send(
            "POST",
            &format!("/v1/alerts/{}/enable", stored.id.simple()),
            Some(json!({"expected_revision": 1})),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        refused.body
    );
    let shown = refused.json();
    assert!(
        shown.to_string().contains("destination") && shown.to_string().contains("retired"),
        "{shown}"
    );
    assert!(harness.schedule.entries().is_empty());
    assert_eq!(
        harness
            .send("GET", &format!("/v1/alerts/{}", stored.id.simple()), None)
            .await
            .json()["enabled"],
        json!(false)
    );

    Ok(())
}

#[tokio::test]
async fn a_rule_is_refused_for_what_it_gets_wrong() -> R {
    let harness = TestHarness::new();
    let cases: Vec<(&str, Value, &str)> = vec![
        ("blank name", rule(|body| body["name"] = json!(" ")), "name"),
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
            "text threshold that is not an integer",
            rule(|body| body["threshold"] = json!("1,5")),
            "threshold",
        ),
        (
            "expected_revision on create",
            rule(|body| body["expected_revision"] = json!(1)),
            "expected_revision",
        ),
        (
            "unknown field",
            rule(|body| body["colour"] = json!("red")),
            "body",
        ),
    ];

    for (case, body, field) in cases {
        let refused = harness.send("POST", "/v1/alerts", Some(body)).await;
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
    harness.create(rule(|_| {})).await;
    harness.create(rule(|_| {})).await;

    let third = harness.send("POST", "/v1/alerts", Some(rule(|_| {}))).await;
    assert_eq!(third.status, StatusCode::CONFLICT, "{:?}", third.body);
    let shown = third.json().to_string();
    assert!(
        shown.contains(r#""subject":"alerts""#) && !shown.contains(r#""subject":"name""#),
        "the limit is about the collection, not the name: {shown}"
    );

    Ok(())
}

#[tokio::test]
async fn disabling_takes_the_rule_off_the_schedule_and_enabling_puts_it_back() -> R {
    let harness = TestHarness::new();
    let id = harness.create(rule(|_| {})).await;

    let off = harness
        .send(
            "POST",
            &format!("/v1/alerts/{id}/disable"),
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
            &format!("/v1/alerts/{id}/enable"),
            Some(json!({"expected_revision": 1})),
        )
        .await;
    assert_eq!(stale.status, StatusCode::CONFLICT);

    let on = harness
        .send(
            "POST",
            &format!("/v1/alerts/{id}/enable"),
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
    let id = harness.create(rule(|_| {})).await;
    let path = format!("/v1/alerts/{id}");

    let deleted = harness.send("DELETE", &path, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert!(harness.schedule.entries().is_empty());
    assert_not_found(&harness.send("GET", &path, None).await, &id);
    assert_not_found(&harness.send("DELETE", &path, None).await, &id);
    assert_not_found(
        &harness
            .send("GET", &format!("{path}/notifications"), None)
            .await,
        &id,
    );
    assert_not_found(
        &harness
            .send(
                "POST",
                &format!("{path}/enable"),
                Some(json!({"expected_revision": 1})),
            )
            .await,
        &id,
    );

    Ok(())
}

#[tokio::test]
async fn notifications_and_destinations_are_listed() -> R {
    let harness = TestHarness::new();
    let id = harness.create(rule(|_| {})).await;

    let none = harness
        .send("GET", &format!("/v1/alerts/{id}/notifications"), None)
        .await;
    assert_eq!(none.status, StatusCode::OK);
    assert_eq!(none.json()["notifications"], json!([]));
    assert_eq!(
        harness
            .send(
                "GET",
                &format!("/v1/alerts/{}/notifications", uuid::Uuid::now_v7().simple()),
                None
            )
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
    let id = uuid::Uuid::now_v7().simple().to_string();
    let calls = [
        ("GET", "/v1/alerts".to_owned(), None),
        ("POST", "/v1/alerts".to_owned(), Some(rule(|_| {}))),
        ("PUT", format!("/v1/alerts/{id}"), Some(rule(|_| {}))),
        ("GET", format!("/v1/alerts/{id}"), None),
        ("DELETE", format!("/v1/alerts/{id}"), None),
        (
            "POST",
            format!("/v1/alerts/{id}/enable"),
            Some(json!({"expected_revision": 1})),
        ),
        (
            "POST",
            format!("/v1/alerts/{id}/disable"),
            Some(json!({"expected_revision": 1})),
        ),
        ("GET", format!("/v1/alerts/{id}/notifications"), None),
        ("GET", "/v1/alert-destinations".to_owned(), None),
    ];

    for (method, uri, body) in calls {
        let refused = harness.send(method, &uri, body).await;
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
    let harness =
        TestHarness::build_with(true, true, MemoryAlerts::refusing(), MemorySchedule::new());

    let answered = harness.send("GET", "/v1/alerts", None).await;
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
async fn a_write_the_store_took_is_answered_even_when_the_schedule_is_down() -> R {
    let harness =
        TestHarness::build_with(true, true, MemoryAlerts::new(), MemorySchedule::refusing());

    let created = harness.send("POST", "/v1/alerts", Some(rule(|_| {}))).await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let id = created.json()["id"].as_str().unwrap_or_default().to_owned();
    let path = format!("/v1/alerts/{id}");

    let disabled = harness
        .send(
            "POST",
            &format!("{path}/disable"),
            Some(json!({"expected_revision": 1})),
        )
        .await;
    assert_eq!(disabled.status, StatusCode::OK, "{:?}", disabled.body);
    assert_eq!(disabled.json()["enabled"], json!(false));

    let deleted = harness.send("DELETE", &path, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{:?}", deleted.body);

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

#[tokio::test]
async fn a_notification_shows_its_delivery_state() -> R {
    use crate::domain::alerts::delivery::{Attempted, Receipt};
    use crate::domain::alerts::rule::{AlertStore as _, Recorded, Recording};

    let store = MemoryAlerts::new();
    let harness = TestHarness::build_with(true, true, store, MemorySchedule::new());
    let id = harness.create(rule(|_| {})).await;
    let store = harness.store.clone();
    let stored = store
        .get(uuid::Uuid::parse_str(&id)?)
        .await?
        .unwrap_or_else(|| panic!("the rule is stored"));
    let Recorded::Accepted(accepted) = store
        .record(Recording {
            rule_id: stored.id,
            revision: stored.revision,
            outcome: crate::domain::alerts::Outcome::Valid {
                value: crate::domain::alerts::Number::Int(12),
                breached: true,
            },
            evaluated_at: chrono::Utc::now(),
        })
        .await?
    else {
        panic!("the breach is recorded");
    };
    let owed = accepted
        .notification
        .unwrap_or_else(|| panic!("the breach owes a notification"));
    store
        .record_attempt(owed.id, &Attempted::Retry("answered 502".to_owned()))
        .await?;
    store
        .record_attempt(owed.id, &Attempted::Sent(Receipt("m-9".to_owned())))
        .await?;

    let listed = harness
        .send("GET", &format!("/v1/alerts/{id}/notifications"), None)
        .await;
    let shown = &listed.json()["notifications"][0];
    assert_eq!(shown["status"], json!("sent"));
    assert_eq!(shown["attempts"], json!(2));
    assert_eq!(shown["provider_receipt"], json!("m-9"));
    assert_eq!(shown["value"], json!(12));

    Ok(())
}
