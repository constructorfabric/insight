use serde_json::{Value, json};

use super::*;
use crate::domain::query::metric_query::{CompiledQuery, FieldType, People, TableEngine};
use crate::domain::query::time_window::{RequestedRange, Window};

fn composite(body: Value) -> Composite {
    serde_json::from_value(body).unwrap_or_else(|error| panic!("the composite parses: {error}"))
}

fn query(body: Value) -> MetricQuery {
    serde_json::from_value(body).unwrap_or_else(|error| panic!("the child parses: {error}"))
}

/// A child over its own table: one row per team, and optionally a day clock
/// and a filter of its own.
fn per_team(table: &str, extra_fields: &[Value], filters: &[Value]) -> MetricQuery {
    let mut fields = vec![json!({ "column": "team", "type": "string", "as_name": "team" })];
    fields.extend(extra_fields.iter().cloned());

    query(json!({
        "table": table,
        "time": { "column": "occurred_at" },
        "fields": fields,
        "group_by": ["team"],
        "filters": filters
    }))
}

fn counted() -> Value {
    json!({ "agg": "count", "type": "int", "as_name": "count" })
}

fn incidents_per_deploy() -> Composite {
    composite(json!({
        "combine": { "incidents": "incident-counts", "deploys": "deploy-counts" },
        "group_by": ["team"],
        "fields": [
            { "as_name": "incidents", "field": "incidents.count" },
            { "as_name": "deploys", "field": "deploys.count" },
            { "as_name": "incidents_per_deploy", "divide": ["incidents", "deploys"] }
        ],
        "order_by": { "field": "incidents_per_deploy", "direction": "desc" },
        "limit": 50
    }))
}

fn last_week(bucketed: bool) -> Window {
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-10T15:00:00Z")
        .unwrap_or_else(|error| panic!("the synthetic clock parses: {error}"))
        .to_utc();
    let resolved = RequestedRange::parse("P7D")
        .and_then(|range| range.resolve(now))
        .unwrap_or_else(|error| panic!("the range resolves: {error}"));

    if bucketed {
        resolved
    } else {
        resolved.unbucketed()
    }
}

/// The composite over these children, each named by its alias and stacked in
/// the order the composite combines them, as a run stacks them.
fn compiled(
    composite: &Composite,
    children: &[(&str, MetricQuery)],
    window: &Window,
) -> CompiledQuery {
    let people = People::new("identity");
    let stacked: Vec<Stacked<'_>> = composite
        .combine()
        .iter()
        .map(|combined| {
            let Some((_, child)) = children
                .iter()
                .find(|(alias, _)| *alias == combined.alias.as_str())
            else {
                panic!("the case gives `{}` a child", combined.alias.as_str());
            };
            let compiled = child
                .compile_inner(&people, window, TableEngine::MergeTree, None)
                .unwrap_or_else(|error| panic!("the child compiles: {error}"));

            Stacked {
                alias: &combined.alias,
                query: child,
                compiled,
            }
        })
        .collect();

    composite.compile(&stacked, window.grain().is_some())
}

fn inner_sql(child: &MetricQuery, window: &Window) -> String {
    child
        .compile_inner(
            &People::new("identity"),
            window,
            TableEngine::MergeTree,
            None,
        )
        .unwrap_or_else(|error| panic!("the child compiles: {error}"))
        .sql()
        .to_owned()
}

#[test]
fn a_child_compiled_for_a_composite_keeps_every_row_in_no_order() {
    let ordered = query(json!({
        "table": "deploys",
        "fields": [
            { "column": "team", "type": "string", "as_name": "team" },
            { "agg": "count", "type": "int", "as_name": "count" }
        ],
        "group_by": ["team"],
        "order_by": { "field": "count", "direction": "desc" }
    }));

    let sql = inner_sql(&ordered, &Window::legacy());

    assert_eq!(
        sql,
        "SELECT `team` AS `team`, count() AS `count` FROM `deploys` GROUP BY `team`"
    );
}

