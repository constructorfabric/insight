//! Compiling a metric definition into `ClickHouse` SQL.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use super::clock::{TimeField, add_window_predicates, bucket_expression, clock_expression};
use super::field::{FieldType, is_identifier};
use super::filter::FilterBind;
use super::over::Over;
use super::people::{People, PersonHandle};
use super::{CompiledQuery, FACT_ALIAS, MetricQuery, MetricQueryError, UndatedQuery};
use crate::domain::kinds::dataset::declaration::BUCKET_COLUMN;
use crate::domain::kinds::metric::answerable::effective_clock;
use crate::domain::query::time_window::Window;
use crate::store::catalog::TableEngine;

const DEFAULT_LIMIT: u32 = 1000;
const MAX_LIMIT: u32 = 10000;

/// What the field list compiled to.
#[derive(Debug)]
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

impl MetricQuery {
    fn time_expression(
        &self,
        window: &Window,
        over: Option<Over<'_>>,
        qualifier: Option<&str>,
        as_names: &HashSet<&str>,
    ) -> Result<Option<String>, MetricQueryError> {
        if let Some(over) = over {
            return self.dataset_clock(window, over, qualifier, as_names);
        }

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

    /// The date a windowed run over a dataset selects by: the metric's own
    /// when it names one, the dataset's main date otherwise.
    fn dataset_clock(
        &self,
        window: &Window,
        over: Over<'_>,
        qualifier: Option<&str>,
        as_names: &HashSet<&str>,
    ) -> Result<Option<String>, MetricQueryError> {
        let clock = effective_clock(self, over.declaration).map(|(field, _)| field);
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
        if window.grain().is_some() && as_names.contains(BUCKET_COLUMN) {
            return Err(MetricQueryError::BucketAlias);
        }

        clock
            .map(|field| over.read(Some(field), "time", qualifier))
            .transpose()
    }

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

    /// Each field as it is selected, with whatever joining in a person's
    /// name takes with it.
    fn selection(
        &self,
        over: Option<Over<'_>>,
        qualifier: Option<&str>,
    ) -> Result<Selection<'_>, MetricQueryError> {
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
                None => field.expression(over, qualifier, &mut selection.binds)?,
            };
            selection
                .parts
                .push(format!("{expression} AS `{}`", field.as_name));
        }

        Ok(selection)
    }

    pub(crate) fn compile(&self, people: &People) -> Result<CompiledQuery, MetricQueryError> {
        self.compile_window(people, &Window::legacy(), TableEngine::Other, None)
    }

    /// The same, over the dataset this metric reads.
    pub(crate) fn compile_over(
        &self,
        people: &People,
        window: &Window,
        over: Over<'_>,
    ) -> Result<CompiledQuery, MetricQueryError> {
        self.compile_window(people, window, TableEngine::Other, Some(over))
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
        over: Option<Over<'_>>,
    ) -> Result<CompiledQuery, MetricQueryError> {
        if self.fields.is_empty() {
            return Err(MetricQueryError::NoFields);
        }
        let (_, table) = self.split();
        if over.is_none() {
            self.validate_shape(table)?;
        }

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
        } = self.selection(over, qualifier)?;

        let time_expression = self.time_expression(window, over, qualifier, &as_names)?;
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
        self.add_filters(over, qualifier, &mut where_parts, &mut binds)?;

        let from = self.table_source(over, qualifier, engine);
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
        over: Option<Over<'_>>,
        qualifier: Option<&str>,
        where_parts: &mut Vec<String>,
        binds: &mut Vec<FilterBind>,
    ) -> Result<(), MetricQueryError> {
        for filter in &self.filters {
            let (read, bound) = filter.compare(over, qualifier)?;
            where_parts.push(format!("{read} {} ?", filter.op.sql()));
            binds.push(bound);
        }

        Ok(())
    }

    fn table_source(
        &self,
        over: Option<Over<'_>>,
        qualifier: Option<&str>,
        engine: TableEngine,
    ) -> String {
        if let Some(over) = over {
            return over.relation(qualifier);
        }

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
        over: Option<Over<'_>>,
    ) -> Result<Option<UndatedQuery>, MetricQueryError> {
        let (_, table) = self.split();
        if over.is_none() {
            self.validate_shape(table)?;
        }

        let Some(clock) = self.undated_clock(over)? else {
            return Ok(None);
        };

        let mut where_parts = Vec::with_capacity(self.filters.len());
        let mut binds = Vec::with_capacity(self.filters.len());
        self.add_filters(over, None, &mut where_parts, &mut binds)?;

        // Over the collapsed relation, not the raw table, so the count and
        // the rows describe the same records.
        let mut sql = format!(
            "SELECT countIf(isNull({clock})) AS undated FROM {}",
            self.table_source(over, None, engine)
        );
        if !where_parts.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_parts.join(" AND "));
        }

        Ok(Some(UndatedQuery { sql, binds }))
    }

    /// The date the undated count asks about, when there is one.
    fn undated_clock(&self, over: Option<Over<'_>>) -> Result<Option<String>, MetricQueryError> {
        let Some(over) = over else {
            return self
                .time
                .as_ref()
                .map(|time| clock_expression(time.source()?, None))
                .transpose();
        };

        effective_clock(self, over.declaration)
            .map(|(field, _)| over.read(Some(field), "time", None))
            .transpose()
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
