//! A condition a metric keeps rows by, and the value bound into it.

use serde::Deserialize;

use super::MetricQueryError;
use super::field::{FieldType, Source};
use super::over::Over;
use crate::domain::kinds::dataset::declaration::FieldType as DeclaredType;
use crate::domain::kinds::dataset::read::Form;

/// How a bound value is written into the condition, so a moment reads back
/// as the same instant the record's own value was parsed into.
fn placeholder(declared: DeclaredType) -> &'static str {
    match declared {
        DeclaredType::Datetime => "parseDateTime64BestEffortOrNull(?, 3, 'UTC')",
        DeclaredType::String | DeclaredType::Int | DeclaredType::Float | DeclaredType::Bool => "?",
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct Filter {
    /// The declared field of the dataset this compares.
    #[serde(default, rename = "field")]
    pub(super) declared: Option<String>,
    #[serde(default)]
    pub(super) json: Option<String>,
    #[serde(default)]
    pub(super) column: Option<String>,
    pub(super) r#type: FieldType,
    pub(super) op: FilterOp,
    pub(super) value: serde_json::Value,
}

impl Filter {
    /// The declared field this compares, when it names one.
    pub(super) fn reads(&self) -> Option<&str> {
        self.declared.as_deref()
    }

    /// Where this condition addresses a record itself rather than naming a
    /// declared field.
    pub(super) fn physical_keys(&self, at: &str, into: &mut Vec<(String, &'static str)>) {
        if self.json.is_some() {
            into.push((format!("{at}.json"), "json"));
        }
        if self.column.is_some() {
            into.push((format!("{at}.column"), "column"));
        }
    }

    pub(super) fn source(&self) -> Result<Source<'_>, MetricQueryError> {
        Source::resolve(self.json.as_deref(), self.column.as_deref())
            .ok_or_else(|| MetricQueryError::FieldSource("a filter".to_owned()))
    }

    /// The whole condition: what this filter reads, the comparison, and the
    /// value bound against it.
    ///
    /// Over a dataset the declaration decides the type, because that is what
    /// the record was read as; the metric's own `type` describes the legacy
    /// relation only.
    pub(super) fn predicate(
        &self,
        over: Option<Over<'_>>,
        qualifier: Option<&str>,
    ) -> Result<(String, FilterBind), MetricQueryError> {
        let operator = self.op.sql();

        if let Some(over) = over {
            // Raw: a substitute is what a reader is shown, never what a
            // condition is judged against, or filtering on the substitute
            // would match every record that carries no value at all.
            let read = over.read_as(self.declared.as_deref(), "a filter", qualifier, Form::Raw)?;
            let declared = over.type_of(self.declared.as_deref(), "a filter")?;
            let named = self.declared.as_deref().unwrap_or("a filter");
            let bound = self.declared_bind(declared, named)?;

            return Ok((
                format!("{read} {operator} {}", placeholder(declared)),
                bound,
            ));
        }

        let source = self.source()?;
        let read = source.sql(self.r#type, qualifier)?;

        Ok((format!("{read} {operator} ?"), self.bind(source)?))
    }

    /// The value as the declared type takes it.
    fn declared_bind(
        &self,
        declared: DeclaredType,
        named: &str,
    ) -> Result<FilterBind, MetricQueryError> {
        match declared {
            // A moment is bound as the text it was written in and parsed by
            // the warehouse, the same way the record's own value is.
            DeclaredType::String | DeclaredType::Datetime => {
                self.value.as_str().map(|v| FilterBind::Str(v.to_owned()))
            }
            DeclaredType::Int => self.value.as_i64().map(FilterBind::Int),
            DeclaredType::Float => self.value.as_f64().map(FilterBind::Float),
            DeclaredType::Bool => self.value.as_bool().map(FilterBind::Bool),
        }
        .ok_or_else(|| MetricQueryError::FilterValue(named.to_owned()))
    }

    pub(super) fn bind(&self, source: Source<'_>) -> Result<FilterBind, MetricQueryError> {
        match self.r#type {
            FieldType::String => self.value.as_str().map(|v| FilterBind::Str(v.to_owned())),
            FieldType::Int => self.value.as_i64().map(FilterBind::Int),
            FieldType::Float => self.value.as_f64().map(FilterBind::Float),
        }
        .ok_or_else(|| MetricQueryError::FilterValue(source.name().to_owned()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum FilterOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
}

impl FilterOp {
    pub(super) fn sql(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::Ne => "!=",
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::Lt => "<",
            Self::Lte => "<=",
        }
    }
}

/// A single filter value, typed per its declared [`FieldType`].
///
/// `MetricQuery::compile` validates each filter value against its declared
/// type up front, so binding a numeric filter produces a numeric SQL literal
/// rather than a quoted string compared against a `JSONExtractInt`/`Float`
/// expression.
#[derive(Debug, Clone)]
pub(crate) enum FilterBind {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl FilterBind {
    #[cfg(test)]
    fn as_display_string(&self) -> String {
        match self {
            Self::Str(value) => value.clone(),
            Self::Int(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Bool(value) => value.to_string(),
        }
    }

    pub(super) fn bind_onto(&self, query: clickhouse::query::Query) -> clickhouse::query::Query {
        match self {
            Self::Str(value) => query.bind(value),
            Self::Int(value) => query.bind(value),
            Self::Float(value) => query.bind(value),
            // `Bool` is the warehouse's own one-byte number.
            Self::Bool(value) => query.bind(u8::from(*value)),
        }
    }
}

/// Compares against the string form asserted by `MetricQuery::compile`'s
/// unit tests, without stringifying the value used to actually bind the
/// query (see [`FilterBind::bind_onto`]).
#[cfg(test)]
impl PartialEq<String> for FilterBind {
    fn eq(&self, other: &String) -> bool {
        self.as_display_string() == *other
    }
}
