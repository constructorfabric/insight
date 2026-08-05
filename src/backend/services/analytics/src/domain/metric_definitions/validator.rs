use std::collections::{BTreeSet, HashMap};

use chrono::NaiveDate;
use clickhouse::Row;
use sea_orm::DatabaseConnection;
use serde::Deserialize;

use crate::domain::metric_definitions::definition::{
    CohortSource, EvidenceGranularity, EvidenceRelation, MetricInput, ObservationRelation,
    SourceKind,
};
use crate::domain::metric_definitions::error_code::{MetricSchemaErrorCode, SchemaStatus};
use crate::domain::metric_definitions::repository::{
    MetricDefinitionValidationSpec, all_managed_sources, managed_definition_validation_specs,
    source_evidence_granularities, update_definition_status, update_definitions_for_source_status,
    update_evidence_status, update_source_status,
};
use crate::domain::metric_drilldown::{
    EVIDENCE_QUERY_MEMORY_BYTES, EVIDENCE_QUERY_READ_BYTES, EVIDENCE_QUERY_RESULT_BYTES,
    EVIDENCE_QUERY_TIMEOUT_SECS,
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

        for source in sources {
            self.validate_evidence(
                source.id,
                &source.source_key,
                &source.source_kind,
                &source.source_ref,
                source.evidence_ref.as_deref(),
                &source.config_revision,
            )
            .await;
            let outcome = self
                .validate_source(source.source_kind.as_str(), source.source_ref.as_str())
                .await;

            match outcome {
                ProbeOutcome::Definitive(state) => {
                    let (status, error_code) = state.as_db();
                    if let Err(error) = update_source_status(
                        &self.db,
                        source.id,
                        &source.config_revision,
                        status,
                        error_code,
                    )
                    .await
                    {
                        tracing::warn!(error = %error, "metric definition source status update failed");
                        continue;
                    }
                    if state.is_ok() {
                        self.validate_definitions_for_source(source.id, source.source_ref.as_str())
                            .await;
                    } else if let Err(error) = update_definitions_for_source_status(
                        &self.db, source.id, status, error_code,
                    )
                    .await
                    {
                        tracing::warn!(error = %error, "metric definition status update failed");
                    }
                }
                ProbeOutcome::Inconclusive => {
                    tracing::warn!(
                        source_ref = %source.source_ref,
                        "metric source validation inconclusive; keeping previous status"
                    );
                }
            }
        }
    }

    async fn validate_evidence(
        &self,
        source_id: uuid::Uuid,
        source_key: &str,
        source_kind: &str,
        source_ref: &str,
        evidence_ref: Option<&str>,
        config_revision: &str,
    ) {
        if SourceKind::from_db(source_kind) == Some(SourceKind::CustomObservationSql) {
            return;
        }
        let configured_evidence = match evidence_ref {
            None => Ok(None),
            Some(reference) => EvidenceRelation::parse(reference)
                .map(Some)
                .ok_or(reference),
        };
        let Ok(configured_evidence) = configured_evidence else {
            tracing::warn!(
                source_key,
                evidence_ref,
                "metric evidence relation name is not a valid evidence relation"
            );
            self.write_evidence_status(
                source_id,
                config_revision,
                ValidationState::Error(MetricSchemaErrorCode::Unknown),
            )
            .await;
            return;
        };
        let state = match (configured_evidence, ObservationRelation::parse(source_ref)) {
            (Some(relation), Some(observation_relation)) => match self
                .has_exact_columns(relation.table_ref(), EVIDENCE_COLUMN_TYPES)
                .await
            {
                Ok(ColumnCheck::Present) => {
                    let expected = match source_evidence_granularities(&self.db, source_id).await {
                        Ok(expected) => expected,
                        Err(error) => {
                            tracing::warn!(error = %error, "metric evidence granularity metadata load failed");
                            return;
                        }
                    };
                    match self
                        .evidence_granularities_match(
                            &relation,
                            &observation_relation,
                            source_key,
                            &expected,
                        )
                        .await
                    {
                        Ok(true) => Some(ValidationState::Ok),
                        Ok(false) => {
                            tracing::warn!(
                                source_key,
                                evidence_ref,
                                expected = ?expected,
                                "metric evidence granularity does not match configured measures"
                            );
                            Some(ValidationState::Error(MetricSchemaErrorCode::Unknown))
                        }
                        Err(error) => {
                            tracing::warn!(error = %error, "metric evidence granularity validation failed");
                            None
                        }
                    }
                }
                Ok(missing) => Some(ValidationState::Error(missing.error_code())),
                Err(error) => {
                    tracing::warn!(error = %error, "metric evidence validation failed");
                    None
                }
            },
            _ => Some(ValidationState::Unchecked),
        };
        let Some(state) = state else {
            return;
        };
        self.write_evidence_status(source_id, config_revision, state)
            .await;
    }

    async fn write_evidence_status(
        &self,
        source_id: uuid::Uuid,
        config_revision: &str,
        state: ValidationState,
    ) {
        let (status, error_code) = state.as_db();
        if let Err(error) =
            update_evidence_status(&self.db, source_id, config_revision, status, error_code).await
        {
            tracing::warn!(error = %error, "metric evidence status update failed");
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

    async fn has_exact_columns(
        &self,
        table: (&str, &str),
        columns: &[(&str, &str)],
    ) -> Result<ColumnCheck, clickhouse::error::Error> {
        let (database, table) = table;
        let exact_columns = columns
            .iter()
            .map(|(name, r#type)| format!("(name = '{name}' AND type = '{type}')"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let sql = format!(
            "SELECT \
                count() AS total_columns, \
                countIf({exact_columns}) AS matching_columns \
             FROM system.columns \
             WHERE database = ? AND table = ?"
        );
        let row: ColumnProbeRow = self
            .ch
            .query(&sql)
            .bind(database)
            .bind(table)
            .fetch_one()
            .await?;
        if row.total_columns == 0 {
            return Ok(ColumnCheck::TableMissing);
        }
        if row.matching_columns < columns.len() as u64 {
            return Ok(ColumnCheck::ColumnsMissing);
        }
        Ok(ColumnCheck::Present)
    }

    async fn evidence_granularities_match(
        &self,
        relation: &EvidenceRelation,
        observation_relation: &ObservationRelation,
        source_key: &str,
        expected: &[(String, Option<String>)],
    ) -> Result<bool, clickhouse::error::Error> {
        if !expected_granularities_are_known(expected) {
            return Ok(false);
        }

        let (observation_database, observation_table) = observation_relation.table_ref();
        let observation_sql =
            evidence_observation_dates_sql(observation_database, observation_table, expected.len());
        let mut observation_query = self
            .ch
            .query(&observation_sql)
            .with_option(
                "max_execution_time",
                EVIDENCE_QUERY_TIMEOUT_SECS.to_string(),
            )
            .with_option("max_memory_usage", EVIDENCE_QUERY_MEMORY_BYTES.to_string())
            .with_option("max_bytes_to_read", EVIDENCE_QUERY_READ_BYTES.to_string())
            .with_option("max_result_bytes", EVIDENCE_QUERY_RESULT_BYTES.to_string())
            .bind(source_key);
        for (measure_key, _) in expected {
            observation_query = observation_query.bind(measure_key);
        }
        let observed_dates = parse_measure_last_dates(observation_query.fetch_all().await?)?;
        let observed_measures = observed_dates.keys().cloned().collect::<BTreeSet<_>>();
        let window_start = evidence_window_start(&observed_dates);

        let (database, table) = relation.table_ref();
        let sql = evidence_granularity_sql(database, table, expected.len(), window_start.is_some());
        let mut query = self
            .ch
            .query(&sql)
            .with_option(
                "max_execution_time",
                EVIDENCE_QUERY_TIMEOUT_SECS.to_string(),
            )
            .with_option("max_memory_usage", EVIDENCE_QUERY_MEMORY_BYTES.to_string())
            .with_option("max_bytes_to_read", EVIDENCE_QUERY_READ_BYTES.to_string())
            .with_option("max_result_bytes", EVIDENCE_QUERY_RESULT_BYTES.to_string())
            .bind(source_key);
        for (measure_key, _) in expected {
            query = query.bind(measure_key);
        }
        if let Some(window_start) = window_start {
            query = query.bind(window_start.to_string());
        }
        let actual = query
            .fetch_all::<EvidenceGranularityProbeRow>()
            .await?
            .into_iter()
            .map(|row| (row.measure_key, row.granularities))
            .collect::<HashMap<_, _>>();

        Ok(granularities_match(expected, &actual, &observed_measures))
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

// The columns the RUNTIME actually reads. `entity_id` carries the canonical
// person id since the identity cutover — same column, canonical content — so
// the list is unchanged by it: a second identity column would have made the
// duplication part of this published contract.
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

const EVIDENCE_COLUMN_TYPES: &[(&str, &str)] = &[
    ("tenant_id", "String"),
    ("source_key", "String"),
    ("entity_type", "String"),
    ("entity_id", "String"),
    ("metric_date", "Date"),
    ("observed_at", "Nullable(DateTime64(3))"),
    ("measure_key", "String"),
    ("record_id", "String"),
    ("record_kind", "String"),
    ("granularity", "String"),
    ("record_label", "String"),
    ("contribution", "Nullable(Float64)"),
    ("subject_key", "Nullable(String)"),
    (
        "dimensions",
        "Array(Tuple(key String, value String, label Nullable(String)))",
    ),
    ("details", "Map(String, String)"),
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

#[derive(Row, Deserialize)]
struct EvidenceGranularityProbeRow {
    measure_key: String,
    granularities: Vec<String>,
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

fn expected_granularities_are_known(expected: &[(String, Option<String>)]) -> bool {
    !expected.is_empty()
        && expected.iter().all(|(_, value)| {
            value
                .as_deref()
                .and_then(EvidenceGranularity::from_db)
                .is_some()
        })
}

fn evidence_observation_dates_sql(database: &str, table: &str, measure_count: usize) -> String {
    let placeholders = vec!["?"; measure_count].join(", ");
    format!(
        "SELECT measure_key, toString(max(metric_date)) AS last_date \
         FROM {database}.{table} \
         WHERE source_key = ? AND measure_key IN ({placeholders}) \
         GROUP BY measure_key"
    )
}

fn evidence_granularity_sql(
    database: &str,
    table: &str,
    measure_count: usize,
    windowed: bool,
) -> String {
    let placeholders = vec!["?"; measure_count].join(", ");
    let window_sql = if windowed {
        " AND metric_date >= toDate(?)"
    } else {
        ""
    };
    format!(
        "SELECT measure_key, groupUniqArray(granularity) AS granularities \
         FROM {database}.{table} \
         WHERE source_key = ? AND measure_key IN ({placeholders}){window_sql} \
         GROUP BY measure_key"
    )
}

// The window trails the oldest per-measure last-observed date so a stale
// measure widens it instead of falling outside it.
fn evidence_window_start(observed_dates: &HashMap<String, NaiveDate>) -> Option<NaiveDate> {
    observed_dates
        .values()
        .min()
        .map(|date| *date - chrono::Duration::days(i64::from(PROBE_WINDOW_DAYS)))
}

fn granularities_match(
    expected: &[(String, Option<String>)],
    actual: &HashMap<String, Vec<String>>,
    observed_measures: &BTreeSet<String>,
) -> bool {
    expected.iter().all(|(measure_key, granularity)| {
        let Some(granularity) = granularity.as_deref() else {
            return false;
        };
        let matches = match actual.get(measure_key) {
            Some(values) => values.len() == 1 && values[0] == granularity,
            None => !observed_measures.contains(measure_key),
        };
        if !matches {
            tracing::warn!(
                measure_key,
                expected_granularity = granularity,
                actual_granularities = ?actual.get(measure_key),
                observed = observed_measures.contains(measure_key),
                "metric evidence measure granularity mismatch"
            );
        }
        matches
    })
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

    fn granularity(measure_key: &str, value: Option<&str>) -> (String, Option<String>) {
        (measure_key.to_owned(), value.map(str::to_owned))
    }

    #[test]
    fn expected_granularities_reject_empty_and_unknown_values() {
        for (expected, want, case) in [
            (vec![granularity("a", Some("event"))], true, "known"),
            (
                vec![
                    granularity("a", Some("event")),
                    granularity("b", Some("source_summary")),
                ],
                true,
                "all known",
            ),
            (Vec::new(), false, "empty"),
            (vec![granularity("a", None)], false, "missing"),
            (vec![granularity("a", Some("nonsense"))], false, "unknown"),
            (
                vec![granularity("a", Some("event")), granularity("b", None)],
                false,
                "one missing",
            ),
        ] {
            assert_eq!(
                expected_granularities_are_known(&expected),
                want,
                "case: {case}"
            );
        }
    }

    #[test]
    fn evidence_sql_binds_one_placeholder_per_measure_and_windows_on_demand() {
        let dates = evidence_observation_dates_sql("insight", "ai_metric_observations", 2);
        assert!(dates.contains("measure_key IN (?, ?)"));
        assert!(dates.contains("max(metric_date)"));
        assert!(!dates.contains("entity_type"));

        let windowed = evidence_granularity_sql("insight", "ai_metric_evidence", 3, true);
        assert!(windowed.contains("measure_key IN (?, ?, ?)"));
        assert!(windowed.contains("groupUniqArray(granularity)"));
        assert!(windowed.contains("AND metric_date >= toDate(?)"));

        let unwindowed = evidence_granularity_sql("insight", "ai_metric_evidence", 1, false);
        assert!(!unwindowed.contains("metric_date >="));
    }

    #[test]
    fn evidence_window_trails_the_oldest_observed_measure() {
        assert_eq!(evidence_window_start(&HashMap::new()), None);

        let observed = HashMap::from([
            ("fresh".to_owned(), date("2026-07-30")),
            ("stale".to_owned(), date("2026-01-15")),
        ]);
        assert_eq!(
            evidence_window_start(&observed),
            Some(date("2026-01-15") - chrono::Duration::days(i64::from(PROBE_WINDOW_DAYS)))
        );
    }

    #[test]
    fn granularities_match_only_on_a_single_expected_value_per_measure() {
        let expected = vec![granularity("a", Some("event"))];
        let observed = BTreeSet::from(["a".to_owned()]);

        let exact = HashMap::from([("a".to_owned(), vec!["event".to_owned()])]);
        assert!(granularities_match(&expected, &exact, &observed));

        let wrong = HashMap::from([("a".to_owned(), vec!["source_summary".to_owned()])]);
        assert!(!granularities_match(&expected, &wrong, &observed));

        let mixed = HashMap::from([(
            "a".to_owned(),
            vec!["event".to_owned(), "source_summary".to_owned()],
        )]);
        assert!(!granularities_match(&expected, &mixed, &observed));

        assert!(!granularities_match(
            &[granularity("a", None)],
            &exact,
            &observed
        ));
    }

    #[test]
    fn column_check_maps_missing_table_and_columns_to_distinct_codes() {
        assert_eq!(
            ColumnCheck::TableMissing.error_code(),
            MetricSchemaErrorCode::TableNotFound
        );
        assert_eq!(
            ColumnCheck::ColumnsMissing.error_code(),
            MetricSchemaErrorCode::ColumnNotFound
        );
        assert_eq!(
            ColumnCheck::Present.error_code(),
            MetricSchemaErrorCode::ColumnNotFound
        );
    }

    #[test]
    fn validation_state_carries_an_error_code_only_when_erroring() {
        assert!(ValidationState::Ok.is_ok());
        assert!(!ValidationState::Unchecked.is_ok());
        assert!(!ValidationState::Error(MetricSchemaErrorCode::Unknown).is_ok());

        assert_eq!(
            ValidationState::Ok.as_db(),
            (SchemaStatus::Ok, None),
            "ok carries no code"
        );
        assert_eq!(
            ValidationState::Unchecked.as_db(),
            (SchemaStatus::Unchecked, None),
            "unchecked carries no code"
        );
        assert_eq!(
            ValidationState::Error(MetricSchemaErrorCode::TableNotFound).as_db(),
            (
                SchemaStatus::Error,
                Some(MetricSchemaErrorCode::TableNotFound)
            ),
            "error carries its code"
        );
    }

    #[test]
    fn measures_without_evidence_pass_only_when_never_observed() {
        let expected = vec![granularity("a", Some("event"))];
        let empty = HashMap::new();

        assert!(granularities_match(&expected, &empty, &BTreeSet::new()));
        assert!(!granularities_match(
            &expected,
            &empty,
            &BTreeSet::from(["a".to_owned()])
        ));
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
