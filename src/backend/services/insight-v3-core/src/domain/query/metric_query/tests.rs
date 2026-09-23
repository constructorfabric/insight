use serde_json::json;

use super::field::{MAX_IDENTIFIER_CHARS, coerce_value};
use super::*;
use crate::domain::query::metric_query::TableEngine;
use crate::domain::query::time_window::RequestedRange;

fn query(value: serde_json::Value) -> MetricQuery {
    serde_json::from_value(value).unwrap_or_else(|error| panic!("valid metric: {error}"))
}

fn people() -> People {
    People::new("identity")
}

#[test]
fn a_plain_column_selected_beside_an_aggregate_must_be_grouped() {
    let mixed = query(json!({
        "table": "events",
        "fields": [
            { "json": "author", "type": "string", "as_name": "author" },
            { "json": "lines", "type": "int", "agg": "sum", "as_name": "lines" }
        ],
        "group_by": [],
        "filters": []
    }));

    let refusal = mixed.compile(&people());

    assert!(
        matches!(&refusal, Err(MetricQueryError::Ungrouped(name)) if name == "author"),
        "ClickHouse would refuse this itself, and only after the caller got a 500: {refusal:?}"
    );
}

fn window(token: &str, bucketed: bool) -> crate::domain::query::time_window::Window {
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-10T15:00:00Z")
        .unwrap_or_else(|error| panic!("the synthetic clock parses: {error}"))
        .to_utc();
    let resolved = RequestedRange::parse(token)
        .and_then(|range| range.resolve(now))
        .unwrap_or_else(|error| panic!("`{token}` resolves: {error}"));

    if bucketed {
        resolved
    } else {
        resolved.unbucketed()
    }
}

fn timed_metric(time: &serde_json::Value) -> MetricQuery {
    query(json!({
        "table": "events",
        "time": time,
        "fields": [{ "agg": "count", "type": "int", "as_name": "total" }],
        "filters": []
    }))
}

fn compiled(
    metric: &MetricQuery,
    token: &str,
    bucketed: bool,
    engine: TableEngine,
) -> CompiledQuery {
    metric
        .compile_window(&people(), &window(token, bucketed), engine, None)
        .unwrap_or_else(|error| panic!("`{token}` compiles: {error}"))
}

#[test]
fn json_and_column_clocks_compile_as_datetime_sources() {
    let json_clock = timed_metric(&json!({ "json": "occurred_at" }));
    let column_clock = timed_metric(&json!({ "column": "occurred_at", "type": "datetime" }));

    let json_sql = compiled(&json_clock, "P7D", true, TableEngine::MergeTree).sql;
    let column_sql = compiled(&column_clock, "P7D", true, TableEngine::MergeTree).sql;

    assert!(
        json_sql
            .contains("parseDateTimeBestEffortOrNull(JSONExtractString(raw_data, 'occurred_at'))"),
        "{json_sql}"
    );
    assert!(
        column_sql.contains("toStartOfDay(`occurred_at`, 'UTC') AS `bucket`"),
        "{column_sql}"
    );
}

#[test]
fn a_clock_reads_the_payload_column_it_names() {
    let metric = timed_metric(&json!({
        "column": "event_json", "json": "occurred_at", "type": "datetime"
    }));

    let compiled = metric
        .compile_window(
            &people(),
            &window("P7D", true),
            TableEngine::MergeTree,
            None,
        )
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled.sql.contains(
            "parseDateTimeBestEffortOrNull(JSONExtractString(`event_json`, 'occurred_at'))"
        ),
        "{}",
        compiled.sql
    );
}

#[test]
fn a_clock_reads_a_json_key_by_the_same_rule_a_field_does() {
    let metric = timed_metric(&json!({ "json": "meta.Occurred At", "type": "datetime" }));

    let compiled = compiled(&metric, "P7D", true, TableEngine::MergeTree);

    assert!(
        compiled.sql.contains(
            "parseDateTimeBestEffortOrNull(JSONExtractString(raw_data, 'meta', 'Occurred At'))"
        ),
        "{}",
        compiled.sql
    );
}

#[test]
fn malformed_or_non_datetime_clocks_are_refused() {
    for time in [
        json!({}),
        json!({ "column": "not-safe`", "type": "datetime" }),
        json!({ "column": "at", "type": "string" }),
    ] {
        let metric = timed_metric(&time);
        assert!(
            metric
                .compile_window(
                    &people(),
                    &window("P7D", true),
                    TableEngine::MergeTree,
                    None
                )
                .is_err(),
            "should reject {time}"
        );
    }
}

