use std::error::Error;

use serde_json::json;

use super::*;
use crate::catalog::Catalog;
use crate::definitions::memory::MemoryDefinitions;
use crate::metric_query::{MetricRunner, People};

type R = Result<(), Box<dyn Error>>;

struct Fixture {
    definitions: MemoryDefinitions,
    metrics: MetricRunner,
    catalog: Catalog,
}

impl Fixture {
    fn new() -> Self {
        let client = || {
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                "http://clickhouse.invalid",
                "insight",
            ))
        };

        Self {
            definitions: MemoryDefinitions::new(),
            metrics: MetricRunner::new(client(), People::new("identity")),
            catalog: Catalog::new(client(), "insight".to_owned()),
        }
    }

    fn surfaces(&self) -> Surfaces<'_> {
        Surfaces::new(&self.definitions, &self.metrics, &self.catalog)
    }
}

fn legacy() -> crate::time_window::WindowRequest {
    crate::time_window::WindowRequest::parse(None, None)
        .unwrap_or_else(|error| panic!("an empty request parses: {error}"))
}

fn ranged(token: &str) -> crate::time_window::WindowRequest {
    crate::time_window::WindowRequest::parse(Some(token), None)
        .unwrap_or_else(|error| panic!("`{token}` parses: {error}"))
}

fn name(value: &str) -> DefinitionName {
    let Ok(parsed) = DefinitionName::parse(value) else {
        panic!("should be a valid definition name: {value}");
    };

    parsed
}

fn metric_body() -> serde_json::Value {
    json!({
        "table": "events",
        "fields": [
            {"json": "actor", "type": "string", "as_name": "actor"},
            {"json": "actor", "type": "string", "agg": "count", "as_name": "total"}
        ],
        "group_by": ["actor"]
    })
}

fn line_widget(metric: &str, y: &str) -> serde_json::Value {
    json!({"type": "line", "metric": metric, "x": "actor", "y": y})
}

#[tokio::test]
async fn reading_a_definition_that_was_never_stored_reports_it_missing() {
    let fixture = Fixture::new();

    let Err(error) = fixture
        .surfaces()
        .get(DefinitionKind::Metric, &name("absent"))
        .await
    else {
        panic!("an unstored metric has no body to read");
    };

    assert!(matches!(error, CustomError::NotFound { .. }), "{error:?}");
    assert!(error.to_string().contains("metric `absent`"), "{error}");
}

#[tokio::test]
async fn a_widget_naming_a_column_its_metric_does_not_produce_is_refused() -> R {
    let fixture = Fixture::new();
    let surfaces = fixture.surfaces();
    surfaces
        .put(DefinitionKind::Metric, &name("per-actor"), &metric_body())
        .await?;

    let Err(error) = surfaces
        .put(
            DefinitionKind::Widget,
            &name("chart"),
            &line_widget("per-actor", "lines"),
        )
        .await
    else {
        panic!("the metric produces `total`, not `lines`");
    };

    assert!(matches!(error, CustomError::Widget(_)), "{error:?}");
    assert!(error.to_string().contains("lines"), "{error}");

    Ok(())
}

#[tokio::test]
async fn a_widget_naming_a_metric_that_is_not_stored_is_refused() {
    let fixture = Fixture::new();

    let Err(error) = fixture
        .surfaces()
        .put(
            DefinitionKind::Widget,
            &name("chart"),
            &json!({"type": "table", "metric": "absent", "columns": []}),
        )
        .await
    else {
        panic!("a widget cannot draw a metric that is not there");
    };

    assert!(matches!(error, CustomError::Widget(_)), "{error:?}");
}

