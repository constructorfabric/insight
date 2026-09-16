//! A metric definition: the JSON a reader stores, and what it compiles into.

mod clock;
mod compiler;
mod field;
mod filter;
mod people;
mod runner;

use std::collections::HashMap;

use serde::Deserialize;
use thiserror::Error;

use crate::domain::query::time_window::MaximumRange;

use field::FieldType;
use filter::FilterBind;
pub(crate) use people::People;
pub(crate) use runner::{MetricRunError, MetricRunner, RunResult};

use clock::TimeField;
use field::is_identifier;
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

/// What this query addresses, and whether the clock it declares is sound.
impl MetricQuery {
    /// The table alone, with any database it was written with stripped off.
    pub(crate) fn table(&self) -> &str {
        self.split().1
    }

    /// The database, whether it came in its own field or qualified the table.
    pub(crate) fn database(&self) -> Option<&str> {
        self.split().0
    }

    /// A table written `database.table` is read as both.
    ///
    /// The map the model is shown, and the lookup tool it calls, both address
    /// a table as `database.table` - so it writes the qualified name in the
    /// table field, and refusing that only spends a round trip teaching it a
    /// distinction the wire format makes and nothing else does. Split only on
    /// a single dot with an identifier either side; anything else stays whole
    /// and is refused by the identifier check as before.
    fn split(&self) -> (Option<&str>, &str) {
        if self.database.is_some() {
            return (self.database.as_deref(), &self.table);
        }

        match self.table.split_once('.') {
            Some((database, table)) if is_identifier(database) && is_identifier(table) => {
                (Some(database), table)
            }
            _ => (None, &self.table),
        }
    }

    /// The table as the query addressed it, database and all.
    pub(crate) fn qualified(&self) -> String {
        match self.split() {
            (Some(database), table) => format!("{database}.{table}"),
            (None, table) => table.to_owned(),
        }
    }

    /// The columns a result carries, in order — each field's `as_name`. What
    /// a widget must name to draw anything.
    pub(crate) fn column_names(&self) -> Vec<String> {
        let clocked = self.valid_clock();
        let mut names = Vec::with_capacity(self.fields.len() + usize::from(clocked));
        if clocked {
            names.push("bucket".to_owned());
        }
        names.extend(self.fields.iter().map(|field| field.as_name.clone()));
        names
    }

    fn valid_clock(&self) -> bool {
        self.has_clock().unwrap_or(false)
    }

    fn maximum(&self) -> Result<Option<MaximumRange>, MetricQueryError> {
        Ok(self
            .max_range
            .as_deref()
            .map(MaximumRange::parse)
            .transpose()?)
    }

    pub(crate) fn check_window(&self) -> Result<(), MetricQueryError> {
        self.has_clock()?;
        self.maximum()?;

        Ok(())
    }

    /// Whether this metric names a timestamp to window and bucket by, and a
    /// refusal when it names one it cannot read.
    pub(crate) fn has_clock(&self) -> Result<bool, MetricQueryError> {
        Ok(self
            .time
            .as_ref()
            .map(TimeField::source)
            .transpose()?
            .is_some())
    }
}

/// The one read that resolves a relative window before the metric runs.
#[derive(Debug)]
pub(crate) struct UndatedQuery {
    sql: String,
    binds: Vec<FilterBind>,
}

#[derive(Debug)]
pub(crate) struct CompiledQuery {
    sql: String,
    binds: Vec<FilterBind>,
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
