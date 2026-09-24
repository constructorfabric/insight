use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode};
use clickhouse::test::{Mock, handlers};
use serde::Serialize;
use serde_json::json;
use toolkit::api::OpenApiRegistryImpl;
use tower::ServiceExt as _;

use super::*;
use crate::api::AppState;
use crate::chat::ChatClient;
use crate::domain::definition::Definitions as _;
use crate::domain::query::metric_query::MetricRunner;
use crate::store::definitions::memory::MemoryDefinitions;

const BODY_LIMIT_BYTES: usize = 64 * 1024;

/// What `system.tables` answers about a name nothing holds.
#[derive(Debug, Serialize, clickhouse::Row)]
struct NoTable {
    sorting_key: String,
}

/// How many records this harness's preview shows.
const PREVIEW_ROWS: u64 = 3;

struct TestHarness {
    mock: Mock,
    router: Router,
    definitions: Arc<MemoryDefinitions>,
}

impl TestHarness {
    fn new() -> Self {
        Self::with_caller(true)
    }

    /// A caller who does or does not hold the admin role.
    fn with_caller(is_admin: bool) -> Self {
        let mut mock = Mock::new();
        mock.non_exhaustive();
        let openapi = OpenApiRegistryImpl::new();
        let url = mock.url();
        let datasets = crate::api::Datasets::new(
            Arc::new(crate::store::datasets::memory::MemoryDatasets::at(
                chrono::Utc::now(),
            )),
            crate::store::dataset_tables::DatasetTables::new(insight_clickhouse::Client::new(
                insight_clickhouse::Config::new(url, "insight_datasets"),
            )),
            "insight_datasets".to_owned(),
            PREVIEW_ROWS,
        );
        let definitions = Arc::new(MemoryDefinitions::new());
        let state = Arc::new(AppState::new(
            MetricRunner::new(
                insight_clickhouse::Client::new(insight_clickhouse::Config::new(url, "insight")),
                crate::domain::query::metric_query::People::new("identity"),
            ),
            definitions.clone(),
            ChatClient::keyless(),
            crate::store::identity::IdentityClient::fixed(is_admin),
            datasets,
            crate::store::catalog::Catalog::fixed(Vec::new()),
        ));
        let router = register_routes(Router::new(), &openapi, &state);

        Self {
            mock,
            router,
            definitions,
        }
    }

    /// The datasets database answers that nothing holds the name, and takes
    /// whatever statement follows.
    fn a_free_name(&self) {
        self.mock.add(handlers::provide(Vec::<NoTable>::new()));
        self.mock.add(handlers::record_ddl());
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, Bytes) {
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .unwrap_or_else(|error| panic!("router must respond: {error}"));
        let status = response.status();
        let body = to_bytes(response.into_body(), BODY_LIMIT_BYTES)
            .await
            .unwrap_or_else(|error| panic!("the body must be readable: {error}"));

        (status, body)
    }

    async fn put(&self, name: &str, body: serde_json::Value) -> (StatusCode, Bytes) {
        let request = Request::builder()
            .method("PUT")
            .uri(format!("/v1/datasets/{name}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap_or_else(
                |error| panic!("test JSON must serialize: {error}"),
            )))
            .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

        self.send(request).await
    }

    async fn get(&self, path: &str) -> (StatusCode, Bytes) {
        let request = Request::builder()
            .method("GET")
            .uri(path)
            .body(Body::empty())
            .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

        self.send(request).await
    }

    async fn delete(&self, name: &str) -> (StatusCode, Bytes) {
        let request = Request::builder()
            .method("DELETE")
            .uri(format!("/v1/datasets/{name}"))
            .body(Body::empty())
            .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

        self.send(request).await
    }
}

fn declaration() -> serde_json::Value {
    json!({
        "title": "Commits",
        "fields": [
            { "name": "day", "path": "day", "type": "datetime", "default_clock": true },
            { "name": "lines", "path": "lines", "type": "int" }
        ]
    })
}

fn read(body: &Bytes) -> serde_json::Value {
    serde_json::from_slice(body).unwrap_or_else(|error| panic!("the body is JSON: {error}"))
}