/// The body's map holds its aliases in name order, so `deploys` stacks
/// first; each field keeps the slot its place in `fields` gives it.
#[test]
fn a_composite_stacks_its_children_and_folds_them_by_key() {
    let incidents = per_team("incidents", &[counted()], &[]);
    let deploys = per_team("deploys", &[counted()], &[]);
    let window = Window::legacy();
    let incidents_sql = inner_sql(&incidents, &window);
    let deploys_sql = inner_sql(&deploys, &window);

    let compiled = compiled(
        &incidents_per_deploy(),
        &[("incidents", incidents), ("deploys", deploys)],
        &window,
    );

    assert_eq!(
        compiled.sql(),
        format!(
            "SELECT `team`, coalesce(any(`__c0`), 0) AS `incidents`, \
             coalesce(any(`__c1`), 0) AS `deploys`, \
             (`incidents` / nullIf(`deploys`, 0)) AS `incidents_per_deploy` \
             FROM (SELECT `team`, NULL AS `__c0`, `count` AS `__c1` FROM ({deploys_sql}) \
             UNION ALL SELECT `team`, `count` AS `__c0`, NULL AS `__c1` FROM ({incidents_sql})) \
             GROUP BY `team` ORDER BY `incidents_per_deploy` DESC LIMIT 50"
        )
    );
}

#[test]
fn a_windowed_composite_folds_by_the_bucket_each_child_carries() {
    let incidents = per_team("incidents", &[counted()], &[]);
    let deploys = per_team("deploys", &[counted()], &[]);
    let window = last_week(true);
    let incidents_sql = inner_sql(&incidents, &window);
    let deploys_sql = inner_sql(&deploys, &window);

    let compiled = compiled(
        &incidents_per_deploy(),
        &[("incidents", incidents), ("deploys", deploys)],
        &window,
    );

    assert_eq!(
        compiled.sql(),
        format!(
            "SELECT `bucket`, `team`, coalesce(any(`__c0`), 0) AS `incidents`, \
             coalesce(any(`__c1`), 0) AS `deploys`, \
             (`incidents` / nullIf(`deploys`, 0)) AS `incidents_per_deploy` \
             FROM (SELECT `bucket`, `team`, NULL AS `__c0`, `count` AS `__c1` FROM ({deploys_sql}) \
             UNION ALL SELECT `bucket`, `team`, `count` AS `__c0`, NULL AS `__c1` \
             FROM ({incidents_sql})) \
             GROUP BY `bucket`, `team` ORDER BY `incidents_per_deploy` DESC LIMIT 50"
        )
    );
}

#[test]
fn an_unbucketed_window_folds_by_the_keys_alone() {
    let incidents = per_team("incidents", &[counted()], &[]);
    let deploys = per_team("deploys", &[counted()], &[]);

    let compiled = compiled(
        &incidents_per_deploy(),
        &[("incidents", incidents), ("deploys", deploys)],
        &last_week(false),
    );

    assert!(
        compiled.sql().starts_with("SELECT `team`, ")
            && compiled.sql().contains(" GROUP BY `team` ORDER BY"),
        "{}",
        compiled.sql()
    );
}

/// What a pick reads where its child has no row for the key.
#[test]
fn only_a_count_or_a_sum_reads_zero_where_its_child_has_no_row() {
    let incidents = per_team(
        "incidents",
        &[
            counted(),
            json!({ "column": "minutes", "type": "int", "agg": "sum", "as_name": "minutes" }),
            json!({ "column": "minutes", "type": "int", "agg": "avg", "as_name": "mean" }),
            json!({ "column": "minutes", "type": "int", "agg": "max", "as_name": "longest" }),
            json!({ "type": "float", "as_name": "per_incident", "divide": ["minutes", "count"] }),
        ],
        &[],
    );
    let deploys = per_team("deploys", &[counted()], &[]);
    let composite = composite(json!({
        "combine": { "incidents": "incident-counts", "deploys": "deploy-counts" },
        "group_by": ["team"],
        "fields": [
            { "as_name": "count", "field": "incidents.count" },
            { "as_name": "minutes", "field": "incidents.minutes" },
            { "as_name": "mean", "field": "incidents.mean" },
            { "as_name": "longest", "field": "incidents.longest" },
            { "as_name": "per_incident", "field": "incidents.per_incident" },
            { "as_name": "deploys", "field": "deploys.count" }
        ]
    }));

    let compiled = compiled(
        &composite,
        &[("incidents", incidents), ("deploys", deploys)],
        &Window::legacy(),
    );

    for expected in [
        "coalesce(any(`__c0`), 0) AS `count`",
        "coalesce(any(`__c1`), 0) AS `minutes`",
        "any(`__c2`) AS `mean`",
        "any(`__c3`) AS `longest`",
        "any(`__c4`) AS `per_incident`",
        "coalesce(any(`__c5`), 0) AS `deploys`",
    ] {
        assert!(
            compiled.sql().contains(&format!(", {expected}")),
            "should select {expected}: {}",
            compiled.sql()
        );
    }
}

