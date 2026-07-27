use std::collections::{BTreeSet, HashMap};

use chrono::NaiveDate;
use clickhouse::Row;
use sea_orm::DatabaseConnection;
use serde::Deserialize;

use crate::domain::metric_definitions::definition::{
    CohortSource, MetricInput, ObservationRelation, SourceKind,
};
use crate::domain::metric_definitions::error_code::{MetricSchemaErrorCode, SchemaStatus};
use crate::domain::metric_definitions::repository::{
    MetricDefinitionValidationSpec, all_managed_sources, managed_definition_validation_specs,
    update_definition_status, update_definitions_for_source_status, update_source_status,
};

// Dimension coverage is checked over a trailing window anchored at the
// newest observed row, not at today(): rows predating a dimension's
// introduction would otherwise fail coverage forever, and a paused
// connector would empty the window entirely. Measure existence and
// freshness are probed over all history — schema validity is structural
// and does not decay with time.
const PROBE_WINDOW_DAYS: u32 = 35;
// Managed observation relations are dbt-created and can appear (or regress)
// while the service is running — a one-shot startup scan would pin
// `table_not_found` until the next pod restart. Sweeps are idempotent:
// transient probe failures never overwrite an established status, and
// status writes pin `updated_at`.
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_mins(5);

#[derive(Clone)]
pub struct MetricDefinitionValidator {
    db: DatabaseConnection,
    ch: insight_clickhouse::Client,
}

impl MetricDefinitionValidator {
    pub fn new(db: DatabaseConnection, ch: insight_clickhouse::Client) -> Self {
        Self { db, ch }
    }

