//! A condition a metric keeps rows by, and the value bound into it.

use serde::Deserialize;

use super::MetricQueryError;
use super::field::{FieldType, Source};
use super::over::Over;

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

    pub(super) fn source(&self) -> Result<Source<'_>, MetricQueryError> {
        Source::resolve(self.json.as_deref(), self.column.as_deref())
            .ok_or_else(|| MetricQueryError::FieldSource("a filter".to_owned()))
    }

    /// What this filter reads, and the value bound against it.
    pub(super) fn compare(
        &self,
        over: Option<Over<'_>>,
        qualifier: Option<&str>,
    ) -> Result<(String, FilterBind), MetricQueryError> {
        if let Some(over) = over {
            let read = over.read(self.declared.as_deref(), "a filter", qualifier)?;
            let bound = self.bind_value(self.declared.as_deref().unwrap_or("a filter"))?;

            return Ok((read, bound));
        }

        let source = self.source()?;

        Ok((source.sql(self.r#type, qualifier)?, self.bind(source)?))
    }

    fn bind_value(&self, named: &str) -> Result<FilterBind, MetricQueryError> {
        match self.r#type {
            FieldType::String => self.value.as_str().map(|v| FilterBind::Str(v.to_owned())),
            FieldType::Int => self.value.as_i64().map(FilterBind::Int),
            FieldType::Float => self.value.as_f64().map(FilterBind::Float),
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
}

impl FilterBind {
    #[cfg(test)]
    fn as_display_string(&self) -> String {
        match self {
            Self::Str(value) => value.clone(),
            Self::Int(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
        }
    }

    pub(super) fn bind_onto(&self, query: clickhouse::query::Query) -> clickhouse::query::Query {
        match self {
            Self::Str(value) => query.bind(value),
            Self::Int(value) => query.bind(value),
            Self::Float(value) => query.bind(value),
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
