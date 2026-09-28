//! Compiling a metric definition into `ClickHouse` SQL.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use super::clock::{TimeField, add_window_predicates, bucket_expression, clock_expression};
use super::field::{FieldType, OrderBy, is_identifier};
use super::filter::FilterBind;
use super::over::Over;
use super::people::{People, PersonHandle};
use super::{
    CompiledQuery, FACT_ALIAS, MetricQuery, MetricQueryError, RECORD_COLUMN, TWIN_COLUMN,
    UndatedQuery,
};
use crate::domain::kinds::dataset::declaration::BUCKET_COLUMN;
use crate::domain::kinds::dataset::read::Form;
use crate::domain::kinds::metric::answerable::effective_clock;
use crate::domain::query::metric_query::TableEngine;
use crate::domain::query::time_window::Window;

const DEFAULT_LIMIT: u32 = 1000;
const MAX_LIMIT: u32 = 10000;

/// How much of the result a compiled query is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bound {
    /// Everything a reader drawing the whole result gets: ordered as the
    /// metric says, cut at its own limit or the default.
    Whole,
    /// A result something else orders and pages. Only a limit the author
    /// wrote survives, and then the order behind it is made total so the
    /// same N rows answer every page.
    Paged,
}

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
            .map(|field| over.read_as(Some(field), "time", qualifier, Form::Raw))
            .transpose()
    }

    /// Whether this metric answers one row per group rather than one per
    /// record, which is what decides whether a windowed run buckets or merely
    /// reports the bucket each row falls in.
    fn groups_its_rows(&self) -> bool {
        !self.group_by.is_empty() || self.fields.iter().any(|field| field.agg.is_some())
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
            // The bucket is injected by a windowed run rather than selected,
            // so a metric may name it whether or not this run windows.
            if group != BUCKET_COLUMN && !as_names.contains(group.as_str()) {
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

            let carried = match over {
                Some(over) => over.person_of(field.declared.as_deref()),
                None => field.person,
            };
            let expression = match carried {
                Some(handle) => {
                    if !selection.handles.contains(&handle) {
                        selection.handles.push(handle);
                    }
                    let read = match over {
                        // A join key reads the record's own value: a
                        // substitute would look up a person nobody has.
                        Some(over) => over.read_as(
                            field.declared.as_deref(),
                            "a person field",
                            qualifier,
                            Form::Raw,
                        )?,
                        None => field.source()?.sql(field.r#type, qualifier)?,
                    };
                    let shown = match over {
                        Some(over) => {
                            over.read(field.declared.as_deref(), "a person field", qualifier)?
                        }
                        None => read.clone(),
                    };
                    let alias = format!("__p{index}");
                    let _ = write!(
                        selection.joins,
                        " LEFT JOIN {} AS `{alias}` ON `{alias}`.`handle` = {}",
                        handle.cte(),
                        handle.key(&read)
                    );
                    // Identity knows nobody by this handle: show what the
                    // record itself says, never a blank.
                    let resolved =
                        format!("coalesce(nullIf(`{alias}`.`display_name`, ''), {shown})");
                    let condition = field.condition(over, qualifier, &mut selection.binds)?;

                    field.aggregated_if(resolved, condition)
                }
                None => field.expression(over, qualifier, &mut selection.binds)?,
            };
            selection
                .parts
                .push(format!("{expression} AS `{}`", field.as_name));
        }

        Ok(selection)
    }

    #[cfg(test)]
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

    pub(crate) fn compile_window(
        &self,
        people: &People,
        window: &Window,
        engine: TableEngine,
        over: Option<Over<'_>>,
    ) -> Result<CompiledQuery, MetricQueryError> {
        self.assemble(people, window, engine, over, Bound::Whole)
    }

    /// The same query for a reader that orders and pages it itself.
    pub(crate) fn compile_paged(
        &self,
        people: &People,
        window: &Window,
        engine: TableEngine,
        over: Option<Over<'_>>,
    ) -> Result<CompiledQuery, MetricQueryError> {
        self.assemble(people, window, engine, over, Bound::Paged)
    }

    fn assemble(
        &self,
        people: &People,
        window: &Window,
        engine: TableEngine,
        over: Option<Over<'_>>,
        bound: Bound,
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
        let joins_a_person = self.fields.iter().any(|field| match over {
            Some(over) => over.person_of(field.declared.as_deref()).is_some(),
            None => field.person.is_some(),
        });
        let qualifier = joins_a_person.then_some(FACT_ALIAS);
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
        let mut as_names = as_names;
        if let Some((grain, clock)) = bucket {
            parts.insert(
                0,
                format!("{} AS `{BUCKET_COLUMN}`", bucket_expression(grain, clock)),
            );
            as_names.insert(BUCKET_COLUMN);
        }

        self.check_grouping(&as_names)?;

        let (hidden, numbered) = self.distinguished(bound, over.is_some(), qualifier, &mut parts);

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
        // A metric may name the bucket; the run injects it whenever it
        // windows. Either way it is grouped once, and a run that does not
        // window has no such column to group by.
        //
        // INVARIANT: the bucket joins the grouping only where there is one.
        // Adding it to a query that selects plain columns and aggregates
        // nothing would make that query aggregating, and leave those columns
        // neither grouped nor under an aggregate.
        let mut groups: Vec<String> = self
            .group_by
            .iter()
            .filter(|group| *group != BUCKET_COLUMN)
            .cloned()
            .collect();
        if bucket.is_some() && self.groups_its_rows() {
            groups.insert(0, BUCKET_COLUMN.to_owned());
        }
        // A metric may order by the bucket, which a run that does not window
        // never produces. Falling back to the grouping's own order keeps the
        // rows a `LIMIT` cuts deterministic.
        let ordering = match &self.order_by {
            Some(order) if as_names.contains(order.field.as_str()) => Some(order),
            Some(order) if order.field == BUCKET_COLUMN => None,
            Some(order) => return Err(MetricQueryError::OrderBy(order.field.clone())),
            None => None,
        };

        let backticked: Vec<String> = groups.iter().map(|group| format!("`{group}`")).collect();
        if !groups.is_empty() {
            sql.push_str(" GROUP BY ");
            sql.push_str(&backticked.join(", "));
        }

        let mut columns: Vec<String> = self
            .fields
            .iter()
            .map(|field| field.as_name.clone())
            .collect();
        if bucket.is_some() {
            columns.insert(0, BUCKET_COLUMN.to_owned());
        }
        // A hidden column this query carries itself is the last term of its
        // order, so a cut that two twins straddle keeps the same one each
        // time. One the reader numbers outside is not here yet to order by.
        let carried: &[String] = if numbered { &[] } else { &hidden };
        let ordered: Vec<String> = columns.iter().chain(carried.iter()).cloned().collect();
        self.bound_by(&mut sql, bound, ordering, &groups, &ordered);

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
            hidden,
            numbered,
        })
    }

    /// What tells two rows apart that the selected columns cannot, for a
    /// paged read of a plain metric: the hidden columns it carries, and
    /// whether the reader paging it has to number the twins itself.
    ///
    /// A record has an id of its own, carried beside the columns. A row of
    /// a warehouse table has nothing of the kind, so its twins are counted
    /// off by whoever pages the result - the column is named here so the
    /// reader strips it, and produced there.
    fn distinguished(
        &self,
        bound: Bound,
        over_a_dataset: bool,
        qualifier: Option<&str>,
        parts: &mut Vec<String>,
    ) -> (Vec<String>, bool) {
        if bound != Bound::Paged || self.groups_its_rows() {
            return (Vec::new(), false);
        }
        if !over_a_dataset {
            return (vec![TWIN_COLUMN.to_owned()], true);
        }

        let id = match qualifier {
            Some(alias) => format!("`{alias}`.`id`"),
            None => "`id`".to_owned(),
        };
        parts.push(format!("{id} AS `{RECORD_COLUMN}`"));

        (vec![RECORD_COLUMN.to_owned()], false)
    }

    /// The order and the cut at the end of the statement, as the bound asks.
    ///
    /// A bounded read is an ordered read: which rows a `LIMIT` keeps is only
    /// an answer when something says which come first.
    fn bound_by(
        &self,
        sql: &mut String,
        bound: Bound,
        ordering: Option<&OrderBy>,
        groups: &[String],
        columns: &[String],
    ) {
        let limit = match bound {
            Bound::Whole => Some(self.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT)),
            Bound::Paged => self.limit.map(|limit| limit.min(MAX_LIMIT)),
        };
        let Some(limit) = limit else {
            return;
        };

        let mut terms: Vec<String> = match ordering {
            Some(order) => vec![format!("`{}` {}", order.field, order.direction.sql())],
            None => groups.iter().map(|group| format!("`{group}`")).collect(),
        };
        if bound == Bound::Paged {
            // INVARIANT: total. A page is cut out of this result by
            // position, and two rows the order leaves interchangeable
            // could swap sides of the cut between one page and the next.
            let named: Vec<&str> = match ordering {
                Some(order) => vec![order.field.as_str()],
                None => groups.iter().map(String::as_str).collect(),
            };
            terms.extend(
                columns
                    .iter()
                    .filter(|column| !named.contains(&column.as_str()))
                    .map(|column| format!("`{column}`")),
            );
        }
        if !terms.is_empty() {
            sql.push_str(" ORDER BY ");
            sql.push_str(&terms.join(", "));
        }
        let _ = write!(sql, " LIMIT {limit}");
    }

    fn add_filters(
        &self,
        over: Option<Over<'_>>,
        qualifier: Option<&str>,
        where_parts: &mut Vec<String>,
        binds: &mut Vec<FilterBind>,
    ) -> Result<(), MetricQueryError> {
        for filter in &self.filters {
            let (condition, bound) = filter.predicate(over, qualifier)?;
            where_parts.push(condition);
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
            .map(|(field, _)| over.read_as(Some(field), "time", None, Form::Raw))
            .transpose()
    }

    /// Whether the table this metric names could be read at all, checked
    /// before anything asks the warehouse about it.
    pub(crate) fn check_table(&self) -> Result<(), MetricQueryError> {
        self.validate_shape(self.split().1)
    }

    fn validate_shape(&self, table: &str) -> Result<(), MetricQueryError> {
        if table.is_empty() {
            return Err(MetricQueryError::NoTable);
        }
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
