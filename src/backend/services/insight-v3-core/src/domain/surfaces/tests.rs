use std::error::Error;

use serde_json::json;

use super::*;
use crate::domain::query::metric_query::{MetricRunner, People};
use crate::store::definitions::memory::MemoryDefinitions;

type R = Result<(), Box<dyn Error>>;

struct Fixture {
    definitions: MemoryDefinitions,
    datasets: crate::store::datasets::memory::MemoryDatasets,
    metrics: MetricRunner,
}

impl Fixture {
    /// A stand whose datasets stand ready, since every stored metric reads
    /// one and is checked against what it declares.
    async fn new() -> Self {
        let client = || {
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(
                "http://clickhouse.invalid",
                "insight",
            ))
        };

        let fixture = Self {
            definitions: MemoryDefinitions::new(),
            metrics: MetricRunner::new(client(), People::new("identity")),
            datasets: crate::store::datasets::memory::MemoryDatasets::at(chrono::Utc::now()),
        };

        for (named, declaration) in [
            // Marks no main date, so a metric naming no clock of its own has
            // none at all.
            ("commits", commits()),
            ("pull_requests", pull_requests()),
        ] {
            fixture
                .a_ready_dataset(named, &declaration)
                .await
                .unwrap_or_else(|error| panic!("`{named}` should be declarable: {error}"));
        }

        fixture
    }

    /// Declares a dataset that stands, ready to be read.
    async fn a_ready_dataset(&self, named: &str, declaration: &serde_json::Value) -> R {
        let name = name(named);
        let attempt = self
            .datasets
            .take_create(&name, declaration)
            .await?
            .attempt();
        for written in [
            crate::domain::datasets::Finish::Provisioned(format!("ds_{named}_1")),
            crate::domain::datasets::Finish::Ready,
        ] {
            self.datasets.finish(&name, &attempt.token, written).await?;
        }

        Ok(())
    }

    fn metric_runs(&self) -> crate::domain::metric_run::MetricRuns<'_> {
        crate::domain::metric_run::MetricRuns::new(
            &self.definitions,
            &self.metrics,
            &self.datasets,
            "insight_datasets",
        )
    }

    fn surfaces(&self) -> Surfaces<'_> {
        Surfaces::new(
            &self.definitions,
            &self.datasets,
            "insight_datasets",
            self.metrics.people(),
        )
    }
}

fn legacy() -> crate::domain::query::time_window::WindowRequest {
    crate::domain::query::time_window::WindowRequest::parse(None, None)
        .unwrap_or_else(|error| panic!("an empty request parses: {error}"))
}

fn ranged(token: &str) -> crate::domain::query::time_window::WindowRequest {
    crate::domain::query::time_window::WindowRequest::parse(Some(token), None)
        .unwrap_or_else(|error| panic!("`{token}` parses: {error}"))
}

fn name(value: &str) -> DefinitionName {
    let Ok(parsed) = DefinitionName::parse(value) else {
        panic!("should be a valid definition name: {value}");
    };

    parsed
}

fn commits() -> serde_json::Value {
    json!({
        "title": "Commits",
        "fields": [{ "name": "actor", "path": "actor", "type": "string" }]
    })
}

fn pull_requests() -> serde_json::Value {
    json!({
        "title": "Pull requests",
        "fields": [
            { "name": "pull_request", "path": "pull_request", "type": "int" },
            { "name": "opened_at", "path": "opened_at", "type": "datetime" },
            { "name": "merged_at", "path": "merged_at", "type": "datetime" }
        ]
    })
}

fn metric_body() -> serde_json::Value {
    json!({
        "dataset": "commits",
        "fields": [
            {"field": "actor", "type": "string", "as_name": "actor"},
            {"field": "actor", "type": "string", "agg": "count", "as_name": "total"}
        ],
        "group_by": ["actor"]
    })
}

/// The one field `commits` declares, and what a metric may ask of it.
mod what_the_dataset_answers {
    use super::*;

    /// Every violation a refusal carried, so a test can name the one it means.
    fn refused(error: &CustomError) -> Vec<(String, String)> {
        let CustomError::Unanswerable(violations) = error else {
            panic!("should be refused as unanswerable: {error:?}");
        };

        violations
            .iter()
            .map(|violation| (violation.field.clone(), violation.detail.clone()))
            .collect()
    }

