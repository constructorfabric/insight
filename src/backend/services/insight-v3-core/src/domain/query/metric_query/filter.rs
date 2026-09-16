//! A condition a metric keeps rows by, and the value bound into it.

use serde::Deserialize;

use super::MetricQueryError;
use super::field::{FieldType, Source};

#[derive(Debug, Deserialize)]
pub(super) struct Filter {
    #[serde(default)]
    pub(super) json: Option<String>,
    #[serde(default)]
    pub(super) column: Option<String>,
    pub(super) r#type: FieldType,
    pub(super) op: FilterOp,
    pub(super) value: serde_json::Value,
}

impl Filter {
    pub(super) fn source(&self) -> Result<Source<'_>, MetricQueryError> {
        Source::resolve(self.json.as_deref(), self.column.as_deref())
            .ok_or_else(|| MetricQueryError::FieldSource("a filter".to_owned()))
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