#[test]
fn a_bucket_is_injected_into_select_grouping_and_default_order() {
    let metric = timed_metric(&json!({ "column": "occurred_at" }));
    let compiled = compiled(&metric, "PQC", true, TableEngine::MergeTree);

    assert!(
        compiled.sql.starts_with(
            "SELECT toStartOfWeek(`occurred_at`, 1, 'UTC') AS `bucket`, count() AS `total`"
        ),
        "{}",
        compiled.sql
    );
    assert!(
        compiled.sql.contains("GROUP BY `bucket` ORDER BY `bucket`"),
        "{}",
        compiled.sql
    );
    assert_eq!(compiled.binds.len(), 2);
}

#[test]
fn an_unbucketed_window_keeps_its_half_open_predicates() {
    let metric = timed_metric(&json!({ "column": "occurred_at" }));
    let compiled = compiled(&metric, "P30D", false, TableEngine::MergeTree);

    assert!(!compiled.sql.contains(" AS `bucket`"), "{}", compiled.sql);
    assert!(
        compiled.sql.contains(
            "WHERE `occurred_at` >= fromUnixTimestamp64Milli(?, 'UTC') AND `occurred_at` < fromUnixTimestamp64Milli(?, 'UTC')"
        ),
        "{}",
        compiled.sql
    );
}

#[test]
fn a_clock_cannot_also_be_a_filter_and_bucket_is_reserved() {
    let filtered = query(json!({
        "table": "events",
        "time": { "json": "occurred_at" },
        "fields": [{ "agg": "count", "type": "int", "as_name": "total" }],
        "filters": [{ "json": "occurred_at", "type": "string", "op": "gte", "value": "synthetic" }]
    }));
    let colliding = query(json!({
        "table": "events",
        "time": { "column": "occurred_at" },
        "fields": [{ "column": "kind", "type": "string", "as_name": "bucket" }],
        "filters": []
    }));

    assert!(matches!(
        filtered.compile_window(
            &people(),
            &window("P7D", true),
            TableEngine::MergeTree,
            None
        ),
        Err(MetricQueryError::ClockFilter(_))
    ));
    assert!(matches!(
        colliding.compile_window(
            &people(),
            &window("P7D", true),
            TableEngine::MergeTree,
            None
        ),
        Err(MetricQueryError::BucketAlias)
    ));
}

#[test]
fn a_clock_cannot_be_reused_by_an_aggregate_condition() {
    let metric = query(json!({
        "table": "events",
        "time": { "column": "occurred_at" },
        "fields": [{
            "agg": "count", "type": "int", "as_name": "total",
            "when": [{ "column": "occurred_at", "type": "string", "op": "gte", "value": "synthetic" }]
        }]
    }));

    assert!(matches!(
        metric.compile_window(
            &people(),
            &window("P7D", true),
            TableEngine::MergeTree,
            None
        ),
        Err(MetricQueryError::ClockFilter(_))
    ));
}

#[test]
fn a_direct_window_on_a_clockless_metric_is_refused() {
    let metric = query(json!({
        "table": "events",
        "fields": [{ "agg": "count", "type": "int", "as_name": "total" }]
    }));

    assert!(matches!(
        metric.compile_window(
            &people(),
            &window("P7D", false),
            TableEngine::MergeTree,
            None
        ),
        Err(MetricQueryError::ClocklessWindow)
    ));
    assert!(metric.compile(&people()).is_ok());
}

#[test]
fn finite_metric_caps_reject_wider_and_unbounded_windows() {
    let metric = query(json!({
        "table": "events",
        "time": { "column": "occurred_at" },
        "max_range": "P30D",
        "fields": [{ "agg": "count", "type": "int", "as_name": "total" }]
    }));

    assert!(
        metric
            .compile_window(
                &people(),
                &window("P30D", false),
                TableEngine::MergeTree,
                None
            )
            .is_ok()
    );
    assert!(matches!(
        metric.compile_window(
            &people(),
            &window("P1Y", false),
            TableEngine::MergeTree,
            None
        ),
        Err(MetricQueryError::RangeExceedsMaximum(_))
    ));
    assert!(matches!(
        metric.compile_window(
            &people(),
            &window("inf", false),
            TableEngine::MergeTree,
            None
        ),
        Err(MetricQueryError::RangeExceedsMaximum(_))
    ));
}

#[test]
fn replacing_engines_alone_compile_with_final() {
    let metric = timed_metric(&json!({ "column": "occurred_at" }));
    let plain = compiled(&metric, "P7D", false, TableEngine::MergeTree);
    let replacing = compiled(&metric, "P7D", false, TableEngine::ReplacingMergeTree);

    assert!(plain.sql.contains("FROM `events` WHERE"), "{}", plain.sql);
    assert!(
        replacing.sql.contains("FROM `events` FINAL WHERE"),
        "{}",
        replacing.sql
    );
}

#[test]
fn a_ratio_over_two_aggregates_needs_no_group_by_of_its_own() {
    let rate = query(json!({
        "table": "events",
        "fields": [
            { "column": "value", "type": "int", "agg": "sum", "as_name": "passed" },
            { "column": "value", "type": "int", "agg": "sum", "as_name": "runs" },
            { "type": "float", "as_name": "rate", "divide": ["passed", "runs"], "percent": true }
        ],
        "group_by": [],
        "filters": []
    }));

    assert!(rate.compile(&people()).is_ok());
}

