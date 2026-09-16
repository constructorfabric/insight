//! Compiles a metric's JSON definition into `ClickHouse` SQL and runs it.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fmt::Write as _;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::catalog::TableEngine;
use crate::time_window::{Bounds, Grain, MaximumRange, Window, WindowError};
use crate::undated::UndatedCount;

const MAX_IDENTIFIER_CHARS: usize = 128;
const DEFAULT_LIMIT: u32 = 1000;
const MAX_LIMIT: u32 = 10000;
const FETCH_TIMEOUT_SECS: u64 = 30;
const FACT_ALIAS: &str = "__f";
const NAME_CTE: &str = "__person_name";
const EMAIL_CTE: &str = "__people_by_email";
const ID_CTE: &str = "__people_by_id";
const IDENTITY_TABLE: &str = "identity_persons";
const MAX_RESULT_BYTES: usize = 5 * 1024 * 1024;

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

#[derive(Debug, Deserialize)]
struct TimeField {
    #[serde(default)]
    json: Option<String>,
    #[serde(default)]
    column: Option<String>,
    #[serde(default = "datetime_type")]
    r#type: String,
}

fn datetime_type() -> String {
    "datetime".to_owned()
}

impl TimeField {
    fn source(&self) -> Result<Source<'_>, MetricQueryError> {
        if self.r#type != "datetime" {
            return Err(MetricQueryError::ClockType(self.r#type.clone()));
        }
        let source = Source::resolve(self.json.as_deref(), self.column.as_deref())
            .ok_or(MetricQueryError::ClockSource)?;
        if !is_identifier(source.name()) {
            return Err(MetricQueryError::Identifier(source.name().to_owned()));
        }

        Ok(source)
    }
}

/// How a clock is read out of a row. A payload key that is absent or
/// unparseable reads as no clock, not as a failed query.
fn clock_expression(
    source: Source<'_>,
    qualifier: Option<&str>,
) -> Result<String, MetricQueryError> {
    let read = source.sql(FieldType::String, qualifier)?;

    Ok(match source {
        Source::Json { .. } => format!("parseDateTimeBestEffortOrNull({read})"),
        Source::Column(_) => read,
    })
}

/// How to sort the rows. Without it the grouping's own columns order the
/// result, which cannot answer "the most" or "the largest".
#[derive(Debug, Deserialize)]
struct OrderBy {
    /// One of the query's own `as_name` values.
    field: String,
    #[serde(default)]
    direction: Direction,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Direction {
    #[default]
    Asc,
    Desc,
}

impl Direction {
    fn sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

#[derive(Debug, Deserialize)]
struct Field {
    #[serde(default)]
    json: Option<String>,
    #[serde(default)]
    column: Option<String>,
    r#type: FieldType,
    #[serde(default)]
    agg: Option<Agg>,
    as_name: String,
    /// What this column holds a person by, when it holds one.
    #[serde(default)]
    person: Option<PersonHandle>,
    /// What this aggregate counts, when it counts only part of the rows.
    ///
    /// A rate's numerator and denominator live in the same column, told apart
    /// by another - `gate_passed` and `gate_runs` are both `value` - so each
    /// aggregate carries its own condition rather than the query carrying one.
    #[serde(default)]
    when: Vec<Filter>,
    /// Which element of an array payload this field reads, when the elements
    /// are told apart only by a name inside each one.
    #[serde(default, rename = "where")]
    r#where: Option<Filter>,
    /// Two of this query's own fields, divided: `[numerator, denominator]`.
    #[serde(default)]
    divide: Option<Vec<String>>,
    /// Whether that division reads as a percentage.
    #[serde(default)]
    percent: bool,
}

/// Which handle a person column carries.
///
/// A fact table names a person the way its source system did - the address
/// they committed under, the id an API returned. Neither is the name anyone
/// would recognise them by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PersonHandle {
    Email,
    Id,
}

impl PersonHandle {
    fn cte(self) -> &'static str {
        match self {
            Self::Email => EMAIL_CTE,
            Self::Id => ID_CTE,
        }
    }

