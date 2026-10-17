use serde_json::{Value, json};

use super::super::*;

fn grouped() -> MetricQuery {
    serde_json::from_value(json!({
        "dataset": "events",
        "fields": [
            { "field": "day", "type": "string", "as_name": "day" },
            { "field": "who", "type": "string", "as_name": "author" },
            { "field": "lines", "type": "int", "agg": "sum", "as_name": "total_lines" },
            { "field": "files", "type": "int", "agg": "sum", "as_name": "total_files" },
            { "field": "goal", "type": "int", "agg": "max", "as_name": "goal" }
        ],
        "group_by": ["day", "author"],
        "filters": []
    }))
    .unwrap_or_else(|error| panic!("the fixture parses: {error}"))
}

fn parsed(body: &Value) -> Widget {
    serde_json::from_value(body.clone())
        .unwrap_or_else(|error| panic!("should parse: {body}: {error}"))
}

fn every_kind() -> Vec<Value> {
    vec![
        json!({ "type": "table", "metric": "m", "columns": ["day", "total_lines"] }),
        json!({ "type": "line", "metric": "m", "x": "day", "y": "total_lines", "series": "author", "target": "goal" }),
        json!({ "type": "bar", "metric": "m", "x": "day", "y": "total_lines", "series": "author" }),
        json!({ "type": "area", "metric": "m", "x": "day", "y": "total_lines" }),
        json!({ "type": "stat", "metric": "m", "value": "total_lines" }),
        json!({ "type": "pie", "metric": "m", "label": "author", "value": "total_lines" }),
        json!({ "type": "donut", "metric": "m", "label": "author", "value": "total_lines" }),
        json!({ "type": "ranked", "metric": "m", "label": "author", "value": "total_lines" }),
        json!({ "type": "treemap", "metric": "m", "label": "author", "value": "total_lines" }),
        json!({ "type": "funnel", "metric": "m", "label": "author", "value": "total_lines" }),
        json!({ "type": "waterfall", "metric": "m", "label": "day", "value": "total_lines" }),
        json!({ "type": "stacked", "metric": "m", "label": "day", "value": "total_lines", "series": "author" }),
        json!({ "type": "composed", "metric": "m", "x": "day", "y": "total_lines", "y2": "total_files" }),
        json!({ "type": "scatter", "metric": "m", "x": "total_files", "y": "total_lines", "series": "author" }),
        json!({ "type": "bubble", "metric": "m", "x": "total_files", "y": "total_lines", "size": "goal" }),
        json!({ "type": "radar", "metric": "m", "label": "author", "value": "total_lines", "target": "goal" }),
        json!({ "type": "radial", "metric": "m", "value": "total_lines", "max": "goal" }),
        json!({ "type": "heatmap", "metric": "m", "x": "day", "value": "total_lines" }),
        json!({ "type": "pulse", "metric": "m", "x": "day", "y": "total_lines" }),
    ]
}

#[test]
fn every_kind_is_accepted_over_a_metric_that_produces_its_columns() {
    for body in every_kind() {
        let checked = parsed(&body).check_against(&grouped(), false);

        assert!(checked.is_ok(), "should accept: {body}: {checked:?}");
    }
}

#[test]
fn every_kind_is_one_the_vocabulary_lists() {
    let kinds: Vec<String> = every_kind()
        .iter()
        .map(|body| body["type"].as_str().unwrap_or_default().to_owned())
        .collect();

    assert_eq!(kinds, KINDS);
}

