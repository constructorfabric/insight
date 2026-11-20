use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use super::{KindError, Reference};
use crate::domain::datasets::Datasets;
use crate::domain::definition::{DefinitionId, DefinitionKind, Lookup};
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
        series: Option<String>,
        #[serde(default)]
        #[expect(
            dead_code,
            reason = "parsed only to refuse a `labels` that is not a flag"
        )]
        labels: bool,
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

    fn columns(&self) -> Vec<&str> {
        match self {
            Self::Table { columns, .. } => columns.iter().map(String::as_str).collect(),
            Self::Line { x, y, .. } | Self::Area { x, y, .. } => vec![x.as_str(), y.as_str()],
            Self::Bar { x, y, series, .. } => [Some(x), Some(y), series.as_ref()]
                .into_iter()
                .flatten()
                .map(String::as_str)
                .collect(),
            Self::Stat { value, .. } => vec![value.as_str()],
            Self::Pie { label, value, .. } => vec![label.as_str(), value.as_str()],
        }
    }

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
         area, value for a stat, label and value for a pie; a bar may add a `series` column \
         to stack by and `labels: true` to print each bar's total"
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

pub(crate) async fn check(
    body: &Value,
    definitions: &dyn Lookup,
    datasets: &dyn Datasets,
) -> Result<(), KindError> {
    let widget: Widget =
        serde_json::from_value(body.clone()).map_err(|error| KindError::Widget(error.into()))?;

    let metric_name = DefinitionId::parse(widget.metric())
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

async fn stored_metric(named: &str, definitions: &dyn Lookup) -> Result<MetricQuery, KindError> {
    let name = DefinitionId::parse(named)
        .map_err(|_| KindError::Widget(WidgetError::NoMetric(named.to_owned())))?;

    let stored = definitions
        .get(DefinitionKind::Metric, &name)
        .await
        .map_err(KindError::Store)?
        .ok_or_else(|| KindError::Widget(WidgetError::NoMetric(named.to_owned())))?;

    serde_json::from_value(stored).map_err(|error| KindError::Widget(error.into()))
}

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

pub(crate) fn drawn_metric(body: &Value) -> Option<&str> {
    body.get("metric")?.as_str()
}

pub(crate) fn drilled_metric(body: &Value) -> Option<&str> {
    body.get("detail")
        .and_then(Value::as_str)
        .or_else(|| drawn_metric(body))
}

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