#[test]
fn binds_follow_the_order_the_children_appear_in() {
    let incidents = per_team(
        "incidents",
        &[counted()],
        &[json!({ "column": "severity", "type": "int", "op": "gte", "value": 2 })],
    );
    let deploys = per_team(
        "deploys",
        &[counted()],
        &[
            json!({ "column": "env", "type": "string", "op": "eq", "value": "prod" }),
            json!({ "column": "attempt", "type": "int", "op": "eq", "value": 1 }),
        ],
    );

    let compiled = compiled(
        &incidents_per_deploy(),
        &[("incidents", incidents), ("deploys", deploys)],
        &Window::legacy(),
    );

    assert_eq!(
        format!("{:?}", compiled.binds()),
        r#"[Str("prod"), Int(1), Int(2)]"#
    );
}

#[test]
fn a_composite_naming_no_order_is_ordered_by_its_keys_and_cut_where_a_plain_metric_is() {
    let cases = [
        (json!({}), " ORDER BY `team` LIMIT 1000"),
        (json!({ "limit": 20 }), " ORDER BY `team` LIMIT 20"),
        (json!({ "limit": 50_000 }), " ORDER BY `team` LIMIT 10000"),
        (
            json!({ "order_by": { "field": "bucket" } }),
            " ORDER BY `team` LIMIT 1000",
        ),
        (
            json!({ "order_by": { "field": "deploys" } }),
            " ORDER BY `deploys` ASC LIMIT 1000",
        ),
    ];

    for (extra, tail) in cases {
        let mut body = json!({
            "combine": { "incidents": "incident-counts", "deploys": "deploy-counts" },
            "group_by": ["team"],
            "fields": [
                { "as_name": "incidents", "field": "incidents.count" },
                { "as_name": "deploys", "field": "deploys.count" }
            ]
        });
        for (key, value) in extra.as_object().into_iter().flatten() {
            body[key] = value.clone();
        }

        let compiled = compiled(
            &composite(body.clone()),
            &[
                ("incidents", per_team("incidents", &[counted()], &[])),
                ("deploys", per_team("deploys", &[counted()], &[])),
            ],
            &Window::legacy(),
        );

        assert!(
            compiled.sql().ends_with(tail),
            "should end {tail:?} for {body}: {}",
            compiled.sql()
        );
    }
}

#[test]
fn a_ratio_and_the_picks_say_what_their_columns_hold() {
    let incidents = per_team(
        "incidents",
        &[
            counted(),
            json!({ "column": "minutes", "type": "int", "agg": "sum", "as_name": "minutes" }),
            json!({ "type": "float", "as_name": "share", "divide": ["minutes", "count"], "percent": true }),
        ],
        &[],
    );
    let deploys = per_team("deploys", &[counted()], &[]);
    let composite = composite(json!({
        "combine": { "incidents": "incident-counts", "deploys": "deploy-counts" },
        "group_by": ["team"],
        "fields": [
            { "as_name": "incidents", "field": "incidents.count" },
            { "as_name": "share", "field": "incidents.share" },
            { "as_name": "deploys", "field": "deploys.count" },
            { "as_name": "rate", "divide": ["incidents", "deploys"], "percent": true }
        ]
    }));

    let compiled = compiled(
        &composite,
        &[("incidents", incidents), ("deploys", deploys)],
        &Window::legacy(),
    );

    assert_eq!(compiled.percents(), ["share".to_owned(), "rate".to_owned()]);
    assert!(
        compiled
            .sql()
            .contains(", (100 * (`incidents` / nullIf(`deploys`, 0))) AS `rate`"),
        "{}",
        compiled.sql()
    );
    assert_eq!(compiled.column_type("incidents"), Some(FieldType::Int));
    assert_eq!(compiled.column_type("rate"), Some(FieldType::Float));
    assert_eq!(compiled.column_type("team"), Some(FieldType::String));
}