#[tokio::test]
async fn a_declared_dataset_is_answered_back_and_can_be_read_again() {
    let harness = TestHarness::new();
    harness.a_free_name();

    let (declared, body) = harness.put("commits", declaration()).await;

    assert_eq!(declared, StatusCode::OK);
    assert_eq!(read(&body)["declaration"], declaration());

    let (found, body) = harness.get("/v1/datasets/commits").await;
    assert_eq!(found, StatusCode::OK);
    assert_eq!(read(&body)["name"], "commits");
    assert_eq!(read(&body)["declaration"], declaration());
}

#[tokio::test]
async fn a_declaration_wrong_in_several_places_is_answered_with_all_of_them() {
    let harness = TestHarness::new();

    let (refused, body) = harness
        .put(
            "commits",
            json!({
                "title": "Commits",
                "fields": [
                    { "name": "day", "path": "day", "type": "datetime" },
                    { "name": "day", "path": "other", "type": "int" }
                ],
                "row_identity": ["nowhere"]
            }),
        )
        .await;

    assert_eq!(refused, StatusCode::BAD_REQUEST);
    let said = String::from_utf8_lossy(&body);
    assert!(said.contains("fields[1].name"), "{said}");
    assert!(said.contains("row_identity[0]"), "{said}");
}

#[tokio::test]
async fn a_dataset_nobody_declared_is_not_found() {
    let harness = TestHarness::new();

    let (answered, _) = harness.get("/v1/datasets/commits").await;

    assert_eq!(answered, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn declaring_a_dataset_needs_the_admin_role() {
    let harness = TestHarness::with_caller(false);

    let (refused, _) = harness.put("commits", declaration()).await;
    let (removal, _) = harness.delete("commits").await;

    assert_eq!(refused, StatusCode::FORBIDDEN);
    assert_eq!(removal, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn every_dataset_is_listed_by_name_with_how_many_there_are() {
    let harness = TestHarness::new();
    harness.a_free_name();
    harness.put("commits", declaration()).await;

    let (listed, body) = harness.get("/v1/datasets").await;

    assert_eq!(listed, StatusCode::OK);
    assert_eq!(read(&body)["names"], json!(["commits"]));
    assert_eq!(read(&body)["total"], json!(1));
}

/// "Which dataset holds this field" is the question a catalogue is asked,
/// and a name cannot answer it.
#[tokio::test]
async fn a_search_matches_a_name_and_a_declaration() {
    let harness = TestHarness::new();
    harness.a_free_name();
    harness.put("commits", declaration()).await;

    let (_, by_field) = harness.get("/v1/datasets?q=lines").await;
    assert_eq!(read(&by_field)["names"], json!(["commits"]));

    let (_, by_name) = harness.get("/v1/datasets?q=comm").await;
    assert_eq!(read(&by_name)["names"], json!(["commits"]));

    let (_, unlike) = harness.get("/v1/datasets?q=nothing_like_this").await;
    assert_eq!(read(&unlike)["names"], json!([]));
    assert_eq!(read(&unlike)["total"], json!(0));
}

#[tokio::test]
async fn a_page_answers_its_own_slice_and_the_whole_count() {
    let harness = TestHarness::new();
    for named in ["a_one", "b_two", "c_three"] {
        harness.a_free_name();
        harness.put(named, declaration()).await;
    }

    let (_, body) = harness.get("/v1/datasets?limit=2&offset=1").await;

    assert_eq!(read(&body)["names"], json!(["b_two", "c_three"]));
    assert_eq!(read(&body)["total"], json!(3));
}

/// The Custom zone is not shown to a reader without the role, and the API
/// says the same rather than trusting the rail to hide it.
#[tokio::test]
async fn reading_the_catalogue_needs_the_admin_role() {
    let harness = TestHarness::with_caller(false);

    let (listed, _) = harness.get("/v1/datasets").await;
    let (read_one, _) = harness.get("/v1/datasets/commits").await;

    assert_eq!(listed, StatusCode::FORBIDDEN);
    assert_eq!(read_one, StatusCode::FORBIDDEN);
}

/// A dataset mid-removal is gone from every surface at once, rather than
/// lingering on the one that forgot to ask.
#[tokio::test]
async fn a_dataset_still_being_made_is_shown_by_nothing() {
    let harness = TestHarness::new();
    // The name is free, and then the table cannot be made: the row stays
    // claimed, holding the name, with nothing published under it. The attempt
    // takes back whatever it may have made before giving up.
    harness.mock.add(handlers::provide(Vec::<NoTable>::new()));
    harness.mock.add(handlers::failure(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
    ));
    harness.mock.add(handlers::record_ddl());
    let (declared, _) = harness.put("commits", declaration()).await;

    let (listed, listing) = harness.get("/v1/datasets").await;
    let (read_one, _) = harness.get("/v1/datasets/commits").await;
    let (records, _) = harness.get("/v1/datasets/commits/records").await;

    assert_eq!(declared, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(listed, StatusCode::OK);
    assert_eq!(read(&listing)["names"], json!([]));
    assert_eq!(read_one, StatusCode::NOT_FOUND);
    assert_eq!(records, StatusCode::NOT_FOUND);
}

/// A removal that could not take the table away leaves the dataset where it
/// was: unreachable, its name unclaimable, and the request repeatable once
/// the lease lapses.
#[tokio::test]
async fn a_removal_that_cannot_drop_the_table_leaves_the_dataset_mid_removal() {
    let harness = TestHarness::new();
    harness.a_free_name();
    harness.put("commits", declaration()).await;
    harness.mock.add(handlers::failure(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
    ));

    let (removed, _) = harness.delete("commits").await;

    assert_eq!(removed, StatusCode::INTERNAL_SERVER_ERROR);
    let (listed, listing) = harness.get("/v1/datasets").await;
    let (read_one, _) = harness.get("/v1/datasets/commits").await;
    let (retaken, _) = harness.put("commits", declaration()).await;

    assert_eq!(listed, StatusCode::OK);
    assert_eq!(read(&listing)["names"], json!([]), "nothing lists it");
    assert_eq!(read_one, StatusCode::NOT_FOUND, "nothing reads it");
    assert_eq!(
        retaken,
        StatusCode::CONFLICT,
        "and nothing may take the name"
    );
}

/// Replacing a dataset that stands changes its declaration where it lies; it
/// is never taken for a create, which would leave its records behind.
#[tokio::test]
async fn declaring_a_name_that_stands_replaces_it_without_touching_its_table() {
    let harness = TestHarness::new();
    harness.a_free_name();
    harness.put("commits", declaration()).await;

    let (replaced, body) = harness
        .put(
            "commits",
            json!({
                "title": "Commits",
                "fields": [
                    { "name": "day", "path": "day", "type": "datetime", "default_clock": true },
                    { "name": "lines", "path": "lines", "type": "int" },
                    { "name": "repo", "path": "repo", "type": "string" }
                ]
            }),
        )
        .await;

    // No shape read and no DDL were queued for this one: a replacement that
    // reached the warehouse would have found the mock empty and failed.
    assert_eq!(replaced, StatusCode::OK);
    assert_eq!(read(&body)["declaration"]["fields"][2]["name"], "repo");
}

#[tokio::test]
async fn a_removed_dataset_leaves_the_catalogue() {
    let harness = TestHarness::new();
    harness.a_free_name();
    harness.put("commits", declaration()).await;
    harness.mock.add(handlers::provide(vec![NoTable {
        sorting_key: "table_name, received_at, id".to_owned(),
    }]));
    harness.mock.add(handlers::record_ddl());

    let (removed, _) = harness.delete("commits").await;

    assert_eq!(removed, StatusCode::NO_CONTENT);
    let (listed, body) = harness.get("/v1/datasets").await;
    assert_eq!(listed, StatusCode::OK);
    assert_eq!(read(&body)["names"], json!([]));
}

#[tokio::test]
async fn a_name_outside_the_charset_is_refused_before_anything_is_read() {
    let harness = TestHarness::new();

    let (refused, _) = harness.put("drop%20table", declaration()).await;

    assert_eq!(refused, StatusCode::BAD_REQUEST);
}

/// A record as the dataset's table holds it, for the preview to read back.
#[derive(Debug, Serialize, clickhouse::Row)]
struct StoredRecord {
    #[serde(with = "clickhouse::serde::uuid")]
    id: uuid::Uuid,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    received_at: chrono::DateTime<chrono::Utc>,
    raw_data: String,
}

/// What the table answers when asked how many records it holds.
#[derive(Debug, Serialize, clickhouse::Row)]
struct Counted {
    total: u64,
}

#[tokio::test]
async fn a_dataset_page_shows_the_latest_records_as_they_arrived() {
    let harness = TestHarness::new();
    harness.a_free_name();
    harness.put("commits", declaration()).await;
    harness.mock.add(handlers::provide(vec![StoredRecord {
        id: uuid::Uuid::nil(),
        received_at: chrono::DateTime::UNIX_EPOCH,
        raw_data: r#"{"lines":7}"#.to_owned(),
    }]));
    harness
        .mock
        .add(handlers::provide(vec![Counted { total: 757 }]));

    let (looked, body) = harness
        .get(&format!(
            "/v1/datasets/commits/records?limit={PREVIEW_ROWS}"
        ))
        .await;

    assert_eq!(looked, StatusCode::OK);
    assert_eq!(read(&body)["records"][0]["raw_data"], json!({"lines": 7}));
    // The look is a slice; the count says how much lies behind it.
    assert_eq!(read(&body)["total"], json!(757));
    // And how wide the slice was, so a reader stepping by offset does not
    // have to assume the limit it asked for was the one applied.
    assert_eq!(read(&body)["limit"], json!(PREVIEW_ROWS));
}

/// A page is a window on what arrived: how many, from where, in what order.
#[tokio::test]
async fn a_page_is_ordered_by_a_declared_field_when_one_is_named() {
    let harness = TestHarness::new();
    harness.a_free_name();
    harness.put("commits", declaration()).await;
    harness
        .mock
        .add(handlers::provide(Vec::<StoredRecord>::new()));
    harness
        .mock
        .add(handlers::provide(vec![Counted { total: 0 }]));

    let (looked, _) = harness
        .get("/v1/datasets/commits/records?order_by=day&direction=asc&offset=40")
        .await;

    assert_eq!(looked, StatusCode::OK);
}

/// A page cut down to the cap without saying so would let a reader paging by
/// the size it asked for step over the records the smaller page left behind.
/// A page of none is no page at all, and is refused against the same field.
#[tokio::test]
async fn a_page_of_a_size_this_installation_does_not_serve_is_refused() {
    for asked in [0, PREVIEW_ROWS + 1] {
        let harness = TestHarness::new();
        harness.a_free_name();
        harness.put("commits", declaration()).await;

        let (looked, body) = harness
            .get(&format!("/v1/datasets/commits/records?limit={asked}"))
            .await;

        assert_eq!(looked, StatusCode::BAD_REQUEST, "asked {asked}");
        let violation = &read(&body)["context"]["field_violations"][0];
        assert_eq!(violation["field"], json!("limit"));
        assert!(
            violation["description"]
                .as_str()
                .is_some_and(|said| said.contains(&PREVIEW_ROWS.to_string())),
            "should name the range: {violation}"
        );
    }
}

/// The reader names a field; the declaration says how it is read. A name it
/// does not declare is refused against the parameter that carried it.
#[tokio::test]
async fn a_page_ordered_by_a_field_the_dataset_does_not_declare_is_refused() {
    let harness = TestHarness::new();
    harness.a_free_name();
    harness.put("commits", declaration()).await;

    let (looked, body) = harness
        .get("/v1/datasets/commits/records?order_by=nonsense")
        .await;

    assert_eq!(looked, StatusCode::BAD_REQUEST);
    let violation = &read(&body)["context"]["field_violations"][0];
    assert_eq!(violation["field"], "order_by");
    assert!(
        violation["description"]
            .as_str()
            .unwrap_or_default()
            .contains("`nonsense` is not a field of this dataset"),
        "{violation}"
    );
}

/// The table is not asked about at all: there is none to ask.
#[tokio::test]
async fn the_records_of_a_dataset_nobody_declared_are_not_found() {
    let harness = TestHarness::new();

    let (looked, _) = harness.get("/v1/datasets/commits/records").await;

    assert_eq!(looked, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reading_records_needs_the_admin_role() {
    let harness = TestHarness::with_caller(false);

    let (looked, _) = harness.get("/v1/datasets/commits/records").await;

    assert_eq!(looked, StatusCode::FORBIDDEN);
}

/// A removal takes the metrics' source with it, so a reader is shown what
/// reads the dataset before they ask for one.
#[tokio::test]
async fn a_dataset_names_the_metrics_that_read_it() {
    let harness = TestHarness::new();
    harness.a_free_name();
    harness.put("commits", declaration()).await;
    harness
        .definitions
        .put(
            crate::domain::definition::DefinitionKind::Metric,
            &crate::domain::definition::DefinitionName::parse("lines_per_day")
                .unwrap_or_else(|error| panic!("the name parses: {error}")),
            &json!({
                "dataset": "commits",
                "fields": [{"field": "lines", "type": "int", "agg": "sum", "as_name": "total"}]
            }),
        )
        .await
        .unwrap_or_else(|error| panic!("the metric is stored: {error}"));

    let (asked, body) = harness.get("/v1/datasets/commits/dependents").await;

    assert_eq!(asked, StatusCode::OK);
    assert_eq!(read(&body)["metrics"], json!(["lines_per_day"]));
}

/// A metric over another dataset is not a dependent, however it reads.
#[tokio::test]
async fn a_dataset_nothing_reads_names_nothing() {
    let harness = TestHarness::new();
    harness.a_free_name();
    harness.put("commits", declaration()).await;

    let (asked, body) = harness.get("/v1/datasets/commits/dependents").await;

    assert_eq!(asked, StatusCode::OK);
    assert_eq!(read(&body)["metrics"], json!([]));
}

/// A removal a metric would break, and a replacement that would break one, are
/// conflicts rather than malformed requests: the body is fine, the stand is
/// not, and the portal branches on the status.
#[tokio::test]
async fn a_dataset_something_still_reads_answers_a_conflict() {
    let harness = TestHarness::new();
    harness.a_free_name();
    harness.put("commits", declaration()).await;
    harness
        .definitions
        .put(
            crate::domain::definition::DefinitionKind::Metric,
            &crate::domain::definition::DefinitionName::parse("lines_per_day")
                .unwrap_or_else(|error| panic!("the name parses: {error}")),
            &json!({
                "dataset": "commits",
                "fields": [{"field": "lines", "type": "int", "agg": "sum", "as_name": "total"}]
            }),
        )
        .await
        .unwrap_or_else(|error| panic!("the metric is stored: {error}"));

    let (removed, said) = harness.delete("commits").await;
    let (replaced, would) = harness
        .put(
            "commits",
            json!({
                "title": "Commits",
                "fields": [
                    { "name": "day", "path": "day", "type": "datetime", "default_clock": true }
                ]
            }),
        )
        .await;

    assert_eq!(removed, StatusCode::CONFLICT);
    assert!(
        format!("{:?}", read(&said)).contains("lines_per_day"),
        "the refusal names what reads it: {:?}",
        read(&said)
    );
    assert_eq!(replaced, StatusCode::CONFLICT);
    assert!(
        format!("{:?}", read(&would)).contains("lines_per_day"),
        "the refusal names what would break: {:?}",
        read(&would)
    );
}

/// Every place a declaration is wrong is answered at once, against the field
/// that carries it, so a form marks them all in one pass.
#[tokio::test]
async fn a_declaration_wrong_in_several_ways_names_every_place() {
    let harness = TestHarness::new();

    let (refused, body) = harness
        .put(
            "commits",
            json!({
                "title": "Commits",
                "fields": [
                    { "name": "day", "path": "day", "type": "moment" },
                    { "name": "lines", "path": "lines", "type": "int", "colour": "red" }
                ]
            }),
        )
        .await;

    assert_eq!(refused, StatusCode::BAD_REQUEST);
    let said = format!("{:?}", read(&body));
    assert!(said.contains("fields[0].type"), "{said}");
    assert!(said.contains("fields[1].colour"), "{said}");
}

/// A caller who may not write learns nothing from the body they sent: the
/// refusal is theirs before the body is even read.
#[tokio::test]
async fn a_body_that_cannot_be_read_is_answered_only_after_the_role_is() {
    let refused = TestHarness::with_caller(false);
    let request = Request::builder()
        .method("PUT")
        .uri("/v1/datasets/commits")
        .header("content-type", "application/json")
        .body(Body::from("{"))
        .unwrap_or_else(|error| panic!("test request must be valid: {error}"));

    let (status, _) = refused.send(request).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// Every other read of a dataset asks for the ready state; this one did not.
#[tokio::test]
async fn the_dependents_of_a_dataset_nobody_declared_are_not_found() {
    let harness = TestHarness::new();

    let (asked, _) = harness.get("/v1/datasets/commits/dependents").await;

    assert_eq!(asked, StatusCode::NOT_FOUND);
}
