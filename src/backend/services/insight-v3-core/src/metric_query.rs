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
mod tests {
    use serde_json::json;

    use crate::catalog::TableEngine;
    use crate::time_window::RequestedRange;

    use super::*;

    fn query(value: serde_json::Value) -> MetricQuery {
        serde_json::from_value(value).unwrap_or_else(|error| panic!("valid metric: {error}"))
    }

    fn people() -> People {
        People::new("identity")
    }

    #[test]
    fn a_plain_column_selected_beside_an_aggregate_must_be_grouped() {
        let mixed = query(json!({
            "table": "events",
            "fields": [
                { "json": "author", "type": "string", "as_name": "author" },
                { "json": "lines", "type": "int", "agg": "sum", "as_name": "lines" }
            ],
            "group_by": [],
            "filters": []
        }));

        let refusal = mixed.compile(&people());

        assert!(
            matches!(&refusal, Err(MetricQueryError::Ungrouped(name)) if name == "author"),
            "ClickHouse would refuse this itself, and only after the caller got a 500: {refusal:?}"
        );
    }

    fn window(token: &str, bucketed: bool) -> crate::time_window::Window {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-10T15:00:00Z")
            .unwrap_or_else(|error| panic!("the synthetic clock parses: {error}"))
            .to_utc();
        let resolved = RequestedRange::parse(token)
            .and_then(|range| range.resolve(now))
            .unwrap_or_else(|error| panic!("`{token}` resolves: {error}"));

        if bucketed {
            resolved
        } else {
            resolved.unbucketed()
        }
    }

    fn timed_metric(time: &serde_json::Value) -> MetricQuery {
        query(json!({
            "table": "events",
            "time": time,
            "fields": [{ "agg": "count", "type": "int", "as_name": "total" }],
            "filters": []
        }))
    }

    fn compiled(
        metric: &MetricQuery,
        token: &str,
        bucketed: bool,
        engine: TableEngine,
    ) -> CompiledQuery {
        metric
            .compile_window(&people(), &window(token, bucketed), engine)
            .unwrap_or_else(|error| panic!("`{token}` compiles: {error}"))
    }

    #[test]
    fn json_and_column_clocks_compile_as_datetime_sources() {
        let json_clock = timed_metric(&json!({ "json": "occurred_at" }));
        let column_clock = timed_metric(&json!({ "column": "occurred_at", "type": "datetime" }));

        let json_sql = compiled(&json_clock, "P7D", true, TableEngine::MergeTree).sql;
        let column_sql = compiled(&column_clock, "P7D", true, TableEngine::MergeTree).sql;

        assert!(
            json_sql.contains(
                "parseDateTimeBestEffortOrNull(JSONExtractString(raw_data, 'occurred_at'))"
            ),
            "{json_sql}"
        );
        assert!(
            column_sql.contains("toStartOfDay(`occurred_at`, 'UTC') AS `bucket`"),
            "{column_sql}"
        );
    }