    /// The handle as the identity side spells it: ids are compared as text,
    /// because the fact table stores one as a UUID, a String, or neither.
    fn key(self, read: &str) -> String {
        match self {
            Self::Email => read.to_owned(),
            Self::Id => format!("toString({read})"),
        }
    }
}

/// Where a person's name is resolved from.
///
/// The identity database on this stand. Only the database is configurable:
/// the table and the `value_type` rows inside it are identity's own schema,
/// not something a stand chooses.
#[derive(Debug, Clone)]
pub(crate) struct People {
    database: String,
}

impl People {
    pub(crate) fn new(database: impl Into<String>) -> Self {
        Self {
            database: database.into(),
        }
    }

    /// The `WITH` clauses the joins read from.
    ///
    /// A person accumulates a row per name they have ever had, so the latest
    /// one wins by `argMax` before anything joins to it. Joining the rows
    /// directly multiplies every fact by that history - it turned 752 merged
    /// pull requests into 18,800.
    fn prelude(&self, handles: &[PersonHandle]) -> Result<String, MetricQueryError> {
        if !is_identifier(&self.database) {
            return Err(MetricQueryError::Identifier(self.database.clone()));
        }
        let persons = format!("`{}`.`{IDENTITY_TABLE}`", self.database);

        let mut ctes = vec![format!(
            "{NAME_CTE} AS (SELECT `person_id`, argMax(`value_effective`, `created_at`) AS `display_name` FROM {persons} WHERE `value_type` = 'display_name' GROUP BY `person_id`)"
        )];
        for handle in handles {
            ctes.push(match handle {
                PersonHandle::Email => format!(
                    "{EMAIL_CTE} AS (SELECT `h`.`handle` AS `handle`, `n`.`display_name` AS `display_name` FROM (SELECT `value_effective` AS `handle`, argMax(`person_id`, `created_at`) AS `person_id` FROM {persons} WHERE `value_type` = 'email' GROUP BY `value_effective`) AS `h` INNER JOIN {NAME_CTE} AS `n` ON `n`.`person_id` = `h`.`person_id`)"
                ),
                PersonHandle::Id => format!(
                    "{ID_CTE} AS (SELECT toString(`person_id`) AS `handle`, `display_name` FROM {NAME_CTE})"
                ),
            });
        }

        Ok(format!("WITH {} ", ctes.join(", ")))
    }
}

#[derive(Debug, Deserialize)]
struct Filter {
    #[serde(default)]
    json: Option<String>,
    #[serde(default)]
    column: Option<String>,
    r#type: FieldType,
    op: FilterOp,
    value: serde_json::Value,
}

impl Filter {
    fn source(&self) -> Result<Source<'_>, MetricQueryError> {
        Source::resolve(self.json.as_deref(), self.column.as_deref())
            .ok_or_else(|| MetricQueryError::FieldSource("a filter".to_owned()))
    }

    fn bind(&self, source: Source<'_>) -> Result<FilterBind, MetricQueryError> {
        match self.r#type {
            FieldType::String => self.value.as_str().map(|v| FilterBind::Str(v.to_owned())),
            FieldType::Int => self.value.as_i64().map(FilterBind::Int),
            FieldType::Float => self.value.as_f64().map(FilterBind::Float),
        }
        .ok_or_else(|| MetricQueryError::FilterValue(source.name().to_owned()))
    }
}