/// What a body may say is refused before any child is read.
#[test]
fn a_composite_body_that_does_not_hold_together_is_refused() {
    let cases = [
        (json!({ "dataset": "incidents" }), "unknown field `dataset`"),
        (json!({ "filters": [] }), "unknown field `filters`"),
        (
            json!({ "combine": { "only": "incident-counts" } }),
            "combines 1",
        ),
        (
            json!({ "combine": { "bucket": "incident-counts", "deploys": "deploy-counts" } }),
            "`bucket` cannot name a combined metric",
        ),
        (
            json!({ "group_by": ["bucket"] }),
            "`bucket` cannot be a key",
        ),
        (
            json!({ "fields": [
                { "as_name": "incidents", "field": "incidents.count" },
                { "as_name": "deploys", "field": "deploys.count" },
                { "as_name": "rate", "divide": ["incidents", "outages"] }
            ] }),
            "`rate` divides `outages`",
        ),
        (
            json!({ "fields": [
                { "as_name": "rate", "divide": ["incidents", "deploys"] },
                { "as_name": "incidents", "field": "incidents.count" },
                { "as_name": "deploys", "field": "deploys.count" }
            ] }),
            "`rate` divides `incidents`",
        ),
        (
            json!({ "fields": [
                { "as_name": "incidents", "field": "incidents.count", "agg": "sum" }
            ] }),
            "unknown field `agg`",
        ),
        (
            json!({ "fields": [
                { "as_name": "incidents", "field": "incidents.count" },
                { "as_name": "deploys", "field": "outages.count" }
            ] }),
            "`deploys` reads `outages`",
        ),
        (
            json!({ "fields": [{ "as_name": "incidents", "field": "incidents.count" }] }),
            "`deploys` is combined but no field reads it",
        ),
        (
            json!({ "fields": [
                { "as_name": "team", "field": "incidents.count" },
                { "as_name": "deploys", "field": "deploys.count" }
            ] }),
            "`team` cannot name a field",
        ),
        (
            json!({ "fields": [
                { "as_name": "__c1", "field": "incidents.count" },
                { "as_name": "deploys", "field": "deploys.count" }
            ] }),
            "`__c1` cannot name a field",
        ),
        (
            json!({ "fields": [
                { "as_name": "incidents", "field": "incidents.count", "divide": ["a", "b"] },
                { "as_name": "deploys", "field": "deploys.count" }
            ] }),
            "exactly one of `field` or `divide`",
        ),
        (
            json!({ "order_by": { "field": "count" } }),
            "`count` cannot order the rows",
        ),
    ];

    for (overrides, said) in cases {
        let mut body = json!({
            "combine": { "incidents": "incident-counts", "deploys": "deploy-counts" },
            "group_by": ["team"],
            "fields": [
                { "as_name": "incidents", "field": "incidents.count" },
                { "as_name": "deploys", "field": "deploys.count" }
            ]
        });
        for (key, value) in overrides.as_object().into_iter().flatten() {
            body[key] = value.clone();
        }

        let refusal = serde_json::from_value::<Composite>(body.clone());

        let Err(error) = refusal else {
            panic!("should be refused: {body}");
        };
        assert!(
            error.to_string().contains(said),
            "should say {said:?} for {body}: {error}"
        );
    }
}
