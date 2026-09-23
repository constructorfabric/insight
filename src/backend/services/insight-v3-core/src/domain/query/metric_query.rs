//! A metric definition: the JSON a reader stores, and what it compiles into.

mod clock;
mod compiler;
mod engine;
mod field;
mod filter;
pub(crate) mod over;
mod people;
mod runner;

use std::collections::HashMap;

use serde::Deserialize;
use thiserror::Error;

use crate::domain::kinds::dataset::declaration::BUCKET_COLUMN;
use crate::domain::query::time_window::MaximumRange;

pub(crate) use engine::TableEngine;
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

/// One place a metric names a field of its dataset.
#[derive(Debug)]
pub(crate) struct Reference<'a> {
    /// Where in the submitted body this reference sits.
    pub(crate) at: String,
    pub(crate) field: &'a str,
    pub(crate) used: Used<'a>,
}

/// What a metric does with the field it names, which is what decides whether
/// the declared type admits it.
#[derive(Debug)]
pub(crate) enum Used<'a> {
    /// Selected, with the aggregate applied to it if there is one.
    Selected(Option<Aggregate>),
    /// Compared against a value.
    Compared(&'a serde_json::Value),
    /// Read as the window's clock.
    Clock,
}

/// The aggregates that need a number under them, told from those that do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Aggregate {
    /// Sum or average, which only a number admits.
    Arithmetic,
    /// Count, minimum or maximum, which any type admits.
    Ordering,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MetricQuery {
    /// The dataset this metric reads. The relation below it is refused, and
    /// is read back only to say so.
    #[serde(default)]
    dataset: Option<String>,
    #[serde(default)]
    database: Option<String>,
    #[serde(default)]
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
    /// The dataset this metric reads, when it names one.
    pub(crate) fn dataset(&self) -> Option<&str> {
        self.dataset.as_deref()
    }

    /// Whether this metric addresses a relation of its own, which a metric
    /// over a dataset may not.
    pub(crate) fn addresses_a_relation(&self) -> bool {
        !self.table.is_empty() || self.database.is_some()
    }

    /// Every place this metric reaches into a record itself instead of naming
    /// a field the dataset declares.
    ///
    /// Where a value sits, and whether it holds a person, are the
    /// declaration's to answer; a metric that answers them itself reads
    /// something the dataset never promised.
    pub(crate) fn physical_references(&self) -> Vec<(String, &'static str)> {
        let mut reached = Vec::new();

        for (index, field) in self.fields.iter().enumerate() {
            let at = format!("fields[{index}]");
            field.physical_keys(&at, &mut reached);

            for (inner, condition) in field.conditions().iter().enumerate() {
                condition.physical_keys(&format!("{at}.when[{inner}]"), &mut reached);
            }
        }

        for (index, filter) in self.filters.iter().enumerate() {
            filter.physical_keys(&format!("filters[{index}]"), &mut reached);
        }

        if let Some(time) = self.time.as_ref() {
            time.physical_keys("time", &mut reached);
        }

        reached
    }

    /// Every declared field this metric names: the source of each selected
    /// field, of each filter, and of the clock.
    ///
    /// Told apart from the names the metric produces, which are its own
    /// columns and answer to nothing in the declaration.
    pub(crate) fn field_references(&self) -> Vec<Reference<'_>> {
        let mut named = Vec::new();

        for (index, field) in self.fields.iter().enumerate() {
            if let Some(read) = field.reads() {
                named.push(Reference {
                    at: format!("fields[{index}].field"),
                    field: read,
                    used: Used::Selected(field.aggregate()),
                });
            }
            for (inner, condition) in field.conditions().iter().enumerate() {
                if let Some(read) = condition.reads() {
                    named.push(Reference {
                        at: format!("fields[{index}].when[{inner}].field"),
                        field: read,
                        used: Used::Compared(&condition.value),
                    });
                }
            }
        }

        for (index, filter) in self.filters.iter().enumerate() {
            if let Some(read) = filter.reads() {
                named.push(Reference {
                    at: format!("filters[{index}].field"),
                    field: read,
                    used: Used::Compared(&filter.value),
                });
            }
        }

        if let Some(read) = self.time.as_ref().and_then(TimeField::reads) {
            named.push(Reference {
                at: "time.field".to_owned(),
                field: read,
                used: Used::Clock,
            });
        }

        named
    }

    /// The columns this metric produces, which is what its grouping and its
    /// ordering may name.
    pub(crate) fn output_names(&self) -> Vec<&str> {
        self.fields
            .iter()
            .map(|field| field.as_name.as_str())
            .collect()
    }

    /// Where the grouping and the ordering name a column, with the place in
    /// the body each came from.
    pub(crate) fn output_references(&self) -> Vec<(String, &str)> {
        let mut named: Vec<(String, &str)> = self
            .group_by
            .iter()
            .enumerate()
            .map(|(index, group)| (format!("group_by[{index}]"), group.as_str()))
            .collect();

        if let Some(order) = self.order_by.as_ref() {
            named.push(("order_by.field".to_owned(), order.field.as_str()));
        }

        named
    }

    /// The clock the metric names itself, if it names one.
    pub(crate) fn own_clock(&self) -> Option<&str> {
        self.time.as_ref().and_then(TimeField::reads)
    }
    /// The database, whether it came in its own field or qualified the table.
    pub(crate) fn database(&self) -> Option<&str> {
        self.split().0
    }

    pub(crate) fn table_name(&self) -> &str {
        self.split().1
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

    /// The columns a result carries, in order — each field's `as_name`. What
    /// a widget must name to draw anything.
    /// `clocked` says whether a window over this metric has a date to bucket
    /// by, which for a metric over a dataset only the declaration knows.
    pub(crate) fn column_names(&self, clocked: bool) -> Vec<String> {
        let mut names = Vec::with_capacity(self.fields.len() + usize::from(clocked));
        if clocked {
            names.push(BUCKET_COLUMN.to_owned());
        }
        names.extend(self.fields.iter().map(|field| field.as_name.clone()));
        names
    }

    fn maximum(&self) -> Result<Option<MaximumRange>, MetricQueryError> {
        Ok(self
            .max_range
            .as_deref()
            .map(MaximumRange::parse)
            .transpose()?)
    }

    /// Whether the window this metric declares holds up on its own.
    ///
    /// A metric over a dataset names a declared field as its clock, and
    /// whether that field is a date is the declaration's to answer, not this.
    pub(crate) fn check_window(&self) -> Result<(), MetricQueryError> {
        if self.dataset.is_none() {
            self.has_clock()?;
        }
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
    #[error("`{0}` must name a json key of 1-128 characters between its dots")]
    JsonKey(String),
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
    #[error("`{0}` is not a field of the dataset this metric reads")]
    UnknownField(String),
    #[error("a metric must name the `table` it reads")]
    NoTable,
    #[error("a metric reads a dataset, so it may not name a `table` or a `database`")]
    AddressesARelation,
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
