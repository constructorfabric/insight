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
    state: Arc<AppState>,
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
        Self::build_over(is_admin, alerts_on, alerts, schedule, None)
    }

    /// A harness whose metric runs are answered by the warehouse at `url`.
    fn over_warehouse(url: &str, is_admin: bool) -> Self {
        Self::build_over(
            is_admin,
            true,
            MemoryAlerts::new(),
            MemorySchedule::new(),
            Some(url),
        )
    }

    fn build_over(
        is_admin: bool,
        alerts_on: bool,
        alerts: MemoryAlerts,
        schedule: MemorySchedule,
        warehouse: Option<&str>,
    ) -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let openapi = OpenApiRegistryImpl::new();
        let warehouse_url = warehouse.map_or_else(|| mock.url().to_owned(), str::to_owned);
        let client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            &warehouse_url,
            "insight",
        ));
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
            let grouped = crate::domain::definition::DefinitionName::parse("prs-by-team")
                .unwrap_or_else(|error| panic!("{error}"));
            definitions
                .put(
                    DefinitionKind::Metric,
                    &grouped,
                    &json!({
                        "table": "gold.prs",
                        "fields": [
                            {"column": "team", "type": "string", "as_name": "team"},
                            {"agg": "count", "type": "int", "as_name": "total"}
                        ],
                        "group_by": ["team"]
                    }),
                )
                .await
                .unwrap_or_else(|error| panic!("the grouped metric is stored: {error}"));
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
        let state = Arc::new(state);
        let router = register_routes(Router::new(), &openapi, &state);

        Self {
            _clickhouse: mock,
            state,
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
        "condition": { "rows": "one", "where": { "column": "total", "op": ">", "value": 10 } },
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
    assert_eq!(
        shown["condition"],
        json!({ "rows": "one", "where": { "column": "total", "op": ">", "value": 10 } })
    );
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
                body["condition"]["where"]["value"] = json!(9_007_199_254_740_993_u64);
            })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let shown = created.json();
    assert_eq!(
        shown["condition"]["where"]["value"],
        json!("9007199254740993")
    );
    let id = shown["id"].as_str().unwrap_or_default().to_owned();

    let written_back = harness
        .send(
            "PUT",
            &format!("/v1/alerts/{id}"),
            Some(rule(|body| {
                body["condition"]["where"]["value"] = json!("9007199254740993");
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
    assert_eq!(
        written_back.json()["condition"]["where"]["value"],
        json!("9007199254740993")
    );

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
            Some(rule(|body| body["condition"]["where"]["value"] = json!(20))),
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
                body["condition"]["where"]["value"] = json!(20);
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
                body["condition"]["where"]["value"] = json!(20);
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
                body["condition"]["where"]["value"] = json!(20);
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
    let columns = [(
        "total".to_owned(),
        crate::domain::query::metric_query::ColumnKind::Number,
    )];
    let spec = RuleSpec::parse(&draft, Limits::default(), &once_configured, &columns)?;
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
            rule(|body| body["condition"]["where"]["value"] = json!("1,5")),
            "condition.where.value",
        ),
        (
            "a column the metric does not answer",
            rule(|body| body["condition"]["where"]["column"] = json!("closed")),
            "condition.where.column",
        ),
        (
            "a text operator on a number column",
            rule(|body| body["condition"]["where"]["op"] = json!("contains")),
            "condition.where.op",
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
                breached: true,
                witness: crate::domain::alerts::condition::Witness::one_value(
                    "total",
                    json!(12),
                    true,
                ),
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
    assert_eq!(
        shown["witness"],
        json!({ "matched": 1, "answered": 1, "columns": ["total"], "rows": [[12]] })
    );

    Ok(())
}

/// No rows of `engine String`, as `RowBinaryWithNamesAndTypes` writes them:
/// one column, its name, its type.
const NO_ENGINE: &[u8] = b"\x01\x06engine\x06String";

/// A warehouse that knows no table engine and answers every metric read
/// with two teams' counts.
async fn two_teams() -> (String, tokio::task::JoinHandle<()>) {
    use axum::Json;
    use axum::response::IntoResponse as _;
    use axum::routing::post;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("the warehouse must bind: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("the warehouse must have an address: {error}"));
    let warehouse = Router::new().route(
        "/",
        post(|sql: String| async move {
            if sql.contains("system.tables") {
                return NO_ENGINE.to_vec().into_response();
            }

            Json(json!({
                "meta": [{"name": "team", "type": "String"}, {"name": "total", "type": "UInt64"}],
                "data": [{"team": "core", "total": "12"}, {"team": "infra", "total": "3"}]
            }))
            .into_response()
        }),
    );
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, warehouse).await;
    });

    (format!("http://{address}"), server)
}

fn preview(test: &Value) -> Value {
    json!({ "metric": "prs-by-team", "condition": { "rows": "any", "where": test } })
}

#[tokio::test]
async fn a_preview_judges_the_condition_over_the_rows_a_check_would_read() -> R {
    let (url, server) = two_teams().await;
    let harness = TestHarness::over_warehouse(&url, true);

    let answered = harness
        .send(
            "POST",
            "/v1/alerts/preview",
            Some(preview(
                &json!({ "column": "total", "op": ">", "value": 10 }),
            )),
        )
        .await;
    server.abort();

    assert_eq!(answered.status, StatusCode::OK, "{:?}", answered.body);
    assert_eq!(
        answered.json(),
        json!({
            "outcome": "breach",
            "reason": null,
            "witness": { "matched": 1, "answered": 2, "columns": ["team", "total"], "rows": [["core", 12]] }
        })
    );
    let held = crate::domain::alerts::rule::AlertStore::enabled(harness.store.as_ref()).await?;
    assert!(held.is_empty(), "a preview records nothing");
    assert!(harness.schedule.entries().is_empty());

    Ok(())
}

#[tokio::test]
async fn a_preview_over_a_metric_with_many_rows_asking_for_one_is_unknown() -> R {
    let (url, server) = two_teams().await;
    let harness = TestHarness::over_warehouse(&url, true);

    let answered = harness
        .send(
            "POST",
            "/v1/alerts/preview",
            Some(json!({
                "metric": "prs-by-team",
                "condition": { "rows": "one", "where": { "column": "total", "op": ">", "value": 10 } }
            })),
        )
        .await;
    server.abort();

    assert_eq!(
        answered.json(),
        json!({ "outcome": "unknown", "reason": "many_rows", "witness": null })
    );

    Ok(())
}

#[tokio::test]
async fn a_preview_is_refused_on_the_part_of_the_condition_that_is_wrong() -> R {
    let harness = TestHarness::new();
    let cases = [
        (
            preview(&json!({ "column": "total", "op": "contains", "value": "1" })),
            "condition.where.op",
        ),
        (
            preview(&json!({ "all": [{ "column": "team", "op": "in", "value": [] }] })),
            "condition.where.all[0].value",
        ),
        (
            json!({ "metric": "nothing-here", "condition": { "rows": "any", "where": {} } }),
            "metric",
        ),
        (
            json!({ "metric": "prs-by-team", "condition": null }),
            "condition",
        ),
    ];

    for (body, field) in cases {
        let refused = harness
            .send("POST", "/v1/alerts/preview", Some(body.clone()))
            .await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "should refuse {body}"
        );
        let shown = refused.json();
        assert_eq!(
            shown["context"]["field_violations"][0]["field"],
            json!(field),
            "should name `{field}`: {shown}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn only_an_administrator_may_preview() -> R {
    let harness = TestHarness::build(false, true);

    let refused = harness
        .send(
            "POST",
            "/v1/alerts/preview",
            Some(preview(
                &json!({ "column": "total", "op": ">", "value": 10 }),
            )),
        )
        .await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn a_nested_condition_over_a_grouped_metric_is_stored_and_read_back() -> R {
    let harness = TestHarness::new();
    let condition = json!({
        "rows": { "count": { "op": ">=", "value": 2 } },
        "where": { "all": [
            { "column": "total", "op": "between", "value": [5, 50] },
            { "not": { "column": "team", "op": "in", "value": ["core", "infra"] } },
        ] }
    });

    let created = harness
        .send(
            "POST",
            "/v1/alerts",
            Some(rule(|body| {
                body["metric"] = json!("prs-by-team");
                body["condition"] = condition.clone();
            })),
        )
        .await;

    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    assert_eq!(created.json()["condition"], condition);

    Ok(())
}

#[tokio::test]
async fn a_preview_waiting_past_its_turn_is_refused_as_busy() -> R {
    let harness = TestHarness::new();
    let slots = Arc::clone(harness.state.preview_slots());
    let held = slots
        .acquire_many_owned(u32::try_from(MAX_CONCURRENT_PREVIEWS)?)
        .await?;

    let refused = harness
        .send(
            "POST",
            "/v1/alerts/preview",
            Some(preview(
                &json!({ "column": "total", "op": ">", "value": 10 }),
            )),
        )
        .await;
    drop(held);

    assert_eq!(
        refused.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{:?}",
        refused.body
    );

    Ok(())
}