    /// Periodic sweep: validates immediately, then every [`SWEEP_INTERVAL`].
    /// Never returns; run it on a spawned task.
    pub async fn run(self) {
        let mut ticks = tokio::time::interval(SWEEP_INTERVAL);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            self.validate_all().await;
        }
    }

    pub async fn validate_all(&self) {
        let sources = match all_managed_sources(&self.db).await {
            Ok(sources) => sources,
            Err(error) => {
                tracing::warn!(error = %error, "metric definition validation source load failed");
                return;
            }
        };

        for (source_id, source_kind, source_ref) in sources {
            let outcome = self
                .validate_source(source_kind.as_str(), source_ref.as_str())
                .await;

            match outcome {
                ProbeOutcome::Definitive(state) => {
                    let (status, error_code) = state.as_db();
                    if let Err(error) =
                        update_source_status(&self.db, source_id, status, error_code).await
                    {
                        tracing::warn!(error = %error, "metric definition source status update failed");
                        continue;
                    }
                    if state.is_ok() {
                        self.validate_definitions_for_source(source_id, source_ref.as_str())
                            .await;
                    } else if let Err(error) = update_definitions_for_source_status(
                        &self.db, source_id, status, error_code,
                    )
                    .await
                    {
                        tracing::warn!(error = %error, "metric definition status update failed");
                    }
                }
                ProbeOutcome::Inconclusive => {
                    tracing::warn!(
                        source_ref = %source_ref,
                        "metric source validation inconclusive; keeping previous status"
                    );
                }
            }
        }
    }

    async fn validate_source(&self, source_kind: &str, source_ref: &str) -> ProbeOutcome {
        match SourceKind::from_db(source_kind) {
            Some(SourceKind::ManagedObservation) => {}
            Some(SourceKind::CustomObservationSql) => return ProbeOutcome::Inconclusive,
            None => {
                return ProbeOutcome::Definitive(ValidationState::Error(
                    MetricSchemaErrorCode::Unknown,
                ));
            }
        }

        let Some(relation) = ObservationRelation::parse(source_ref) else {
            return ProbeOutcome::Definitive(ValidationState::Error(
                MetricSchemaErrorCode::Unknown,
            ));
        };
        let cohort = CohortSource::MetricEntityCohortsCurrent;

        match self
            .has_columns(relation.table_ref(), OBSERVATION_COLUMNS)
            .await
        {
            Ok(ColumnCheck::Present) => {}
            Ok(missing) => {
                return ProbeOutcome::Definitive(ValidationState::Error(missing.error_code()));
            }
            Err(error) => {
                tracing::warn!(error = %error, "metric observation source validation failed");
                return ProbeOutcome::Inconclusive;
            }
        }

        match self.has_columns(cohort.table_ref(), COHORT_COLUMNS).await {
            Ok(ColumnCheck::Present) => ProbeOutcome::Definitive(ValidationState::Ok),
            Ok(missing) => ProbeOutcome::Definitive(ValidationState::Error(missing.error_code())),
            Err(error) => {
                tracing::warn!(error = %error, "metric cohort source validation failed");
                ProbeOutcome::Inconclusive
            }
        }
    }

    async fn validate_definitions_for_source(&self, source_id: uuid::Uuid, source_ref: &str) {
        let Some(relation) = ObservationRelation::parse(source_ref) else {
            return;
        };
        let specs = match managed_definition_validation_specs(&self.db, source_id).await {
            Ok(specs) => specs,
            Err(error) => {
                tracing::warn!(error = %error, "metric definition validation spec load failed");
                return;
            }
        };

        for spec in specs {
            let (outcome, last_observed) = self.validate_definition(&relation, &spec).await;
            match outcome {
                ProbeOutcome::Definitive(state) => {
                    let (status, error_code) = state.as_db();
                    if let Err(error) = update_definition_status(
                        &self.db,
                        spec.definition_id,
                        status,
                        error_code,
                        last_observed,
                    )
                    .await
                    {
                        tracing::warn!(
                            error = %error,
                            metric_key = %spec.metric_key,
                            "metric definition status update failed"
                        );
                    }
                }
                ProbeOutcome::Inconclusive => {
                    tracing::warn!(
                        metric_key = %spec.metric_key,
                        "metric definition validation inconclusive; keeping previous status"
                    );
                }
            }
        }
    }

    async fn validate_definition(
        &self,
        relation: &ObservationRelation,
        spec: &MetricDefinitionValidationSpec,
    ) -> (ProbeOutcome, Option<NaiveDate>) {
        let Some(target) = resolve_probe_target(&spec.inputs, relation) else {
            return (
                ProbeOutcome::Definitive(ValidationState::Error(MetricSchemaErrorCode::Unknown)),
                None,
            );
        };

        // One probe answers both questions: which declared measures have
        // ever been observed (schema), and how fresh each one is (data).
        let measure_keys = target.measure_keys.iter().copied().collect::<Vec<_>>();
        let last_dates = match self
            .measure_last_dates(
                relation,
                target.source_key,
                spec.entity_type.as_str(),
                &measure_keys,
            )
            .await
        {
            Ok(last_dates) => last_dates,
            Err(error) => {
                tracing::warn!(error = %error, "metric measure probe failed");
                return (ProbeOutcome::Inconclusive, None);
            }
        };

        let freshness = classify_freshness(&target.measure_keys, &last_dates);
        if freshness == Freshness::NeverObserved {
            return (ProbeOutcome::Definitive(ValidationState::Unchecked), None);
        }
        let last_observed = last_dates.values().max().copied();

        let observed_keys = target
            .measure_keys
            .iter()
            .copied()
            .filter(|key| last_dates.contains_key(*key))
            .collect::<Vec<_>>();
        if let Some(outcome) = self
            .check_dimension_coverage(
                relation,
                target.source_key,
                spec,
                &observed_keys,
                &last_dates,
            )
            .await
        {
            return (outcome, last_observed);
        }

        match freshness {
            Freshness::Complete(_) => {
                (ProbeOutcome::Definitive(ValidationState::Ok), last_observed)
            }
            // A declared measure with no observation ever is a data condition,
            // not a schema error: filtered measures (e.g. tool-scoped
            // conversations) legitimately stay quiet, so the definition stays
            // unchecked but runtime-available.
            Freshness::Partial(_) | Freshness::NeverObserved => {
                let unobserved = target
                    .measure_keys
                    .iter()
                    .copied()
                    .filter(|key| !last_dates.contains_key(*key))
                    .collect::<Vec<_>>();
                tracing::warn!(
                    metric_key = %spec.metric_key,
                    unobserved = ?unobserved,
                    "declared measures without observations; definition stays unchecked"
                );
                (
                    ProbeOutcome::Definitive(ValidationState::Unchecked),
                    last_observed,
                )
            }
        }
    }

    async fn check_dimension_coverage(
        &self,
        relation: &ObservationRelation,
        source_key: &str,
        spec: &MetricDefinitionValidationSpec,
        observed_keys: &[&str],
        last_dates: &HashMap<String, NaiveDate>,
    ) -> Option<ProbeOutcome> {
        // Window each measure against its OWN newest observation, never the
        // definition-wide max: a stale measure judged against a fresher
        // sibling's date sees an empty window and fakes DimensionNotCovered.
        let windows = measure_windows(observed_keys, last_dates);
        if windows.is_empty() {
            return None;
        }
        for dimension in &spec.dimensions {
            match self
                .dimension_present_on_all_rows(
                    relation,
                    source_key,
                    spec.entity_type.as_str(),
                    &windows,
                    dimension,
                )
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    return Some(ProbeOutcome::Definitive(ValidationState::Error(
                        MetricSchemaErrorCode::DimensionNotCovered,
                    )));
                }
                Err(error) => {
                    tracing::warn!(error = %error, "metric dimension probe failed");
                    return Some(ProbeOutcome::Inconclusive);
                }
            }
        }
        None
    }

    async fn has_columns(
        &self,
        table: (&str, &str),
        columns: &[&str],
    ) -> Result<ColumnCheck, clickhouse::error::Error> {
        let (database, table) = table;
        let column_list = columns
            .iter()
            .map(|column| format!("'{column}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT \
                count() AS total_columns, \
                countIf(name IN ({column_list})) AS matching_columns \
             FROM system.columns \
             WHERE database = ? AND table = ?"
        );
        let mut query = self.ch.query(&sql);
        query = query.bind(database).bind(table);
        let row: ColumnProbeRow = query.fetch_one().await?;
        if row.total_columns == 0 {
            return Ok(ColumnCheck::TableMissing);
        }
        if row.matching_columns < columns.len() as u64 {
            return Ok(ColumnCheck::ColumnsMissing);
        }
        Ok(ColumnCheck::Present)
    }

    async fn measure_last_dates(
        &self,
        relation: &ObservationRelation,
        source_key: &str,
        entity_type: &str,
        measure_keys: &[&str],
    ) -> Result<HashMap<String, NaiveDate>, clickhouse::error::Error> {
        let (database, table) = relation.table_ref();
        let sql = measure_last_dates_sql(database, table, measure_keys.len());
        let mut query = self.ch.query(&sql).bind(source_key).bind(entity_type);
        for measure_key in measure_keys {
            query = query.bind(*measure_key);
        }
        let rows = query.fetch_all::<MeasureLastDateProbeRow>().await?;
        parse_measure_last_dates(rows)
    }

    async fn dimension_present_on_all_rows(
        &self,
        relation: &ObservationRelation,
        source_key: &str,
        entity_type: &str,
        measure_windows: &[(&str, NaiveDate)],
        dimension: &str,
    ) -> Result<bool, clickhouse::error::Error> {
        let rows = self
            .dimension_coverage(
                relation,
                source_key,
                entity_type,
                measure_windows,
                dimension,
            )
            .await?;
        Ok(all_measures_covered(measure_windows, rows))
    }

    async fn dimension_coverage(
        &self,
        relation: &ObservationRelation,
        source_key: &str,
        entity_type: &str,
        measure_windows: &[(&str, NaiveDate)],
        dimension: &str,
    ) -> Result<Vec<DimensionCoverageProbeRow>, clickhouse::error::Error> {
        let (database, table) = relation.table_ref();
        let sql = dimension_coverage_sql(database, table, measure_windows.len());
        let mut query = self
            .ch
            .query(&sql)
            .bind(dimension)
            .bind(source_key)
            .bind(entity_type);
        for (measure_key, date) in measure_windows {
            query = query.bind(*measure_key).bind(date.to_string());
        }
        query.fetch_all().await
    }
}

