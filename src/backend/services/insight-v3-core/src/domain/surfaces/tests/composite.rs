//! A composite: what it may combine, what it keeps standing, and what draws
//! or drills into it.

use super::*;
use crate::domain::kinds::metric::answerable::ClockSource;
use crate::domain::query::composite::CompositeError;
use crate::domain::query::metric_query::ColumnKind;
use crate::domain::query::time_window::Window;

const INCIDENTS: &str = "incident-counts";
const DEPLOYS: &str = "deploy-counts";
const COMPOSITE: &str = "incidents-per-deploy";

/// A metric over its own table answering one row per team, windowed by when
/// each row happened.
fn per_team(table: &str) -> Value {
    json!({
        "table": table,
        "time": { "column": "occurred_at" },
        "fields": [
            { "column": "team", "type": "string", "as_name": "team" },
            { "agg": "count", "type": "int", "as_name": "count" }
        ],
        "group_by": ["team"]
    })
}

fn incidents_per_deploy() -> Value {
    json!({
        "combine": { "incidents": INCIDENTS, "deploys": DEPLOYS },
        "group_by": ["team"],
        "fields": [
            { "as_name": "incidents", "field": "incidents.count" },
            { "as_name": "deploys", "field": "deploys.count" },
            { "as_name": "incidents_per_deploy", "divide": ["incidents", "deploys"] }
        ],
        "order_by": { "field": "incidents_per_deploy", "direction": "desc" }
    })
}

/// The same composite with one key of its body replaced.
fn with(key: &str, value: Value) -> Value {
    let mut body = incidents_per_deploy();
    body[key] = value;
    body
}

/// Stores both children unchecked, as a store holding them already would.
async fn children_stored(fixture: &Fixture, incidents: &Value) -> R {
    stored_unchecked(fixture, INCIDENTS, incidents).await?;
    stored_unchecked(fixture, DEPLOYS, &per_team("deploys")).await
}

async fn stored_unchecked(fixture: &Fixture, id: &str, body: &Value) -> R {
    fixture
        .definitions
        .seed(DefinitionKind::Metric, &name(id), body)
        .await?;

    Ok(())
}

/// A stand holding both children and the composite over them.
async fn composite_stored() -> Result<Fixture, Box<dyn Error>> {
    let fixture = Fixture::new().await;
    children_stored(&fixture, &per_team("incidents")).await?;
    fixture
        .surfaces()
        .stored_as(
            DefinitionKind::Metric,
            &name(COMPOSITE),
            &incidents_per_deploy(),
        )
        .await?;

    Ok(fixture)
}

#[tokio::test]
async fn a_composite_over_metrics_that_each_answer_one_row_per_key_is_stored() -> R {
    let fixture = composite_stored().await?;

    assert_eq!(
        fixture
            .surfaces()
            .get(DefinitionKind::Metric, &name(COMPOSITE))
            .await?,
        incidents_per_deploy()
    );

    Ok(())
}

/// A composite may read a dataset beside a table, so long as both answer the
/// same keys the same way.
#[tokio::test]
async fn a_composite_may_combine_a_dataset_with_a_table() -> R {
    let fixture = Fixture::new().await;
    fixture
        .definitions
        .seed(DefinitionKind::Metric, &name("per-actor"), &metric_body())
        .await?;
    fixture
        .definitions
        .seed(
            DefinitionKind::Metric,
            &name("reviews"),
            &json!({
                "table": "reviews",
                "fields": [
                    { "column": "actor", "type": "string", "as_name": "actor" },
                    { "agg": "count", "type": "int", "as_name": "reviews" }
                ],
                "group_by": ["actor"]
            }),
        )
        .await?;

    fixture
        .surfaces()
        .stored_as(
            DefinitionKind::Metric,
            &name(COMPOSITE),
            &json!({
                "combine": { "commits": "per-actor", "reviews": "reviews" },
                "group_by": ["actor"],
                "fields": [
                    { "as_name": "commits", "field": "commits.total" },
                    { "as_name": "reviews", "field": "reviews.reviews" },
                    { "as_name": "reviews_per_commit", "divide": ["reviews", "commits"] }
                ]
            }),
        )
        .await?;

    Ok(())
}

