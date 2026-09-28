use serde_json::json;

use super::*;

/// A metric whose run has no date to bucket by.
fn metric() -> MetricQuery {
    serde_json::from_value(json!({
        "dataset": "events",
        "fields": [
            { "field": "day", "type": "string", "as_name": "day" },
            { "field": "lines", "type": "int", "agg": "sum", "as_name": "total_lines" }
        ],
        "group_by": ["day"],
        "filters": []
    }))
    .unwrap_or_else(|error| panic!("the fixture parses: {error}"))
}

/// The same metric read as one a window can bucket, which is what the
/// declaration behind it decides.
const CLOCKED: bool = true;
const CLOCKLESS: bool = false;

fn widget(value: serde_json::Value) -> Widget {
    serde_json::from_value(value).unwrap_or_else(|error| panic!("the fixture parses: {error}"))
}

#[test]
fn every_kind_that_draws_two_columns_checks_both() {
    for kind in ["line", "bar", "area"] {
        let drawn = widget(json!({
            "type": kind, "metric": "lines_per_day", "x": "day", "y": "total_lines"
        }));
        assert!(
            drawn.check_against(&metric(), CLOCKLESS).is_ok(),
            "{kind} draws its metric"
        );

        let wrong = widget(json!({
            "type": kind, "metric": "lines_per_day", "x": "day", "y": "lines"
        }));
        assert!(
            matches!(
                wrong.check_against(&metric(), CLOCKLESS),
                Err(WidgetError::UnknownColumn { .. })
            ),
            "{kind} refuses a column the metric has not got"
        );
    }
}

#[test]
fn a_stat_reads_one_value() {
    let drawn = widget(json!({
        "type": "stat", "metric": "lines_per_day", "value": "total_lines", "label": "Lines"
    }));

    assert!(drawn.check_against(&metric(), CLOCKLESS).is_ok());
}

#[test]
fn a_chart_can_author_the_bucket_injected_by_a_clocked_metric() {
    let drawn = widget(json!({
        "type": "line", "metric": "events_over_time", "x": "bucket", "y": "total_lines"
    }));

    assert!(drawn.check_against(&metric(), CLOCKED).is_ok());
}

/// A metric nothing dates produces no bucket, so a chart drawing one would
/// draw an empty column at every run.
#[test]
fn a_clockless_metric_has_no_bucket_to_draw() {
    let drawn = widget(json!({
        "type": "line", "metric": "lines_per_day", "x": "bucket", "y": "total_lines"
    }));

    assert!(matches!(
        drawn.check_against(&metric(), CLOCKLESS),
        Err(WidgetError::UnknownColumn { .. })
    ));
}

#[test]
fn a_stat_naming_nothing_the_metric_returns_is_refused() {
    let drawn = widget(json!({
        "type": "stat", "metric": "lines_per_day", "value": "lines"
    }));

    assert!(matches!(
        drawn.check_against(&metric(), CLOCKLESS),
        Err(WidgetError::UnknownColumn { .. })
    ));
}

#[test]
fn a_pie_reads_a_label_and_a_value() {
    let drawn = widget(json!({
        "type": "pie", "metric": "lines_per_day", "label": "day", "value": "total_lines"
    }));

    assert!(drawn.check_against(&metric(), CLOCKLESS).is_ok());
}

#[test]
fn a_pie_whose_label_is_not_a_column_is_refused() {
    let drawn = widget(json!({
        "type": "pie", "metric": "lines_per_day", "label": "author", "value": "total_lines"
    }));

    assert!(matches!(
        drawn.check_against(&metric(), CLOCKLESS),
        Err(WidgetError::UnknownColumn { .. })
    ));
}

#[test]
fn a_kind_the_renderer_does_not_have_is_not_a_widget() {
    // The tool schema and the renderer are two views of one vocabulary,
    // and this is the check that keeps a third out of the store.
    let refused: Result<Widget, _> = serde_json::from_value(json!({
        "type": "sankey", "metric": "lines_per_day", "x": "day", "y": "total_lines"
    }));

    assert!(refused.is_err());
}

#[test]
fn a_line_naming_the_metrics_own_columns_is_accepted() {
    let widget = widget(json!({
        "type": "line", "metric": "lines_per_day", "x": "day", "y": "total_lines"
    }));

    assert!(widget.check_against(&metric(), CLOCKLESS).is_ok());
}

#[test]
fn a_line_naming_the_raw_field_instead_of_the_alias_is_refused() {
    // Seen live: the chart drew its axes and no line, which reads as
    // missing data rather than as a widget naming a column that is not
    // there.
    let widget = widget(json!({
        "type": "line", "metric": "lines_per_day", "x": "day", "y": "lines"
    }));

    let Err(error) = widget.check_against(&metric(), CLOCKLESS) else {
        panic!("a column the metric does not produce must be refused");
    };

    let message = error.to_string();
    assert!(message.contains("`lines` is not a column"), "{message}");
    // The message carries what IS available, so the repair round can fix it.
    assert!(message.contains("day, total_lines"), "{message}");
}