const OBSERVATION_COLUMNS: &[&str] = &[
    "tenant_id",
    "source_key",
    "entity_type",
    "entity_id",
    "metric_date",
    "observed_at",
    "measure_key",
    "value",
    "subject_key",
    "dimensions",
];

const COHORT_COLUMNS: &[&str] = &[
    "tenant_id",
    "entity_type",
    "entity_id",
    "cohort_key",
    "cohort_id",
];

#[derive(Row, Deserialize)]
struct ColumnProbeRow {
    total_columns: u64,
    matching_columns: u64,
}

#[derive(Row, Deserialize)]
struct MeasureLastDateProbeRow {
    measure_key: String,
    last_date: String,
}

#[derive(Row, Deserialize)]
struct DimensionCoverageProbeRow {
    measure_key: String,
    total_rows: u64,
    matching_rows: u64,
}

#[derive(Debug, Clone, Copy)]
enum ColumnCheck {
    Present,
    ColumnsMissing,
    TableMissing,
}

impl ColumnCheck {
    fn error_code(self) -> MetricSchemaErrorCode {
        match self {
            Self::TableMissing => MetricSchemaErrorCode::TableNotFound,
            Self::ColumnsMissing | Self::Present => MetricSchemaErrorCode::ColumnNotFound,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ProbeOutcome {
    Definitive(ValidationState),
    Inconclusive,
}

#[derive(Debug, Clone, Copy)]
enum ValidationState {
    Ok,
    Error(MetricSchemaErrorCode),
    Unchecked,
}

impl ValidationState {
    fn is_ok(self) -> bool {
        matches!(self, Self::Ok)
    }

    fn as_db(self) -> (SchemaStatus, Option<MetricSchemaErrorCode>) {
        match self {
            Self::Ok => (SchemaStatus::Ok, None),
            Self::Error(code) => (SchemaStatus::Error, Some(code)),
            Self::Unchecked => (SchemaStatus::Unchecked, None),
        }
    }
}

/// The single observation source + measure set a definition probes, resolved
/// from its inputs for a given relation. `None` = misconfigured (no inputs for
/// the relation, or inputs spanning more than one source).
struct ProbeTarget<'a> {
    source_key: &'a str,
    measure_keys: BTreeSet<&'a str>,
}

fn resolve_probe_target<'a>(
    inputs: &'a [MetricInput],
    relation: &ObservationRelation,
) -> Option<ProbeTarget<'a>> {
    let filtered = inputs
        .iter()
        .filter(|input| &input.observation_relation == relation)
        .collect::<Vec<_>>();
    if filtered.is_empty() {
        return None;
    }
    let source_keys = filtered
        .iter()
        .map(|input| input.source_key.as_str())
        .collect::<BTreeSet<_>>();
    if source_keys.len() != 1 {
        return None;
    }
    let source_key = *source_keys.iter().next()?;
    let measure_keys = filtered
        .iter()
        .map(|input| input.measure_key.as_str())
        .collect::<BTreeSet<_>>();
    Some(ProbeTarget {
        source_key,
        measure_keys,
    })
}

