mod clock;
mod compiler;
mod engine;
mod field;
mod filter;
pub(crate) mod over;
mod people;
mod runner;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::kinds::dataset::declaration::{
    BUCKET_COLUMN, Declaration, FieldType as DeclaredType,
};
use crate::domain::query::time_window::MaximumRange;

pub(crate) use engine::TableEngine;
use field::FieldType;
pub(crate) use filter::FilterBind;
pub(crate) use people::People;
pub(crate) use runner::{MetricRunError, MetricRunner, RunResult};

use clock::TimeField;
use field::is_identifier;
use field::{Field, OrderBy};
use filter::Filter;

use crate::domain::query::time_window::WindowError;

const FACT_ALIAS: &str = "__f";

pub(crate) const RECORD_COLUMN: &str = "__drilldown_record";

pub(crate) const TWIN_COLUMN: &str = "__drilldown_twin";

#[derive(Debug)]
pub(crate) struct Reference<'a> {
    pub(crate) at: String,
    pub(crate) field: &'a str,
    pub(crate) used: Used<'a>,
}

#[derive(Debug)]
pub(crate) enum Used<'a> {
    Selected(Option<Aggregate>),
    Compared(&'a serde_json::Value),
    Clock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ColumnKind {
    Text,
    Number,
    Date,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Aggregate {
    Arithmetic,
    Ordering,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MetricQuery {
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

impl MetricQuery {
    pub(crate) fn dataset(&self) -> Option<&str> {
        self.dataset.as_deref()
    }

    pub(crate) fn addresses_a_relation(&self) -> bool {
        !self.table.is_empty() || self.database.is_some()
    }

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

    pub(crate) fn output_names(&self) -> Vec<&str> {
        self.fields
            .iter()
            .map(|field| field.as_name.as_str())
            .collect()
    }

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

    pub(crate) fn own_clock(&self) -> Option<&str> {
        self.time.as_ref().and_then(TimeField::reads)
    }
    pub(crate) fn database(&self) -> Option<&str> {
        self.split().0
    }

    pub(crate) fn table_name(&self) -> &str {
        self.split().1
    }

    pub(crate) fn table_clock(&self) -> Option<&str> {
        let time = self.time.as_ref()?;

        time.column.as_deref().or(time.json.as_deref())
    }

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

    pub(crate) fn groups(&self) -> &[String] {
        &self.group_by
    }

    pub(crate) fn own_order(&self) -> Option<(&str, bool)> {
        self.order_by.as_ref().map(|order| {
            (
                order.field.as_str(),
                matches!(order.direction, field::Direction::Desc),
            )
        })
    }

    pub(crate) fn column_kinds(
        &self,
        declaration: Option<&Declaration>,
        clocked: bool,
    ) -> Vec<(String, ColumnKind)> {
        let mut kinds = Vec::with_capacity(self.fields.len() + usize::from(clocked));
        if clocked {
            kinds.push((BUCKET_COLUMN.to_owned(), ColumnKind::Date));
        }

        for field in &self.fields {
            let counted = field.agg == Some(field::Agg::Count)
                || field.agg.is_some_and(field::Agg::arithmetic)
                || field.divide.is_some();
            let kind = if counted {
                ColumnKind::Number
            } else {
                let declared = declaration
                    .zip(field.reads())
                    .and_then(|(declaration, named)| declaration.field(named))
                    .map(|declared| declared.r#type);
                match declared {
                    Some(DeclaredType::Int | DeclaredType::Float) => ColumnKind::Number,
                    Some(DeclaredType::Datetime) => ColumnKind::Date,
                    Some(DeclaredType::String | DeclaredType::Bool) => ColumnKind::Text,
                    None => match field.r#type {
                        FieldType::Int | FieldType::Float => ColumnKind::Number,
                        FieldType::String => ColumnKind::Text,
                    },
                }
            };
            kinds.push((field.as_name.clone(), kind));
        }

        kinds
    }

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

    pub(crate) fn check_window(&self) -> Result<(), MetricQueryError> {
        if self.dataset.is_none() {
            self.has_clock()?;
        }
        self.maximum()?;

        Ok(())
    }

    pub(crate) fn has_clock(&self) -> Result<bool, MetricQueryError> {
        Ok(self
            .time
            .as_ref()
            .map(TimeField::source)
            .transpose()?
            .is_some())
    }
}

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
    percents: Vec<String>,
    exacts: Vec<String>,
    hidden: Vec<String>,
    numbered: bool,
}

impl CompiledQuery {
    pub(crate) fn sql(&self) -> &str {
        &self.sql
    }

    pub(crate) fn binds(&self) -> &[FilterBind] {
        &self.binds
    }

    pub(crate) fn percents(&self) -> &[String] {
        &self.percents
    }

    pub(crate) fn exacts(&self) -> &[String] {
        &self.exacts
    }

    pub(crate) fn hidden(&self) -> &[String] {
        &self.hidden
    }

    pub(crate) fn numbered(&self) -> bool {
        self.numbered
    }
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
    #[error("a metric must name the `dataset` or the `table` it reads")]
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