impl Field {
    fn source(&self) -> Result<Source<'_>, MetricQueryError> {
        Source::resolve(self.json.as_deref(), self.column.as_deref())
            .ok_or_else(|| MetricQueryError::FieldSource(format!("field `{}`", self.as_name)))
    }

    /// What this field selects.
    ///
    /// Counting rows reads no value at all, so `count` alone is `count()` —
    /// the most natural aggregate there is, and one the language could not
    /// express while every field had to name a source. Every other
    /// aggregate, and every plain field, still reads exactly one.
    fn expression(
        &self,
        qualifier: Option<&str>,
        binds: &mut Vec<FilterBind>,
    ) -> Result<String, MetricQueryError> {
        if self.agg == Some(Agg::Count) && self.json.is_none() && self.column.is_none() {
            return Ok(match self.condition(qualifier, binds)? {
                Some(condition) => format!("countIf({condition})"),
                None => "count()".to_owned(),
            });
        }

        // The read binds before the condition does, because it is the read
        // that SQL states first.
        let read = self.read(qualifier, binds)?;
        let condition = self.condition(qualifier, binds)?;

        Ok(match (self.agg, condition) {
            (Some(agg), Some(condition)) => format!("{}If({read}, {condition})", agg.sql()),
            (Some(agg), None) => format!("{}({read})", agg.sql()),
            (None, _) => read,
        })
    }

    fn read(
        &self,
        qualifier: Option<&str>,
        binds: &mut Vec<FilterBind>,
    ) -> Result<String, MetricQueryError> {
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
    fn condition(
        &self,
        qualifier: Option<&str>,
        binds: &mut Vec<FilterBind>,
    ) -> Result<Option<String>, MetricQueryError> {
        if self.when.is_empty() {
            return Ok(None);
        }

        let mut parts = Vec::with_capacity(self.when.len());
        for one in &self.when {
            let source = one.source()?;
            let read = source.sql(one.r#type, qualifier)?;
            parts.push(format!("{read} {} ?", one.op.sql()));
            binds.push(one.bind(source)?);
        }

        Ok(Some(parts.join(" AND ")))
    }

    /// One field over another, when this field is a rate rather than a value.
    ///
    /// The two are this query's own `as_name`s, already selected: `ClickHouse`
    /// resolves an alias inside the same select list, so a rate needs no
    /// second pass over the table. `nullIf` keeps a zero denominator a
    /// missing rate instead of an error.
    fn ratio(&self, selected: &HashSet<&str>) -> Result<Option<String>, MetricQueryError> {
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

    fn aggregated(&self, read: String) -> String {
        match self.agg {
            Some(agg) => format!("{}({read})", agg.sql()),
            None => read,
        }
    }
}

/// Where a value is read from. A JSON payload is `raw_data` unless the
/// source names the column holding it.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Source<'a> {
    Json {
        payload: Option<&'a str>,
        path: &'a str,
    },
    Column(&'a str),
}

impl<'a> Source<'a> {
    fn resolve(json: Option<&'a str>, column: Option<&'a str>) -> Option<Self> {
        match (json, column) {
            (Some(path), payload) => Some(Self::Json { payload, path }),
            (None, Some(column)) => Some(Self::Column(column)),
            (None, None) => None,
        }
    }

    fn name(self) -> &'a str {
        match self {
            Self::Json { path: name, .. } | Self::Column(name) => name,
        }
    }

    fn payload_sql(self, qualifier: Option<&str>) -> Result<String, MetricQueryError> {
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

    fn sql(
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
    fn extract(self, payload: &str, path: &str) -> Result<String, MetricQueryError> {
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
pub(crate) enum Agg {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl Agg {
    fn sql(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FilterOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
}

impl FilterOp {
    fn sql(self) -> &'static str {
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
    fn as_display_string(&self) -> String {
        match self {
            Self::Str(value) => value.clone(),
            Self::Int(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
        }
    }

    fn bind_onto(&self, query: clickhouse::query::Query) -> clickhouse::query::Query {
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
impl PartialEq<String> for FilterBind {
    fn eq(&self, other: &String) -> bool {
        self.as_display_string() == *other
    }
}

/// What the field list compiled to.
struct Selection<'a> {
    parts: Vec<String>,
    as_names: HashSet<&'a str>,
    column_types: HashMap<String, FieldType>,
    /// The joins a person's name needs, if any field asked for one.
    joins: String,
    handles: Vec<PersonHandle>,
    /// Values bound by the fields themselves, before any filter's.
    binds: Vec<FilterBind>,
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

    fn time_expression(
        &self,
        window: &Window,
        qualifier: Option<&str>,
        as_names: &HashSet<&str>,
    ) -> Result<Option<String>, MetricQueryError> {
        let clock = self.time.as_ref().map(TimeField::source).transpose()?;
        if matches!(window, Window::Requested { .. }) && clock.is_none() {
            return Err(MetricQueryError::ClocklessWindow);
        }

        if let Some(maximum) = self.maximum()?
            && !maximum.allows(window)
        {
            return Err(MetricQueryError::RangeExceedsMaximum(
                self.max_range.clone().unwrap_or_default(),
            ));
        }

        if let Some(clock) = clock {
            let filters = self
                .filters
                .iter()
                .chain(self.fields.iter().flat_map(|field| &field.when));
            for filter in filters {
                if filter.source()? == clock {
                    return Err(MetricQueryError::ClockFilter(clock.name().to_owned()));
                }
            }
        }
        if window.grain().is_some() && as_names.contains("bucket") {
            return Err(MetricQueryError::BucketAlias);
        }

        clock
            .map(|source| clock_expression(source, qualifier))
            .transpose()
    }

    /// Each field as it is selected, with whatever joining in a person's
    /// name takes with it.
    /// Both directions of the `GROUP BY`: every group names a selected
    /// column, and every plain column selected beside an aggregate is
    /// grouped. Without the second, `ClickHouse` refuses the SQL instead —
    /// too late to tell the caller which column to fix.
    fn check_grouping(&self, as_names: &HashSet<&str>) -> Result<(), MetricQueryError> {
        for group in &self.group_by {
            if !is_identifier(group) {
                return Err(MetricQueryError::Identifier(group.clone()));
            }
            if !as_names.contains(group.as_str()) {
                return Err(MetricQueryError::GroupBy(group.clone()));
            }
        }

        if !self.fields.iter().any(|field| field.agg.is_some()) {
            return Ok(());
        }

        for field in &self.fields {
            // A ratio reads two fields this query already selects, so it
            // stands or falls with them.
            let derived = field.agg.is_some() || field.divide.is_some();
            if !derived && !self.group_by.contains(&field.as_name) {
                return Err(MetricQueryError::Ungrouped(field.as_name.clone()));
            }
        }

        Ok(())
    }

    fn selection(&self, qualifier: Option<&str>) -> Result<Selection<'_>, MetricQueryError> {
        let mut selection = Selection {
            parts: Vec::with_capacity(self.fields.len()),
            as_names: HashSet::with_capacity(self.fields.len()),
            column_types: HashMap::with_capacity(self.fields.len()),
            joins: String::new(),
            handles: Vec::new(),
            binds: Vec::new(),
        };

        for (index, field) in self.fields.iter().enumerate() {
            if !is_identifier(&field.as_name) {
                return Err(MetricQueryError::Identifier(field.as_name.clone()));
            }
            selection
                .column_types
                .insert(field.as_name.clone(), field.r#type);

            if let Some(ratio) = field.ratio(&selection.as_names)? {
                selection.as_names.insert(field.as_name.as_str());
                selection
                    .parts
                    .push(format!("{ratio} AS `{}`", field.as_name));
                continue;
            }
            selection.as_names.insert(field.as_name.as_str());

            let expression = match field.person {
                Some(handle) => {
                    if !selection.handles.contains(&handle) {
                        selection.handles.push(handle);
                    }
                    let read = field.source()?.sql(field.r#type, qualifier)?;
                    let alias = format!("__p{index}");
                    let _ = write!(
                        selection.joins,
                        " LEFT JOIN {} AS `{alias}` ON `{alias}`.`handle` = {}",
                        handle.cte(),
                        handle.key(&read)
                    );
                    // Identity knows nobody by this handle: show what the
                    // table itself says, never a blank.
                    field.aggregated(format!(
                        "coalesce(nullIf(`{alias}`.`display_name`, ''), {read})"
                    ))
                }
                None => field.expression(qualifier, &mut selection.binds)?,
            };
            selection
                .parts
                .push(format!("{expression} AS `{}`", field.as_name));
        }

        Ok(selection)
    }

    pub(crate) fn compile(&self, people: &People) -> Result<CompiledQuery, MetricQueryError> {
        self.compile_window(people, &Window::legacy(), TableEngine::Other)
    }

    /// Whether this definition is shaped like something runnable, answered
    /// without compiling it or reading anything.
    pub(crate) fn check(&self) -> Result<(), MetricQueryError> {
        let (_, table) = self.split();

        self.validate_shape(table)
    }

    pub(crate) fn compile_window(
        &self,
        people: &People,
        window: &Window,
        engine: TableEngine,
    ) -> Result<CompiledQuery, MetricQueryError> {
        let (_, table) = self.split();
        self.validate_shape(table)?;

        // Resolving a name joins another table in, and then a bare column
        // could mean either side - so every read carries the fact table's
        // alias exactly when there is something to be ambiguous with.
        let qualifier = self
            .fields
            .iter()
            .any(|field| field.person.is_some())
            .then_some(FACT_ALIAS);
        let Selection {
            mut parts,
            as_names,
            column_types,
            joins,
            handles,
            mut binds,
        } = self.selection(qualifier)?;

        let time_expression = self.time_expression(window, qualifier, &as_names)?;
        let bucket = window.grain().zip(time_expression.as_deref());
        if let Some((grain, clock)) = bucket {
            parts.insert(
                0,
                format!("{} AS `bucket`", bucket_expression(grain, clock)),
            );
        }

        self.check_grouping(&as_names)?;

        let mut where_parts = Vec::with_capacity(self.filters.len() + 2);
        binds.reserve(self.filters.len() + 2);
        add_window_predicates(
            window,
            time_expression.as_deref(),
            &mut where_parts,
            &mut binds,
        )?;
        self.add_filters(qualifier, &mut where_parts, &mut binds)?;

        let from = self.table_source(qualifier, engine);
        let prelude = if handles.is_empty() {
            String::new()
        } else {
            people.prelude(&handles)?
        };
        let mut sql = format!("{prelude}SELECT {} FROM {from}{joins}", parts.join(", "));
        if !where_parts.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_parts.join(" AND "));
        }
        let mut groups = self.group_by.clone();
        if bucket.is_some() {
            groups.insert(0, "bucket".to_owned());
        }
        if !groups.is_empty() {
            let backticked: Vec<String> = groups.iter().map(|group| format!("`{group}`")).collect();
            sql.push_str(" GROUP BY ");
            sql.push_str(&backticked.join(", "));

            if self.order_by.is_none() {
                sql.push_str(" ORDER BY ");
                sql.push_str(&backticked.join(", "));
            }
        }
        if let Some(order) = &self.order_by {
            if !as_names.contains(order.field.as_str()) {
                return Err(MetricQueryError::OrderBy(order.field.clone()));
            }
            let _ = write!(sql, " ORDER BY `{}` {}", order.field, order.direction.sql());
        }
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        let _ = write!(sql, " LIMIT {limit}");

        Ok(CompiledQuery {
            sql,
            binds,
            column_types,
            percents: self
                .fields
                .iter()
                .filter(|field| field.percent)
                .map(|field| field.as_name.clone())
                .collect(),
        })
    }

    fn add_filters(
        &self,
        qualifier: Option<&str>,
        where_parts: &mut Vec<String>,
        binds: &mut Vec<FilterBind>,
    ) -> Result<(), MetricQueryError> {
        for filter in &self.filters {
            let source = filter.source()?;
            let read = source.sql(filter.r#type, qualifier)?;
            where_parts.push(format!("{read} {} ?", filter.op.sql()));
            binds.push(filter.bind(source)?);
        }

        Ok(())
    }

    fn table_source(&self, qualifier: Option<&str>, engine: TableEngine) -> String {
        let (database, table) = self.split();
        let mut from = match database {
            Some(database) => format!("`{database}`.`{table}`"),
            None => format!("`{table}`"),
        };

        if let Some(alias) = qualifier {
            let _ = write!(from, " AS `{alias}`");
        }
        if engine.requires_final() {
            from.push_str(" FINAL");
        }

        from
    }

    /// How many of the rows a metric reads carry no clock — one read of the
    /// same rows. A metric with no clock has none, and answers `None`.
    pub(crate) fn undated_query(
        &self,
        engine: TableEngine,
    ) -> Result<Option<UndatedQuery>, MetricQueryError> {
        let (_, table) = self.split();
        self.validate_shape(table)?;

        let Some(time) = self.time.as_ref() else {
            return Ok(None);
        };
        let clock = clock_expression(time.source()?, None)?;

        let mut where_parts = Vec::with_capacity(self.filters.len());
        let mut binds = Vec::with_capacity(self.filters.len());
        self.add_filters(None, &mut where_parts, &mut binds)?;

        let mut sql = format!(
            "SELECT countIf(isNull({clock})) AS undated FROM {}",
            self.table_source(None, engine)
        );
        if !where_parts.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_parts.join(" AND "));
        }

        Ok(Some(UndatedQuery { sql, binds }))
    }

    fn validate_shape(&self, table: &str) -> Result<(), MetricQueryError> {
        if !is_identifier(table) {
            return Err(MetricQueryError::Identifier(self.table.clone()));
        }
        if let Some(database) = self.database()
            && !is_identifier(database)
        {
            return Err(MetricQueryError::Identifier(database.to_owned()));
        }
        if self.fields.is_empty() {
            return Err(MetricQueryError::NoFields);
        }

        Ok(())
    }
}

fn bucket_expression(grain: Grain, clock: &str) -> String {
    match grain {
        Grain::Hour => format!("toStartOfHour({clock}, 'UTC')"),
        Grain::Day => format!("toStartOfDay({clock}, 'UTC')"),
        Grain::Week => format!("toStartOfWeek({clock}, 1, 'UTC')"),
        Grain::Month => format!("toStartOfMonth({clock}, 'UTC')"),
    }
}

fn add_window_predicates(
    window: &Window,
    time_expression: Option<&str>,
    where_parts: &mut Vec<String>,
    binds: &mut Vec<FilterBind>,
) -> Result<(), MetricQueryError> {
    let Window::Requested { bounds, .. } = window else {
        return Ok(());
    };
    let Some(clock) = time_expression else {
        return Err(MetricQueryError::ClocklessWindow);
    };

    match *bounds {
        Bounds::Finite { from, to } => {
            where_parts.push(format!("{clock} >= fromUnixTimestamp64Milli(?, 'UTC')"));
            where_parts.push(format!("{clock} < fromUnixTimestamp64Milli(?, 'UTC')"));
            binds.push(FilterBind::Int(from.timestamp_millis()));
            binds.push(FilterBind::Int(to.timestamp_millis()));
        }
        Bounds::Unbounded => where_parts.push(format!("{clock} IS NOT NULL")),
    }

    Ok(())
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_IDENTIFIER_CHARS
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `ClickHouse`'s `JSON` format serialises wide integers as JSON strings;
/// parse declared `int`/`float` columns back into numbers.
fn coerce_value(value: serde_json::Value, field_type: FieldType) -> serde_json::Value {
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

/// The result of running a [`CompiledQuery`] against `ClickHouse`.
#[derive(Debug, Serialize)]
pub(crate) struct RunResult {
    pub(crate) columns: Vec<String>,
    pub(crate) rows: Vec<Vec<serde_json::Value>>,
    /// Which of those columns are percentages. `83.9` and `83.9%` are the
    /// same number until something says which one it is.
    pub(crate) percents: Vec<String>,
    /// How many rows the window left out because they carry no clock. Only
    /// a run that asked for a window leaves any out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) undated: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ResultMeta {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ClickHouseJsonResult {
    meta: Vec<ResultMeta>,
    data: Vec<serde_json::Map<String, serde_json::Value>>,
}

pub(crate) struct MetricRunner {
    client: insight_clickhouse::Client,
    fetch_timeout: Duration,
    people: People,
}

impl MetricRunner {
    pub(crate) fn new(client: insight_clickhouse::Client, people: People) -> Self {
        Self {
            client,
            fetch_timeout: Duration::from_secs(FETCH_TIMEOUT_SECS),
            people,
        }
    }

    /// Where the queries this runs resolve a person's name from.
    pub(crate) fn people(&self) -> &People {
        &self.people
    }

    pub(crate) async fn undated(
        &self,
        query: &UndatedQuery,
    ) -> Result<UndatedCount, MetricRunError> {
        let bytes = self.fetch(&query.sql, &query.binds).await?;

        Ok(UndatedCount::parse(&bytes)?)
    }

    pub(crate) async fn run(&self, compiled: &CompiledQuery) -> Result<RunResult, MetricRunError> {
        let bytes = self.fetch(&compiled.sql, &compiled.binds).await?;

        let parsed: ClickHouseJsonResult = serde_json::from_slice(&bytes)?;
        let columns: Vec<String> = parsed.meta.into_iter().map(|column| column.name).collect();
        let rows = parsed
            .data
            .into_iter()
            .map(|mut row| {
                columns
                    .iter()
                    .map(|name| {
                        let value = row.remove(name).unwrap_or(serde_json::Value::Null);
                        match compiled.column_types.get(name) {
                            Some(field_type) => coerce_value(value, *field_type),
                            None => value,
                        }
                    })
                    .collect()
            })
            .collect();

        Ok(RunResult {
            columns,
            rows,
            percents: compiled.percents.clone(),
            undated: None,
        })
    }

    async fn fetch(&self, sql: &str, binds: &[FilterBind]) -> Result<Vec<u8>, MetricRunError> {
        let mut query = self.client.query(sql);
        for bind in binds {
            query = bind.bind_onto(query);
        }
        let mut cursor = query.fetch_bytes("JSON")?;

        let fetch = async {
            let mut bytes = Vec::new();
            while let Some(chunk) = cursor.next().await? {
                let next_len = bytes.len().saturating_add(chunk.len());
                if next_len > MAX_RESULT_BYTES {
                    return Err(MetricRunError::ResultTooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok::<_, MetricRunError>(bytes)
        };

        tokio::time::timeout(self.fetch_timeout, fetch)
            .await
            .map_err(|_| MetricRunError::Timeout)?
    }
}

impl fmt::Debug for MetricRunner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetricRunner")
            .field("fetch_timeout", &self.fetch_timeout)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error)]
pub(crate) enum MetricRunError {
    #[error("metric query timed out")]
    Timeout,
    #[error("metric query result exceeded the size limit")]
    ResultTooLarge,
    #[error(transparent)]
    ClickHouse(clickhouse::error::Error),
    #[error(transparent)]
    InvalidResponse(#[from] serde_json::Error),
}

impl From<clickhouse::error::Error> for MetricRunError {
    fn from(error: clickhouse::error::Error) -> Self {
        match error {
            clickhouse::error::Error::TimedOut => Self::Timeout,
            error => Self::ClickHouse(error),
        }
    }
}

#[cfg(test)]
#[path = "metric_query/tests.rs"]
mod tests;