/// Data-freshness classification of a definition's declared measures against
/// the dates observed for them. Orthogonal to dimension coverage, which gates
/// separately.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Freshness {
    /// No declared measure has ever been observed.
    NeverObserved,
    /// Some, but not all, declared measures have been observed.
    Partial(Option<NaiveDate>),
    /// Every declared measure has been observed.
    Complete(Option<NaiveDate>),
}

fn classify_freshness(
    measure_keys: &BTreeSet<&str>,
    last_dates: &HashMap<String, NaiveDate>,
) -> Freshness {
    if last_dates.is_empty() {
        return Freshness::NeverObserved;
    }
    let last_observed = last_dates.values().max().copied();
    let observed = measure_keys
        .iter()
        .filter(|key| last_dates.contains_key(**key))
        .count();
    if observed < measure_keys.len() {
        Freshness::Partial(last_observed)
    } else {
        Freshness::Complete(last_observed)
    }
}

/// Pair each observed measure with its own newest observation date. Measures
/// absent from `last_dates` are dropped (they have no window to check).
fn measure_windows<'a>(
    observed_keys: &[&'a str],
    last_dates: &HashMap<String, NaiveDate>,
) -> Vec<(&'a str, NaiveDate)> {
    observed_keys
        .iter()
        .filter_map(|key| last_dates.get(*key).map(|date| (*key, *date)))
        .collect()
}

