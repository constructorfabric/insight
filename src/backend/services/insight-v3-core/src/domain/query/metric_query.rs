//! A metric definition: the JSON a reader stores, and what it compiles into.

mod clock;
mod compiler;
mod field;
mod filter;
pub(crate) mod people;
pub(crate) mod runner;

use std::collections::HashMap;

use serde::Deserialize;
use thiserror::Error;

pub(crate) use field::FieldType;
pub(crate) use filter::FilterBind;
pub(crate) use people::People;
pub(crate) use runner::{MetricRunError, MetricRunner, RunResult};

use clock::TimeField;
use field::{Field, OrderBy};
use filter::Filter;

use crate::domain::query::time_window::WindowError;

/// The alias the fact table is read under, so a column of the metric's own
/// table is never mistaken for one a join brought in.
const FACT_ALIAS: &str = "__f";

#[derive(Debug, Deserialize)]
pub(crate) struct MetricQuery {
    #[serde(default)]
    database: Option<String>,
    table: String,
    #[serde(default)]
    time: Option<TimeField>,
    #[serde(default)]
    max_range: Option<String>,
    fields: Vec<Field>,
    #[serde(default)]
    group_by: Vec<String>,
    #[serde(default)]
    filters: Vec<Filter>,
    #[serde(default)]
    order_by: Option<OrderBy>,
    #[serde(default)]
    limit: Option<u32>,
}

/// The one read that resolves a relative window before the metric runs.
#[derive(Debug)]
pub(crate) struct UndatedQuery {
    pub(crate) sql: String,
    pub(crate) binds: Vec<FilterBind>,
}

#[derive(Debug)]
pub(crate) struct CompiledQuery {
    pub(crate) sql: String,
    pub(crate) binds: Vec<FilterBind>,
    column_types: HashMap<String, FieldType>,
    /// The columns whose numbers are percentages, so a reader can say so.
    percents: Vec<String>,
}

#[derive(Debug, Error)]
pub(crate) enum MetricQueryError {
    #[error("`{0}` must be 1-128 characters of letters, digits or underscore")]
    Identifier(String),
    #[error("`{0}` must name one of this query's as_name values")]
    GroupBy(String),
    #[error(
        "`{0}` is selected beside an aggregate, so it has to be in `group_by` \
         or carry an aggregate of its own"
    )]
    Ungrouped(String),
    #[error("`{0}` cannot order the rows: it is not one of this query's as_name values")]
    OrderBy(String),
    #[error("a metric must select at least one field")]
    NoFields,
    #[error("filter value for `{0}` does not match its declared type")]
    FilterValue(String),
    #[error("{0} must name `json`, `column`, or both")]
    FieldSource(String),
    #[error("`{0}` selects an array element, so it and its `where` must read json")]
    Selector(String),
    #[error("{0}")]
    Ratio(String),
    #[error("time must name exactly one of `json` or `column`")]
    ClockSource,
    #[error("time type `{0}` is not datetime")]
    ClockType(String),
    #[error("time source `{0}` cannot also be a metric filter")]
    ClockFilter(String),
    #[error("`bucket` is reserved for a metric's time bucket")]
    BucketAlias,
    #[error("a requested time range needs a metric time source")]
    ClocklessWindow,
    #[error("the requested range exceeds maximum `{0}`")]
    RangeExceedsMaximum(String),
    #[error(transparent)]
    Window(#[from] WindowError),
}

#[cfg(test)]
mod tests;