#[test]
fn every_column_a_kind_draws_is_checked_against_the_metric() {
    let cases = [
        (
            json!({ "type": "line", "metric": "m", "x": "day", "y": "total_lines", "series": "nope" }),
            "nope",
        ),
        (
            json!({ "type": "line", "metric": "m", "x": "day", "y": "total_lines", "target": "nope" }),
            "nope",
        ),
        (
            json!({ "type": "bar", "metric": "m", "x": "day", "y": "total_lines", "series": "nope" }),
            "nope",
        ),
        (
            json!({ "type": "area", "metric": "m", "x": "day", "y": "total_lines", "series": "nope" }),
            "nope",
        ),
        (
            json!({ "type": "donut", "metric": "m", "label": "nope", "value": "total_lines" }),
            "nope",
        ),
        (
            json!({ "type": "ranked", "metric": "m", "label": "author", "value": "nope" }),
            "nope",
        ),
        (
            json!({ "type": "stacked", "metric": "m", "label": "day", "value": "total_lines", "series": "nope" }),
            "nope",
        ),
        (
            json!({ "type": "composed", "metric": "m", "x": "day", "y": "total_lines", "y2": "nope" }),
            "nope",
        ),
        (
            json!({ "type": "scatter", "metric": "m", "x": "total_files", "y": "total_lines", "series": "nope" }),
            "nope",
        ),
        (
            json!({ "type": "bubble", "metric": "m", "x": "total_files", "y": "total_lines", "size": "nope" }),
            "nope",
        ),
        (
            json!({ "type": "radar", "metric": "m", "label": "author", "value": "total_lines", "target": "nope" }),
            "nope",
        ),
        (
            json!({ "type": "radial", "metric": "m", "value": "total_lines", "max": "nope" }),
            "nope",
        ),
        (
            json!({ "type": "heatmap", "metric": "m", "x": "day", "value": "nope" }),
            "nope",
        ),
        (
            json!({ "type": "pulse", "metric": "m", "x": "nope", "y": "total_lines" }),
            "nope",
        ),
    ];

    for (body, missing) in cases {
        let checked = parsed(&body).check_against(&grouped(), false);

        assert!(
            matches!(checked, Err(WidgetError::UnknownColumn { ref column, .. }) if column == missing),
            "should refuse `{missing}`: {body}: {checked:?}"
        );
    }
}

#[test]
fn a_kind_without_a_column_it_requires_is_not_a_widget() {
    let cases = [
        json!({ "type": "stacked", "metric": "m", "label": "day", "value": "total_lines" }),
        json!({ "type": "composed", "metric": "m", "x": "day", "y": "total_lines" }),
        json!({ "type": "bubble", "metric": "m", "x": "total_files", "y": "total_lines" }),
        json!({ "type": "heatmap", "metric": "m", "x": "day" }),
        json!({ "type": "funnel", "metric": "m", "label": "author" }),
    ];

    for body in cases {
        let refused: Result<Widget, _> = serde_json::from_value(body.clone());

        assert!(refused.is_err(), "should refuse: {body}");
    }
}

#[test]
fn each_kind_narrows_its_detail_by_the_groups_it_stands_for() {
    let cases = [
        (
            json!({ "type": "line", "metric": "m", "x": "day", "y": "total_lines", "series": "author", "target": "goal" }),
            vec!["day", "author"],
        ),
        (
            json!({ "type": "bar", "metric": "m", "x": "day", "y": "total_lines" }),
            vec!["day"],
        ),
        (
            json!({ "type": "donut", "metric": "m", "label": "author", "value": "total_lines" }),
            vec!["author"],
        ),
        (
            json!({ "type": "waterfall", "metric": "m", "label": "day", "value": "total_lines" }),
            vec!["day"],
        ),
        (
            json!({ "type": "stacked", "metric": "m", "label": "day", "value": "total_lines", "series": "author" }),
            vec!["day", "author"],
        ),
        (
            json!({ "type": "composed", "metric": "m", "x": "day", "y": "total_lines", "y2": "total_files" }),
            vec!["day"],
        ),
        (
            json!({ "type": "scatter", "metric": "m", "x": "total_files", "y": "total_lines", "series": "author" }),
            vec!["author"],
        ),
        (
            json!({ "type": "bubble", "metric": "m", "x": "total_files", "y": "total_lines", "size": "goal" }),
            vec![],
        ),
        (
            json!({ "type": "radar", "metric": "m", "label": "author", "value": "total_lines" }),
            vec!["author"],
        ),
        (
            json!({ "type": "radial", "metric": "m", "value": "total_lines" }),
            vec![],
        ),
        (
            json!({ "type": "heatmap", "metric": "m", "x": "day", "value": "total_lines" }),
            vec!["day"],
        ),
        (
            json!({ "type": "pulse", "metric": "m", "x": "day", "y": "total_lines" }),
            vec![],
        ),
    ];

    for (body, expected) in cases {
        let drawn = grouped();
        let widget = parsed(&body);

        assert_eq!(
            widget.narrowing_columns(&drawn),
            expected,
            "should narrow by {expected:?}: {body}"
        );
    }
}

#[test]
fn a_stored_stat_with_a_caption_still_parses() {
    let stored = json!({ "type": "stat", "metric": "m", "value": "total_lines", "label": "Lines", "title": "Lines" });

    assert!(parsed(&stored).check_against(&grouped(), false).is_ok());
}
