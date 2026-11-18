//! A stored metric, read once: a query over one dataset or table, or a
//! composite of such queries.

use serde_json::Value;

use crate::domain::query::composite::Composite;
use crate::domain::query::metric_query::MetricQuery;

/// The key only a composite's body holds: the metrics it combines.
pub(crate) const COMBINE_KEY: &str = "combine";

#[derive(Debug)]
pub(crate) enum Metric {
    Query(MetricQuery),
    Composite(Composite),
}

impl Metric {
    /// Reads a stored metric body: one naming `combine` is a composite, any
    /// other a plain query.
    pub(crate) fn parse(body: Value) -> Result<Self, serde_json::Error> {
        if body.get(COMBINE_KEY).is_some() {
            return serde_json::from_value(body).map(Self::Composite);
        }

        serde_json::from_value(body).map(Self::Query)
    }

    /// The columns a result carries, in order. `clocked` says whether a
    /// windowed run has a date to bucket by, which only what the metric reads
    /// can answer.
    pub(crate) fn column_names(&self, clocked: bool) -> Vec<String> {
        match self {
            Self::Query(query) => query.column_names(clocked),
            Self::Composite(composite) => composite.column_names(clocked),
        }
    }

    /// The columns the rows are grouped by.
    pub(crate) fn groups(&self) -> &[String] {
        match self {
            Self::Query(query) => query.groups(),
            Self::Composite(composite) => composite.groups(),
        }
    }

    /// Where a whole run stops without the metric having asked it to.
    pub(crate) fn imposed_limit(&self) -> Option<u32> {
        match self {
            Self::Query(query) => query.imposed_limit(),
            Self::Composite(composite) => composite.imposed_limit(),
        }
    }
}