    #[test]
    fn a_clock_reads_the_payload_column_it_names() {
        let metric = timed_metric(&json!({
            "column": "event_json", "json": "occurred_at", "type": "datetime"
        }));

        let compiled = metric
            .compile_window(&people(), &window("P7D", true), TableEngine::MergeTree)
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled.sql.contains(
                "parseDateTimeBestEffortOrNull(JSONExtractString(`event_json`, 'occurred_at'))"
            ),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn malformed_or_non_datetime_clocks_are_refused() {
        for time in [
            json!({}),
            json!({ "column": "not-safe`", "type": "datetime" }),
            json!({ "column": "at", "type": "string" }),
        ] {
            let metric = timed_metric(&time);
            assert!(
                metric
                    .compile_window(&people(), &window("P7D", true), TableEngine::MergeTree)
                    .is_err(),
                "should reject {time}"
            );
        }
    }

    #[test]
    fn a_bucket_is_injected_into_select_grouping_and_default_order() {
        let metric = timed_metric(&json!({ "column": "occurred_at" }));
        let compiled = compiled(&metric, "PQC", true, TableEngine::MergeTree);

        assert!(
            compiled.sql.starts_with(
                "SELECT toStartOfWeek(`occurred_at`, 1, 'UTC') AS `bucket`, count() AS `total`"
            ),
            "{}",
            compiled.sql
        );
        assert!(
            compiled.sql.contains("GROUP BY `bucket` ORDER BY `bucket`"),
            "{}",
            compiled.sql
        );
        assert_eq!(compiled.binds.len(), 2);
    }

    #[test]
    fn an_unbucketed_window_keeps_its_half_open_predicates() {
        let metric = timed_metric(&json!({ "column": "occurred_at" }));
        let compiled = compiled(&metric, "P30D", false, TableEngine::MergeTree);

        assert!(!compiled.sql.contains(" AS `bucket`"), "{}", compiled.sql);
        assert!(
            compiled.sql.contains(
                "WHERE `occurred_at` >= fromUnixTimestamp64Milli(?, 'UTC') AND `occurred_at` < fromUnixTimestamp64Milli(?, 'UTC')"
            ),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn a_clock_cannot_also_be_a_filter_and_bucket_is_reserved() {
        let filtered = query(json!({
            "table": "events",
            "time": { "json": "occurred_at" },
            "fields": [{ "agg": "count", "type": "int", "as_name": "total" }],
            "filters": [{ "json": "occurred_at", "type": "string", "op": "gte", "value": "synthetic" }]
        }));
        let colliding = query(json!({
            "table": "events",
            "time": { "column": "occurred_at" },
            "fields": [{ "column": "kind", "type": "string", "as_name": "bucket" }],
            "filters": []
        }));

        assert!(matches!(
            filtered.compile_window(&people(), &window("P7D", true), TableEngine::MergeTree),
            Err(MetricQueryError::ClockFilter(_))
        ));
        assert!(matches!(
            colliding.compile_window(&people(), &window("P7D", true), TableEngine::MergeTree),
            Err(MetricQueryError::BucketAlias)
        ));
    }

    #[test]
    fn a_clock_cannot_be_reused_by_an_aggregate_condition() {
        let metric = query(json!({
            "table": "events",
            "time": { "column": "occurred_at" },
            "fields": [{
                "agg": "count", "type": "int", "as_name": "total",
                "when": [{ "column": "occurred_at", "type": "string", "op": "gte", "value": "synthetic" }]
            }]
        }));

        assert!(matches!(
            metric.compile_window(&people(), &window("P7D", true), TableEngine::MergeTree),
            Err(MetricQueryError::ClockFilter(_))
        ));
    }

    #[test]
    fn a_direct_window_on_a_clockless_metric_is_refused() {
        let metric = query(json!({
            "table": "events",
            "fields": [{ "agg": "count", "type": "int", "as_name": "total" }]
        }));

        assert!(matches!(
            metric.compile_window(&people(), &window("P7D", false), TableEngine::MergeTree),
            Err(MetricQueryError::ClocklessWindow)
        ));
        assert!(metric.compile(&people()).is_ok());
    }

    #[test]
    fn finite_metric_caps_reject_wider_and_unbounded_windows() {
        let metric = query(json!({
            "table": "events",
            "time": { "column": "occurred_at" },
            "max_range": "P30D",
            "fields": [{ "agg": "count", "type": "int", "as_name": "total" }]
        }));

        assert!(
            metric
                .compile_window(&people(), &window("P30D", false), TableEngine::MergeTree)
                .is_ok()
        );
        assert!(matches!(
            metric.compile_window(&people(), &window("P1Y", false), TableEngine::MergeTree),
            Err(MetricQueryError::RangeExceedsMaximum(_))
        ));
        assert!(matches!(
            metric.compile_window(&people(), &window("inf", false), TableEngine::MergeTree),
            Err(MetricQueryError::RangeExceedsMaximum(_))
        ));
    }

    #[test]
    fn replacing_engines_alone_compile_with_final() {
        let metric = timed_metric(&json!({ "column": "occurred_at" }));
        let plain = compiled(&metric, "P7D", false, TableEngine::MergeTree);
        let replacing = compiled(&metric, "P7D", false, TableEngine::ReplacingMergeTree);

        assert!(plain.sql.contains("FROM `events` WHERE"), "{}", plain.sql);
        assert!(
            replacing.sql.contains("FROM `events` FINAL WHERE"),
            "{}",
            replacing.sql
        );
    }

    #[test]
    fn a_ratio_over_two_aggregates_needs_no_group_by_of_its_own() {
        let rate = query(json!({
            "table": "events",
            "fields": [
                { "column": "value", "type": "int", "agg": "sum", "as_name": "passed" },
                { "column": "value", "type": "int", "agg": "sum", "as_name": "runs" },
                { "type": "float", "as_name": "rate", "divide": ["passed", "runs"], "percent": true }
            ],
            "group_by": [],
            "filters": []
        }));

        assert!(rate.compile(&people()).is_ok());
    }

    #[test]
    fn final_follows_the_fact_alias_when_identity_joins_require_one() {
        let metric = query(json!({
            "table": "events",
            "time": { "column": "occurred_at" },
            "fields": [
                { "column": "actor", "type": "string", "as_name": "actor", "person": "email" },
                { "agg": "count", "type": "int", "as_name": "total" }
            ],
            "group_by": ["actor"]
        }));

        let compiled = compiled(&metric, "P7D", true, TableEngine::ReplacingMergeTree);

        assert!(
            compiled
                .sql
                .contains("FROM `events` AS `__f` FINAL LEFT JOIN"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn an_undated_query_counts_the_rows_without_a_clock() {
        let metric = timed_metric(&json!({ "column": "occurred_at" }));

        let undated = metric
            .undated_query(TableEngine::MergeTree)
            .unwrap_or_else(|error| panic!("the undated count compiles: {error}"))
            .unwrap_or_else(|| panic!("a clocked metric has an undated count"));

        assert_eq!(
            undated.sql,
            "SELECT countIf(isNull(`occurred_at`)) AS undated FROM `events`"
        );
    }

    #[test]
    fn an_undated_query_reads_the_same_rows_the_metric_does() {
        let metric = query(json!({
            "table": "events",
            "time": { "column": "occurred_at" },
            "fields": [{ "agg": "count", "type": "int", "as_name": "total" }],
            "filters": [{ "column": "repo", "type": "string", "op": "eq", "value": "one" }]
        }));

        let undated = metric
            .undated_query(TableEngine::ReplacingMergeTree)
            .unwrap_or_else(|error| panic!("the undated count compiles: {error}"))
            .unwrap_or_else(|| panic!("a clocked metric has an undated count"));

        assert!(
            undated.sql.contains("FROM `events` FINAL WHERE `repo` = ?"),
            "{}",
            undated.sql
        );
        assert_eq!(undated.binds.len(), 1);
    }

    #[test]
    fn a_clockless_metric_has_no_undated_count_to_read() {
        let metric = timed_metric(&serde_json::Value::Null);

        let undated = metric
            .undated_query(TableEngine::MergeTree)
            .unwrap_or_else(|error| panic!("a clockless metric is not an error: {error}"));

        assert!(undated.is_none());
    }

    #[test]
    fn an_all_time_window_leaves_out_the_rows_with_no_clock() {
        let metric = timed_metric(&json!({ "column": "occurred_at" }));

        let compiled = compiled(&metric, "inf", true, TableEngine::MergeTree);

        assert!(
            compiled.sql.contains("WHERE `occurred_at` IS NOT NULL"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn a_json_clock_reads_a_missing_key_as_no_clock_rather_than_a_failure() {
        let metric = timed_metric(&json!({ "json": "occurred_at" }));

        let compiled = compiled(&metric, "P7D", true, TableEngine::MergeTree);

        assert!(
            compiled.sql.contains(
                "parseDateTimeBestEffortOrNull(JSONExtractString(raw_data, 'occurred_at'))"
            ),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn only_a_clocked_metric_exposes_the_injected_bucket_column() {
        let clocked = timed_metric(&json!({ "column": "occurred_at" }));
        let clockless = timed_metric(&serde_json::Value::Null);

        assert_eq!(clocked.column_names(), ["bucket", "total"]);
        assert_eq!(clockless.column_names(), ["total"]);
    }

    fn merged_by_author(person: &str) -> MetricQuery {
        query(json!({
            "table": "silver.class_git_pull_requests",
            "fields": [
                { "column": "author_email", "type": "string", "as_name": "author", "person": person },
                { "agg": "count", "column": "pr_id", "type": "int", "as_name": "merged" }
            ],
            "group_by": ["author"],
            "filters": [{ "column": "state", "type": "string", "op": "eq", "value": "MERGED" }],
            "order_by": { "field": "merged", "direction": "desc" }
        }))
    }

    #[test]
    fn a_field_aggregates_only_what_its_own_condition_matches() {
        // A rate's two halves live in one column, told apart by another: the
        // numerator and the denominator cannot each have their own query.
        let metric = query(json!({
            "database": "insight",
            "table": "ci_metric_observations",
            "fields": [
                { "column": "value", "type": "int", "agg": "sum", "as_name": "passed",
                  "when": [{ "column": "measure_key", "type": "string", "op": "eq",
                             "value": "gate_passed" }] },
                { "column": "value", "type": "int", "agg": "sum", "as_name": "runs",
                  "when": [{ "column": "measure_key", "type": "string", "op": "eq",
                             "value": "gate_runs" }] }
            ],
            "group_by": [],
            "filters": []
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled
                .sql
                .contains("sumIf(`value`, `measure_key` = ?) AS `passed`"),
            "{}",
            compiled.sql
        );
        assert_eq!(
            compiled.binds,
            vec!["gate_passed".to_owned(), "gate_runs".to_owned()]
        );
    }

    #[test]
    fn a_condition_on_a_count_needs_no_column() {
        let metric = query(json!({
            "table": "events",
            "fields": [
                { "agg": "count", "type": "int", "as_name": "merged",
                  "when": [{ "json": "state", "type": "string", "op": "eq",
                             "value": "MERGED" }] }
            ],
            "group_by": [],
            "filters": []
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled
                .sql
                .contains("countIf(JSONExtractString(raw_data, 'state') = ?) AS `merged`"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn a_ratio_divides_two_of_the_querys_own_fields() {
        let metric = query(json!({
            "database": "insight",
            "table": "ci_metric_observations",
            "fields": [
                { "column": "value", "type": "int", "agg": "sum", "as_name": "passed" },
                { "column": "value", "type": "int", "agg": "sum", "as_name": "runs" },
                { "divide": ["passed", "runs"], "type": "float", "as_name": "pass_rate" }
            ],
            "group_by": [],
            "filters": []
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        // nullIf, so a denominator of zero is no rate rather than an error.
        assert!(
            compiled
                .sql
                .contains("(`passed` / nullIf(`runs`, 0)) AS `pass_rate`"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn a_ratio_can_read_as_a_percentage() {
        let metric = query(json!({
            "table": "events",
            "fields": [
                { "agg": "count", "type": "int", "as_name": "part" },
                { "agg": "count", "type": "int", "as_name": "whole" },
                { "divide": ["part", "whole"], "percent": true, "type": "float",
                  "as_name": "share" }
            ],
            "group_by": [],
            "filters": []
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled
                .sql
                .contains("(100 * (`part` / nullIf(`whole`, 0))) AS `share`"),
            "{}",
            compiled.sql
        );
        // Named, so whoever draws 83.9 can draw 83.9% instead.
        assert_eq!(compiled.percents, ["share"]);
    }

    #[test]
    fn a_ratio_naming_a_field_that_is_not_there_is_refused() {
        let metric = query(json!({
            "table": "events",
            "fields": [
                { "agg": "count", "type": "int", "as_name": "part" },
                { "divide": ["part", "nothing_like_this"], "type": "float",
                  "as_name": "share" }
            ],
            "group_by": [],
            "filters": []
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::Ratio(_))
        ));
    }

    #[test]
    fn a_ratio_reading_a_field_declared_after_it_is_refused() {
        // The alias only exists once it has been selected, so the order in the
        // field list is the order the SQL can resolve.
        let metric = query(json!({
            "table": "events",
            "fields": [
                { "divide": ["part", "whole"], "type": "float", "as_name": "share" },
                { "agg": "count", "type": "int", "as_name": "part" },
                { "agg": "count", "type": "int", "as_name": "whole" }
            ],
            "group_by": [],
            "filters": []
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::Ratio(_))
        ));
    }

    #[test]
    fn a_ratio_needs_exactly_two_fields_to_divide() {
        let metric = query(json!({
            "table": "events",
            "fields": [
                { "agg": "count", "type": "int", "as_name": "part" },
                { "divide": ["part"], "type": "float", "as_name": "share" }
            ],
            "group_by": [],
            "filters": []
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::Ratio(_))
        ));
    }

    #[test]
    fn a_field_condition_binds_before_the_query_wide_filters() {
        // The select list comes before the WHERE clause, so its values bind
        // first or every placeholder after it takes the wrong one.
        let metric = query(json!({
            "database": "insight",
            "table": "ci_metric_observations",
            "fields": [
                { "column": "value", "type": "int", "agg": "sum", "as_name": "passed",
                  "when": [{ "column": "measure_key", "type": "string", "op": "eq",
                             "value": "gate_passed" }] }
            ],
            "group_by": [],
            "filters": [
                { "column": "metric_date", "type": "string", "op": "gte",
                  "value": "2026-08-09" }
            ]
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert_eq!(
            compiled.binds,
            vec!["gate_passed".to_owned(), "2026-08-09".to_owned()]
        );
    }

    #[test]
    fn a_person_column_selects_the_name_identity_knows() {
        let compiled = merged_by_author("email")
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled.sql.contains(
                "coalesce(nullIf(`__p0`.`display_name`, ''), `__f`.`author_email`) AS `author`"
            ),
            "{}",
            compiled.sql
        );
        assert!(
            compiled.sql.contains(
                "LEFT JOIN __people_by_email AS `__p0` ON `__p0`.`handle` = `__f`.`author_email`"
            ),
            "{}",
            compiled.sql
        );
        assert!(
            compiled
                .sql
                .contains("FROM `silver`.`class_git_pull_requests` AS `__f`"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn the_latest_name_wins_before_anything_joins_to_it() {
        // A person carries a row per name they have ever had. Joining those
        // rows directly multiplies every fact by that history.
        let compiled = merged_by_author("email")
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled.sql.starts_with(
                "WITH __person_name AS (SELECT `person_id`, argMax(`value_effective`, `created_at`) AS `display_name` FROM `identity`.`identity_persons` WHERE `value_type` = 'display_name' GROUP BY `person_id`)"
            ),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn resolving_a_name_qualifies_every_other_read() {
        let compiled = merged_by_author("email")
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled.sql.contains("count(`__f`.`pr_id`) AS `merged`"),
            "{}",
            compiled.sql
        );
        assert!(
            compiled.sql.contains("WHERE `__f`.`state` = ?"),
            "{}",
            compiled.sql
        );
        assert_eq!(compiled.binds, vec!["MERGED".to_owned()]);
    }

    #[test]
    fn a_query_that_names_no_person_joins_nothing() {
        let compiled = query(json!({
            "table": "silver.class_git_pull_requests",
            "fields": [{ "column": "author_email", "type": "string", "as_name": "author" }],
            "group_by": ["author"],
            "filters": []
        }))
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(!compiled.sql.contains("WITH "), "{}", compiled.sql);
        assert!(!compiled.sql.contains("JOIN"), "{}", compiled.sql);
        assert!(!compiled.sql.contains("__f"), "{}", compiled.sql);
    }

    #[test]
    fn a_person_id_is_compared_as_text() {
        // The same column is a UUID in one table and a String in the next.
        let compiled = query(json!({
            "table": "silver.class_git_pull_requests",
            "fields": [
                { "column": "author_person_id", "type": "string", "as_name": "author", "person": "id" },
                { "agg": "count", "type": "int", "as_name": "prs" }
            ],
            "group_by": ["author"],
            "filters": []
        }))
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled.sql.contains(
                "LEFT JOIN __people_by_id AS `__p0` ON `__p0`.`handle` = toString(`__f`.`author_person_id`)"
            ),
            "{}",
            compiled.sql
        );
        assert!(
            compiled
                .sql
                .contains("__people_by_id AS (SELECT toString(`person_id`) AS `handle`"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn two_person_columns_resolve_one_each() {
        let compiled = query(json!({
            "table": "silver.class_git_pr_review_events",
            "fields": [
                { "column": "author_email", "type": "string", "as_name": "author", "person": "email" },
                { "column": "actor_person_id", "type": "string", "as_name": "reviewer", "person": "id" },
                { "agg": "count", "type": "int", "as_name": "reviews" }
            ],
            "group_by": ["author", "reviewer"],
            "filters": []
        }))
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(compiled.sql.contains("AS `__p0`"), "{}", compiled.sql);
        assert!(compiled.sql.contains("AS `__p1`"), "{}", compiled.sql);
        assert_eq!(
            compiled.sql.matches("LEFT JOIN").count(),
            2,
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn a_person_inside_the_payload_resolves_the_same_way() {
        let compiled = query(json!({
            "table": "events",
            "fields": [
                { "json": "author", "type": "string", "as_name": "author", "person": "email" },
                { "agg": "sum", "json": "lines", "type": "int", "as_name": "lines" }
            ],
            "group_by": ["author"],
            "filters": []
        }))
        .compile(&people())
        .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled
                .sql
                .contains("ON `__p0`.`handle` = JSONExtractString(`__f`.raw_data, 'author')"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn the_identity_database_is_whatever_the_stand_configured() {
        let compiled = merged_by_author("email")
            .compile(&People::new("identity_two"))
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled.sql.contains("`identity_two`.`identity_persons`"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn an_identity_database_outside_the_charset_is_refused() {
        assert!(matches!(
            merged_by_author("email").compile(&People::new("identity`; DROP TABLE x; --")),
            Err(MetricQueryError::Identifier(_))
        ));
    }

    #[test]
    fn a_grouped_count_compiles_to_json_extraction_over_the_payload() {
        let metric = query(json!({
            "table": "events",
            "fields": [
                { "json": "day", "type": "string", "as_name": "day" },
                { "json": "lines", "type": "int", "agg": "sum", "as_name": "lines" }
            ],
            "group_by": ["day"],
            "filters": [],
            "limit": 100
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert_eq!(
            compiled.sql,
            "SELECT JSONExtractString(raw_data, 'day') AS `day`, \
             sum(JSONExtractInt(raw_data, 'lines')) AS `lines` \
             FROM `events` GROUP BY `day` ORDER BY `day` LIMIT 100"
        );
        assert!(compiled.binds.is_empty());
    }

    #[test]
    fn filters_bind_their_values() {
        let metric = query(json!({
            "table": "events",
            "fields": [{ "json": "author", "type": "string", "as_name": "author" }],
            "group_by": [],
            "filters": [
                { "json": "event", "type": "string", "op": "eq", "value": "commit" }
            ]
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled
                .sql
                .contains("WHERE JSONExtractString(raw_data, 'event') = ?")
        );
        assert_eq!(compiled.binds, vec!["commit".to_owned()]);
    }

    #[test]
    fn an_identifier_outside_the_charset_is_refused() {
        let metric = query(json!({
            "table": "events`; DROP TABLE events; --",
            "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
            "group_by": [],
            "filters": []
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::Identifier(_))
        ));
    }

    #[test]
    fn ordering_by_a_selected_value_beats_the_grouping_order() {
        // "Which author changed the most lines" is unanswerable without
        // this: ordering by the grouped column returns whoever sorts first
        // alphabetically, and the reply presents it as the largest.
        let metric = query(json!({
            "table": "events",
            "fields": [
                { "json": "author", "type": "string", "as_name": "author" },
                { "json": "lines", "type": "int", "agg": "sum", "as_name": "total_lines" }
            ],
            "group_by": ["author"],
            "filters": [],
            "order_by": { "field": "total_lines", "direction": "desc" },
            "limit": 1
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("the query compiles: {error}"));

        assert!(
            compiled.sql.contains("ORDER BY `total_lines` DESC"),
            "{}",
            compiled.sql
        );
        // One ORDER BY, not the grouping's as well.
        assert_eq!(
            compiled.sql.matches("ORDER BY").count(),
            1,
            "{}",
            compiled.sql
        );
        assert!(compiled.sql.ends_with(" LIMIT 1"), "{}", compiled.sql);
    }

    #[test]
    fn ordering_defaults_to_ascending() {
        let metric = query(json!({
            "table": "events",
            "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
            "group_by": [],
            "filters": [],
            "order_by": { "field": "day" }
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("the query compiles: {error}"));

        assert!(
            compiled.sql.contains("ORDER BY `day` ASC"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn ordering_by_a_column_the_query_does_not_select_is_refused() {
        let metric = query(json!({
            "table": "events",
            "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
            "group_by": [],
            "filters": [],
            "order_by": { "field": "lines" }
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::OrderBy(_))
        ));
    }

    #[test]
    fn counting_rows_needs_no_column() {
        // "How many rows are in this table" is the first thing anyone asks,
        // and it reads no value.
        let metric = query(json!({
            "table": "bronze_github.commits",
            "fields": [{ "agg": "count", "type": "int", "as_name": "rows" }],
            "group_by": [],
            "filters": []
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("the query compiles: {error}"));

        assert!(
            compiled.sql.contains("count() AS `rows`"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn counting_one_column_still_names_it() {
        let metric = query(json!({
            "table": "events",
            "fields": [{ "agg": "count", "column": "author", "type": "string", "as_name": "n" }],
            "group_by": [],
            "filters": []
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("the query compiles: {error}"));

        assert!(
            compiled.sql.contains("count(`author`) AS `n`"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn an_aggregate_that_is_not_count_still_needs_a_source() {
        // sum() of nothing is not a question.
        let metric = query(json!({
            "table": "events",
            "fields": [{ "agg": "sum", "type": "int", "as_name": "total" }],
            "group_by": [],
            "filters": []
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::FieldSource(_))
        ));
    }

    #[test]
    fn a_table_written_with_its_database_is_read_as_both() {
        // The map and the lookup tool both address a table as
        // `database.table`, so the model writes it that way.
        let metric = query(json!({
            "table": "bronze_github.commits",
            "fields": [{ "column": "sha", "type": "string", "as_name": "sha" }],
            "group_by": [],
            "filters": []
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("the query compiles: {error}"));

        assert!(
            compiled.sql.contains("FROM `bronze_github`.`commits`"),
            "{}",
            compiled.sql
        );
        assert_eq!(metric.database(), Some("bronze_github"));
        assert_eq!(metric.table(), "commits");
    }

    #[test]
    fn a_database_field_wins_over_a_qualified_table() {
        let metric = query(json!({
            "database": "silver",
            "table": "class_git_commits",
            "fields": [{ "column": "sha", "type": "string", "as_name": "sha" }],
            "group_by": [],
            "filters": []
        }));

        assert_eq!(metric.database(), Some("silver"));
        assert_eq!(metric.table(), "class_git_commits");
    }

    #[test]
    fn a_table_with_two_dots_is_still_refused() {
        // Splitting only rescues the one shape the model writes; anything
        // else stays whole and fails the identifier check.
        let metric = query(json!({
            "table": "a.b.c",
            "fields": [{ "column": "x", "type": "string", "as_name": "x" }],
            "group_by": [],
            "filters": []
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::Identifier(_))
        ));
    }

    #[test]
    fn a_metric_with_no_fields_is_refused() {
        let metric = query(json!({
            "table": "events", "fields": [], "group_by": [], "filters": []
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::NoFields)
        ));
    }

    #[test]
    fn numeric_filters_bind_the_numeric_value_not_its_json_encoding() {
        let metric = query(json!({
            "table": "events",
            "fields": [{ "json": "lines", "type": "int", "as_name": "lines" }],
            "group_by": [],
            "filters": [
                { "json": "lines", "type": "int", "op": "gt", "value": 5 }
            ]
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert_eq!(compiled.binds, vec!["5".to_owned()]);
    }

    #[test]
    fn a_filter_value_that_does_not_match_its_declared_type_is_refused() {
        let metric = query(json!({
            "table": "events",
            "fields": [{ "json": "lines", "type": "int", "as_name": "lines" }],
            "group_by": [],
            "filters": [
                { "json": "lines", "type": "int", "op": "gt", "value": "not-a-number" }
            ]
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::FilterValue(_))
        ));
    }

    #[test]
    fn coerces_string_typed_clickhouse_numbers_to_json_numbers() {
        assert_eq!(coerce_value(json!("132"), FieldType::Int), json!(132));
        assert_eq!(coerce_value(json!("12.5"), FieldType::Float), json!(12.5));
        assert_eq!(
            coerce_value(json!("2026-09-01"), FieldType::String),
            json!("2026-09-01")
        );
    }

    #[test]
    fn group_by_must_reference_a_selected_field() {
        let metric = query(json!({
            "table": "events",
            "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
            "group_by": ["not_selected"],
            "filters": []
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::GroupBy(_))
        ));
    }

    #[test]
    fn a_database_qualifies_the_table() {
        let metric = query(json!({
            "database": "silver",
            "table": "git_commits",
            "fields": [{ "column": "author", "type": "string", "as_name": "author" }]
        }));

        assert_eq!(metric.database(), Some("silver"));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled.sql.contains("FROM `silver`.`git_commits`"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn a_query_without_a_database_still_reads_the_bare_table() {
        let metric = query(json!({
            "table": "events",
            "fields": [{ "json": "day", "type": "string", "as_name": "day" }]
        }));

        assert_eq!(metric.database(), None);

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(compiled.sql.contains("FROM `events`"), "{}", compiled.sql);
    }

    #[test]
    fn a_field_naming_a_column_reads_it_without_json_extraction() {
        let metric = query(json!({
            "database": "silver",
            "table": "git_commits",
            "fields": [{ "column": "lines_changed", "type": "int", "as_name": "lines" }]
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled.sql.contains("`lines_changed` AS `lines`"),
            "{}",
            compiled.sql
        );
        assert!(!compiled.sql.contains("JSONExtract"), "{}", compiled.sql);
    }

    #[test]
    fn an_aggregate_wraps_a_column_as_it_wraps_an_extraction() {
        let metric = query(json!({
            "database": "silver",
            "table": "git_commits",
            "fields": [
                { "column": "lines_changed", "type": "int", "agg": "sum", "as_name": "total" }
            ]
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled.sql.contains("sum(`lines_changed`) AS `total`"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn a_filter_on_a_column_compares_it_and_still_binds_the_value() {
        let metric = query(json!({
            "database": "silver",
            "table": "git_commits",
            "fields": [{ "column": "author", "type": "string", "as_name": "author" }],
            "filters": [
                { "column": "event", "type": "string", "op": "eq", "value": "commit" }
            ]
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled.sql.contains("WHERE `event` = ?"),
            "{}",
            compiled.sql
        );
        assert_eq!(compiled.binds, vec!["commit".to_owned()]);
    }

    #[test]
    fn a_where_over_a_plain_column_is_refused() {
        let metric = query(json!({
            "table": "project_items",
            "fields": [{
                "column": "milestone_title",
                "type": "string",
                "as_name": "milestone",
                "where": { "json": "field.name", "type": "string", "op": "eq", "value": "Status" }
            }]
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::Selector(name)) if name == "milestone"
        ));
    }

    #[test]
    fn a_field_naming_neither_a_json_key_nor_a_column_is_refused() {
        let metric = query(json!({
            "table": "git_commits",
            "fields": [{ "type": "int", "as_name": "lines" }]
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::FieldSource(_))
        ));
    }

    #[test]
    fn a_filter_naming_neither_a_json_key_nor_a_column_is_refused() {
        let metric = query(json!({
            "table": "git_commits",
            "fields": [{ "column": "author", "type": "string", "as_name": "author" }],
            "filters": [{ "type": "string", "op": "eq", "value": "commit" }]
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::FieldSource(_))
        ));
    }

    #[test]
    fn a_database_outside_the_identifier_charset_is_refused() {
        let metric = query(json!({
            "database": "silver`; DROP TABLE git_commits; --",
            "table": "git_commits",
            "fields": [{ "json": "author", "type": "string", "as_name": "author" }]
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::Identifier(_))
        ));
    }
    #[test]
    fn a_json_field_reads_the_payload_column_it_names() {
        let metric = query(json!({
            "table": "project_items",
            "fields": [
                { "column": "field_values_json", "json": "status", "type": "string", "as_name": "status" }
            ],
            "group_by": [],
            "filters": []
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled
                .sql
                .contains("JSONExtractString(`field_values_json`, 'status')"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn a_dotted_json_path_reads_a_nested_key() {
        let metric = query(json!({
            "table": "events",
            "fields": [
                { "json": "field.name", "type": "string", "as_name": "field_name" }
            ],
            "group_by": [],
            "filters": []
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled
                .sql
                .contains("JSONExtractString(raw_data, 'field', 'name')"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn a_where_picks_the_one_array_element_the_field_means() {
        let metric = query(json!({
            "table": "project_items",
            "fields": [{
                "column": "field_values_json",
                "json": "name",
                "type": "string",
                "as_name": "status",
                "where": { "json": "field.name", "type": "string", "op": "eq", "value": "Status" }
            }],
            "group_by": [],
            "filters": []
        }));

        let compiled = metric
            .compile(&people())
            .unwrap_or_else(|error| panic!("compiles: {error}"));

        assert!(
            compiled.sql.contains(
                "JSONExtractString(arrayFirst(x -> JSONExtractString(x, 'field', 'name') = ?, \
                 JSONExtractArrayRaw(`field_values_json`)), 'name')"
            ),
            "{}",
            compiled.sql
        );
        assert_eq!(compiled.binds, vec!["Status".to_owned()]);
    }

    #[test]
    fn a_json_path_segment_outside_the_charset_is_refused() {
        let metric = query(json!({
            "table": "events",
            "fields": [
                { "json": "field.name'); DROP TABLE events; --", "type": "string", "as_name": "bad" }
            ],
            "group_by": [],
            "filters": []
        }));

        assert!(matches!(
            metric.compile(&people()),
            Err(MetricQueryError::Identifier(_))
        ));
    }
}
