use serde::Deserialize;

use crate::domain::kinds::dataset::declaration::BUCKET_COLUMN;
use crate::domain::query::metric_query::MetricQuery;

pub(crate) const KINDS: [&str; 19] = [
    "table",
    "line",
    "bar",
    "area",
    "stat",
    "pie",
    "donut",
    "ranked",
    "treemap",
    "funnel",
    "waterfall",
    "stacked",
    "composed",
    "scatter",
    "bubble",
    "radar",
    "radial",
    "heatmap",
    "pulse",
];

#[derive(Debug, Deserialize)]
pub(super) struct Widget {
    pub(super) metric: String,
    #[serde(default)]
    pub(super) detail: Option<String>,
    #[serde(flatten)]
    pub(super) draws: Draws,
}

#[derive(Debug, Deserialize)]
pub(super) struct Category {
    label: String,
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(super) enum Draws {
    Table {
        #[serde(default)]
        columns: Vec<String>,
    },
    Line {
        x: String,
        y: String,
        #[serde(default)]
        series: Option<String>,
        #[serde(default)]
        target: Option<String>,
    },
    Bar {
        x: String,
        y: String,
        #[serde(default)]
        series: Option<String>,
    },
    Area {
        x: String,
        y: String,
        #[serde(default)]
        series: Option<String>,
    },
    /// One number, which is what most questions actually answer with.
    Stat {
        value: String,
    },
    Pie(Category),
    Donut(Category),
    Ranked(Category),
    Treemap(Category),
    Funnel(Category),
    Waterfall(Category),
    Stacked {
        label: String,
        value: String,
        series: String,
    },
    Composed {
        x: String,
        y: String,
        y2: String,
    },
    Scatter {
        x: String,
        y: String,
        #[serde(default)]
        series: Option<String>,
    },
    Bubble {
        x: String,
        y: String,
        size: String,
        #[serde(default)]
        series: Option<String>,
    },
    Radar {
        label: String,
        value: String,
        #[serde(default)]
        target: Option<String>,
    },
    Radial {
        value: String,
        #[serde(default)]
        max: Option<String>,
    },
    Heatmap {
        x: String,
        value: String,
    },
    Pulse {
        x: String,
        y: String,
    },
}

impl Widget {
    /// The metric this widget draws.
    pub(super) fn metric(&self) -> &str {
        &self.metric
    }

    /// The metric a reader drills into, when the author named one: the facts
    /// that went into the figure rather than the figure itself.
    pub(super) fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// The columns it reads out of that metric's result.
    pub(super) fn columns(&self) -> Vec<&str> {
        self.draws.columns()
    }

    /// The columns a reader narrows the detail by: the group a bar, a slice
    /// or a row stands for. A table row stands for the groups its metric
    /// makes, where the table draws them; a single figure stands for no group.
    pub(super) fn narrowing_columns<'w>(&'w self, drawn: &'w MetricQuery) -> Vec<&'w str> {
        self.draws.narrowing_columns(drawn)
    }
}

impl Draws {
    fn columns(&self) -> Vec<&str> {
        match self {
            Self::Table { columns } => columns.iter().map(String::as_str).collect(),
            Self::Line {
                x,
                y,
                series,
                target,
            } => with_optional(vec![x, y], [series.as_ref(), target.as_ref()]),
            Self::Bar { x, y, series }
            | Self::Area { x, y, series }
            | Self::Scatter { x, y, series } => with_optional(vec![x, y], [series.as_ref()]),
            Self::Stat { value } => vec![value.as_str()],
            Self::Pie(category)
            | Self::Donut(category)
            | Self::Ranked(category)
            | Self::Treemap(category)
            | Self::Funnel(category)
            | Self::Waterfall(category) => vec![category.label.as_str(), category.value.as_str()],
            Self::Stacked {
                label,
                value,
                series,
            } => vec![label.as_str(), value.as_str(), series.as_str()],
            Self::Composed { x, y, y2 } => vec![x.as_str(), y.as_str(), y2.as_str()],
            Self::Bubble { x, y, size, series } => {
                with_optional(vec![x, y, size], [series.as_ref()])
            }
            Self::Radar {
                label,
                value,
                target,
            } => with_optional(vec![label, value], [target.as_ref()]),
            Self::Radial { value, max } => with_optional(vec![value], [max.as_ref()]),
            Self::Heatmap { x, value } => vec![x.as_str(), value.as_str()],
            Self::Pulse { x, y } => vec![x.as_str(), y.as_str()],
        }
    }

    fn narrowing_columns<'w>(&'w self, drawn: &'w MetricQuery) -> Vec<&'w str> {
        match self {
            Self::Line { x, series, .. }
            | Self::Bar { x, series, .. }
            | Self::Area { x, series, .. } => with_optional(vec![x], [series.as_ref()]),
            Self::Pie(category)
            | Self::Donut(category)
            | Self::Ranked(category)
            | Self::Treemap(category)
            | Self::Funnel(category)
            | Self::Waterfall(category) => vec![category.label.as_str()],
            Self::Stacked { label, series, .. } => vec![label.as_str(), series.as_str()],
            Self::Composed { x, .. } | Self::Heatmap { x, .. } => vec![x.as_str()],
            Self::Scatter { series, .. } | Self::Bubble { series, .. } => {
                series.iter().map(String::as_str).collect()
            }
            Self::Radar { label, .. } => vec![label.as_str()],
            Self::Table { columns } => columns
                .iter()
                .map(String::as_str)
                .filter(|column| {
                    *column == BUCKET_COLUMN || drawn.groups().contains(&(*column).to_owned())
                })
                .collect(),
            Self::Stat { .. } | Self::Radial { .. } | Self::Pulse { .. } => Vec::new(),
        }
    }
}

fn with_optional<'w, const N: usize>(
    required: Vec<&'w String>,
    optional: [Option<&'w String>; N],
) -> Vec<&'w str> {
    required
        .into_iter()
        .chain(optional.into_iter().flatten())
        .map(String::as_str)
        .collect()
}