#[tokio::test]
async fn a_metric_a_widget_still_draws_is_kept_and_its_dependents_named() -> R {
    let fixture = Fixture::new();
    let surfaces = fixture.surfaces();
    surfaces
        .put(DefinitionKind::Metric, &name("per-actor"), &metric_body())
        .await?;
    surfaces
        .put(
            DefinitionKind::Widget,
            &name("chart"),
            &line_widget("per-actor", "total"),
        )
        .await?;

    let Err(error) = surfaces
        .delete(DefinitionKind::Metric, &name("per-actor"))
        .await
    else {
        panic!("a metric a widget draws is kept");
    };

    match error {
        CustomError::InUse { used_by } => assert_eq!(used_by, vec!["chart".to_owned()]),
        other => panic!("should refuse as in use: {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn a_widget_a_dashboard_still_holds_is_kept_and_its_dependents_named() -> R {
    let fixture = Fixture::new();
    let surfaces = fixture.surfaces();
    surfaces
        .put(DefinitionKind::Metric, &name("per-actor"), &metric_body())
        .await?;
    surfaces
        .put(
            DefinitionKind::Widget,
            &name("chart"),
            &line_widget("per-actor", "total"),
        )
        .await?;
    surfaces
        .put(
            DefinitionKind::Dashboard,
            &name("board"),
            &json!({"title": "Example board", "widgets": ["chart"]}),
        )
        .await?;

    let Err(error) = surfaces
        .delete(DefinitionKind::Widget, &name("chart"))
        .await
    else {
        panic!("a widget a dashboard holds is kept");
    };

    match error {
        CustomError::InUse { used_by } => assert_eq!(used_by, vec!["board".to_owned()]),
        other => panic!("should refuse as in use: {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn a_dashboard_holds_widgets_so_nothing_reports_it_as_a_dependent() -> R {
    let fixture = Fixture::new();
    let surfaces = fixture.surfaces();
    surfaces
        .put(
            DefinitionKind::Dashboard,
            &name("board"),
            &json!({"title": "Example board", "widgets": []}),
        )
        .await?;

    surfaces
        .delete(DefinitionKind::Dashboard, &name("board"))
        .await?;

    assert!(surfaces.list(DefinitionKind::Dashboard).await?.is_empty());

    Ok(())
}

#[tokio::test]
async fn deleting_a_definition_that_was_never_stored_reports_it_missing() {
    let fixture = Fixture::new();

    let Err(error) = fixture
        .surfaces()
        .delete(DefinitionKind::Metric, &name("absent"))
        .await
    else {
        panic!("there is nothing to remove");
    };

    assert!(matches!(error, CustomError::NotFound { .. }), "{error:?}");
}

#[tokio::test]
async fn running_a_metric_whose_body_is_not_a_query_reports_the_body() -> R {
    let fixture = Fixture::new();
    let surfaces = fixture.surfaces();
    surfaces
        .put(
            DefinitionKind::Metric,
            &name("broken"),
            &json!({"table": "events"}),
        )
        .await?;

    let Err(error) = surfaces.run_metric(&name("broken"), &legacy()).await else {
        panic!("a query with no fields does not deserialize");
    };

    assert!(matches!(error, CustomError::Body(_)), "{error:?}");

    Ok(())
}

#[tokio::test]
async fn running_a_metric_that_was_never_stored_reports_it_missing() {
    let fixture = Fixture::new();

    let Err(error) = fixture
        .surfaces()
        .run_metric(&name("absent"), &legacy())
        .await
    else {
        panic!("there is no such metric to run");
    };

    assert!(matches!(error, CustomError::NotFound { .. }), "{error:?}");
}

#[tokio::test]
async fn listing_names_them_in_the_order_the_store_gives() -> R {
    let fixture = Fixture::new();
    let surfaces = fixture.surfaces();
    surfaces
        .put(DefinitionKind::Metric, &name("alpha"), &metric_body())
        .await?;
    surfaces
        .put(DefinitionKind::Metric, &name("beta"), &metric_body())
        .await?;

    assert_eq!(
        surfaces.list(DefinitionKind::Metric).await?,
        vec!["alpha".to_owned(), "beta".to_owned()]
    );

    Ok(())
}

#[tokio::test]
async fn a_stored_metric_reads_back_as_it_was_written() -> R {
    let fixture = Fixture::new();
    let surfaces = fixture.surfaces();
    surfaces
        .put(DefinitionKind::Metric, &name("per-actor"), &metric_body())
        .await?;

    assert_eq!(
        surfaces
            .get(DefinitionKind::Metric, &name("per-actor"))
            .await?,
        metric_body()
    );

    Ok(())
}

#[tokio::test]
async fn a_range_asked_of_a_metric_with_no_clock_is_refused_before_any_read() -> R {
    let fixture = Fixture::new();
    let surfaces = fixture.surfaces();
    surfaces
        .put(DefinitionKind::Metric, &name("clockless"), &metric_body())
        .await?;

    let Err(error) = surfaces
        .run_metric(&name("clockless"), &ranged("P7D"))
        .await
    else {
        panic!("a clockless metric cannot answer a range");
    };

    assert!(
        matches!(
            error,
            CustomError::Compile(crate::metric_query::MetricQueryError::ClocklessWindow)
        ),
        "{error:?}"
    );

    Ok(())
}

mod the_demo_definitions {
    use super::*;

    fn opened() -> serde_json::Value {
        json!({
            "table": "pull_requests",
            "time": {"json": "opened_at"},
            "fields": [{"json": "pull_request", "type": "int", "agg": "count", "as_name": "opened"}]
        })
    }

    fn merged() -> serde_json::Value {
        json!({
            "table": "pull_requests",
            "time": {"json": "merged_at"},
            "fields": [{"json": "pull_request", "type": "int", "agg": "count", "as_name": "merged"}]
        })
    }

    fn all_time() -> serde_json::Value {
        json!({
            "table": "pull_requests",
            "fields": [{"json": "pull_request", "type": "int", "agg": "count", "as_name": "total"}]
        })
    }

    #[tokio::test]
    async fn two_clocks_over_one_table_are_two_storable_metrics() -> R {
        let fixture = Fixture::new();
        let surfaces = fixture.surfaces();

        surfaces
            .put(DefinitionKind::Metric, &name("prs_opened"), &opened())
            .await?;
        surfaces
            .put(DefinitionKind::Metric, &name("prs_merged"), &merged())
            .await?;

        Ok(())
    }

    #[tokio::test]
    async fn a_clocked_line_draws_the_bucket_its_metric_injects() -> R {
        let fixture = Fixture::new();
        let surfaces = fixture.surfaces();
        surfaces
            .put(DefinitionKind::Metric, &name("prs_opened"), &opened())
            .await?;

        surfaces
            .put(
                DefinitionKind::Widget,
                &name("prs_opened_line"),
                &json!({
                    "type": "line",
                    "metric": "prs_opened",
                    "x": "bucket",
                    "y": "opened",
                }),
            )
            .await?;

        Ok(())
    }

    #[tokio::test]
    async fn a_clockless_line_has_no_bucket_to_draw() -> R {
        let fixture = Fixture::new();
        let surfaces = fixture.surfaces();
        surfaces
            .put(DefinitionKind::Metric, &name("prs_all"), &all_time())
            .await?;

        let refused = surfaces
            .put(
                DefinitionKind::Widget,
                &name("prs_all_line"),
                &json!({
                    "type": "line",
                    "metric": "prs_all",
                    "x": "bucket",
                    "y": "total",
                }),
            )
            .await;

        assert!(
            matches!(refused, Err(CustomError::Widget(_))),
            "{refused:?}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn a_board_offering_all_time_is_stored_with_the_windows_it_offers() -> R {
        let fixture = Fixture::new();
        let surfaces = fixture.surfaces();
        surfaces
            .put(DefinitionKind::Metric, &name("prs_all"), &all_time())
            .await?;
        surfaces
            .put(
                DefinitionKind::Widget,
                &name("prs_all_stat"),
                &json!({"type": "stat", "metric": "prs_all", "value": "total", "label": "Total"}),
            )
            .await?;

        let board = json!({
            "title": "Pull requests",
            "time_ranges": ["PDC", "P7D", "P30D", "PMC", "PQC", "P1Y", "inf"],
            "default_range": "P30D",
            "items": [{"widget": "prs_all_stat"}]
        });
        surfaces
            .put(DefinitionKind::Dashboard, &name("pull_requests"), &board)
            .await?;

        let stored = surfaces
            .get(DefinitionKind::Dashboard, &name("pull_requests"))
            .await?;

        assert_eq!(stored["default_range"], "P30D");
        assert_eq!(stored["time_ranges"][6], "inf");

        Ok(())
    }
}

#[tokio::test]
async fn a_board_offering_a_window_the_server_cannot_resolve_is_not_stored() -> R {
    let fixture = Fixture::new();
    let surfaces = fixture.surfaces();

    for board in [
        json!({"title": "Board", "time_ranges": ["P14D"], "items": []}),
        json!({"title": "Board", "time_ranges": ["P30D"], "default_range": "nope", "items": []}),
        json!({"title": "Board", "time_ranges": [30], "items": []}),
    ] {
        let refused = surfaces
            .put(DefinitionKind::Dashboard, &name("board"), &board)
            .await;

        assert!(matches!(refused, Err(CustomError::Range(_))), "{board}");
    }

    Ok(())
}

#[tokio::test]
async fn a_board_that_offers_no_windows_is_stored_as_it_always_was() -> R {
    let fixture = Fixture::new();

    fixture
        .surfaces()
        .put(
            DefinitionKind::Dashboard,
            &name("board"),
            &json!({"title": "Board", "items": []}),
        )
        .await?;

    Ok(())
}
