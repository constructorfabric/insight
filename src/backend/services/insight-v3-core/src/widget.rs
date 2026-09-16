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
#[path = "widget/tests.rs"]
mod tests;