#[test]
fn a_table_column_the_metric_does_not_produce_is_refused() {
    let widget = widget(json!({
        "type": "table", "metric": "lines_per_day", "columns": ["day", "author"]
    }));

    assert!(widget.check_against(&metric(), CLOCKLESS).is_err());
}

#[test]
fn a_table_naming_no_columns_is_refused() {
    let widget = widget(json!({ "type": "table", "metric": "lines_per_day" }));

    let refusal = widget.check_against(&metric(), CLOCKLESS);

    assert!(
        matches!(refusal, Err(WidgetError::NoColumns)),
        "an empty column list renders an empty table: {refusal:?}"
    );
}

#[test]
fn a_table_drawing_every_column_it_names_is_accepted() {
    let widget = widget(json!({
        "type": "table", "metric": "lines_per_day", "columns": ["day", "total_lines"]
    }));

    assert!(widget.check_against(&metric(), CLOCKLESS).is_ok());
}

/// The facts behind the figure: the same dataset's rows, not grouped, with
/// the column a bar stands for still in them.
fn detail() -> MetricQuery {
    serde_json::from_value(json!({
        "dataset": "events",
        "fields": [
            { "field": "day", "type": "string", "as_name": "day" },
            { "field": "lines", "type": "int", "as_name": "lines" }
        ]
    }))
    .unwrap_or_else(|error| panic!("the fixture parses: {error}"))
}

#[test]
fn a_detail_that_carries_the_column_a_bar_stands_for_is_accepted() {
    let drawn = widget(json!({
        "type": "bar", "metric": "lines_per_day", "detail": "line_events",
        "x": "day", "y": "total_lines"
    }));

    assert!(
        drawn
            .check_detail_against(&metric(), &detail(), CLOCKLESS)
            .is_ok()
    );
}

/// A reader who clicks one bar is shown that bar's rows. A detail without
/// the bar's column has no rows to narrow to, and an empty dialog is the
/// worst way to find that out.
#[test]
fn a_detail_without_the_column_a_bar_stands_for_is_refused() {
    let drawn = widget(json!({
        "type": "bar", "metric": "lines_per_day", "detail": "line_events",
        "x": "author", "y": "total_lines"
    }));

    assert!(matches!(
        drawn.check_detail_against(&metric(), &detail(), CLOCKLESS),
        Err(WidgetError::DetailLacksColumn { ref column, ref detail, .. })
            if column == "author" && detail == "line_events"
    ));
}

#[test]
fn a_stat_stands_for_no_group_so_any_detail_narrows_it() {
    let drawn = widget(json!({
        "type": "stat", "metric": "lines_per_day", "detail": "line_events",
        "value": "total_lines"
    }));

    assert!(
        drawn
            .check_detail_against(&metric(), &detail(), CLOCKLESS)
            .is_ok()
    );
}

#[test]
fn a_chart_over_time_needs_a_detail_a_window_can_bucket() {
    let drawn = widget(json!({
        "type": "line", "metric": "events_over_time", "detail": "line_events",
        "x": "bucket", "y": "total_lines"
    }));

    assert!(
        drawn
            .check_detail_against(&metric(), &detail(), CLOCKED)
            .is_ok()
    );
    assert!(matches!(
        drawn.check_detail_against(&metric(), &detail(), CLOCKLESS),
        Err(WidgetError::DetailLacksColumn { .. })
    ));
}

#[test]
fn a_widget_names_its_detail_beside_its_metric_and_once_when_they_are_one() {
    let two = json!({ "type": "bar", "metric": "m", "detail": "d", "x": "a", "y": "b" });
    let one = json!({ "type": "bar", "metric": "m", "detail": "m", "x": "a", "y": "b" });

    assert_eq!(
        refers_to(&two),
        vec![
            Reference::new(DefinitionKind::Metric, "m"),
            Reference::new(DefinitionKind::Metric, "d"),
        ]
    );
    assert_eq!(
        refers_to(&one),
        vec![Reference::new(DefinitionKind::Metric, "m")]
    );
}

#[test]
fn renaming_a_metric_follows_it_into_the_detail() {
    let before = json!({ "type": "bar", "metric": "m", "detail": "old", "x": "a", "y": "b" });

    let after = rename_reference(before, "old", "new");

    assert_eq!(after["detail"], "new");
    assert_eq!(after["metric"], "m");
}

/// A table row stands for the group its metric made, so only the grouped
/// columns it draws have to be in the detail - never the aggregates.
#[test]
fn a_table_narrows_its_detail_by_the_groups_it_draws_and_not_by_its_sums() {
    let drawn = widget(json!({
        "type": "table", "metric": "lines_per_day", "detail": "line_events",
        "columns": ["day", "total_lines"]
    }));

    assert!(
        drawn
            .check_detail_against(&metric(), &detail(), CLOCKLESS)
            .is_ok()
    );

    let elsewhere = widget(json!({
        "type": "table", "metric": "lines_per_day", "detail": "line_events",
        "columns": ["author", "total_lines"]
    }));
    // `author` is not a group of the drawn metric, so it narrows nothing.
    assert!(
        elsewhere
            .check_detail_against(&metric(), &detail(), CLOCKLESS)
            .is_ok()
    );
}
