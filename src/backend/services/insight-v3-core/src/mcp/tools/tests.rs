use std::error::Error;
use std::sync::Arc;

use axum::http::request::Parts;
use clickhouse::test::{Mock, handlers};
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use serde::Serialize;
use serde_json::{Value, json};

use super::*;
use crate::api::AppState;
use crate::chat::ChatClient;
use crate::domain::kinds::dashboard::{HeadingItem, TextItem, WidgetItem};
use crate::domain::query::metric_query::{MetricRunner, People};
use crate::store::catalog::{Catalog, Column, Layer, TableSchema};
use crate::store::definitions::memory::MemoryDefinitions;
use crate::store::identity::IdentityClient;

type R = Result<(), Box<dyn Error>>;

/// A stand whose one dataset stands ready, since every stored metric reads
/// one.
fn surfaces() -> CustomSurfaces {
    surfaces_over("http://offline.invalid")
}

fn surfaces_over(url: &str) -> CustomSurfaces {
    built_over(
        Catalog::fixed(Vec::new()),
        crate::api::Datasets::holding(
            url,
            &[(
                "commits",
                json!({
                    "title": "Commits",
                    "fields": [
                        { "name": "actor", "path": "actor", "type": "string" },
                        { "name": "lines_added", "path": "lines_added", "type": "int" },
                        { "name": "occurred_at", "path": "occurred_at", "type": "datetime" }
                    ]
                }),
            )],
        ),
    )
}

/// A stand where nobody has declared anything.
fn surfaces_without_a_dataset() -> CustomSurfaces {
    built_over(
        Catalog::fixed(Vec::new()),
        crate::api::Datasets::offline("http://offline.invalid"),
    )
}

/// A stand whose warehouse holds two tables and no dataset.
fn surfaces_over_a_warehouse() -> CustomSurfaces {
    built_over(
        Catalog::fixed(vec![
            table(
                "bronze_github",
                "issues",
                Layer::Bronze,
                &[("number", "Int64"), ("title", "String")],
            ),
            table("silver", "fct_commit", Layer::Silver, &[("sha", "String")]),
        ]),
        crate::api::Datasets::offline("http://offline.invalid"),
    )
}

fn table(database: &str, table: &str, layer: Layer, columns: &[(&str, &str)]) -> TableSchema {
    TableSchema {
        database: database.to_owned(),
        table: table.to_owned(),
        layer,
        engine: "ReplacingMergeTree".to_owned(),
        columns: columns
            .iter()
            .map(|(name, kind)| Column {
                name: (*name).to_owned(),
                kind: (*kind).to_owned(),
            })
            .collect(),
    }
}

fn built_over(catalog: Catalog, datasets: crate::api::Datasets) -> CustomSurfaces {
    let client = || {
        insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            "http://clickhouse.invalid",
            "insight",
        ))
    };

    let Ok(identity) = IdentityClient::new("http://identity.invalid") else {
        panic!("a plain http base URL builds an identity client");
    };

    let state = Arc::new(AppState::new(
        MetricRunner::new(client(), People::new("identity")),
        Arc::new(MemoryDefinitions::new()),
        ChatClient::keyless(),
        identity,
        datasets,
        catalog,
    ));

    CustomSurfaces::new(state)
}

fn metric_body() -> Value {
    json!({
        "dataset": "commits",
        "fields": [
            {"field": "actor", "type": "string", "as_name": "actor"},
            {"field": "actor", "type": "string", "agg": "count", "as_name": "total"}
        ],
        "group_by": ["actor"]
    })
}

fn put(name: &str, body: Value) -> Parameters<WriteRequest> {
    Parameters(WriteRequest {
        id: None,
        name: name.to_owned(),
        body,
    })
}

fn replacing(id: &str, name: &str, body: Value) -> Parameters<WriteRequest> {
    Parameters(WriteRequest {
        id: Some(id.to_owned()),
        name: name.to_owned(),
        body,
    })
}

fn declaring(name: &str, body: Value) -> Parameters<PutRequest> {
    Parameters(PutRequest {
        name: name.to_owned(),
        body,
    })
}

/// The id an accepted write answers.
fn created(result: &CallToolResult) -> String {
    let written = assert_accepted(result);

    written["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a write answers its id: {written}"))
        .to_owned()
}