    async fn refusal_of(body: &serde_json::Value) -> CustomError {
        let fixture = Fixture::new().await;
        let Err(error) = fixture
            .surfaces()
            .put(DefinitionKind::Metric, &name("asked"), body)
            .await
        else {
            panic!("the dataset cannot answer this: {body}");
        };

        error
    }

    #[tokio::test]
    async fn a_metric_naming_a_field_the_dataset_never_declared_is_refused_on_write() {
        let error = refusal_of(&json!({
            "dataset": "commits",
            "fields": [{"field": "lines", "type": "int", "agg": "sum", "as_name": "total"}]
        }))
        .await;

        let violations = refused(&error);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].0, "fields[0].field");
        assert!(violations[0].1.contains("`lines`"), "{violations:?}");
    }

    #[tokio::test]
    async fn a_metric_summing_a_field_no_number_lives_in_is_refused_on_write() {
        let error = refusal_of(&json!({
            "dataset": "commits",
            "fields": [{"field": "actor", "type": "string", "agg": "sum", "as_name": "total"}]
        }))
        .await;

        let violations = refused(&error);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].0, "fields[0].field");
    }

    #[tokio::test]
    async fn a_metric_comparing_a_field_against_another_type_is_refused_on_write() {
        let error = refusal_of(&json!({
            "dataset": "commits",
            "fields": [{"field": "actor", "type": "string", "as_name": "actor"}],
            "filters": [{"field": "actor", "type": "string", "op": "eq", "value": 7}]
        }))
        .await;

        assert_eq!(refused(&error)[0].0, "filters[0].field");
    }

    #[tokio::test]
    async fn a_metric_windowing_by_a_field_holding_no_date_is_refused_on_write() {
        let error = refusal_of(&json!({
            "dataset": "commits",
            "time": {"field": "actor", "type": "datetime"},
            "fields": [{"field": "actor", "type": "string", "agg": "count", "as_name": "total"}]
        }))
        .await;

        assert_eq!(refused(&error)[0].0, "time.field");
    }

    /// One answer carries every refusal, so an editor marks them all at once
    /// rather than one round trip per mistake.
    #[tokio::test]
    async fn a_metric_wrong_in_several_places_is_refused_once_with_all_of_them() {
        let error = refusal_of(&json!({
            "dataset": "commits",
            "fields": [
                {"field": "lines", "type": "int", "agg": "sum", "as_name": "total"},
                {"field": "repository", "type": "string", "as_name": "repository"}
            ]
        }))
        .await;

        let violations = refused(&error);
        assert_eq!(
            violations
                .iter()
                .map(|(at, _)| at.clone())
                .collect::<Vec<_>>(),
            vec!["fields[0].field", "fields[1].field"]
        );
    }

    /// Where a value sits, and whether it holds a person, are the
    /// declaration's to answer.
    #[tokio::test]
    async fn a_metric_reaching_into_the_record_itself_is_refused_on_write() {
        let cases = [
            (
                json!({
                    "dataset": "commits",
                    "fields": [{"json": "actor", "type": "string", "as_name": "actor"}]
                }),
                "fields[0].json",
            ),
            (
                json!({
                    "dataset": "commits",
                    "fields": [{"column": "actor", "type": "string", "as_name": "actor"}]
                }),
                "fields[0].column",
            ),
            (
                json!({
                    "dataset": "commits",
                    "fields": [{
                        "field": "actor", "type": "string", "as_name": "actor",
                        "person": "email"
                    }]
                }),
                "fields[0].person",
            ),
            (
                json!({
                    "dataset": "commits",
                    "fields": [{"field": "actor", "type": "string", "as_name": "actor"}],
                    "filters": [{"json": "actor", "type": "string", "op": "eq", "value": "a"}]
                }),
                "filters[0].json",
            ),
            (
                json!({
                    "dataset": "commits",
                    "time": {"column": "committed_at"},
                    "fields": [{"field": "actor", "type": "string", "as_name": "actor"}]
                }),
                "time.column",
            ),
            (
                json!({
                    "dataset": "commits",
                    "fields": [{
                        "field": "actor", "type": "string", "as_name": "actor",
                        "where": {"field": "actor", "type": "string", "op": "eq", "value": "a"}
                    }]
                }),
                "fields[0].where",
            ),
        ];

        for (body, at) in cases {
            let error = refusal_of(&body).await;

            assert_eq!(
                refused(&error).first().map(|(field, _)| field.clone()),
                Some(at.to_owned()),
                "should be refused at {at}: {body}"
            );
        }
    }

    /// Everything the compiler refuses, the write refuses too: a body stored
    /// here and refused at every run is a definition nobody can act on.
    #[tokio::test]
    async fn a_metric_the_compiler_would_refuse_is_refused_on_write() {
        let cases = [
            (
                json!({ "dataset": "commits", "fields": [] }),
                "a metric must select at least one field",
            ),
            (
                json!({
                    "dataset": "commits",
                    "fields": [{"type": "int", "as_name": "x"}]
                }),
                "must name",
            ),
            (
                json!({
                    "dataset": "commits",
                    "fields": [
                        {"field": "actor", "type": "string", "as_name": "actor"},
                        {"field": "actor", "type": "string", "agg": "count", "as_name": "n"}
                    ]
                }),
                "`actor` is selected beside an aggregate",
            ),
            (
                json!({
                    "dataset": "commits",
                    "fields": [{"field": "actor", "type": "string", "as_name": "an actor"}]
                }),
                "1-128 characters",
            ),
        ];

        for (body, said) in cases {
            let fixture = Fixture::new().await;
            let Err(error) = fixture
                .surfaces()
                .put(DefinitionKind::Metric, &name("asked"), &body)
                .await
            else {
                panic!("the compiler would refuse this: {body}");
            };

            assert!(
                error.to_string().contains(said),
                "should mention {said:?}: {error}"
            );
        }
    }

    /// A trend metric names the bucket its windowed run injects; the same
    /// metric answers an unwindowed run, which has no bucket to group by.
    #[tokio::test]
    async fn a_metric_may_group_by_the_bucket_a_window_injects() -> R {
        let fixture = Fixture::new().await;
        fixture
            .surfaces()
            .put(
                DefinitionKind::Metric,
                &name("per_day"),
                &json!({
                    "dataset": "pull_requests",
                    "time": {"field": "opened_at"},
                    "group_by": ["bucket"],
                    "fields": [{"agg": "count", "type": "int", "as_name": "total"}]
                }),
            )
            .await?;

        Ok(())
    }

    #[tokio::test]
    async fn a_metric_over_a_dataset_nobody_declared_is_refused_on_write() {
        let error = refusal_of(&json!({
            "dataset": "deploys",
            "fields": [{"field": "actor", "type": "string", "as_name": "actor"}]
        }))
        .await;

        assert!(
            matches!(&error, CustomError::DatasetNotReady(named) if named == "deploys"),
            "{error:?}"
        );
    }
}

