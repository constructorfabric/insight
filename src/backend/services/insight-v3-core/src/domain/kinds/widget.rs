//! What a widget draws, and whether its metric can supply it.
//!
//! A widget names columns by the `as_name` its metric gives them, so a name
//! the metric never produces is a broken definition, not missing data.

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use super::{KindError, Reference};
use crate::domain::datasets::Datasets;
use crate::domain::definition::{DefinitionKind, DefinitionName, Lookup};
use crate::domain::kinds::dataset::declaration::BUCKET_COLUMN;
use crate::domain::kinds::metric::answerable::effective_clock;
use crate::domain::query::metric_query::MetricQuery;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Widget {
    Table {
        metric: String,
        #[serde(default)]
        columns: Vec<String>,
        #[serde(default)]
        detail: Option<String>,
    },
    Line {
        metric: String,
        x: String,
        y: String,
        #[serde(default)]
        detail: Option<String>,
    },
    Bar {
        metric: String,
        x: String,
        y: String,
        #[serde(default)]
        detail: Option<String>,
    },
    Area {
        metric: String,
        x: String,
        y: String,
        #[serde(default)]
        detail: Option<String>,
    },
    /// One number, which is what most questions actually answer with.
    Stat {
        metric: String,
        value: String,
        #[serde(default)]
        detail: Option<String>,
    },
    Pie {
        metric: String,
        label: String,
        value: String,
        #[serde(default)]
        detail: Option<String>,
    },
}

impl Widget {
    /// The metric this widget draws.
    fn metric(&self) -> &str {
        match self {
            Self::Table { metric, .. }
            | Self::Line { metric, .. }
            | Self::Bar { metric, .. }
            | Self::Area { metric, .. }
            | Self::Stat { metric, .. }
            | Self::Pie { metric, .. } => metric,
        }
    }

    /// The metric a reader drills into, when the author named one: the facts
    /// that went into the figure rather than the figure itself.
    fn detail(&self) -> Option<&str> {
        match self {
            Self::Table { detail, .. }
            | Self::Line { detail, .. }
            | Self::Bar { detail, .. }
            | Self::Area { detail, .. }
            | Self::Stat { detail, .. }
            | Self::Pie { detail, .. } => detail.as_deref(),
        }
    }

    /// The columns a reader narrows the detail by: the group a bar, a slice
    /// or a row stands for. A table row stands for the groups its metric
    /// makes, where the table draws them; a stat stands for no group.
    fn narrowing_columns<'w>(&'w self, drawn: &'w MetricQuery) -> Vec<&'w str> {
        match self {
            Self::Line { x, .. } | Self::Bar { x, .. } | Self::Area { x, .. } => vec![x.as_str()],
            Self::Pie { label, .. } => vec![label.as_str()],
            Self::Table { columns, .. } => columns
                .iter()
                .map(String::as_str)
                .filter(|column| {
                    *column == BUCKET_COLUMN || drawn.groups().contains(&(*column).to_owned())
                })
                .collect(),
            Self::Stat { .. } => Vec::new(),
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
    fn check_against(&self, metric: &MetricQuery, clocked: bool) -> Result<(), WidgetError> {
        let available = metric.column_names(clocked);

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

    /// Refuses a detail metric that cannot be narrowed to what the widget
    /// draws: a reader who clicks one bar is shown that bar's rows, which
    /// takes the bar's column in the detail as well.
    fn check_detail_against(
        &self,
        drawn: &MetricQuery,
        detail: &MetricQuery,
        clocked: bool,
    ) -> Result<(), WidgetError> {
        let available = detail.column_names(clocked);

        for column in self.narrowing_columns(drawn) {
            if !available.iter().any(|name| name == column) {
                return Err(WidgetError::DetailLacksColumn {
                    column: column.to_owned(),
                    detail: self.detail().unwrap_or_default().to_owned(),
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
    #[error(
        "`{column}` is not a column of the detail metric `{detail}`, so its rows could not be \
         narrowed to what this widget draws; the detail's columns are: {available}"
    )]
    DetailLacksColumn {
        column: String,
        detail: String,
        available: String,
    },
}

/// Checks a widget against the metric it draws.
///
/// The metric is read because a widget names the columns that metric produces,
/// and naming one it does not is the mistake this catches.
pub(crate) async fn check(
    body: &Value,
    definitions: &dyn Lookup,
    datasets: &dyn Datasets,
) -> Result<(), KindError> {
    let widget: Widget =
        serde_json::from_value(body.clone()).map_err(|error| KindError::Widget(error.into()))?;

    let metric_name = DefinitionName::parse(widget.metric())
        .map_err(|_| KindError::Widget(WidgetError::NoMetric(widget.metric().to_owned())))?;

    let stored = definitions
        .get(DefinitionKind::Metric, &metric_name)
        .await
        .map_err(KindError::Store)?
        .ok_or_else(|| KindError::Widget(WidgetError::NoMetric(widget.metric().to_owned())))?;

    let metric: MetricQuery =
        serde_json::from_value(stored).map_err(|error| KindError::Widget(error.into()))?;
    let dated = clocked(&metric, datasets).await;

    widget
        .check_against(&metric, dated)
        .map_err(KindError::Widget)?;

    let Some(named) = widget.detail() else {
        return Ok(());
    };
    let detail = stored_metric(named, definitions).await?;
    let dated = clocked(&detail, datasets).await;

    widget
        .check_detail_against(&metric, &detail, dated)
        .map_err(KindError::Widget)
}

/// A stored metric, read as one, or the refusal that there is none.
async fn stored_metric(named: &str, definitions: &dyn Lookup) -> Result<MetricQuery, KindError> {
    let name = DefinitionName::parse(named)
        .map_err(|_| KindError::Widget(WidgetError::NoMetric(named.to_owned())))?;

    let stored = definitions
        .get(DefinitionKind::Metric, &name)
        .await
        .map_err(KindError::Store)?
        .ok_or_else(|| KindError::Widget(WidgetError::NoMetric(named.to_owned())))?;

    serde_json::from_value(stored).map_err(|error| KindError::Widget(error.into()))
}

/// Whether a windowed run of this metric has a date to bucket by, and so
/// whether it produces a `bucket` column at all.
///
/// A metric over a dataset may name no date and inherit the dataset's, so the
/// metric body alone cannot answer it.
async fn clocked(metric: &MetricQuery, datasets: &dyn Datasets) -> bool {
    let Some(named) = metric.dataset() else {
        return metric.has_clock().unwrap_or(false);
    };
    let held = crate::domain::datasets::ready(datasets, named).await;
    let Ok(Some(ready)) = held else {
        return false;
    };

    effective_clock(metric, &ready.declaration).is_some()
}

const METRIC_KEYS: [&str; 2] = ["metric", "detail"];

/// The metrics a widget draws and drills into, when its body names them
/// readably. One metric named twice is one reference.
pub(crate) fn refers_to(body: &Value) -> Vec<Reference> {
    let mut named = Vec::new();

    for key in METRIC_KEYS {
        let Some(metric) = body.get(key).and_then(Value::as_str) else {
            continue;
        };
        let reference = Reference::new(DefinitionKind::Metric, metric);
        if !named.contains(&reference) {
            named.push(reference);
        }
    }

    named
}

/// The same widget, drawing and drilling into `to` where it named `from`.
pub(crate) fn rename_reference(mut body: Value, from: &str, to: &str) -> Value {
    for key in METRIC_KEYS {
        if body.get(key).and_then(Value::as_str) == Some(from) {
            body[key] = Value::String(to.to_owned());
        }
    }

    body
}

#[cfg(test)]
mod tests;
