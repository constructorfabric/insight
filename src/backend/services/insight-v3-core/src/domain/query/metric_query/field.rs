//! A column a metric selects, and where its value is read from.

use std::collections::HashSet;
use std::fmt::Write as _;

use serde::Deserialize;

use super::MetricQueryError;
use super::filter::{Filter, FilterBind};
use super::over::Over;
use super::people::PersonHandle;

const MAX_IDENTIFIER_CHARS: usize = 128;

/// How to sort the rows. Without it the grouping's own columns order the
/// result, which cannot answer "the most" or "the largest".
#[derive(Debug, Deserialize)]
pub(super) struct OrderBy {
    /// One of the query's own `as_name` values.
    pub(super) field: String,
    #[serde(default)]
    pub(super) direction: Direction,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Direction {
    #[default]
    Asc,
    Desc,
}

impl Direction {
    pub(super) fn sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct Field {
    /// The declared field of the dataset this reads. The two below it are
    /// refused on a metric that reads one.
    #[serde(default, rename = "field")]
    pub(super) declared: Option<String>,
    #[serde(default)]
    pub(super) json: Option<String>,
    #[serde(default)]
    pub(super) column: Option<String>,
    pub(super) r#type: FieldType,
    #[serde(default)]
    pub(super) agg: Option<Agg>,
    pub(super) as_name: String,
    /// What this column holds a person by, when it holds one.
    #[serde(default)]
    pub(super) person: Option<PersonHandle>,
    /// What this aggregate counts, when it counts only part of the rows.
    ///
    /// A rate's numerator and denominator live in the same column, told apart
    /// by another - `gate_passed` and `gate_runs` are both `value` - so each
    /// aggregate carries its own condition rather than the query carrying one.
    #[serde(default)]
    pub(super) when: Vec<Filter>,
    /// Which element of an array payload this field reads, when the elements
    /// are told apart only by a name inside each one.
    #[serde(default, rename = "where")]
    pub(super) r#where: Option<Filter>,
    /// Two of this query's own fields, divided: `[numerator, denominator]`.
    #[serde(default)]
    pub(super) divide: Option<Vec<String>>,
    /// Whether that division reads as a percentage.
    #[serde(default)]
    pub(super) percent: bool,
}

impl Field {
    /// The declared field this reads, when it names one.
    pub(super) fn reads(&self) -> Option<&str> {
        self.declared.as_deref()
    }

    /// What this field does to the value it reads.
    pub(super) fn aggregate(&self) -> Option<super::Aggregate> {
        self.agg.map(|agg| {
            if agg.arithmetic() {
                super::Aggregate::Arithmetic
            } else {
                super::Aggregate::Ordering
            }
        })
    }

    /// The conditions this aggregate alone keeps rows by.
    pub(super) fn conditions(&self) -> &[Filter] {
        &self.when
    }

    /// Where this field addresses a record itself rather than naming a
    /// declared field.
    pub(super) fn physical_keys(&self, at: &str, into: &mut Vec<(String, &'static str)>) {
        if self.json.is_some() {
            into.push((format!("{at}.json"), "json"));
        }
        if self.column.is_some() {
            into.push((format!("{at}.column"), "column"));
        }
        if self.person.is_some() {
            into.push((format!("{at}.person"), "person"));
        }
        if self.r#where.is_some() {
            into.push((format!("{at}.where"), "where"));
        }
    }

    pub(super) fn source(&self) -> Result<Source<'_>, MetricQueryError> {
        Source::resolve(self.json.as_deref(), self.column.as_deref())
            .ok_or_else(|| MetricQueryError::FieldSource(format!("field `{}`", self.as_name)))
    }