fn line_widget(metric: &str, y: &str) -> serde_json::Value {
    json!({"type": "line", "metric": metric, "x": "actor", "y": y})
}

#[tokio::test]
async fn reading_a_definition_that_was_never_stored_reports_it_missing() {
    let fixture = Fixture::new().await;

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
    let fixture = Fixture::new().await;
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
    let fixture = Fixture::new().await;

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
    let fixture = Fixture::new().await;
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
    let fixture = Fixture::new().await;
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
    let fixture = Fixture::new().await;
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
    let fixture = Fixture::new().await;

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
async fn a_body_that_cannot_be_read_as_a_metric_is_refused_before_it_is_stored() -> R {
    let fixture = Fixture::new().await;

    let refusal = fixture
        .surfaces()
        .put(
            DefinitionKind::Metric,
            &name("broken"),
            &json!({"dataset": "commits"}),
        )
        .await;

    assert!(matches!(refusal, Err(CustomError::Body(_))), "{refusal:?}");

    Ok(())
}

#[tokio::test]
async fn running_a_metric_whose_stored_body_is_not_a_query_reports_the_body() -> R {
    let fixture = Fixture::new().await;
    fixture
        .definitions
        .put(
            DefinitionKind::Metric,
            &name("broken"),
            &json!({"dataset": "commits"}),
        )
        .await?;

    let Err(error) = fixture.metric_runs().run(&name("broken"), &legacy()).await else {
        panic!("a query with no fields does not deserialize");
    };

    assert!(matches!(error, CustomError::Body(_)), "{error:?}");

    Ok(())
}

#[tokio::test]
async fn running_a_metric_that_was_never_stored_reports_it_missing() {
    let fixture = Fixture::new().await;

    let Err(error) = fixture.metric_runs().run(&name("absent"), &legacy()).await else {
        panic!("there is no such metric to run");
    };

    assert!(matches!(error, CustomError::NotFound { .. }), "{error:?}");
}

#[tokio::test]
async fn listing_names_them_in_the_order_the_store_gives() -> R {
    let fixture = Fixture::new().await;
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
    let fixture = Fixture::new().await;
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
    let fixture = Fixture::new().await;
    let surfaces = fixture.surfaces();
    surfaces
        .put(DefinitionKind::Metric, &name("clockless"), &metric_body())
        .await?;
    let Err(error) = fixture
        .metric_runs()
        .run(&name("clockless"), &ranged("P7D"))
        .await
    else {
        panic!("a clockless metric cannot answer a range");
    };

    assert!(
        matches!(
            error,
            CustomError::Compile(
                crate::domain::query::metric_query::MetricQueryError::ClocklessWindow
            )
        ),
        "{error:?}"
    );

    Ok(())
}

mod the_demo_definitions {
    use super::*;

    fn opened() -> serde_json::Value {
        json!({
            "dataset": "pull_requests",
            "time": {"field": "opened_at"},
            "fields": [{"field": "pull_request", "type": "int", "agg": "count", "as_name": "opened"}]
        })
    }

    fn merged() -> serde_json::Value {
        json!({
            "dataset": "pull_requests",
            "time": {"field": "merged_at"},
            "fields": [{"field": "pull_request", "type": "int", "agg": "count", "as_name": "merged"}]
        })
    }

    fn all_time() -> serde_json::Value {
        json!({
            "dataset": "pull_requests",
            "fields": [{"field": "pull_request", "type": "int", "agg": "count", "as_name": "total"}]
        })
    }

    #[tokio::test]
    async fn two_clocks_over_one_table_are_two_storable_metrics() -> R {
        let fixture = Fixture::new().await;
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
        let fixture = Fixture::new().await;
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
    async fn a_board_offering_all_time_is_stored_with_the_windows_it_offers() -> R {
        let fixture = Fixture::new().await;
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
    let fixture = Fixture::new().await;
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
    let fixture = Fixture::new().await;

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

#[tokio::test]
async fn a_widget_may_draw_a_metric_arriving_in_the_same_batch() -> R {
    let fixture = Fixture::new().await;

    let batch = vec![
        Definition::new(DefinitionKind::Metric, name("per_actor"), metric_body()),
        Definition::new(
            DefinitionKind::Widget,
            name("chart"),
            line_widget("per_actor", "total"),
        ),
    ];

    fixture.surfaces().check_batch(&batch).await?;

    Ok(())
}

#[tokio::test]
async fn one_refused_body_refuses_the_whole_batch() -> R {
    let fixture = Fixture::new().await;

    let batch = vec![
        Definition::new(DefinitionKind::Metric, name("per_actor"), metric_body()),
        Definition::new(
            DefinitionKind::Widget,
            name("chart"),
            line_widget("per_actor", "no_such_column"),
        ),
    ];

    let refusal = fixture.surfaces().check_batch(&batch).await;

    assert!(
        matches!(refusal, Err(CustomError::Widget(_))),
        "{refusal:?}"
    );

    Ok(())
}

#[tokio::test]
async fn renaming_points_every_widget_that_drew_the_old_name_at_the_new_one() -> R {
    let fixture = Fixture::new().await;
    let surfaces = fixture.surfaces();
    surfaces
        .put(DefinitionKind::Metric, &name("per_actor"), &metric_body())
        .await?;
    surfaces
        .put(
            DefinitionKind::Widget,
            &name("chart"),
            &line_widget("per_actor", "total"),
        )
        .await?;

    let rewritten = surfaces
        .rename(
            DefinitionKind::Metric,
            &name("per_actor"),
            &name("by_actor"),
        )
        .await?;

    assert_eq!(rewritten, vec!["chart".to_owned()]);
    let widget = surfaces.get(DefinitionKind::Widget, &name("chart")).await?;
    assert_eq!(widget["metric"], "by_actor");

    Ok(())
}

#[tokio::test]
async fn a_rename_onto_a_name_someone_holds_is_refused_and_writes_nothing() -> R {
    let fixture = Fixture::new().await;
    let surfaces = fixture.surfaces();
    for held in ["per_actor", "by_actor"] {
        surfaces
            .put(DefinitionKind::Metric, &name(held), &metric_body())
            .await?;
    }

    let refusal = surfaces
        .rename(
            DefinitionKind::Metric,
            &name("per_actor"),
            &name("by_actor"),
        )
        .await;

    assert!(
        matches!(
            refusal,
            Err(CustomError::Store(DefinitionStoreError::NameTaken(_)))
        ),
        "{refusal:?}"
    );
    surfaces
        .get(DefinitionKind::Metric, &name("per_actor"))
        .await?;

    Ok(())
}

#[tokio::test]
async fn renaming_a_definition_to_the_name_it_already_has_changes_nothing() -> R {
    let fixture = Fixture::new().await;
    let surfaces = fixture.surfaces();
    surfaces
        .put(DefinitionKind::Metric, &name("per_actor"), &metric_body())
        .await?;
    surfaces
        .put(
            DefinitionKind::Widget,
            &name("chart"),
            &line_widget("per_actor", "total"),
        )
        .await?;

    let rewritten = surfaces
        .rename(
            DefinitionKind::Metric,
            &name("per_actor"),
            &name("per_actor"),
        )
        .await?;

    assert!(rewritten.is_empty(), "{rewritten:?}");
    assert_eq!(
        surfaces
            .get(DefinitionKind::Metric, &name("per_actor"))
            .await?,
        metric_body()
    );
    let widget = surfaces.get(DefinitionKind::Widget, &name("chart")).await?;
    assert_eq!(widget["metric"], "per_actor");

    Ok(())
}

#[tokio::test]
async fn renaming_a_widget_points_every_board_that_held_it_at_the_new_name() -> R {
    let fixture = Fixture::new().await;
    let surfaces = fixture.surfaces();
    surfaces
        .put(DefinitionKind::Metric, &name("per_actor"), &metric_body())
        .await?;
    for held in ["old", "kept"] {
        surfaces
            .put(
                DefinitionKind::Widget,
                &name(held),
                &line_widget("per_actor", "total"),
            )
            .await?;
    }
    surfaces
        .put(
            DefinitionKind::Dashboard,
            &name("listed"),
            &json!({ "title": "b", "items": [{ "heading": "h" }, { "widget": "old" }] }),
        )
        .await?;
    surfaces
        .put(
            DefinitionKind::Dashboard,
            &name("shorthand"),
            &json!({ "title": "b", "widgets": ["old", "kept"] }),
        )
        .await?;

    let mut rewritten = surfaces
        .rename(DefinitionKind::Widget, &name("old"), &name("new"))
        .await?;
    rewritten.sort();

    assert_eq!(rewritten, vec!["listed".to_owned(), "shorthand".to_owned()]);
    let listed = surfaces
        .get(DefinitionKind::Dashboard, &name("listed"))
        .await?;
    assert_eq!(listed["items"][1]["widget"], "new");
    let shorthand = surfaces
        .get(DefinitionKind::Dashboard, &name("shorthand"))
        .await?;
    assert_eq!(shorthand["widgets"], json!(["new", "kept"]));

    Ok(())
}

/// A metric reading a dataset this stand never declared.
fn over_a_dataset() -> serde_json::Value {
    json!({
        "dataset": "deploys",
        "fields": [{ "field": "lines", "type": "int", "agg": "sum", "as_name": "total" }],
        "group_by": [],
        "filters": []
    })
}

fn a_declaration() -> serde_json::Value {
    json!({
        "title": "Deploys",
        "fields": [{ "name": "lines", "path": "lines", "type": "int" }]
    })
}

#[tokio::test]
async fn running_a_metric_over_a_dataset_nobody_declared_says_so() -> R {
    let fixture = Fixture::new().await;
    fixture
        .definitions
        .put(DefinitionKind::Metric, &name("lines"), &over_a_dataset())
        .await?;

    let Err(error) = fixture.metric_runs().run(&name("lines"), &legacy()).await else {
        panic!("there is no such dataset to read");
    };

    assert!(
        matches!(&error, CustomError::DatasetNotReady(named) if named == "deploys"),
        "{error:?}"
    );

    Ok(())
}

#[tokio::test]
async fn running_a_metric_over_a_dataset_still_being_made_says_so() -> R {
    let fixture = Fixture::new().await;
    fixture
        .definitions
        .put(DefinitionKind::Metric, &name("lines"), &over_a_dataset())
        .await?;
    fixture
        .datasets
        .take_create(&name("deploys"), &a_declaration())
        .await?;

    let Err(error) = fixture.metric_runs().run(&name("lines"), &legacy()).await else {
        panic!("a dataset still being made has no table to read");
    };

    assert!(
        matches!(error, CustomError::DatasetNotReady(_)),
        "{error:?}"
    );

    Ok(())
}

/// A store that answers everything except one kind's bodies, for the cases
/// about a read of ours that did not come back.
#[derive(Debug)]
struct Unreadable {
    inner: MemoryDefinitions,
    silent: DefinitionKind,
}

#[async_trait::async_trait]
impl crate::domain::definition::Lookup for Unreadable {
    async fn get(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<Option<serde_json::Value>, DefinitionStoreError> {
        if kind == self.silent {
            return Err(DefinitionStoreError::Database(sea_orm::DbErr::Custom(
                "store is down".to_owned(),
            )));
        }

        self.inner.get(kind, name).await
    }
}

#[async_trait::async_trait]
impl Definitions for Unreadable {
    async fn put(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
        body: &serde_json::Value,
    ) -> Result<(), DefinitionStoreError> {
        self.inner.put(kind, name, body).await
    }

    async fn list(&self, kind: DefinitionKind) -> Result<Vec<String>, DefinitionStoreError> {
        self.inner.list(kind).await
    }

    async fn page(
        &self,
        kind: DefinitionKind,
        needle: &str,
        page: Page,
    ) -> Result<NamePage, DefinitionStoreError> {
        self.inner.page(kind, needle, page).await
    }

    async fn delete(
        &self,
        kind: DefinitionKind,
        name: &DefinitionName,
    ) -> Result<bool, DefinitionStoreError> {
        self.inner.delete(kind, name).await
    }

    async fn apply(&self, changes: &[Change]) -> Result<(), DefinitionStoreError> {
        self.inner.apply(changes).await
    }
}

/// A store that did not answer is ours, not the holder's. Reporting it as
/// "broken" would paint a healthy board as damaged over a blip, and the
/// reader would go looking for a fault in a definition that has none.
#[tokio::test]
async fn a_holder_is_not_called_broken_because_a_read_of_ours_failed() -> R {
    let fixture = Fixture::new().await;
    fixture
        .surfaces()
        .put(DefinitionKind::Metric, &name("per_day"), &json!({"dataset": "commits", "fields": [{"agg": "count", "type": "int", "as_name": "total"}]}))
        .await?;
    fixture
        .surfaces()
        .put(
            DefinitionKind::Widget,
            &name("per_day_table"),
            &json!({"type": "table", "metric": "per_day", "columns": ["total"]}),
        )
        .await?;

    // The widgets are still listed and read; the metric each one checks
    // against is what the store will not answer for.
    let definitions = Unreadable {
        inner: std::mem::take(&mut { fixture.definitions }),
        silent: DefinitionKind::Metric,
    };
    let surfaces = Surfaces::new(
        &definitions,
        &fixture.datasets,
        "insight_datasets",
        fixture.metrics.people(),
    );

    let asked = surfaces
        .dependents_state(DefinitionKind::Metric, &name("per_day"))
        .await;

    assert!(
        matches!(asked, Err(CustomError::Store(_))),
        "a read of ours is not a holder's fault: {asked:?}"
    );

    Ok(())
}
