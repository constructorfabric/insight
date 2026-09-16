//! Compiling a metric definition into `ClickHouse` SQL.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use super::clock::{TimeField, add_window_predicates, bucket_expression, clock_expression};
use super::field::{FieldType, is_identifier};
use super::filter::FilterBind;
use super::people::{People, PersonHandle};
use super::{CompiledQuery, FACT_ALIAS, MetricQuery, MetricQueryError, UndatedQuery};
use crate::domain::query::time_window::{MaximumRange, Window};
use crate::store::catalog::TableEngine;

const DEFAULT_LIMIT: u32 = 1000;
const MAX_LIMIT: u32 = 10000;

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