/// The names a listing answers, in its order.
fn names(listed: &Value) -> Vec<String> {
    listed["items"]
        .as_array()
        .unwrap_or_else(|| panic!("a listing answers items: {listed}"))
        .iter()
        .map(|item| item["name"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct NoTable {
    sorting_key: String,
}

fn parts(caller: Option<&str>) -> Extension<Parts> {
    let Ok(request) = axum::http::Request::builder().body(()) else {
        panic!("an empty request builds");
    };
    let (mut parts, ()) = request.into_parts();
    if let Some(caller) = caller {
        parts
            .extensions
            .insert(super::super::auth::McpCaller(caller.to_owned()));
    }

    Extension(parts)
}

fn administrator() -> Extension<Parts> {
    parts(Some("test-admin"))
}

fn reviews() -> Value {
    json!({
        "title": "Reviews",
        "fields": [
            { "name": "day", "path": "day", "type": "datetime", "default_clock": true },
            { "name": "reviewer", "path": "reviewer", "type": "string" }
        ]
    })
}

fn listing(kind: DefinitionKind) -> Parameters<KindRequest> {
    Parameters(KindRequest {
        kind,
        limit: None,
        offset: None,
        folder: None,
        tags: None,
    })
}

fn finding(kind: DefinitionKind, query: &str) -> Parameters<SearchRequest> {
    Parameters(SearchRequest {
        kind,
        query: query.to_owned(),
        limit: None,
        offset: None,
    })
}

fn named(kind: DefinitionKind, id: &str) -> Parameters<IdRequest> {
    Parameters(IdRequest {
        kind,
        id: id.to_owned(),
    })
}

fn error_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn assert_refused(result: &CallToolResult, expected: &str) {
    assert_eq!(result.is_error, Some(true), "should be refused: {result:?}");
    let text = error_text(result);
    assert!(
        text.contains(expected),
        "should mention {expected:?}: {text}"
    );
}

fn assert_accepted(result: &CallToolResult) -> Value {
    assert_ne!(
        result.is_error,
        Some(true),
        "should be accepted: {result:?}"
    );
    let Some(value) = result.structured_content.clone() else {
        panic!("an accepted call carries structured content: {result:?}");
    };

    value
}

#[test]
fn the_server_announces_exactly_the_eighteen_custom_surface_tools() {
    let tools = CustomSurfaces::tool_router().list_all();

    let mut names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();

    assert_eq!(
        names,
        [
            "arrange_dashboard",
            "create_folder",
            "delete_definition",
            "describe_tables",
            "get_definition",
            "list_datasets",
            "list_definitions",
            "list_folders",
            "list_tables",
            "list_tags",
            "move_dashboard",
            "put_dashboard",
            "put_dataset",
            "put_metric",
            "put_widget",
            "run_metric",
            "search_definitions",
            "set_dashboard_tags",
        ]
    );
}

#[test]
fn every_tool_describes_itself_so_a_client_knows_when_to_reach_for_it() {
    for tool in CustomSurfaces::tool_router().list_all() {
        let Some(description) = tool.description.as_ref() else {
            panic!("tool {} has no description", tool.name);
        };
        assert!(
            description.len() > 30,
            "tool {} is described too thinly: {description}",
            tool.name
        );
    }
}

#[test]
fn the_instructions_point_a_client_at_the_discovery_tool_first() {
    let info = rmcp::ServerHandler::get_info(&surfaces());

    let Some(instructions) = info.instructions else {
        panic!("the server carries instructions");
    };
    assert!(instructions.contains("list_datasets"), "{instructions}");
}

#[tokio::test]
async fn a_stored_metric_is_listed_and_read_back() -> R {
    let surfaces = surfaces();

    let id = created(&surfaces.put_metric(put("Per actor", metric_body())).await);

    let listed = assert_accepted(
        &surfaces
            .list_definitions(listing(DefinitionKind::Metric))
            .await,
    );
    assert_eq!(listed["items"], json!([{ "id": id, "name": "Per actor" }]));

    let read = assert_accepted(
        &surfaces
            .get_definition(named(DefinitionKind::Metric, &id))
            .await,
    );
    assert_eq!(
        read,
        json!({ "id": id, "name": "Per actor", "body": metric_body() })
    );

    Ok(())
}

#[tokio::test]
async fn a_put_without_an_id_creates_and_one_with_an_id_replaces() -> R {
    let surfaces = surfaces();
    let first = created(&surfaces.put_metric(put("Per actor", metric_body())).await);
    let second = created(&surfaces.put_metric(put("Per actor", metric_body())).await);
    let narrower = json!({
        "dataset": "commits",
        "fields": [{"field": "actor", "type": "string", "as_name": "actor"}]
    });

    let replaced = assert_accepted(
        &surfaces
            .put_metric(replacing(&first, "Actors", narrower.clone()))
            .await,
    );

    assert_ne!(
        first, second,
        "a second create of one name is a second metric"
    );
    assert_eq!(
        replaced,
        json!({ "id": first, "name": "Actors", "created": false })
    );
    let read = assert_accepted(
        &surfaces
            .get_definition(named(DefinitionKind::Metric, &first))
            .await,
    );
    assert_eq!(read["body"], narrower);
    let listed = assert_accepted(
        &surfaces
            .list_definitions(listing(DefinitionKind::Metric))
            .await,
    );
    assert_eq!(listed["total"], json!(2));

    Ok(())
}

#[tokio::test]
async fn a_put_replacing_an_id_nobody_holds_is_refused_and_stores_nothing() {
    let surfaces = surfaces();

    let refused = surfaces
        .put_metric(replacing("nowhere", "Per actor", metric_body()))
        .await;

    assert_refused(&refused, "metric `nowhere` was not found");
    let listed = assert_accepted(
        &surfaces
            .list_definitions(listing(DefinitionKind::Metric))
            .await,
    );
    assert_eq!(listed["total"], json!(0));
}

#[tokio::test]
async fn a_put_whose_name_is_blank_is_refused() {
    let refused = surfaces().put_metric(put("   ", metric_body())).await;

    assert_refused(&refused, "a name is 1 to 128 characters");
}

#[tokio::test]
async fn reading_a_definition_that_was_never_stored_says_so() {
    let result = surfaces()
        .get_definition(named(DefinitionKind::Dashboard, "absent"))
        .await;

    assert_refused(&result, "was not found");
}

#[tokio::test]
async fn an_id_that_cannot_be_one_names_nothing() {
    let result = surfaces()
        .get_definition(named(DefinitionKind::Metric, "not a valid id"))
        .await;

    assert_refused(&result, "metric `not a valid id` was not found");
}

#[tokio::test]
async fn a_widget_drawing_a_column_its_metric_does_not_produce_is_refused() -> R {
    let surfaces = surfaces();
    let metric = created(&surfaces.put_metric(put("Per actor", metric_body())).await);

    let result = surfaces
        .put_widget(put(
            "Chart",
            json!({"type": "line", "metric": metric, "x": "actor", "y": "lines"}),
        ))
        .await;

    assert_refused(&result, "lines");

    Ok(())
}

#[tokio::test]
async fn a_widget_naming_a_metric_that_is_not_stored_is_refused() {
    let result = surfaces()
        .put_widget(put(
            "Chart",
            json!({"type": "table", "metric": "absent", "columns": []}),
        ))
        .await;

    assert_refused(&result, "no metric named");
}

#[tokio::test]
async fn a_metric_a_widget_still_draws_is_not_deleted() -> R {
    let surfaces = surfaces();
    let metric = created(&surfaces.put_metric(put("Per actor", metric_body())).await);
    created(
        &surfaces
            .put_widget(put(
                "Actor chart",
                json!({"type": "line", "metric": metric, "x": "actor", "y": "total"}),
            ))
            .await,
    );

    let result = surfaces
        .delete_definition(named(DefinitionKind::Metric, &metric))
        .await;

    assert_refused(&result, "Actor chart");

    Ok(())
}

#[tokio::test]
async fn a_dashboard_is_stored_and_then_removed() -> R {
    let surfaces = surfaces();
    let board = created(
        &surfaces
            .put_dashboard(put(
                "Board",
                json!({"title": "Example board", "widgets": []}),
            ))
            .await,
    );

    assert_accepted(
        &surfaces
            .delete_definition(named(DefinitionKind::Dashboard, &board))
            .await,
    );

    let listed = assert_accepted(
        &surfaces
            .list_definitions(listing(DefinitionKind::Dashboard))
            .await,
    );
    assert_eq!(listed["items"], json!([]));

    Ok(())
}

#[tokio::test]
async fn deleting_a_definition_that_was_never_stored_says_so() {
    let result = surfaces()
        .delete_definition(named(DefinitionKind::Widget, "absent"))
        .await;

    assert_refused(&result, "was not found");
}

#[tokio::test]
async fn running_a_metric_that_was_never_stored_says_so() {
    let result = surfaces()
        .run_metric(Parameters(RunRequest {
            id: "absent".to_owned(),
            range: None,
            bucket: None,
        }))
        .await;

    assert_refused(&result, "was not found");
}

#[tokio::test]
async fn a_range_the_server_does_not_know_is_refused_by_the_tool() {
    let result = surfaces()
        .run_metric(Parameters(RunRequest {
            id: "absent".to_owned(),
            range: Some("P14D".to_owned()),
            bucket: None,
        }))
        .await;

    assert_refused(&result, "P14D");
}

#[tokio::test]
async fn a_metric_over_any_warehouse_table_is_stored() {
    let result = surfaces_without_a_dataset()
        .put_metric(put(
            "sessions_per_tool",
            json!({
                "table": "silver.class_ai_assistant_usage",
                "time": {"column": "day"},
                "fields": [
                    {"column": "tool", "type": "string", "as_name": "tool"},
                    {
                        "column": "surface_metrics_json",
                        "json": "session_count",
                        "type": "int",
                        "agg": "sum",
                        "as_name": "sessions"
                    }
                ],
                "group_by": ["tool"]
            }),
        ))
        .await;

    assert_accepted(&result);
}

#[tokio::test]
async fn a_metric_naming_no_table_is_not_stored() {
    let result = surfaces()
        .put_metric(put(
            "over_nothing",
            json!({
                "fields": [{"agg": "count", "type": "int", "as_name": "total"}]
            }),
        ))
        .await;

    assert_refused(&result, "must name the `dataset` or the `table` it reads");
}

#[tokio::test]
async fn a_metric_whose_maximum_range_is_not_a_duration_is_not_stored() {
    let result = surfaces()
        .put_metric(put(
            "bad_cap",
            json!({
                "dataset": "commits",
                "time": {"field": "occurred_at"},
                "max_range": "P0D",
                "fields": [{"agg": "count", "type": "int", "as_name": "total"}]
            }),
        ))
        .await;

    assert_refused(&result, "P0D");
}

#[tokio::test]
async fn a_metric_that_declares_a_clock_and_a_cap_is_stored() -> R {
    let surfaces = surfaces();

    assert_accepted(
        &surfaces
            .put_metric(put(
                "opened",
                json!({
                    "dataset": "commits",
                    "time": {"field": "occurred_at"},
                    "max_range": "P1Y",
                    "fields": [{"agg": "count", "type": "int", "as_name": "total"}]
                }),
            ))
            .await,
    );

    Ok(())
}

#[tokio::test]
async fn with_nothing_declared_the_listing_says_who_declares_a_dataset() {
    let result = surfaces_without_a_dataset().list_datasets().await;

    assert_eq!(result.is_error, Some(false), "{result:?}");
    let said = format!("{result:?}");
    assert!(said.contains("administrator"), "{said}");
}

#[tokio::test]
async fn search_definitions_matches_a_name_and_a_body() {
    // "Which metrics read this table" is the question a catalogue is asked,
    // and a name cannot answer it.
    let surfaces = surfaces();
    assert_accepted(
        &surfaces
            .put_metric(put(
                "lines_per_day",
                json!({
                    "dataset": "commits",
                    "fields": [{"field": "lines_added", "type": "int", "as_name": "lines"}]
                }),
            ))
            .await,
    );
    assert_accepted(&surfaces.put_metric(put("actors", metric_body())).await);

    let by_body = assert_accepted(
        &surfaces
            .search_definitions(finding(DefinitionKind::Metric, "lines_added"))
            .await,
    );
    assert_eq!(names(&by_body), ["lines_per_day"]);

    let by_name = assert_accepted(
        &surfaces
            .search_definitions(finding(DefinitionKind::Metric, "actors"))
            .await,
    );
    assert_eq!(names(&by_name), ["actors"]);
}

#[tokio::test]
async fn an_empty_search_returns_everything_of_that_kind() {
    let surfaces = surfaces();
    assert_accepted(&surfaces.put_metric(put("actors", metric_body())).await);

    let all = assert_accepted(
        &surfaces
            .search_definitions(finding(DefinitionKind::Metric, "   "))
            .await,
    );

    assert_eq!(names(&all), ["actors"]);
}

#[tokio::test]
async fn a_page_answers_its_own_slice_and_the_whole_count() {
    let surfaces = surfaces();
    for name in ["a_one", "b_two", "c_three"] {
        assert_accepted(&surfaces.put_metric(put(name, metric_body())).await);
    }

    let first = assert_accepted(
        &surfaces
            .list_definitions(Parameters(KindRequest {
                kind: DefinitionKind::Metric,
                limit: Some(2),
                offset: None,
                folder: None,
                tags: None,
            }))
            .await,
    );
    assert_eq!(names(&first), ["a_one", "b_two"]);
    assert_eq!(first["total"], json!(3));

    let second = assert_accepted(
        &surfaces
            .list_definitions(Parameters(KindRequest {
                kind: DefinitionKind::Metric,
                limit: Some(2),
                offset: Some(2),
                folder: None,
                tags: None,
            }))
            .await,
    );
    assert_eq!(names(&second), ["c_three"]);
    assert_eq!(second["total"], json!(3));
}

#[tokio::test]
async fn a_search_pages_the_matches_and_counts_all_of_them() {
    let surfaces = surfaces();
    for name in ["git_one", "git_two", "wiki_one"] {
        assert_accepted(&surfaces.put_metric(put(name, metric_body())).await);
    }

    let page = assert_accepted(
        &surfaces
            .search_definitions(Parameters(SearchRequest {
                kind: DefinitionKind::Metric,
                query: "git_".to_owned(),
                limit: Some(1),
                offset: Some(1),
            }))
            .await,
    );

    assert_eq!(names(&page), ["git_two"]);
    assert_eq!(page["total"], json!(2));
}

#[tokio::test]
async fn a_page_beyond_the_cap_is_refused_rather_than_served() {
    let surfaces = surfaces();

    assert_refused(
        &surfaces
            .list_definitions(Parameters(KindRequest {
                kind: DefinitionKind::Metric,
                limit: Some(5_000),
                offset: None,
                folder: None,
                tags: None,
            }))
            .await,
        "limit must be between 1 and 200",
    );
}

#[tokio::test]
async fn arranging_a_board_orders_its_widgets_and_captions_them() {
    let surfaces = surfaces();
    let metric = created(&surfaces.put_metric(put("Per actor", metric_body())).await);
    let mut widgets = Vec::new();
    for widget in ["First", "Second"] {
        widgets.push(created(
            &surfaces
                .put_widget(put(
                    widget,
                    json!({"type": "table", "metric": metric, "columns": ["actor"]}),
                ))
                .await,
        ));
    }
    let board = created(
        &surfaces
            .put_dashboard(put(
                "Board",
                json!({"title": "Example board", "widgets": widgets}),
            ))
            .await,
    );

    let arranged = assert_accepted(
        &surfaces
            .arrange_dashboard(Parameters(ArrangeRequest {
                id: board.clone(),
                items: vec![
                    Item::Heading(HeadingItem {
                        heading: "Per person".to_owned(),
                    }),
                    Item::Widget(WidgetItem {
                        widget: widgets[1].clone(),
                    }),
                    Item::Text(TextItem {
                        text: "Bots excluded.".to_owned(),
                    }),
                    Item::Widget(WidgetItem {
                        widget: widgets[0].clone(),
                    }),
                ],
            }))
            .await,
    );

    // The title survives, the shorthand does not, and the order is the one
    // that was asked for.
    assert_eq!(
        arranged,
        json!({
            "title": "Example board",
            "items": [
                { "heading": "Per person" },
                { "widget": widgets[1] },
                { "text": "Bots excluded." },
                { "widget": widgets[0] }
            ]
        })
    );
    let read = assert_accepted(
        &surfaces
            .get_definition(named(DefinitionKind::Dashboard, &board))
            .await,
    );
    assert_eq!(read["name"], "Board", "laying a board out keeps its name");
}

#[tokio::test]
async fn arranging_a_board_around_a_widget_that_is_not_there_is_refused() {
    let surfaces = surfaces();
    let board = created(
        &surfaces
            .put_dashboard(put(
                "Board",
                json!({"title": "Example board", "widgets": []}),
            ))
            .await,
    );

    assert_refused(
        &surfaces
            .arrange_dashboard(Parameters(ArrangeRequest {
                id: board,
                items: vec![Item::Widget(WidgetItem {
                    widget: "absent".to_owned(),
                })],
            }))
            .await,
        "widget `absent` was not found",
    );
}

#[tokio::test]
async fn arranging_a_board_that_is_not_there_is_refused() {
    let surfaces = surfaces();

    assert_refused(
        &surfaces
            .arrange_dashboard(Parameters(ArrangeRequest {
                id: "absent".to_owned(),
                items: Vec::new(),
            }))
            .await,
        "dashboard `absent` was not found",
    );
}

#[tokio::test]
async fn a_declared_dataset_is_listed_for_a_metric_to_read() {
    let mock = Mock::new();
    mock.add(handlers::provide(Vec::<NoTable>::new()));
    mock.add(handlers::record_ddl());
    let surfaces = surfaces_over(mock.url());

    let stored = assert_accepted(
        &surfaces
            .put_dataset(administrator(), declaring("reviews", reviews()))
            .await,
    );
    assert_eq!(stored["declaration"], reviews());

    let listed = assert_accepted(&surfaces.list_datasets().await);
    let Some(datasets) = listed["datasets"].as_str() else {
        panic!("the listing describes the datasets: {listed}");
    };
    assert!(datasets.contains("reviews: Reviews"), "{datasets}");
}

#[tokio::test]
async fn a_declaration_wrong_in_several_places_names_every_one() {
    let surfaces = surfaces();

    let refused = surfaces
        .put_dataset(
            administrator(),
            declaring(
                "reviews",
                json!({
                    "title": "Reviews",
                    "fields": [
                        { "name": "day", "path": "day", "type": "datetime" },
                        { "name": "day", "path": "other", "type": "int" }
                    ],
                    "row_identity": ["nowhere"]
                }),
            ),
        )
        .await;

    assert_refused(&refused, "fields[1].name");
    assert_refused(&refused, "row_identity[0]");
}

#[tokio::test]
async fn a_replacement_that_would_break_a_metric_names_the_metric() {
    let surfaces = surfaces();
    let metric = created(&surfaces.put_metric(put("Per actor", metric_body())).await);

    assert_refused(
        &surfaces
            .put_dataset(
                administrator(),
                declaring(
                    "commits",
                    json!({
                        "title": "Commits",
                        "fields": [
                            { "name": "occurred_at", "path": "occurred_at", "type": "datetime" }
                        ]
                    }),
                ),
            )
            .await,
        &metric,
    );
}

#[tokio::test]
async fn a_board_draws_a_metric_over_a_warehouse_table() -> R {
    let surfaces = surfaces_without_a_dataset();
    let metric = created(
        &surfaces
            .put_metric(put(
                "Sessions by day",
                json!({
                    "table": "silver.class_ai_assistant_usage",
                    "time": {"column": "day"},
                    "fields": [{
                        "column": "surface_metrics_json",
                        "json": "session_count",
                        "type": "int",
                        "agg": "sum",
                        "as_name": "sessions"
                    }]
                }),
            ))
            .await,
    );

    let widget = created(
        &surfaces
            .put_widget(put(
                "Sessions line",
                json!({"type": "line", "metric": metric, "x": "bucket", "y": "sessions"}),
            ))
            .await,
    );
    assert_accepted(
        &surfaces
            .put_dashboard(put(
                "AI usage",
                json!({
                    "title": "AI usage",
                    "items": [{"widget": widget}],
                    "time_ranges": ["P7D", "P30D"],
                    "default_range": "P30D"
                }),
            ))
            .await,
    );

    Ok(())
}

#[tokio::test]
async fn every_warehouse_table_is_listed_by_database_name_and_layer() {
    let result = surfaces_over_a_warehouse()
        .list_tables(Parameters(TablesRequest { database: None }))
        .await;

    let listed = assert_accepted(&result);
    assert_eq!(
        listed,
        json!({
            "total": 2,
            "tables": [
                {"database": "bronze_github", "table": "issues", "layer": "bronze"},
                {"database": "silver", "table": "fct_commit", "layer": "silver"}
            ]
        })
    );
}

#[tokio::test]
async fn a_declaration_no_verified_caller_stands_behind_is_refused() {
    let surfaces = surfaces();

    assert_refused(
        &surfaces
            .put_dataset(parts(None), declaring("reviews", reviews()))
            .await,
        "administrator",
    );
}

#[tokio::test]
async fn a_table_listing_can_be_narrowed_to_one_database() {
    let result = surfaces_over_a_warehouse()
        .list_tables(Parameters(TablesRequest {
            database: Some("silver".to_owned()),
        }))
        .await;

    let listed = assert_accepted(&result);
    assert_eq!(listed["total"], json!(1));
    assert_eq!(listed["tables"][0]["table"], json!("fct_commit"));
}

#[tokio::test]
async fn a_described_table_carries_its_columns_and_engine_and_an_unknown_name_is_said_so() {
    let result = surfaces_over_a_warehouse()
        .describe_tables(Parameters(DescribeTablesRequest {
            tables: vec!["silver.fct_commit".to_owned(), "silver.nothing".to_owned()],
        }))
        .await;

    let described = assert_accepted(&result);
    assert_eq!(
        described,
        json!({
            "tables": [{
                "database": "silver",
                "table": "fct_commit",
                "layer": "silver",
                "engine": "ReplacingMergeTree",
                "columns": [{"name": "sha", "type": "String"}]
            }],
            "unknown": ["silver.nothing"]
        })
    );
}

#[tokio::test]
async fn a_description_of_nothing_or_of_too_much_is_refused() {
    let surfaces = surfaces_over_a_warehouse();

    let none = surfaces
        .describe_tables(Parameters(DescribeTablesRequest { tables: Vec::new() }))
        .await;
    let many = surfaces
        .describe_tables(Parameters(DescribeTablesRequest {
            tables: (0..=DESCRIBE_LIMIT).map(|n| format!("db.t{n}")).collect(),
        }))
        .await;

    assert_refused(&none, "at least one table");
    assert_refused(&many, "at most 20 tables");
}

fn listing_in(kind: DefinitionKind, folder: &str) -> Parameters<KindRequest> {
    Parameters(KindRequest {
        kind,
        limit: None,
        offset: None,
        folder: Some(folder.to_owned()),
        tags: None,
    })
}

fn folder_named(name: &str) -> Parameters<FolderRequest> {
    Parameters(FolderRequest {
        name: name.to_owned(),
    })
}

fn moving(dashboard: &str, folder: Option<&str>) -> Parameters<MoveRequest> {
    Parameters(MoveRequest {
        dashboard: dashboard.to_owned(),
        folder: folder.map(str::to_owned),
    })
}

/// A dashboard stored under an id the case chooses, so the folder and tag
/// cases can name it.
async fn board(surfaces: &CustomSurfaces, id: &str) {
    let parsed = DefinitionId::parse(id).unwrap_or_else(|error| panic!("{id}: {error}"));

    surfaces
        .state
        .definitions()
        .create(
            DefinitionKind::Dashboard,
            &parsed,
            &DefinitionLabel::of_id(&parsed),
            &json!({"title": id, "widgets": []}),
        )
        .await
        .unwrap_or_else(|error| panic!("`{id}` must store: {error}"));
}

/// The dashboard's body written again through the tool, under its id.
async fn rewrite(surfaces: &CustomSurfaces, id: &str) {
    assert_accepted(
        &surfaces
            .put_dashboard(replacing(id, id, json!({"title": id, "widgets": []})))
            .await,
    );
}

async fn made_folder(surfaces: &CustomSurfaces, name: &str) -> String {
    let made = assert_accepted(&surfaces.create_folder(folder_named(name)).await);

    made["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a made folder names its id: {made}"))
        .to_owned()
}

async fn folder_of(surfaces: &CustomSurfaces, dashboard: &str) -> Value {
    let read = assert_accepted(
        &surfaces
            .get_definition(named(DefinitionKind::Dashboard, dashboard))
            .await,
    );

    read["folder"].clone()
}

#[test]
fn the_instructions_name_the_folder_tools() {
    let info = rmcp::ServerHandler::get_info(&surfaces());

    let Some(instructions) = info.instructions else {
        panic!("the server carries instructions");
    };
    for tool in ["list_folders", "create_folder", "move_dashboard"] {
        assert!(instructions.contains(tool), "{tool}: {instructions}");
    }
}

#[tokio::test]
async fn a_made_folder_is_listed_with_what_it_holds() {
    let surfaces = surfaces();
    board(&surfaces, "delivery").await;

    let id = made_folder(&surfaces, "Platform").await;
    let listed = assert_accepted(&surfaces.list_folders().await);

    assert_eq!(
        listed,
        json!({
            "folders": [{ "id": id, "name": "Platform", "dashboards": 0 }],
            "unfiled": 1
        })
    );
}

#[tokio::test]
async fn a_folder_name_another_folder_holds_in_another_case_is_refused_readably() {
    let surfaces = surfaces();
    made_folder(&surfaces, "Platform").await;

    let clash = surfaces.create_folder(folder_named("platform")).await;

    assert_refused(&clash, "already exists");
}

#[tokio::test]
async fn a_folder_name_of_nothing_is_refused() {
    let clash = surfaces().create_folder(folder_named("  ")).await;

    assert_refused(&clash, "1 to 64 characters");
}

#[tokio::test]
async fn a_dashboard_is_moved_into_a_folder_and_out_again() {
    let surfaces = surfaces();
    let id = made_folder(&surfaces, "Platform").await;
    board(&surfaces, "delivery").await;

    assert_accepted(&surfaces.move_dashboard(moving("delivery", Some(&id))).await);
    let filed = folder_of(&surfaces, "delivery").await;
    assert_accepted(&surfaces.move_dashboard(moving("delivery", None)).await);

    assert_eq!(filed, json!({ "id": id, "name": "Platform" }));
    assert_eq!(folder_of(&surfaces, "delivery").await, Value::Null);
}

#[tokio::test]
async fn moving_into_a_folder_that_is_not_there_is_refused() {
    let surfaces = surfaces();
    board(&surfaces, "delivery").await;

    let moved = surfaces
        .move_dashboard(moving(
            "delivery",
            Some("0192a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b"),
        ))
        .await;

    assert_refused(&moved, "no folder has the id");
}

#[tokio::test]
async fn moving_a_dashboard_that_is_not_there_is_refused() {
    let surfaces = surfaces();
    let id = made_folder(&surfaces, "Platform").await;

    let moved = surfaces.move_dashboard(moving("nowhere", Some(&id))).await;

    assert_refused(&moved, "no dashboard has the id `nowhere`");
}

#[tokio::test]
async fn rewriting_a_filed_dashboard_keeps_its_folder_and_a_new_one_lands_unfiled() {
    let surfaces = surfaces();
    let id = made_folder(&surfaces, "Platform").await;
    board(&surfaces, "delivery").await;
    assert_accepted(&surfaces.move_dashboard(moving("delivery", Some(&id))).await);

    rewrite(&surfaces, "delivery").await;
    board(&surfaces, "hiring").await;

    assert_eq!(folder_of(&surfaces, "delivery").await["id"], json!(id));
    assert_eq!(folder_of(&surfaces, "hiring").await, Value::Null);
}

#[tokio::test]
async fn a_dashboard_listing_narrows_to_one_folder_or_to_the_unfiled() {
    let surfaces = surfaces();
    let id = made_folder(&surfaces, "Platform").await;
    board(&surfaces, "delivery").await;
    board(&surfaces, "hiring").await;
    assert_accepted(&surfaces.move_dashboard(moving("delivery", Some(&id))).await);

    let filed = assert_accepted(
        &surfaces
            .list_definitions(listing_in(DefinitionKind::Dashboard, &id))
            .await,
    );
    let unfiled = assert_accepted(
        &surfaces
            .list_definitions(listing_in(DefinitionKind::Dashboard, "unfiled"))
            .await,
    );

    assert_eq!(names(&filed), ["delivery"]);
    assert_eq!(names(&unfiled), ["hiring"]);
}

#[tokio::test]
async fn only_dashboards_are_listed_by_folder() {
    let listed = surfaces()
        .list_definitions(listing_in(DefinitionKind::Metric, "unfiled"))
        .await;

    assert_refused(&listed, "are not filed in folders");
}

#[tokio::test]
async fn a_listing_in_a_folder_that_is_not_an_id_is_refused() {
    let listed = surfaces()
        .list_definitions(listing_in(DefinitionKind::Dashboard, "Platform"))
        .await;

    assert_refused(&listed, "folder ids are UUIDs");
}

#[test]
fn a_move_names_its_folder_even_when_the_folder_is_none() {
    let tools = CustomSurfaces::tool_router().list_all();
    let Some(tool) = tools.iter().find(|tool| tool.name == "move_dashboard") else {
        panic!("move_dashboard is announced");
    };

    let schema = Value::Object((*tool.input_schema).clone());

    assert_eq!(
        schema["required"],
        json!(["dashboard", "folder"]),
        "{schema}"
    );
    assert_eq!(
        schema["properties"]["folder"]["type"],
        json!(["string", "null"])
    );
    assert!(schema["properties"]["folder"]["description"].is_string());
}

#[test]
fn a_move_without_its_folder_is_not_read() {
    let read = serde_json::from_value::<MoveRequest>(json!({ "dashboard": "delivery" }));

    assert!(read.is_err(), "{read:?}");
}

fn tagging(id: &str, tags: &[&str]) -> Parameters<TagsRequest> {
    Parameters(TagsRequest {
        id: id.to_owned(),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
    })
}

fn listing_tagged(
    kind: DefinitionKind,
    folder: Option<&str>,
    tags: &[&str],
) -> Parameters<KindRequest> {
    Parameters(KindRequest {
        kind,
        limit: None,
        offset: None,
        folder: folder.map(str::to_owned),
        tags: Some(tags.iter().map(|tag| (*tag).to_owned()).collect()),
    })
}

async fn tags_of(surfaces: &CustomSurfaces, dashboard: &str) -> Value {
    let read = assert_accepted(
        &surfaces
            .get_definition(named(DefinitionKind::Dashboard, dashboard))
            .await,
    );

    read["tags"].clone()
}

#[test]
fn the_instructions_name_the_tag_tools() {
    let info = rmcp::ServerHandler::get_info(&surfaces());

    let Some(instructions) = info.instructions else {
        panic!("the server carries instructions");
    };
    for tool in ["list_tags", "set_dashboard_tags"] {
        assert!(instructions.contains(tool), "{tool}: {instructions}");
    }
}

#[tokio::test]
async fn a_dashboard_s_tags_are_set_and_listed_with_their_counts() {
    let surfaces = surfaces();
    board(&surfaces, "delivery").await;
    board(&surfaces, "support").await;

    let set = assert_accepted(
        &surfaces
            .set_dashboard_tags(tagging("delivery", &["Platform", "Delivery"]))
            .await,
    );
    assert_accepted(
        &surfaces
            .set_dashboard_tags(tagging("support", &["platform"]))
            .await,
    );
    let listed = assert_accepted(&surfaces.list_tags().await);

    assert_eq!(
        set,
        json!({ "dashboard": "delivery", "tags": ["Delivery", "Platform"] })
    );
    assert_eq!(
        listed,
        json!({
            "tags": [
                { "name": "Delivery", "dashboards": 1 },
                { "name": "Platform", "dashboards": 2 }
            ]
        })
    );
}

#[tokio::test]
async fn a_set_answers_the_stored_spelling_of_a_tag_that_exists() {
    let surfaces = surfaces();
    board(&surfaces, "delivery").await;
    board(&surfaces, "support").await;
    assert_accepted(
        &surfaces
            .set_dashboard_tags(tagging("delivery", &["Delivery"]))
            .await,
    );

    let set = assert_accepted(
        &surfaces
            .set_dashboard_tags(tagging("support", &["DELIVERY"]))
            .await,
    );

    assert_eq!(set["tags"], json!(["Delivery"]));
}

#[tokio::test]
async fn a_dashboard_read_answers_its_body_folder_and_tags() {
    let surfaces = surfaces();
    board(&surfaces, "delivery").await;
    assert_accepted(
        &surfaces
            .set_dashboard_tags(tagging("delivery", &["beta", "Alpha"]))
            .await,
    );

    let read = assert_accepted(
        &surfaces
            .get_definition(named(DefinitionKind::Dashboard, "delivery"))
            .await,
    );

    assert_eq!(
        read,
        json!({
            "id": "delivery",
            "name": "delivery",
            "body": { "title": "delivery", "widgets": [] },
            "folder": null,
            "tags": ["Alpha", "beta"]
        })
    );
}

#[tokio::test]
async fn rewriting_a_tagged_dashboard_keeps_its_tags() {
    let surfaces = surfaces();
    board(&surfaces, "delivery").await;
    assert_accepted(
        &surfaces
            .set_dashboard_tags(tagging("delivery", &["Delivery"]))
            .await,
    );

    rewrite(&surfaces, "delivery").await;

    assert_eq!(tags_of(&surfaces, "delivery").await, json!(["Delivery"]));
}

#[tokio::test]
async fn an_empty_set_clears_and_the_tag_goes() {
    let surfaces = surfaces();
    board(&surfaces, "delivery").await;
    assert_accepted(
        &surfaces
            .set_dashboard_tags(tagging("delivery", &["Delivery"]))
            .await,
    );

    assert_accepted(&surfaces.set_dashboard_tags(tagging("delivery", &[])).await);

    assert_eq!(tags_of(&surfaces, "delivery").await, json!([]));
    assert_eq!(
        assert_accepted(&surfaces.list_tags().await),
        json!({ "tags": [] })
    );
}

#[tokio::test]
async fn a_set_the_rules_refuse_says_why() {
    let surfaces = surfaces();
    board(&surfaces, "delivery").await;
    let eleven: Vec<String> = (0..11).map(|index| format!("t{index}")).collect();
    let eleven: Vec<&str> = eleven.iter().map(String::as_str).collect();

    let blank = surfaces
        .set_dashboard_tags(tagging("delivery", &["  "]))
        .await;
    let crowded = surfaces
        .set_dashboard_tags(tagging("delivery", &eleven))
        .await;
    let nowhere = surfaces
        .set_dashboard_tags(tagging("nowhere", &["Delivery"]))
        .await;

    assert_refused(&blank, "1 to 32 characters");
    assert_refused(&crowded, "at most 10 tags");
    assert_refused(&nowhere, "no dashboard has the id `nowhere`");
}

#[tokio::test]
async fn a_set_past_the_cap_is_refused_and_says_why() {
    let surfaces = surfaces();
    for index in 0..crate::domain::tags::MAX_TAGS / 10 {
        let name = format!("b{index}");
        board(&surfaces, &name).await;
        let full: Vec<String> = (0..10).map(|tag| format!("t{index}_{tag}")).collect();
        let full: Vec<&str> = full.iter().map(String::as_str).collect();
        assert_accepted(&surfaces.set_dashboard_tags(tagging(&name, &full)).await);
    }
    board(&surfaces, "last").await;

    let refused = surfaces
        .set_dashboard_tags(tagging("last", &["one more"]))
        .await;

    assert_refused(&refused, "200 tags");
}

#[tokio::test]
async fn a_dashboard_listing_narrows_to_any_of_the_tags_inside_a_folder() {
    let surfaces = surfaces();
    let id = made_folder(&surfaces, "Platform").await;
    for (name, tags) in [
        ("delivery", &["Alpha"][..]),
        ("delivery_ai", &["Alpha", "Beta"][..]),
        ("hiring", &["Beta"][..]),
        ("support", &["Gamma"][..]),
    ] {
        board(&surfaces, name).await;
        assert_accepted(&surfaces.set_dashboard_tags(tagging(name, tags)).await);
    }
    assert_accepted(&surfaces.move_dashboard(moving("delivery", Some(&id))).await);

    let everywhere = assert_accepted(
        &surfaces
            .list_definitions(listing_tagged(
                DefinitionKind::Dashboard,
                None,
                &["alpha", "BETA"],
            ))
            .await,
    );
    let filed = assert_accepted(
        &surfaces
            .list_definitions(listing_tagged(
                DefinitionKind::Dashboard,
                Some(&id),
                &["alpha", "beta"],
            ))
            .await,
    );

    assert_eq!(names(&everywhere), ["delivery", "delivery_ai", "hiring"]);
    assert_eq!(everywhere["total"], 3);
    assert_eq!(names(&filed), ["delivery"]);
}

#[tokio::test]
async fn only_dashboards_are_listed_by_tag() {
    let listed = surfaces()
        .list_definitions(listing_tagged(DefinitionKind::Metric, None, &["Alpha"]))
        .await;

    assert_refused(&listed, "metrics are not tagged");
}

#[test]
fn a_set_names_its_dashboard_and_its_tags() {
    let tools = CustomSurfaces::tool_router().list_all();
    let Some(tool) = tools.iter().find(|tool| tool.name == "set_dashboard_tags") else {
        panic!("set_dashboard_tags is announced");
    };

    let schema = Value::Object((*tool.input_schema).clone());

    assert_eq!(schema["required"], json!(["id", "tags"]), "{schema}");
    assert!(schema["properties"]["tags"]["description"].is_string());
}