    /// What this field selects.
    ///
    /// Counting rows reads no value at all, so `count` alone is `count()` —
    /// the most natural aggregate there is, and one the language could not
    /// express while every field had to name a source. Every other
    /// aggregate, and every plain field, still reads exactly one.
    pub(super) fn expression(
        &self,
        over: Option<Over<'_>>,
        qualifier: Option<&str>,
        binds: &mut Vec<FilterBind>,
    ) -> Result<String, MetricQueryError> {
        if self.agg == Some(Agg::Count) && self.reads_nothing() {
            return Ok(match self.condition(over, qualifier, binds)? {
                Some(condition) => format!("countIf({condition})"),
                None => "count()".to_owned(),
            });
        }

        // The read binds before the condition does, because it is the read
        // that SQL states first.
        let read = self.read(over, qualifier, binds)?;
        let condition = self.condition(over, qualifier, binds)?;

        Ok(match (self.agg, condition) {
            (Some(agg), Some(condition)) => format!("{}If({read}, {condition})", agg.sql()),
            (Some(agg), None) => format!("{}({read})", agg.sql()),
            (None, _) => read,
        })
    }

    fn read(
        &self,
        over: Option<Over<'_>>,
        qualifier: Option<&str>,
        binds: &mut Vec<FilterBind>,
    ) -> Result<String, MetricQueryError> {
        if let Some(over) = over {
            return over.read(self.declared.as_deref(), &self.as_name, qualifier);
        }

        let source = self.source()?;

        let Some(selector) = &self.r#where else {
            return source.sql(self.r#type, qualifier);
        };

        let Source::Json { path, .. } = source else {
            return Err(MetricQueryError::Selector(self.as_name.clone()));
        };

        let selected = selector.source()?;
        let Source::Json { path: key, .. } = selected else {
            return Err(MetricQueryError::Selector(self.as_name.clone()));
        };

        let matched = selector.r#type.extract("x", key)?;
        binds.push(selector.bind(selected)?);

        let element = format!(
            "arrayFirst(x -> {matched} {} ?, JSONExtractArrayRaw({}))",
            selector.op.sql(),
            source.payload_sql(qualifier)?
        );

        self.r#type.extract(&element, path)
    }

    /// This field's own conditions, as one expression, binding their values.
    pub(super) fn condition(
        &self,
        over: Option<Over<'_>>,
        qualifier: Option<&str>,
        binds: &mut Vec<FilterBind>,
    ) -> Result<Option<String>, MetricQueryError> {
        if self.when.is_empty() {
            return Ok(None);
        }

        let mut parts = Vec::with_capacity(self.when.len());
        for one in &self.when {
            let (condition, bound) = one.predicate(over, qualifier)?;
            parts.push(condition);
            binds.push(bound);
        }

        Ok(Some(parts.join(" AND ")))
    }

    /// Whether this field names nothing to read, which only counting rows may.
    fn reads_nothing(&self) -> bool {
        self.declared.is_none() && self.json.is_none() && self.column.is_none()
    }

    /// One field over another, when this field is a rate rather than a value.
    ///
    /// The two are this query's own `as_name`s, already selected: `ClickHouse`
    /// resolves an alias inside the same select list, so a rate needs no
    /// second pass over the table. `nullIf` keeps a zero denominator a
    /// missing rate instead of an error.
    pub(super) fn ratio(
        &self,
        selected: &HashSet<&str>,
    ) -> Result<Option<String>, MetricQueryError> {
        let Some(names) = &self.divide else {
            return Ok(None);
        };

        let [numerator, denominator] = names.as_slice() else {
            return Err(MetricQueryError::Ratio(format!(
                "`{}` divides exactly two of this query's fields",
                self.as_name
            )));
        };

        for name in [numerator, denominator] {
            if !selected.contains(name.as_str()) {
                return Err(MetricQueryError::Ratio(format!(
                    "`{}` divides `{name}`, which no earlier field of this query selects",
                    self.as_name
                )));
            }
        }

        let division = format!("(`{numerator}` / nullIf(`{denominator}`, 0))");

        Ok(Some(if self.percent {
            format!("(100 * {division})")
        } else {
            division
        }))
    }

