use std::error::Error;
use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use serde_json::{Value, json};

use super::*;
use crate::api::AppState;
use crate::chat::ChatClient;
use crate::domain::kinds::dashboard::{HeadingItem, TextItem, WidgetItem};
use crate::domain::query::metric_query::{MetricRunner, People};
use crate::store::definitions::memory::MemoryDefinitions;
use crate::store::identity::IdentityClient;

type R = Result<(), Box<dyn Error>>;

/// A stand whose one dataset stands ready, since every stored metric reads
/// one.
fn surfaces() -> CustomSurfaces {
    built(crate::api::Datasets::holding(
        "http://offline.invalid",
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
    ))
}

/// A stand where nobody has declared anything.
fn surfaces_without_a_dataset() -> CustomSurfaces {
    built(crate::api::Datasets::offline("http://offline.invalid"))
}

fn built(datasets: crate::api::Datasets) -> CustomSurfaces {
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

fn put(name: &str, body: Value) -> Parameters<PutRequest> {
    Parameters(PutRequest {
        name: name.to_owned(),
        body,
    })
}

fn listing(kind: DefinitionKind) -> Parameters<KindRequest> {
    Parameters(KindRequest {
        kind,
        limit: None,
        offset: None,
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

fn named(kind: DefinitionKind, name: &str) -> Parameters<NamedRequest> {
    Parameters(NamedRequest {
        kind,
        name: name.to_owned(),
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
fn the_server_announces_exactly_the_ten_custom_surface_tools() {
    let tools = CustomSurfaces::tool_router().list_all();

    let mut names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();

    assert_eq!(
        names,
        [
            "arrange_dashboard",
            "delete_definition",
            "get_definition",
            "list_datasets",
            "list_definitions",
            "put_dashboard",
            "put_metric",
            "put_widget",
            "run_metric",
            "search_definitions",
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

    assert_accepted(&surfaces.put_metric(put("per-actor", metric_body())).await);

    let listed = assert_accepted(
        &surfaces
            .list_definitions(listing(DefinitionKind::Metric))
            .await,
    );
    assert_eq!(listed["names"], json!(["per-actor"]));

    let read = assert_accepted(
        &surfaces
            .get_definition(named(DefinitionKind::Metric, "per-actor"))
            .await,
    );
    assert_eq!(read, metric_body());

    Ok(())
}

#[tokio::test]
async fn reading_a_definition_that_was_never_stored_says_so() {
    let result = surfaces()
        .get_definition(named(DefinitionKind::Dashboard, "absent"))
        .await;

    assert_refused(&result, "was not found");
}

#[tokio::test]
async fn a_name_the_store_would_not_accept_is_refused_before_any_read() {
    let result = surfaces()
        .get_definition(named(DefinitionKind::Metric, "not a valid name"))
        .await;

    assert_refused(&result, "definition names");
}

#[tokio::test]
async fn a_widget_drawing_a_column_its_metric_does_not_produce_is_refused() -> R {
    let surfaces = surfaces();
    surfaces.put_metric(put("per-actor", metric_body())).await;

    let result = surfaces
        .put_widget(put(
            "chart",
            json!({"type": "line", "metric": "per-actor", "x": "actor", "y": "lines"}),
        ))
        .await;

    assert_refused(&result, "lines");

    Ok(())
}

#[tokio::test]
async fn a_widget_naming_a_metric_that_is_not_stored_is_refused() {
    let result = surfaces()
        .put_widget(put(
            "chart",
            json!({"type": "table", "metric": "absent", "columns": []}),
        ))
        .await;

    assert_refused(&result, "no metric named");
}

#[tokio::test]
async fn a_metric_a_widget_still_draws_is_not_deleted() -> R {
    let surfaces = surfaces();
    surfaces.put_metric(put("per-actor", metric_body())).await;
    surfaces
        .put_widget(put(
            "chart",
            json!({"type": "line", "metric": "per-actor", "x": "actor", "y": "total"}),
        ))
        .await;

    let result = surfaces
        .delete_definition(named(DefinitionKind::Metric, "per-actor"))
        .await;

    assert_refused(&result, "chart");

    Ok(())
}

#[tokio::test]
async fn a_dashboard_is_stored_and_then_removed() -> R {
    let surfaces = surfaces();
    assert_accepted(
        &surfaces
            .put_dashboard(put(
                "board",
                json!({"title": "Example board", "widgets": []}),
            ))
            .await,
    );

    assert_accepted(
        &surfaces
            .delete_definition(named(DefinitionKind::Dashboard, "board"))
            .await,
    );

    let listed = assert_accepted(
        &surfaces
            .list_definitions(listing(DefinitionKind::Dashboard))
            .await,
    );
    assert_eq!(listed["names"], json!([]));

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
            name: "absent".to_owned(),
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
            name: "absent".to_owned(),
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

    assert_refused(&result, "must name the `table` it reads");
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
    assert_eq!(by_body["names"], json!(["lines_per_day"]));

    let by_name = assert_accepted(
        &surfaces
            .search_definitions(finding(DefinitionKind::Metric, "actors"))
            .await,
    );
    assert_eq!(by_name["names"], json!(["actors"]));
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

    assert_eq!(all["names"], json!(["actors"]));
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
            }))
            .await,
    );
    assert_eq!(first["names"], json!(["a_one", "b_two"]));
    assert_eq!(first["total"], json!(3));

    let second = assert_accepted(
        &surfaces
            .list_definitions(Parameters(KindRequest {
                kind: DefinitionKind::Metric,
                limit: Some(2),
                offset: Some(2),
            }))
            .await,
    );
    assert_eq!(second["names"], json!(["c_three"]));
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

    assert_eq!(page["names"], json!(["git_two"]));
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
            }))
            .await,
        "limit must be between 1 and 200",
    );
}

#[tokio::test]
async fn arranging_a_board_orders_its_widgets_and_captions_them() {
    let surfaces = surfaces();
    assert_accepted(&surfaces.put_metric(put("per-actor", metric_body())).await);
    for widget in ["first", "second"] {
        assert_accepted(
            &surfaces
                .put_widget(put(
                    widget,
                    json!({"type": "table", "metric": "per-actor", "columns": ["actor"]}),
                ))
                .await,
        );
    }
    assert_accepted(
        &surfaces
            .put_dashboard(put(
                "board",
                json!({"title": "Example board", "widgets": ["first", "second"]}),
            ))
            .await,
    );

    let arranged = assert_accepted(
        &surfaces
            .arrange_dashboard(Parameters(ArrangeRequest {
                name: "board".to_owned(),
                items: vec![
                    Item::Heading(HeadingItem {
                        heading: "Per person".to_owned(),
                    }),
                    Item::Widget(WidgetItem {
                        widget: "second".to_owned(),
                    }),
                    Item::Text(TextItem {
                        text: "Bots excluded.".to_owned(),
                    }),
                    Item::Widget(WidgetItem {
                        widget: "first".to_owned(),
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
                { "widget": "second" },
                { "text": "Bots excluded." },
                { "widget": "first" }
            ]
        })
    );
}

#[tokio::test]
async fn arranging_a_board_around_a_widget_that_is_not_there_is_refused() {
    let surfaces = surfaces();
    assert_accepted(
        &surfaces
            .put_dashboard(put(
                "board",
                json!({"title": "Example board", "widgets": []}),
            ))
            .await,
    );

    assert_refused(
        &surfaces
            .arrange_dashboard(Parameters(ArrangeRequest {
                name: "board".to_owned(),
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
                name: "absent".to_owned(),
                items: Vec::new(),
            }))
            .await,
        "dashboard `absent` was not found",
    );
}

#[tokio::test]
async fn a_board_draws_a_metric_over_a_warehouse_table() -> R {
    let surfaces = surfaces_without_a_dataset();
    assert_accepted(
        &surfaces
            .put_metric(put(
                "sessions_by_day",
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

    assert_accepted(
        &surfaces
            .put_widget(put(
                "sessions_line",
                json!({"type": "line", "metric": "sessions_by_day", "x": "bucket", "y": "sessions"}),
            ))
            .await,
    );
    assert_accepted(
        &surfaces
            .put_dashboard(put(
                "ai_usage",
                json!({
                    "title": "AI usage",
                    "items": [{"widget": "sessions_line"}],
                    "time_ranges": ["P7D", "P30D"],
                    "default_range": "P30D"
                }),
            ))
            .await,
    );

    Ok(())
}