#[test]
fn final_follows_the_fact_alias_when_identity_joins_require_one() {
    let metric = query(json!({
        "table": "events",
        "time": { "column": "occurred_at" },
        "fields": [
            { "column": "actor", "type": "string", "as_name": "actor", "person": "email" },
            { "agg": "count", "type": "int", "as_name": "total" }
        ],
        "group_by": ["actor"]
    }));

    let compiled = compiled(&metric, "P7D", true, TableEngine::ReplacingMergeTree);

    assert!(
        compiled
            .sql
            .contains("FROM `events` AS `__f` FINAL LEFT JOIN"),
        "{}",
        compiled.sql
    );
}

#[test]
fn an_undated_query_counts_the_rows_without_a_clock() {
    let metric = timed_metric(&json!({ "column": "occurred_at" }));

    let undated = metric
        .undated_query(TableEngine::MergeTree, None)
        .unwrap_or_else(|error| panic!("the undated count compiles: {error}"))
        .unwrap_or_else(|| panic!("a clocked metric has an undated count"));

    assert_eq!(
        undated.sql,
        "SELECT countIf(isNull(`occurred_at`)) AS undated FROM `events`"
    );
}

#[test]
fn an_undated_query_reads_the_same_rows_the_metric_does() {
    let metric = query(json!({
        "table": "events",
        "time": { "column": "occurred_at" },
        "fields": [{ "agg": "count", "type": "int", "as_name": "total" }],
        "filters": [{ "column": "repo", "type": "string", "op": "eq", "value": "one" }]
    }));

    let undated = metric
        .undated_query(TableEngine::ReplacingMergeTree, None)
        .unwrap_or_else(|error| panic!("the undated count compiles: {error}"))
        .unwrap_or_else(|| panic!("a clocked metric has an undated count"));

    assert!(
        undated.sql.contains("FROM `events` FINAL WHERE `repo` = ?"),
        "{}",
        undated.sql
    );
    assert_eq!(undated.binds.len(), 1);
}

#[test]
fn a_clockless_metric_has_no_undated_count_to_read() {
    let metric = timed_metric(&serde_json::Value::Null);

    let undated = metric
        .undated_query(TableEngine::MergeTree, None)
        .unwrap_or_else(|error| panic!("a clockless metric is not an error: {error}"));

    assert!(undated.is_none());
}

#[test]
fn an_all_time_window_leaves_out_the_rows_with_no_clock() {
    let metric = timed_metric(&json!({ "column": "occurred_at" }));

    let compiled = compiled(&metric, "inf", true, TableEngine::MergeTree);

    assert!(
        compiled.sql.contains("WHERE `occurred_at` IS NOT NULL"),
        "{}",
        compiled.sql
    );
}

#[test]
fn a_json_clock_reads_a_missing_key_as_no_clock_rather_than_a_failure() {
    let metric = timed_metric(&json!({ "json": "occurred_at" }));

    let compiled = compiled(&metric, "P7D", true, TableEngine::MergeTree);

    assert!(
        compiled
            .sql
            .contains("parseDateTimeBestEffortOrNull(JSONExtractString(raw_data, 'occurred_at'))"),
        "{}",
        compiled.sql
    );
}

/// Whether a run has a date to bucket by is the caller's to say: a metric
/// over a dataset may inherit one the body never names.
#[test]
fn only_a_clocked_run_exposes_the_injected_bucket_column() {
    let metric = timed_metric(&json!({ "column": "occurred_at" }));

    assert_eq!(metric.column_names(true), ["bucket", "total"]);
    assert_eq!(metric.column_names(false), ["total"]);
}

fn merged_by_author(person: &str) -> MetricQuery {
    query(json!({
        "table": "silver.class_git_pull_requests",
        "fields": [
            { "column": "author_email", "type": "string", "as_name": "author", "person": person },
            { "agg": "count", "column": "pr_id", "type": "int", "as_name": "merged" }
        ],
        "group_by": ["author"],
        "filters": [{ "column": "state", "type": "string", "op": "eq", "value": "MERGED" }],
        "order_by": { "field": "merged", "direction": "desc" }
    }))
}