    /// The same, keeping only the rows this field's own conditions admit.
    pub(super) fn aggregated_if(&self, read: String, condition: Option<String>) -> String {
        match (self.agg, condition) {
            (Some(agg), Some(condition)) => format!("{}If({read}, {condition})", agg.sql()),
            (Some(agg), None) => format!("{}({read})", agg.sql()),
            (None, _) => read,
        }
    }
}

/// Where a value is read from. A JSON payload is `raw_data` unless the
/// source names the column holding it.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum Source<'a> {
    Json {
        payload: Option<&'a str>,
        path: &'a str,
    },
    Column(&'a str),
}

impl<'a> Source<'a> {
    pub(super) fn resolve(json: Option<&'a str>, column: Option<&'a str>) -> Option<Self> {
        match (json, column) {
            (Some(path), payload) => Some(Self::Json { payload, path }),
            (None, Some(column)) => Some(Self::Column(column)),
            (None, None) => None,
        }
    }

    pub(super) fn name(self) -> &'a str {
        match self {
            Self::Json { path: name, .. } | Self::Column(name) => name,
        }
    }

    pub(super) fn payload_sql(self, qualifier: Option<&str>) -> Result<String, MetricQueryError> {
        let Self::Json { payload, .. } = self else {
            return Err(MetricQueryError::Identifier(self.name().to_owned()));
        };

        Ok(match payload {
            Some(column) => qualified(column, qualifier)?,
            None => match qualifier {
                Some(alias) => format!("`{alias}`.raw_data"),
                None => "raw_data".to_owned(),
            },
        })
    }

    pub(super) fn sql(
        self,
        field_type: FieldType,
        qualifier: Option<&str>,
    ) -> Result<String, MetricQueryError> {
        match self {
            Self::Json { path, .. } => {
                let payload = self.payload_sql(qualifier)?;
                field_type.extract(&payload, path)
            }
            Self::Column(column) => qualified(column, qualifier),
        }
    }
}

fn qualified(column: &str, qualifier: Option<&str>) -> Result<String, MetricQueryError> {
    if !is_identifier(column) {
        return Err(MetricQueryError::Identifier(column.to_owned()));
    }

    Ok(match qualifier {
        Some(alias) => format!("`{alias}`.`{column}`"),
        None => format!("`{column}`"),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FieldType {
    String,
    Int,
    Float,
}

impl FieldType {
    /// `path` is dotted: one key per segment.
    pub(super) fn extract(self, payload: &str, path: &str) -> Result<String, MetricQueryError> {
        let function = match self {
            Self::String => "JSONExtractString",
            Self::Int => "JSONExtractInt",
            Self::Float => "JSONExtractFloat",
        };

        let mut keys = String::new();
        for segment in path.split('.') {
            if !is_identifier(segment) {
                return Err(MetricQueryError::Identifier(path.to_owned()));
            }
            let _ = write!(keys, ", '{segment}'");
        }

        Ok(format!("{function}({payload}{keys})"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Agg {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl Agg {
    /// Whether this aggregate needs a number under it.
    pub(super) fn arithmetic(self) -> bool {
        match self {
            Self::Sum | Self::Avg => true,
            Self::Count | Self::Min | Self::Max => false,
        }
    }

    pub(super) fn sql(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
        }
    }
}

pub(super) fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_IDENTIFIER_CHARS
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `ClickHouse`'s `JSON` format serialises wide integers as JSON strings;
/// parse declared `int`/`float` columns back into numbers.
pub(super) fn coerce_value(value: serde_json::Value, field_type: FieldType) -> serde_json::Value {
    let serde_json::Value::String(text) = value else {
        return value;
    };

    match field_type {
        FieldType::String => {}
        FieldType::Int => {
            if let Ok(number) = text.parse::<i64>() {
                return serde_json::Value::Number(number.into());
            }
        }
        FieldType::Float => {
            if let Some(number) = text
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
            {
                return serde_json::Value::Number(number);
            }
        }
    }

    serde_json::Value::String(text)
}
