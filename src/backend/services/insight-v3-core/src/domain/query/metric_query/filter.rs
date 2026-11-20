use serde::Deserialize;

use super::MetricQueryError;
use super::field::{FieldType, Source};
use super::over::Over;
use crate::domain::kinds::dataset::declaration::FieldType as DeclaredType;
use crate::domain::kinds::dataset::read::Form;

fn placeholder(declared: DeclaredType) -> &'static str {
    match declared {
        DeclaredType::Datetime => "parseDateTime64BestEffortOrNull(?, 3, 'UTC')",
        DeclaredType::String | DeclaredType::Int | DeclaredType::Float | DeclaredType::Bool => "?",
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct Filter {
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
    pub(super) fn reads(&self) -> Option<&str> {
        self.declared.as_deref()
    }

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

    pub(super) fn predicate(
        &self,
        over: Option<Over<'_>>,
        qualifier: Option<&str>,
    ) -> Result<(String, FilterBind), MetricQueryError> {
        let operator = self.op.sql();

        if let Some(over) = over {
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

    fn declared_bind(
        &self,
        declared: DeclaredType,
        named: &str,
    ) -> Result<FilterBind, MetricQueryError> {
        match declared {
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
            Self::Bool(value) => query.bind(u8::from(*value)),
        }
    }
}

#[cfg(test)]
impl PartialEq<String> for FilterBind {
    fn eq(&self, other: &String) -> bool {
        self.as_display_string() == *other
    }
}