#[test]
fn a_field_aggregates_only_what_its_own_condition_matches() {
    // A rate's two halves live in one column, told apart by another: the
    // numerator and the denominator cannot each have their own query.
    let metric = query(json!({
        "database": "insight",
        "table": "ci_metric_observations",
        "fields": [
            { "column": "value", "type": "int", "agg": "sum", "as_name": "passed",
              "when": [{ "column": "measure_key", "type": "string", "op": "eq",
                         "value": "gate_passed" }] },
            { "column": "value", "type": "int", "agg": "sum", "as_name": "runs",
              "when": [{ "column": "measure_key", "type": "string", "op": "eq",
                         "value": "gate_runs" }] }
        ],
        "group_by": [],
        "filters": []
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled
            .sql
            .contains("sumIf(`value`, `measure_key` = ?) AS `passed`"),
        "{}",
        compiled.sql
    );
    assert_eq!(
        compiled.binds,
        vec!["gate_passed".to_owned(), "gate_runs".to_owned()]
    );
}

#[test]
fn a_condition_on_a_count_needs_no_column() {
    let metric = query(json!({
        "table": "events",
        "fields": [
            { "agg": "count", "type": "int", "as_name": "merged",
              "when": [{ "json": "state", "type": "string", "op": "eq",
                         "value": "MERGED" }] }
        ],
        "group_by": [],
        "filters": []
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled
            .sql
            .contains("countIf(JSONExtractString(raw_data, 'state') = ?) AS `merged`"),
        "{}",
        compiled.sql
    );
}

#[test]
fn a_ratio_divides_two_of_the_querys_own_fields() {
    let metric = query(json!({
        "database": "insight",
        "table": "ci_metric_observations",
        "fields": [
            { "column": "value", "type": "int", "agg": "sum", "as_name": "passed" },
            { "column": "value", "type": "int", "agg": "sum", "as_name": "runs" },
            { "divide": ["passed", "runs"], "type": "float", "as_name": "pass_rate" }
        ],
        "group_by": [],
        "filters": []
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    // nullIf, so a denominator of zero is no rate rather than an error.
    assert!(
        compiled
            .sql
            .contains("(`passed` / nullIf(`runs`, 0)) AS `pass_rate`"),
        "{}",
        compiled.sql
    );
}

#[test]
fn a_ratio_can_read_as_a_percentage() {
    let metric = query(json!({
        "table": "events",
        "fields": [
            { "agg": "count", "type": "int", "as_name": "part" },
            { "agg": "count", "type": "int", "as_name": "whole" },
            { "divide": ["part", "whole"], "percent": true, "type": "float",
              "as_name": "share" }
        ],
        "group_by": [],
        "filters": []
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled
            .sql
            .contains("(100 * (`part` / nullIf(`whole`, 0))) AS `share`"),
        "{}",
        compiled.sql
    );
    // Named, so whoever draws 83.9 can draw 83.9% instead.
    assert_eq!(compiled.percents, ["share"]);
}

#[test]
fn a_ratio_naming_a_field_that_is_not_there_is_refused() {
    let metric = query(json!({
        "table": "events",
        "fields": [
            { "agg": "count", "type": "int", "as_name": "part" },
            { "divide": ["part", "nothing_like_this"], "type": "float",
              "as_name": "share" }
        ],
        "group_by": [],
        "filters": []
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::Ratio(_))
    ));
}

#[test]
fn a_ratio_reading_a_field_declared_after_it_is_refused() {
    // The alias only exists once it has been selected, so the order in the
    // field list is the order the SQL can resolve.
    let metric = query(json!({
        "table": "events",
        "fields": [
            { "divide": ["part", "whole"], "type": "float", "as_name": "share" },
            { "agg": "count", "type": "int", "as_name": "part" },
            { "agg": "count", "type": "int", "as_name": "whole" }
        ],
        "group_by": [],
        "filters": []
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::Ratio(_))
    ));
}

#[test]
fn a_ratio_needs_exactly_two_fields_to_divide() {
    let metric = query(json!({
        "table": "events",
        "fields": [
            { "agg": "count", "type": "int", "as_name": "part" },
            { "divide": ["part"], "type": "float", "as_name": "share" }
        ],
        "group_by": [],
        "filters": []
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::Ratio(_))
    ));
}

#[test]
fn a_field_condition_binds_before_the_query_wide_filters() {
    // The select list comes before the WHERE clause, so its values bind
    // first or every placeholder after it takes the wrong one.
    let metric = query(json!({
        "database": "insight",
        "table": "ci_metric_observations",
        "fields": [
            { "column": "value", "type": "int", "agg": "sum", "as_name": "passed",
              "when": [{ "column": "measure_key", "type": "string", "op": "eq",
                         "value": "gate_passed" }] }
        ],
        "group_by": [],
        "filters": [
            { "column": "metric_date", "type": "string", "op": "gte",
              "value": "2026-08-09" }
        ]
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert_eq!(
        compiled.binds,
        vec!["gate_passed".to_owned(), "2026-08-09".to_owned()]
    );
}

#[test]
fn a_person_column_selects_the_name_identity_knows() {
    let compiled = merged_by_author("email")
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled.sql.contains(
            "coalesce(nullIf(`__p0`.`display_name`, ''), `__f`.`author_email`) AS `author`"
        ),
        "{}",
        compiled.sql
    );
    assert!(
        compiled.sql.contains(
            "LEFT JOIN __people_by_email AS `__p0` ON `__p0`.`handle` = `__f`.`author_email`"
        ),
        "{}",
        compiled.sql
    );
    assert!(
        compiled
            .sql
            .contains("FROM `silver`.`class_git_pull_requests` AS `__f`"),
        "{}",
        compiled.sql
    );
}

#[test]
fn the_latest_name_wins_before_anything_joins_to_it() {
    // A person carries a row per name they have ever had. Joining those
    // rows directly multiplies every fact by that history.
    let compiled = merged_by_author("email")
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled.sql.starts_with(
            "WITH __person_name AS (SELECT `person_id`, argMax(`value_effective`, `created_at`) AS `display_name` FROM `identity`.`identity_persons` WHERE `value_type` = 'display_name' GROUP BY `person_id`)"
        ),
        "{}",
        compiled.sql
    );
}

#[test]
fn resolving_a_name_qualifies_every_other_read() {
    let compiled = merged_by_author("email")
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled.sql.contains("count(`__f`.`pr_id`) AS `merged`"),
        "{}",
        compiled.sql
    );
    assert!(
        compiled.sql.contains("WHERE `__f`.`state` = ?"),
        "{}",
        compiled.sql
    );
    assert_eq!(compiled.binds, vec!["MERGED".to_owned()]);
}

#[test]
fn a_query_that_names_no_person_joins_nothing() {
    let compiled = query(json!({
        "table": "silver.class_git_pull_requests",
        "fields": [{ "column": "author_email", "type": "string", "as_name": "author" }],
        "group_by": ["author"],
        "filters": []
    }))
    .compile(&people())
    .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(!compiled.sql.contains("WITH "), "{}", compiled.sql);
    assert!(!compiled.sql.contains("JOIN"), "{}", compiled.sql);
    assert!(!compiled.sql.contains("__f"), "{}", compiled.sql);
}

#[test]
fn a_person_id_is_compared_as_text() {
    // The same column is a UUID in one table and a String in the next.
    let compiled = query(json!({
        "table": "silver.class_git_pull_requests",
        "fields": [
            { "column": "author_person_id", "type": "string", "as_name": "author", "person": "id" },
            { "agg": "count", "type": "int", "as_name": "prs" }
        ],
        "group_by": ["author"],
        "filters": []
    }))
    .compile(&people())
    .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled.sql.contains(
            "LEFT JOIN __people_by_id AS `__p0` ON `__p0`.`handle` = toString(`__f`.`author_person_id`)"
        ),
        "{}",
        compiled.sql
    );
    assert!(
        compiled
            .sql
            .contains("__people_by_id AS (SELECT toString(`person_id`) AS `handle`"),
        "{}",
        compiled.sql
    );
}

#[test]
fn two_person_columns_resolve_one_each() {
    let compiled = query(json!({
        "table": "silver.class_git_pr_review_events",
        "fields": [
            { "column": "author_email", "type": "string", "as_name": "author", "person": "email" },
            { "column": "actor_person_id", "type": "string", "as_name": "reviewer", "person": "id" },
            { "agg": "count", "type": "int", "as_name": "reviews" }
        ],
        "group_by": ["author", "reviewer"],
        "filters": []
    }))
    .compile(&people())
    .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(compiled.sql.contains("AS `__p0`"), "{}", compiled.sql);
    assert!(compiled.sql.contains("AS `__p1`"), "{}", compiled.sql);
    assert_eq!(
        compiled.sql.matches("LEFT JOIN").count(),
        2,
        "{}",
        compiled.sql
    );
}

#[test]
fn a_person_inside_the_payload_resolves_the_same_way() {
    let compiled = query(json!({
        "table": "events",
        "fields": [
            { "json": "author", "type": "string", "as_name": "author", "person": "email" },
            { "agg": "sum", "json": "lines", "type": "int", "as_name": "lines" }
        ],
        "group_by": ["author"],
        "filters": []
    }))
    .compile(&people())
    .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled
            .sql
            .contains("ON `__p0`.`handle` = JSONExtractString(`__f`.raw_data, 'author')"),
        "{}",
        compiled.sql
    );
}

#[test]
fn the_identity_database_is_whatever_the_stand_configured() {
    let compiled = merged_by_author("email")
        .compile(&People::new("identity_two"))
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled.sql.contains("`identity_two`.`identity_persons`"),
        "{}",
        compiled.sql
    );
}

#[test]
fn an_identity_database_outside_the_charset_is_refused() {
    assert!(matches!(
        merged_by_author("email").compile(&People::new("identity`; DROP TABLE x; --")),
        Err(MetricQueryError::Identifier(_))
    ));
}

#[test]
fn a_grouped_count_compiles_to_json_extraction_over_the_payload() {
    let metric = query(json!({
        "table": "events",
        "fields": [
            { "json": "day", "type": "string", "as_name": "day" },
            { "json": "lines", "type": "int", "agg": "sum", "as_name": "lines" }
        ],
        "group_by": ["day"],
        "filters": [],
        "limit": 100
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert_eq!(
        compiled.sql,
        "SELECT JSONExtractString(raw_data, 'day') AS `day`, \
         sum(JSONExtractInt(raw_data, 'lines')) AS `lines` \
         FROM `events` GROUP BY `day` ORDER BY `day` LIMIT 100"
    );
    assert!(compiled.binds.is_empty());
}

#[test]
fn filters_bind_their_values() {
    let metric = query(json!({
        "table": "events",
        "fields": [{ "json": "author", "type": "string", "as_name": "author" }],
        "group_by": [],
        "filters": [
            { "json": "event", "type": "string", "op": "eq", "value": "commit" }
        ]
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled
            .sql
            .contains("WHERE JSONExtractString(raw_data, 'event') = ?")
    );
    assert_eq!(compiled.binds, vec!["commit".to_owned()]);
}

#[test]
fn an_identifier_outside_the_charset_is_refused() {
    let metric = query(json!({
        "table": "events`; DROP TABLE events; --",
        "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
        "group_by": [],
        "filters": []
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::Identifier(_))
    ));
}

#[test]
fn ordering_by_a_selected_value_beats_the_grouping_order() {
    // "Which author changed the most lines" is unanswerable without
    // this: ordering by the grouped column returns whoever sorts first
    // alphabetically, and the reply presents it as the largest.
    let metric = query(json!({
        "table": "events",
        "fields": [
            { "json": "author", "type": "string", "as_name": "author" },
            { "json": "lines", "type": "int", "agg": "sum", "as_name": "total_lines" }
        ],
        "group_by": ["author"],
        "filters": [],
        "order_by": { "field": "total_lines", "direction": "desc" },
        "limit": 1
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("the query compiles: {error}"));

    assert!(
        compiled.sql.contains("ORDER BY `total_lines` DESC"),
        "{}",
        compiled.sql
    );
    // One ORDER BY, not the grouping's as well.
    assert_eq!(
        compiled.sql.matches("ORDER BY").count(),
        1,
        "{}",
        compiled.sql
    );
    assert!(compiled.sql.ends_with(" LIMIT 1"), "{}", compiled.sql);
}

#[test]
fn ordering_defaults_to_ascending() {
    let metric = query(json!({
        "table": "events",
        "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
        "group_by": [],
        "filters": [],
        "order_by": { "field": "day" }
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("the query compiles: {error}"));

    assert!(
        compiled.sql.contains("ORDER BY `day` ASC"),
        "{}",
        compiled.sql
    );
}

#[test]
fn ordering_by_a_column_the_query_does_not_select_is_refused() {
    let metric = query(json!({
        "table": "events",
        "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
        "group_by": [],
        "filters": [],
        "order_by": { "field": "lines" }
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::OrderBy(_))
    ));
}

#[test]
fn counting_rows_needs_no_column() {
    // "How many rows are in this table" is the first thing anyone asks,
    // and it reads no value.
    let metric = query(json!({
        "table": "bronze_github.commits",
        "fields": [{ "agg": "count", "type": "int", "as_name": "rows" }],
        "group_by": [],
        "filters": []
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("the query compiles: {error}"));

    assert!(
        compiled.sql.contains("count() AS `rows`"),
        "{}",
        compiled.sql
    );
}

#[test]
fn counting_one_column_still_names_it() {
    let metric = query(json!({
        "table": "events",
        "fields": [{ "agg": "count", "column": "author", "type": "string", "as_name": "n" }],
        "group_by": [],
        "filters": []
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("the query compiles: {error}"));

    assert!(
        compiled.sql.contains("count(`author`) AS `n`"),
        "{}",
        compiled.sql
    );
}

#[test]
fn an_aggregate_that_is_not_count_still_needs_a_source() {
    // sum() of nothing is not a question.
    let metric = query(json!({
        "table": "events",
        "fields": [{ "agg": "sum", "type": "int", "as_name": "total" }],
        "group_by": [],
        "filters": []
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::FieldSource(_))
    ));
}

#[test]
fn a_table_written_with_its_database_is_read_as_both() {
    // The map and the lookup tool both address a table as
    // `database.table`, so the model writes it that way.
    let metric = query(json!({
        "table": "bronze_github.commits",
        "fields": [{ "column": "sha", "type": "string", "as_name": "sha" }],
        "group_by": [],
        "filters": []
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("the query compiles: {error}"));

    assert!(
        compiled.sql.contains("FROM `bronze_github`.`commits`"),
        "{}",
        compiled.sql
    );
    assert_eq!(metric.database(), Some("bronze_github"));
}

#[test]
fn a_database_field_wins_over_a_qualified_table() {
    let metric = query(json!({
        "database": "silver",
        "table": "class_git_commits",
        "fields": [{ "column": "sha", "type": "string", "as_name": "sha" }],
        "group_by": [],
        "filters": []
    }));

    assert_eq!(metric.database(), Some("silver"));
    assert!(
        metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("the query compiles: {error}"))
            .sql
            .contains("FROM `silver`.`class_git_commits`")
    );
}

#[test]
fn a_table_with_two_dots_is_still_refused() {
    // Splitting only rescues the one shape the model writes; anything
    // else stays whole and fails the identifier check.
    let metric = query(json!({
        "table": "a.b.c",
        "fields": [{ "column": "x", "type": "string", "as_name": "x" }],
        "group_by": [],
        "filters": []
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::Identifier(_))
    ));
}

#[test]
fn a_metric_with_no_fields_is_refused() {
    let metric = query(json!({
        "table": "events", "fields": [], "group_by": [], "filters": []
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::NoFields)
    ));
}

#[test]
fn numeric_filters_bind_the_numeric_value_not_its_json_encoding() {
    let metric = query(json!({
        "table": "events",
        "fields": [{ "json": "lines", "type": "int", "as_name": "lines" }],
        "group_by": [],
        "filters": [
            { "json": "lines", "type": "int", "op": "gt", "value": 5 }
        ]
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert_eq!(compiled.binds, vec!["5".to_owned()]);
}

#[test]
fn a_filter_value_that_does_not_match_its_declared_type_is_refused() {
    let metric = query(json!({
        "table": "events",
        "fields": [{ "json": "lines", "type": "int", "as_name": "lines" }],
        "group_by": [],
        "filters": [
            { "json": "lines", "type": "int", "op": "gt", "value": "not-a-number" }
        ]
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::FilterValue(_))
    ));
}

#[test]
fn coerces_string_typed_clickhouse_numbers_to_json_numbers() {
    assert_eq!(coerce_value(json!("132"), FieldType::Int), json!(132));
    assert_eq!(coerce_value(json!("12.5"), FieldType::Float), json!(12.5));
    assert_eq!(
        coerce_value(json!("2026-09-01"), FieldType::String),
        json!("2026-09-01")
    );
}

#[test]
fn group_by_must_reference_a_selected_field() {
    let metric = query(json!({
        "table": "events",
        "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
        "group_by": ["not_selected"],
        "filters": []
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::GroupBy(_))
    ));
}

#[test]
fn a_database_qualifies_the_table() {
    let metric = query(json!({
        "database": "silver",
        "table": "git_commits",
        "fields": [{ "column": "author", "type": "string", "as_name": "author" }]
    }));

    assert_eq!(metric.database(), Some("silver"));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled.sql.contains("FROM `silver`.`git_commits`"),
        "{}",
        compiled.sql
    );
}

#[test]
fn a_query_without_a_database_still_reads_the_bare_table() {
    let metric = query(json!({
        "table": "events",
        "fields": [{ "json": "day", "type": "string", "as_name": "day" }]
    }));

    assert_eq!(metric.database(), None);

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(compiled.sql.contains("FROM `events`"), "{}", compiled.sql);
}

#[test]
fn a_field_naming_a_column_reads_it_without_json_extraction() {
    let metric = query(json!({
        "database": "silver",
        "table": "git_commits",
        "fields": [{ "column": "lines_changed", "type": "int", "as_name": "lines" }]
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled.sql.contains("`lines_changed` AS `lines`"),
        "{}",
        compiled.sql
    );
    assert!(!compiled.sql.contains("JSONExtract"), "{}", compiled.sql);
}

#[test]
fn an_aggregate_wraps_a_column_as_it_wraps_an_extraction() {
    let metric = query(json!({
        "database": "silver",
        "table": "git_commits",
        "fields": [
            { "column": "lines_changed", "type": "int", "agg": "sum", "as_name": "total" }
        ]
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled.sql.contains("sum(`lines_changed`) AS `total`"),
        "{}",
        compiled.sql
    );
}

#[test]
fn a_filter_on_a_column_compares_it_and_still_binds_the_value() {
    let metric = query(json!({
        "database": "silver",
        "table": "git_commits",
        "fields": [{ "column": "author", "type": "string", "as_name": "author" }],
        "filters": [
            { "column": "event", "type": "string", "op": "eq", "value": "commit" }
        ]
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled.sql.contains("WHERE `event` = ?"),
        "{}",
        compiled.sql
    );
    assert_eq!(compiled.binds, vec!["commit".to_owned()]);
}

#[test]
fn a_where_over_a_plain_column_is_refused() {
    let metric = query(json!({
        "table": "project_items",
        "fields": [{
            "column": "milestone_title",
            "type": "string",
            "as_name": "milestone",
            "where": { "json": "field.name", "type": "string", "op": "eq", "value": "Status" }
        }]
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::Selector(name)) if name == "milestone"
    ));
}

#[test]
fn a_field_naming_neither_a_json_key_nor_a_column_is_refused() {
    let metric = query(json!({
        "table": "git_commits",
        "fields": [{ "type": "int", "as_name": "lines" }]
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::FieldSource(_))
    ));
}

#[test]
fn a_filter_naming_neither_a_json_key_nor_a_column_is_refused() {
    let metric = query(json!({
        "table": "git_commits",
        "fields": [{ "column": "author", "type": "string", "as_name": "author" }],
        "filters": [{ "type": "string", "op": "eq", "value": "commit" }]
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::FieldSource(_))
    ));
}

#[test]
fn a_database_outside_the_identifier_charset_is_refused() {
    let metric = query(json!({
        "database": "silver`; DROP TABLE git_commits; --",
        "table": "git_commits",
        "fields": [{ "json": "author", "type": "string", "as_name": "author" }]
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::Identifier(_))
    ));
}
#[test]
fn a_json_field_reads_the_payload_column_it_names() {
    let metric = query(json!({
        "table": "project_items",
        "fields": [
            { "column": "field_values_json", "json": "status", "type": "string", "as_name": "status" }
        ],
        "group_by": [],
        "filters": []
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled
            .sql
            .contains("JSONExtractString(`field_values_json`, 'status')"),
        "{}",
        compiled.sql
    );
}

#[test]
fn a_dotted_json_path_reads_a_nested_key() {
    let metric = query(json!({
        "table": "events",
        "fields": [
            { "json": "field.name", "type": "string", "as_name": "field_name" }
        ],
        "group_by": [],
        "filters": []
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled
            .sql
            .contains("JSONExtractString(raw_data, 'field', 'name')"),
        "{}",
        compiled.sql
    );
}

#[test]
fn a_where_picks_the_one_array_element_the_field_means() {
    let metric = query(json!({
        "table": "project_items",
        "fields": [{
            "column": "field_values_json",
            "json": "name",
            "type": "string",
            "as_name": "status",
            "where": { "json": "field.name", "type": "string", "op": "eq", "value": "Status" }
        }],
        "group_by": [],
        "filters": []
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled.sql.contains(
            "JSONExtractString(arrayFirst(x -> JSONExtractString(x, 'field', 'name') = ?, \
             JSONExtractArrayRaw(ifNull(`field_values_json`, ''))), 'name')"
        ),
        "{}",
        compiled.sql
    );
    assert_eq!(compiled.binds, vec!["Status".to_owned()]);
}

#[test]
fn a_json_key_may_hold_the_spaces_and_sigils_a_payload_names_it_with() {
    let metric = query(json!({
        "table": "launches",
        "fields": [
            { "json": "environment.Region Name", "type": "string", "as_name": "region" },
            { "json": "$type", "type": "string", "as_name": "kind" }
        ],
        "group_by": ["region", "kind"],
        "filters": []
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled
            .sql
            .contains("JSONExtractString(raw_data, 'environment', 'Region Name')"),
        "{}",
        compiled.sql
    );
    assert!(
        compiled
            .sql
            .contains("JSONExtractString(raw_data, '$type')"),
        "{}",
        compiled.sql
    );
}

#[test]
fn a_json_key_cannot_break_out_of_the_literal_that_carries_it() {
    let metric = query(json!({
        "table": "events",
        "fields": [
            { "json": "field.name'); DROP TABLE events; --", "type": "string", "as_name": "bad" }
        ],
        "group_by": [],
        "filters": []
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled
            .sql
            .contains(r"JSONExtractString(raw_data, 'field', 'name\'); DROP TABLE events; --')"),
        "{}",
        compiled.sql
    );
}

#[test]
fn a_json_key_holding_a_placeholder_does_not_consume_a_bind() {
    let metric = query(json!({
        "table": "events",
        "fields": [{ "json": "who?", "type": "string", "as_name": "who" }],
        "group_by": ["who"],
        "filters": [{ "json": "state", "type": "string", "op": "eq", "value": "open" }]
    }));

    let compiled = metric
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

    assert!(
        compiled
            .sql
            .contains("JSONExtractString(raw_data, 'who??')"),
        "{}",
        compiled.sql
    );
    assert_eq!(compiled.binds, vec!["open".to_owned()]);
}

#[test]
fn a_json_key_longer_than_the_cap_is_refused() {
    let metric = query(json!({
        "table": "events",
        "fields": [
            { "json": "a".repeat(MAX_IDENTIFIER_CHARS + 1), "type": "string", "as_name": "bad" }
        ],
        "group_by": [],
        "filters": []
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::JsonKey(_))
    ));
}

#[test]
fn an_empty_json_key_is_refused() {
    let metric = query(json!({
        "table": "events",
        "fields": [{ "json": "field..name", "type": "string", "as_name": "bad" }],
        "group_by": [],
        "filters": []
    }));

    assert!(matches!(
        metric.compile(&people()),
        Err(MetricQueryError::JsonKey(_))
    ));
}