fn measure_last_dates_sql(database: &str, table: &str, measure_count: usize) -> String {
    let placeholders = vec!["?"; measure_count].join(", ");
    format!(
        "SELECT measure_key, toString(max(metric_date)) AS last_date \
         FROM {database}.{table} \
         WHERE source_key = ? \
           AND entity_type = ? \
           AND measure_key IN ({placeholders}) \
         GROUP BY measure_key"
    )
}

fn parse_measure_last_dates(
    rows: Vec<MeasureLastDateProbeRow>,
) -> Result<HashMap<String, NaiveDate>, clickhouse::error::Error> {
    rows.into_iter()
        .map(|row| {
            let date = row.last_date.parse::<NaiveDate>().map_err(|error| {
                clickhouse::error::Error::Custom(format!(
                    "unparseable metric_date {:?} for measure {}: {error}",
                    row.last_date, row.measure_key
                ))
            })?;
            Ok((row.measure_key, date))
        })
        .collect()
}

fn dimension_coverage_sql(database: &str, table: &str, measure_count: usize) -> String {
    // One batched query, but each measure carries its own freshness window
    // (`measure_key = ? AND metric_date >= toDate(?) - N`) so a stale measure
    // is scored over its own history, not a fresher sibling's.
    let per_measure = (0..measure_count)
        .map(|_| format!("(measure_key = ? AND metric_date >= toDate(?) - {PROBE_WINDOW_DAYS})"))
        .collect::<Vec<_>>()
        .join(" OR ");
    format!(
        "SELECT \
            measure_key, \
            count() AS total_rows, \
            countIf(has(arrayMap(d -> d.key, dimensions), ?)) AS matching_rows \
         FROM {database}.{table} \
         WHERE source_key = ? \
           AND entity_type = ? \
           AND ({per_measure}) \
         GROUP BY measure_key"
    )
}