/// A child the composite cannot combine, the composite naming it, and
/// whether a refusal is for the rule that child breaks.
type Refusal = (&'static str, Value, Value, fn(&CompositeError) -> bool);

/// Stores each case's children, then refuses its composite for its rule.
async fn refused_for_their_rule(cases: Vec<Refusal>) -> R {
    for (case, incidents, composite, refused_for) in cases {
        let fixture = Fixture::new().await;
        children_stored(&fixture, &incidents).await?;
        stored_unchecked(
            &fixture,
            "deploy-total",
            &json!({
                "table": "deploys",
                "fields": [{ "agg": "count", "type": "int", "as_name": "count" }]
            }),
        )
        .await?;

        let refusal = fixture
            .surfaces()
            .stored_as(DefinitionKind::Metric, &name(COMPOSITE), &composite)
            .await;

        match refusal {
            Err(CustomError::Composite(error)) if refused_for(&error) => {}
            other => panic!("{case} should be refused for its rule: {other:?}"),
        }
    }

    Ok(())
}

/// Each child that would answer more or fewer than one row per key.
#[tokio::test]
async fn a_child_that_could_count_a_fact_twice_or_lose_a_key_is_refused_on_write() -> R {
    refused_for_their_rule(vec![
        (
            "a composite",
            json!({
                "combine": { "a": DEPLOYS, "b": DEPLOYS },
                "group_by": ["team"],
                "fields": [
                    { "as_name": "a", "field": "a.count" },
                    { "as_name": "b", "field": "b.count" }
                ]
            }),
            incidents_per_deploy(),
            |error| matches!(error, CompositeError::Nested(alias) if alias == "incidents"),
        ),
        (
            "a child answering a row per record",
            json!({
                "table": "incidents",
                "time": { "column": "occurred_at" },
                "fields": [{ "column": "minutes", "type": "int", "as_name": "minutes" }]
            }),
            json!({
                "combine": { "incidents": INCIDENTS, "deploys": "deploy-total" },
                "fields": [
                    { "as_name": "minutes", "field": "incidents.minutes" },
                    { "as_name": "deploys", "field": "deploys.count" }
                ]
            }),
            |error| matches!(error, CompositeError::Unaggregated(alias) if alias == "incidents"),
        ),
        (
            "a child cut at a limit of its own",
            {
                let mut child = per_team("incidents");
                child["limit"] = json!(10);
                child
            },
            incidents_per_deploy(),
            |error| matches!(error, CompositeError::Limited(alias) if alias == "incidents"),
        ),
        (
            "a child grouping by a column more than the composite",
            json!({
                "table": "incidents",
                "time": { "column": "occurred_at" },
                "fields": [
                    { "column": "team", "type": "string", "as_name": "team" },
                    { "column": "service", "type": "string", "as_name": "service" },
                    { "agg": "count", "type": "int", "as_name": "count" }
                ],
                "group_by": ["team", "service"]
            }),
            incidents_per_deploy(),
            |error| matches!(error, CompositeError::Keys { alias, .. } if alias == "incidents"),
        ),
    ])
    .await
}

/// Each child that answers one row per key, but not the same keys the same
/// way as the others.
#[tokio::test]
async fn a_child_that_disagrees_with_the_others_on_a_key_or_a_window_is_refused_on_write() -> R {
    refused_for_their_rule(vec![
        (
            "a child holding the key as another kind of value",
            json!({
                "table": "incidents",
                "time": { "column": "occurred_at" },
                "fields": [
                    { "column": "team", "type": "int", "as_name": "team" },
                    { "agg": "count", "type": "int", "as_name": "count" }
                ],
                "group_by": ["team"]
            }),
            incidents_per_deploy(),
            |error| {
                matches!(
                    error,
                    CompositeError::KeyKind { key, alias, .. } if key == "team" && alias == "incidents"
                )
            },
        ),
        (
            "an undated child beside a dated one",
            json!({
                "table": "incidents",
                "fields": [
                    { "column": "team", "type": "string", "as_name": "team" },
                    { "agg": "count", "type": "int", "as_name": "count" }
                ],
                "group_by": ["team"]
            }),
            incidents_per_deploy(),
            |error| {
                matches!(
                    error,
                    CompositeError::Clocks { dated, undated } if dated == "deploys" && undated == "incidents"
                )
            },
        ),
        (
            "a pick of a key rather than a value",
            per_team("incidents"),
            json!({
                "combine": { "incidents": INCIDENTS, "deploys": DEPLOYS },
                "group_by": ["team"],
                "fields": [
                    { "as_name": "incidents", "field": "incidents.team" },
                    { "as_name": "deploys", "field": "deploys.count" }
                ]
            }),
            |error| {
                matches!(
                    error,
                    CompositeError::NotAValue { alias, column, .. } if alias == "incidents" && column == "team"
                )
            },
        ),
        (
            "a child that is not there",
            Value::Null,
            with(
                "combine",
                json!({ "incidents": "nowhere", "deploys": DEPLOYS }),
            ),
            |error| matches!(error, CompositeError::Missing { alias, .. } if alias == "incidents"),
        ),
    ])
    .await
}

/// A body naming what only a plain metric may, or a name that resolves to
/// nothing, is refused before any child is read.
#[tokio::test]
async fn a_composite_body_outside_its_vocabulary_is_refused_on_write() -> R {
    let cases = [
        (with("dataset", json!("commits")), "unknown field `dataset`"),
        (
            with("time", json!({ "column": "day" })),
            "unknown field `time`",
        ),
        (
            with(
                "fields",
                json!([
                    { "as_name": "incidents", "field": "incidents.count" },
                    { "as_name": "deploys", "field": "deploys.count" },
                    { "as_name": "rate", "divide": ["incidents", "outages"] }
                ]),
            ),
            "`rate` divides `outages`",
        ),
    ];

    for (body, said) in cases {
        let fixture = Fixture::new().await;
        children_stored(&fixture, &per_team("incidents")).await?;

        let refusal = fixture
            .surfaces()
            .stored_as(DefinitionKind::Metric, &name(COMPOSITE), &body)
            .await;

        match refusal {
            Err(CustomError::Body(error)) if error.to_string().contains(said) => {}
            other => panic!("should be refused saying {said:?}: {other:?}"),
        }
    }

    Ok(())
}

#[tokio::test]
async fn a_composite_and_the_metrics_it_combines_may_arrive_in_one_batch() -> R {
    let fixture = Fixture::new().await;

    let batch = vec![
        Definition::new(
            DefinitionKind::Metric,
            name(COMPOSITE),
            incidents_per_deploy(),
        ),
        Definition::new(
            DefinitionKind::Metric,
            name(INCIDENTS),
            per_team("incidents"),
        ),
        Definition::new(DefinitionKind::Metric, name(DEPLOYS), per_team("deploys")),
    ];

    fixture.surfaces().check_batch(&batch).await?;

    Ok(())
}

#[tokio::test]
async fn a_metric_a_composite_combines_is_kept_while_the_composite_stands() -> R {
    let fixture = composite_stored().await?;
    let surfaces = fixture.surfaces();

    let Err(error) = surfaces
        .delete(DefinitionKind::Metric, &name(INCIDENTS))
        .await
    else {
        panic!("a metric a composite combines is kept");
    };
    match error {
        CustomError::InUse { used_by, hidden } => {
            assert_eq!(used_by, vec![COMPOSITE.to_owned()]);
            assert_eq!(hidden, 0);
        }
        other => panic!("should refuse as in use: {other:?}"),
    }

    surfaces
        .delete(DefinitionKind::Metric, &name(COMPOSITE))
        .await?;
    surfaces
        .delete(DefinitionKind::Metric, &name(INCIDENTS))
        .await?;

    Ok(())
}

#[tokio::test]
async fn a_widget_may_draw_a_composite_by_its_bucket_keys_and_fields() -> R {
    let fixture = composite_stored().await?;

    fixture
        .surfaces()
        .stored_as(
            DefinitionKind::Widget,
            &name("trend"),
            &json!({
                "type": "bar", "metric": COMPOSITE,
                "x": "bucket", "y": "incidents_per_deploy", "series": "team"
            }),
        )
        .await?;

    Ok(())
}

#[tokio::test]
async fn a_widget_cannot_drill_into_a_composite() -> R {
    let fixture = composite_stored().await?;

    let refusal = fixture
        .surfaces()
        .stored_as(
            DefinitionKind::Widget,
            &name("table"),
            &json!({
                "type": "table", "metric": DEPLOYS, "detail": COMPOSITE,
                "columns": ["team", "count"]
            }),
        )
        .await;

    assert!(
        matches!(
            &refusal,
            Err(CustomError::Widget(WidgetError::CompositeDetail(named))) if named == COMPOSITE
        ),
        "{refusal:?}"
    );

    Ok(())
}

#[tokio::test]
async fn drilling_into_a_composite_is_refused() -> R {
    let fixture = composite_stored().await?;

    let refusal = fixture
        .metric_runs()
        .prepare(&name(COMPOSITE), &Window::legacy())
        .await;

    assert!(
        matches!(
            refusal,
            Err(CustomError::Composite(CompositeError::NoRecords))
        ),
        "{refusal:?}"
    );

    Ok(())
}

/// A child may change after the composite is stored; every run holds it to
/// the rules again, before anything is sent to the warehouse.
#[tokio::test]
async fn a_child_changed_after_the_composite_was_stored_is_refused_when_it_runs() -> R {
    let fixture = composite_stored().await?;
    let mut limited = per_team("incidents");
    limited["limit"] = json!(10);
    fixture
        .definitions
        .seed(DefinitionKind::Metric, &name(INCIDENTS), &limited)
        .await?;

    let runs = fixture.metric_runs();
    let ran = runs.run(&name(COMPOSITE), &legacy()).await;
    let described = runs.columns(&name(COMPOSITE)).await;

    for refusal in [ran.map(|_| ()), described.map(|_| ())] {
        assert!(
            matches!(
                &refusal,
                Err(CustomError::Composite(CompositeError::Limited(alias))) if alias == "incidents"
            ),
            "{refusal:?}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn a_composite_reports_its_keys_and_fields_and_the_window_of_its_first_child() -> R {
    let fixture = composite_stored().await?;

    let columns = fixture.metric_runs().columns(&name(COMPOSITE)).await?;
    let clock = fixture
        .surfaces()
        .clock_of(DefinitionKind::Metric, &incidents_per_deploy())
        .await;

    assert_eq!(
        columns.kinds,
        vec![
            ("team".to_owned(), ColumnKind::Text),
            ("incidents".to_owned(), ColumnKind::Number),
            ("deploys".to_owned(), ColumnKind::Number),
            ("incidents_per_deploy".to_owned(), ColumnKind::Number),
        ]
    );
    assert_eq!(columns.groups, vec!["team".to_owned()]);
    assert_eq!(
        clock.map(|clock| (clock.field, clock.from)),
        Some(("occurred_at".to_owned(), ClockSource::Metric))
    );

    Ok(())
}
