use serde_json::json;

use super::*;
use crate::domain::query::metric_query::{MetricQuery, People};
use crate::domain::query::time_window::{RequestedRange, Window};

fn declaration() -> Declaration {
    serde_json::from_value(json!({
        "title": "Commits",
        "fields": [
            { "name": "day", "path": "day", "type": "datetime", "default_clock": true },
            { "name": "merged", "path": "merged", "type": "datetime" },
            { "name": "lines", "path": "changed.lines", "type": "int" },
            { "name": "author", "path": "author", "type": "string" }
        ],
        "row_identity": ["author"]
    }))
    .unwrap_or_else(|error| panic!("the fixture declaration parses: {error}"))
}

fn metric(value: serde_json::Value) -> MetricQuery {
    serde_json::from_value(value)
        .unwrap_or_else(|error| panic!("the fixture metric parses: {error}"))
}

fn lines_per_author() -> serde_json::Value {
    json!({
        "dataset": "commits",
        "fields": [
            { "field": "author", "type": "string", "as_name": "author" },
            { "field": "lines", "type": "int", "agg": "sum", "as_name": "total" }
        ],
        "group_by": ["author"],
        "filters": []
    })
}

fn people() -> People {
    People::new("identity")
}

fn compiled(body: serde_json::Value, window: &Window) -> String {
    let written = metric(body);
    let declared = declaration();
    let over = Over {
        declaration: &declared,
        database: "insight_datasets",
        table: "ds_commits_1",
    };

    written
        .compile_over(&people(), window, over)
        .unwrap_or_else(|error| panic!("the metric compiles: {error}"))
        .sql
}

fn requested(token: &str) -> Window {
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-10T15:00:00Z")
        .unwrap_or_else(|error| panic!("the synthetic clock parses: {error}"))
        .to_utc();

    RequestedRange::parse(token)
        .and_then(|range| range.resolve(now))
        .unwrap_or_else(|error| panic!("`{token}` resolves: {error}"))
}

#[test]
fn a_metric_reads_the_table_the_dataset_names_and_not_one_of_its_own() {
    let sql = compiled(lines_per_author(), &Window::legacy());

    assert!(sql.contains("ds_commits_1"), "{sql}");
    assert!(
        !sql.contains("unused"),
        "the metric's own relation is ignored: {sql}"
    );
}

#[test]
fn a_metric_reads_one_record_per_identity() {
    let sql = compiled(lines_per_author(), &Window::legacy());

    assert!(sql.contains("LIMIT 1 BY"), "{sql}");
    assert!(sql.contains("ORDER BY received_at DESC, id DESC"), "{sql}");
}

#[test]
fn a_field_is_read_from_where_the_declaration_says_it_sits() {
    let sql = compiled(lines_per_author(), &Window::legacy());

    assert!(
        sql.contains("JSONExtract(raw_data, 'changed', 'lines', 'Nullable(Int64)')"),
        "{sql}"
    );
}

#[test]
fn a_filter_compares_the_declared_field_and_binds_its_value() {
    let mut body = lines_per_author();
    body["filters"] = json!([{ "field": "author", "type": "string", "op": "eq", "value": "ada" }]);

    let sql = compiled(body, &Window::legacy());

    assert!(
        sql.contains("JSONExtract(raw_data, 'author', 'Nullable(String)') = ?"),
        "{sql}"
    );
}

#[test]
fn a_window_selects_by_the_datasets_main_date_when_the_metric_names_none() {
    let sql = compiled(lines_per_author(), &requested("P7D"));

    assert!(sql.contains("parseDateTime64BestEffortOrNull"), "{sql}");
    assert!(sql.contains("'day'"), "{sql}");
    assert!(!sql.contains("'merged'"), "{sql}");
}

#[test]
fn a_window_selects_by_the_date_the_metric_names_over_the_datasets_own() {
    let mut body = lines_per_author();
    body["time"] = json!({ "field": "merged" });

    let sql = compiled(body, &requested("P7D"));

    assert!(sql.contains("'merged'"), "{sql}");
}

