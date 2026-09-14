//! What a widget draws, and whether its metric can supply it.
//!
//! A widget names columns by the `as_name` its metric gives them. Nothing
//! checked that, so a widget could name a column the metric never produces —
//! `y: "lines"` against a metric whose column is `total_lines`. The chart then
//! rendered its axes and no line at all, which reads as missing data rather
//! than as a broken definition.

use serde::Deserialize;
use thiserror::Error;

use crate::metric_query::MetricQuery;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum Widget {
    Table {
        metric: String,
        #[serde(default)]
        columns: Vec<String>,
    },
    Line {
        metric: String,
        x: String,
        y: String,
    },
    Bar {
        metric: String,
        x: String,
        y: String,
    },
    Area {
        metric: String,
        x: String,
        y: String,
    },
    /// One number, which is what most questions actually answer with.
    Stat { metric: String, value: String },
    Pie {
        metric: String,
        label: String,
        value: String,
    },
}

impl Widget {
    /// The metric this widget draws.
    pub(crate) fn metric(&self) -> &str {
        match self {
            Self::Table { metric, .. }
            | Self::Line { metric, .. }
            | Self::Bar { metric, .. }
            | Self::Area { metric, .. }
            | Self::Stat { metric, .. }
            | Self::Pie { metric, .. } => metric,
        }
    }

    /// The columns it reads out of that metric's result.
    fn columns(&self) -> Vec<&str> {
        match self {
            Self::Table { columns, .. } => columns.iter().map(String::as_str).collect(),
            Self::Line { x, y, .. } | Self::Bar { x, y, .. } | Self::Area { x, y, .. } => {
                vec![x.as_str(), y.as_str()]
            }
            Self::Stat { value, .. } => vec![value.as_str()],
            Self::Pie { label, value, .. } => vec![label.as_str(), value.as_str()],
        }
    }

    /// Refuses a widget whose metric cannot supply what it draws.
    pub(crate) fn check_against(&self, metric: &MetricQuery) -> Result<(), WidgetError> {
        let available = metric.column_names();

        if self.columns().is_empty() {
            return Err(WidgetError::NoColumns);
        }

        for column in self.columns() {
            if !available.iter().any(|name| name == column) {
                return Err(WidgetError::UnknownColumn {
                    column: column.to_owned(),
                    metric: self.metric().to_owned(),
                    available: available.join(", "),
                });
            }
        }

        Ok(())
    }
}

#[derive(Debug, Error)]
pub(crate) enum WidgetError {
    #[error("a table names the columns it draws; this one names none")]
    NoColumns,
    #[error(
        "a widget needs a metric, a type of `table`, `line`, `bar`, `area`, `stat` or `pie`, \
         and the columns that type draws: columns for a table, x and y for a line, bar or \
         area, value for a stat, label and value for a pie"
    )]
    Shape(#[from] serde_json::Error),
    #[error("`{column}` is not a column of metric `{metric}`; its columns are: {available}")]
    UnknownColumn {
        column: String,
        metric: String,
        available: String,
    },
    #[error("there is no metric named `{0}`")]
    NoMetric(String),
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn metric() -> MetricQuery {
        serde_json::from_value(json!({
            "table": "events",
            "fields": [
                { "json": "day", "type": "string", "as_name": "day" },
                { "json": "lines", "type": "int", "agg": "sum", "as_name": "total_lines" }
            ],
            "group_by": ["day"],
            "filters": []
        }))
        .unwrap_or_else(|error| panic!("the fixture parses: {error}"))
    }

    fn timed_metric() -> MetricQuery {
        serde_json::from_value(json!({
            "table": "events",
            "time": { "column": "occurred_at" },
            "fields": [{ "agg": "count", "type": "int", "as_name": "total" }]
        }))
        .unwrap_or_else(|error| panic!("the fixture parses: {error}"))
    }

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
                drawn.check_against(&metric()).is_ok(),
                "{kind} draws its metric"
            );

            let wrong = widget(json!({
                "type": kind, "metric": "lines_per_day", "x": "day", "y": "lines"
            }));
            assert!(
                matches!(
                    wrong.check_against(&metric()),
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

        assert!(drawn.check_against(&metric()).is_ok());
    }

    #[test]
    fn a_chart_can_author_the_bucket_injected_by_a_clocked_metric() {
        let drawn = widget(json!({
            "type": "line", "metric": "events_over_time", "x": "bucket", "y": "total"
        }));

        assert!(drawn.check_against(&timed_metric()).is_ok());
        assert!(drawn.check_against(&metric()).is_err());
    }

    #[test]
    fn a_stat_naming_nothing_the_metric_returns_is_refused() {
        let drawn = widget(json!({
            "type": "stat", "metric": "lines_per_day", "value": "lines"
        }));

        assert!(matches!(
            drawn.check_against(&metric()),
            Err(WidgetError::UnknownColumn { .. })
        ));
    }

    #[test]
    fn a_pie_reads_a_label_and_a_value() {
        let drawn = widget(json!({
            "type": "pie", "metric": "lines_per_day", "label": "day", "value": "total_lines"
        }));

        assert!(drawn.check_against(&metric()).is_ok());
    }

    #[test]
    fn a_pie_whose_label_is_not_a_column_is_refused() {
        let drawn = widget(json!({
            "type": "pie", "metric": "lines_per_day", "label": "author", "value": "total_lines"
        }));

        assert!(matches!(
            drawn.check_against(&metric()),
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

        assert!(widget.check_against(&metric()).is_ok());
    }

    #[test]
    fn a_line_naming_the_raw_field_instead_of_the_alias_is_refused() {
        // Seen live: the chart drew its axes and no line, which reads as
        // missing data rather than as a widget naming a column that is not
        // there.
        let widget = widget(json!({
            "type": "line", "metric": "lines_per_day", "x": "day", "y": "lines"
        }));

        let Err(error) = widget.check_against(&metric()) else {
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

        assert!(widget.check_against(&metric()).is_err());
    }

    #[test]
    fn a_table_naming_no_columns_is_refused() {
        let widget = widget(json!({ "type": "table", "metric": "lines_per_day" }));

        let refusal = widget.check_against(&metric());

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

        assert!(widget.check_against(&metric()).is_ok());
    }
}
