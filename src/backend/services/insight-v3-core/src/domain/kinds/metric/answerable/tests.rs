use serde_json::json;

use super::*;

fn declaration() -> Declaration {
    serde_json::from_value(json!({
        "title": "Commits",
        "fields": [
            { "name": "day", "path": "day", "type": "datetime", "default_clock": true },
            { "name": "lines", "path": "lines", "type": "int" },
            { "name": "author", "path": "author", "type": "string" },
            { "name": "project_id", "path": "project_id", "type": "int" }
        ]
    }))
    .unwrap_or_else(|error| panic!("the fixture declaration parses: {error}"))
}

fn metric(value: serde_json::Value) -> MetricQuery {
    serde_json::from_value(value)
        .unwrap_or_else(|error| panic!("the fixture metric parses: {error}"))
}

/// A metric the fixture dataset answers, as a base for the cases below.
fn sound() -> serde_json::Value {
    json!({
        "dataset": "commits",
        "table": "commits",
        "fields": [
            { "field": "author", "type": "string", "as_name": "author" },
            { "field": "lines", "type": "int", "agg": "sum", "as_name": "total" }
        ],
        "group_by": ["author"],
        "filters": []
    })
}

#[test]
fn a_metric_whose_every_reference_is_declared_is_answerable() {
    assert_eq!(check(&metric(sound()), &declaration()), Vec::new());
}

#[test]
fn a_field_the_dataset_never_declared_is_named_along_with_the_ones_it_did() {
    let mut body = sound();
    body["fields"][0]["field"] = json!("committer");

    let violations = check(&metric(body), &declaration());

    let [violation] = violations.as_slice() else {
        panic!("one violation, got {violations:?}");
    };
    assert_eq!(violation.field, "fields[0].field");
    assert_eq!(violation.reason, Reason::Unknown);
    assert!(violation.detail.contains("`author`"), "{violation:?}");
}

#[test]
fn only_a_number_is_summed_or_averaged() {
    for aggregate in ["sum", "avg"] {
        let mut body = sound();
        body["fields"][1]["field"] = json!("author");
        body["fields"][1]["agg"] = json!(aggregate);

        let violations = check(&metric(body), &declaration());

        assert!(
            violations
                .iter()
                .any(|violation| violation.reason == Reason::NotAdmissible),
            "should refuse {aggregate} over a string: {violations:?}"
        );
    }
}

#[test]
fn counting_and_the_extremes_are_open_to_every_type() {
    for aggregate in ["count", "min", "max"] {
        let mut body = sound();
        body["fields"][1]["field"] = json!("author");
        body["fields"][1]["agg"] = json!(aggregate);

        assert_eq!(
            check(&metric(body), &declaration()),
            Vec::new(),
            "should admit {aggregate} over a string"
        );
    }
}

#[test]
fn a_number_that_identifies_rather_than_measures_still_groups() {
    let mut body = sound();
    body["fields"][0]["field"] = json!("project_id");
    body["fields"][0]["type"] = json!("int");
    body["fields"][0]["as_name"] = json!("project");
    body["group_by"] = json!(["project"]);

    assert_eq!(check(&metric(body), &declaration()), Vec::new());
}

#[test]
fn a_window_selects_by_a_datetime_and_nothing_else() {
    let mut body = sound();
    body["time"] = json!({ "field": "lines" });

    let violations = check(&metric(body), &declaration());

    let [violation] = violations.as_slice() else {
        panic!("one violation, got {violations:?}");
    };
    assert_eq!(violation.field, "time.field");
    assert_eq!(violation.reason, Reason::NotAdmissible);
}

#[test]
fn a_value_compared_against_a_field_is_read_as_that_fields_type() {
    let mut body = sound();
    body["filters"] = json!([{ "field": "lines", "type": "int", "op": "gt", "value": "many" }]);

    let violations = check(&metric(body), &declaration());

    let [violation] = violations.as_slice() else {
        panic!("one violation, got {violations:?}");
    };
    assert_eq!(violation.field, "filters[0].field");
    assert_eq!(violation.reason, Reason::Malformed);
}

#[test]
fn a_grouping_names_a_column_the_metric_produces_not_a_field_it_reads() {
    let mut body = sound();
    body["group_by"] = json!(["lines"]);

    let violations = check(&metric(body), &declaration());

    let [violation] = violations.as_slice() else {
        panic!("one violation, got {violations:?}");
    };
    assert_eq!(violation.field, "group_by[0]");
    assert!(violation.detail.contains("`total`"), "{violation:?}");
}

#[test]
fn ordering_by_a_total_the_metric_computes_is_admitted() {
    let mut body = sound();
    body["order_by"] = json!({ "field": "total", "direction": "desc" });

    assert_eq!(check(&metric(body), &declaration()), Vec::new());
}

#[test]
fn a_windowed_run_may_be_grouped_by_the_bucket_it_injects() {
    let mut body = sound();
    body["group_by"] = json!(["bucket"]);

    assert_eq!(check(&metric(body), &declaration()), Vec::new());
}

#[test]
fn every_problem_is_reported_at_once() {
    let mut body = sound();
    body["fields"][0]["field"] = json!("committer");
    body["fields"][1]["field"] = json!("author");
    body["group_by"] = json!(["nowhere"]);

    let violations = check(&metric(body), &declaration());

    assert_eq!(violations.len(), 3, "{violations:?}");
}

#[test]
fn a_metric_that_names_no_clock_inherits_the_datasets_own() {
    let written = metric(sound());
    let declared = declaration();

    let clock = effective_clock(&written, &declared);

    assert_eq!(clock, Some(("day", ClockSource::Dataset)));
}

#[test]
fn a_metric_that_names_a_clock_windows_by_that_one() {
    let mut body = sound();
    body["time"] = json!({ "field": "day" });
    let written = metric(body);
    let declared = declaration();

    let clock = effective_clock(&written, &declared);

    assert_eq!(clock, Some(("day", ClockSource::Metric)));
}

#[test]
fn a_dataset_that_marks_no_main_date_leaves_a_metric_answering_every_record() {
    let mut declared = declaration();
    declared.fields[0].default_clock = false;
    let written = metric(sound());

    assert_eq!(effective_clock(&written, &declared), None);
}