fn all_measures_covered(
    measure_windows: &[(&str, NaiveDate)],
    rows: Vec<DimensionCoverageProbeRow>,
) -> bool {
    let by_measure = rows
        .into_iter()
        .map(|row| (row.measure_key.clone(), row))
        .collect::<HashMap<_, _>>();
    measure_windows.iter().all(|(measure_key, _)| {
        by_measure
            .get(*measure_key)
            .is_some_and(|row| row.total_rows > 0 && row.total_rows == row.matching_rows)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_definitions::definition::MetricInputRole;

    fn input(source_key: &str, measure_key: &str) -> MetricInput {
        MetricInput {
            role: MetricInputRole::Value,
            observation_relation: relation(),
            source_key: source_key.to_owned(),
            measure_key: measure_key.to_owned(),
        }
    }

    fn relation() -> ObservationRelation {
        ObservationRelation::parse("ai_metric_observations")
            .unwrap_or_else(|| panic!("relation must parse"))
    }

    fn date(s: &str) -> NaiveDate {
        s.parse().unwrap_or_else(|_| panic!("date must parse: {s}"))
    }

    #[test]
    fn resolve_probe_target_collects_single_source_and_measures() {
        let inputs = vec![
            input("ai_usage", "accepted_lines"),
            input("ai_usage", "cost_usd"),
        ];
        let Some(target) = resolve_probe_target(&inputs, &relation()) else {
            panic!("single source resolves");
        };
        assert_eq!(target.source_key, "ai_usage");
        assert_eq!(
            target.measure_keys,
            BTreeSet::from(["accepted_lines", "cost_usd"])
        );
    }

    #[test]
    fn resolve_probe_target_rejects_no_inputs_and_multi_source() {
        assert!(resolve_probe_target(&[], &relation()).is_none());
        let multi = vec![input("ai_usage", "a"), input("other_source", "b")];
        assert!(resolve_probe_target(&multi, &relation()).is_none());
    }

    #[test]
    fn classify_freshness_covers_every_arm() {
        let keys = BTreeSet::from(["a", "b"]);
        assert_eq!(
            classify_freshness(&keys, &HashMap::new()),
            Freshness::NeverObserved
        );

        let partial = HashMap::from([("a".to_owned(), date("2026-07-01"))]);
        assert_eq!(
            classify_freshness(&keys, &partial),
            Freshness::Partial(Some(date("2026-07-01")))
        );

        let complete = HashMap::from([
            ("a".to_owned(), date("2026-07-01")),
            ("b".to_owned(), date("2026-07-10")),
        ]);
        assert_eq!(
            classify_freshness(&keys, &complete),
            Freshness::Complete(Some(date("2026-07-10")))
        );
    }

    #[test]
    fn measure_windows_pairs_observed_keys_with_their_own_date() {
        let last = HashMap::from([
            ("a".to_owned(), date("2026-07-01")),
            ("b".to_owned(), date("2026-01-01")),
        ]);
        let mut windows = measure_windows(&["a", "b", "unobserved"], &last);
        windows.sort();
        assert_eq!(
            windows,
            vec![("a", date("2026-07-01")), ("b", date("2026-01-01"))]
        );
    }

    #[test]
    fn parse_measure_last_dates_maps_valid_and_rejects_garbage() {
        let Ok(ok) = parse_measure_last_dates(vec![MeasureLastDateProbeRow {
            measure_key: "a".to_owned(),
            last_date: "2026-07-01".to_owned(),
        }]) else {
            panic!("valid date parses");
        };
        assert_eq!(ok.get("a"), Some(&date("2026-07-01")));

        let err = parse_measure_last_dates(vec![MeasureLastDateProbeRow {
            measure_key: "a".to_owned(),
            last_date: "not-a-date".to_owned(),
        }]);
        assert!(err.is_err());
    }

    #[test]
    fn sql_builders_emit_one_window_clause_per_measure() {
        let dates = measure_last_dates_sql("insight", "ai_metric_observations", 2);
        assert!(dates.contains("measure_key IN (?, ?)"));

        let cov = dimension_coverage_sql("insight", "ai_metric_observations", 3);
        assert_eq!(cov.matches("measure_key = ?").count(), 3);
        assert_eq!(cov.matches(" OR ").count(), 2);
        assert!(cov.contains(&format!("toDate(?) - {PROBE_WINDOW_DAYS}")));
    }

    #[test]
    fn all_measures_covered_requires_every_measure_fully_tagged() {
        let windows = [("a", date("2026-07-01")), ("b", date("2026-07-01"))];
        let full = vec![
            DimensionCoverageProbeRow {
                measure_key: "a".to_owned(),
                total_rows: 4,
                matching_rows: 4,
            },
            DimensionCoverageProbeRow {
                measure_key: "b".to_owned(),
                total_rows: 2,
                matching_rows: 2,
            },
        ];
        assert!(all_measures_covered(&windows, full));

        let partial = vec![
            DimensionCoverageProbeRow {
                measure_key: "a".to_owned(),
                total_rows: 4,
                matching_rows: 3,
            },
            DimensionCoverageProbeRow {
                measure_key: "b".to_owned(),
                total_rows: 2,
                matching_rows: 2,
            },
        ];
        assert!(!all_measures_covered(&windows, partial));

        // A measure with no rows at all in its window is not covered.
        let missing = vec![DimensionCoverageProbeRow {
            measure_key: "a".to_owned(),
            total_rows: 4,
            matching_rows: 4,
        }];
        assert!(!all_measures_covered(&windows, missing));
    }
}