#[test]
fn a_range_over_a_dataset_that_marks_no_main_date_is_refused() {
    let mut declared = declaration();
    declared.fields[0].default_clock = false;
    let written = metric(lines_per_author());
    let over = Over {
        declaration: &declared,
        database: "insight_datasets",
        table: "ds_commits_1",
    };

    let refused = written.compile_over(&people(), &requested("P7D"), over);

    assert!(
        matches!(refused, Err(MetricQueryError::ClocklessWindow)),
        "{refused:?}"
    );
}

#[test]
fn a_field_the_dataset_never_declared_is_refused_by_name() {
    let mut body = lines_per_author();
    body["fields"][0]["field"] = json!("committer");
    let written = metric(body);
    let declared = declaration();
    let over = Over {
        declaration: &declared,
        database: "insight_datasets",
        table: "ds_commits_1",
    };

    let refused = written.compile_over(&people(), &Window::legacy(), over);

    assert!(
        matches!(&refused, Err(MetricQueryError::UnknownField(named)) if named == "committer"),
        "{refused:?}"
    );
}

#[test]
fn the_records_a_window_left_out_are_counted_over_the_same_relation_as_the_rows() {
    let written = metric(lines_per_author());
    let declared = declaration();
    let over = Over {
        declaration: &declared,
        database: "insight_datasets",
        table: "ds_commits_1",
    };

    let counted = written
        .undated_query(
            crate::domain::query::metric_query::TableEngine::Other,
            Some(over),
        )
        .unwrap_or_else(|error| panic!("the count compiles: {error}"))
        .unwrap_or_else(|| panic!("a dataset with a main date has a count"));

    assert!(counted.sql.contains("LIMIT 1 BY"), "{}", counted.sql);
    assert!(counted.sql.contains("countIf(isNull("), "{}", counted.sql);
}

/// Which field holds a person is the declaration's to say, so a metric only
/// selects it and the join follows.
#[test]
fn a_declared_person_is_resolved_to_the_name_a_reader_knows() {
    let declared: Declaration = serde_json::from_value(json!({
        "title": "Commits",
        "fields": [
            { "name": "author", "path": "author", "type": "string", "person": "email" },
            { "name": "lines", "path": "lines", "type": "int" }
        ]
    }))
    .unwrap_or_else(|error| panic!("the fixture declaration parses: {error}"));
    let written = metric(json!({
        "dataset": "commits",
        "fields": [
            { "field": "author", "type": "string", "as_name": "author" },
            { "field": "lines", "type": "int", "agg": "sum", "as_name": "total" }
        ],
        "group_by": ["author"]
    }));
    let over = Over {
        declaration: &declared,
        database: "insight_datasets",
        table: "ds_commits_1",
    };

    let sql = written
        .compile_over(&people(), &Window::legacy(), over)
        .unwrap_or_else(|error| panic!("a declared person compiles: {error}"))
        .sql;

    assert!(sql.contains("LEFT JOIN"), "{sql}");
    assert!(sql.contains("display_name"), "{sql}");
}

/// A substitute is what a reader is shown, never what a lookup is keyed on:
/// two records with no author would otherwise resolve to one person.
#[test]
fn a_person_is_looked_up_by_the_record_s_own_value() {
    let declared: Declaration = serde_json::from_value(json!({
        "title": "Commits",
        "fields": [{
            "name": "author", "path": "author", "type": "string",
            "person": "email", "absent_value": "nobody"
        }]
    }))
    .unwrap_or_else(|error| panic!("the fixture declaration parses: {error}"));
    let written = metric(json!({
        "dataset": "commits",
        "fields": [{ "field": "author", "type": "string", "as_name": "author" }]
    }));
    let over = Over {
        declaration: &declared,
        database: "insight_datasets",
        table: "ds_commits_1",
    };

    let sql = written
        .compile_over(&people(), &Window::legacy(), over)
        .unwrap_or_else(|error| panic!("a declared person compiles: {error}"))
        .sql;
    let Some((shown, key)) = sql.split_once("LEFT JOIN") else {
        panic!("the join is there: {sql}");
    };

    assert!(
        !key.contains("nobody"),
        "the join key is the raw value: {key}"
    );
    assert!(
        shown.contains("nobody"),
        "the reader sees the substitute: {shown}"
    );
}
