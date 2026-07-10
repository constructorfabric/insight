use std::collections::{BTreeSet, HashMap};

use clickhouse::Row;
use sea_orm::DatabaseConnection;
use serde::Deserialize;

use crate::domain::metric_definitions::definition::{
    CohortSource, ObservationRelation, SourceKind,
};
use crate::domain::metric_definitions::error_code::{MetricSchemaErrorCode, SchemaStatus};
use crate::domain::metric_definitions::repository::{
    MetricDefinitionValidationSpec, all_managed_sources, managed_definition_validation_specs,
    update_definition_status, update_definitions_for_source_status, update_source_status,
};

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
            match self.validate_definition(&relation, &spec).await {
                ProbeOutcome::Definitive(state) => {
                    let (status, error_code) = state.as_db();
                    if let Err(error) =
                        update_definition_status(&self.db, spec.definition_id, status, error_code)
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
    ) -> ProbeOutcome {
        let inputs = spec
            .inputs
            .iter()
            .filter(|input| &input.observation_relation == relation)
            .collect::<Vec<_>>();
        if inputs.is_empty() {
            return ProbeOutcome::Definitive(ValidationState::Error(
                MetricSchemaErrorCode::Unknown,
            ));
        }

        let source_keys = inputs
            .iter()
            .map(|input| input.source_key.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let Some(source_key) = source_keys.first().copied() else {
            return ProbeOutcome::Definitive(ValidationState::Error(
                MetricSchemaErrorCode::Unknown,
            ));
        };
        if source_keys.len() != 1 {
            return ProbeOutcome::Definitive(ValidationState::Error(
                MetricSchemaErrorCode::Unknown,
            ));
        }

        let measure_keys = inputs
            .iter()
            .map(|input| input.measure_key.as_str())
            .collect::<BTreeSet<_>>();

        match self
            .has_source_rows(relation, source_key, spec.entity_type.as_str())
            .await
        {
            Ok(true) => {}
            Ok(false) => return ProbeOutcome::Definitive(ValidationState::Unchecked),
            Err(error) => {
                tracing::warn!(error = %error, "metric observation row probe failed");
                return ProbeOutcome::Inconclusive;
            }
        }

        // Absence of recent rows for a declared measure is a data condition,
        // not a schema error: filtered measures (e.g. tool-scoped
        // conversations) legitimately go quiet. Only measures that ARE
        // observed can be checked definitively; unobserved ones downgrade
        // the definition to unchecked, which stays runtime-available.
        let observed = match self
            .observed_measure_keys(
                relation,
                source_key,
                spec.entity_type.as_str(),
                &measure_keys.iter().copied().collect::<Vec<_>>(),
            )
            .await
        {
            Ok(observed) => observed,
            Err(error) => {
                tracing::warn!(error = %error, "metric measure probe failed");
                return ProbeOutcome::Inconclusive;
            }
        };
        let observed_keys = measure_keys
            .iter()
            .copied()
            .filter(|key| observed.contains(*key))
            .collect::<Vec<_>>();

        if let Some(outcome) = self
            .check_dimension_coverage(relation, source_key, spec, &observed_keys)
            .await
        {
            return outcome;
        }

        if observed_keys.len() < measure_keys.len() {
            let unobserved = measure_keys
                .iter()
                .copied()
                .filter(|key| !observed.contains(*key))
                .collect::<Vec<_>>();
            tracing::warn!(
                metric_key = %spec.metric_key,
                unobserved = ?unobserved,
                "declared measures without recent observations; definition stays unchecked"
            );
            return ProbeOutcome::Definitive(ValidationState::Unchecked);
        }

        ProbeOutcome::Definitive(ValidationState::Ok)
    }

    async fn check_dimension_coverage(
        &self,
        relation: &ObservationRelation,
        source_key: &str,
        spec: &MetricDefinitionValidationSpec,
        observed_keys: &[&str],
    ) -> Option<ProbeOutcome> {
        if observed_keys.is_empty() {
            return None;
        }
        for dimension in &spec.dimensions {
            match self
                .dimension_present_on_all_rows(
                    relation,
                    source_key,
                    spec.entity_type.as_str(),
                    observed_keys.iter().copied(),
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

    async fn has_source_rows(
        &self,
        relation: &ObservationRelation,
        source_key: &str,
        entity_type: &str,
    ) -> Result<bool, clickhouse::error::Error> {
        let (database, table) = relation.table_ref();
        let sql = format!(
            "SELECT count() AS rows \
             FROM ( \
                SELECT 1 \
                FROM {database}.{table} \
                WHERE source_key = ? \
                  AND entity_type = ? \
                  AND metric_date >= today() - {PROBE_WINDOW_DAYS} \
                LIMIT 1 \
             )"
        );
        let row: CountProbeRow = self
            .ch
            .query(&sql)
            .bind(source_key)
            .bind(entity_type)
            .fetch_one()
            .await?;
        Ok(row.rows > 0)
    }

    async fn observed_measure_keys(
        &self,
        relation: &ObservationRelation,
        source_key: &str,
        entity_type: &str,
        measure_keys: &[&str],
    ) -> Result<BTreeSet<String>, clickhouse::error::Error> {
        let (database, table) = relation.table_ref();
        let placeholders = vec!["?"; measure_keys.len()].join(", ");
        let sql = format!(
            "SELECT measure_key \
             FROM {database}.{table} \
             WHERE source_key = ? \
               AND entity_type = ? \
               AND metric_date >= today() - {PROBE_WINDOW_DAYS} \
               AND measure_key IN ({placeholders}) \
             GROUP BY measure_key"
        );
        let mut query = self.ch.query(&sql).bind(source_key).bind(entity_type);
        for measure_key in measure_keys {
            query = query.bind(*measure_key);
        }
        let rows = query.fetch_all::<MeasureKeyProbeRow>().await?;
        Ok(rows.into_iter().map(|row| row.measure_key).collect())
    }

    async fn dimension_present_on_all_rows<'a>(
        &self,
        relation: &ObservationRelation,
        source_key: &str,
        entity_type: &str,
        measure_keys: impl Iterator<Item = &'a str>,
        dimension: &str,
    ) -> Result<bool, clickhouse::error::Error> {
        let measure_keys = measure_keys.collect::<Vec<_>>();
        let rows = self
            .dimension_coverage(relation, source_key, entity_type, &measure_keys, dimension)
            .await?;
        let by_measure = rows
            .into_iter()
            .map(|row| (row.measure_key.clone(), row))
            .collect::<HashMap<_, _>>();

        Ok(measure_keys.iter().all(|measure_key| {
            by_measure
                .get(*measure_key)
                .is_some_and(|row| row.total_rows > 0 && row.total_rows == row.matching_rows)
        }))
    }

    async fn dimension_coverage(
        &self,
        relation: &ObservationRelation,
        source_key: &str,
        entity_type: &str,
        measure_keys: &[&str],
        dimension: &str,
    ) -> Result<Vec<DimensionCoverageProbeRow>, clickhouse::error::Error> {
        let (database, table) = relation.table_ref();
        let placeholders = vec!["?"; measure_keys.len()].join(", ");
        let sql = format!(
            "SELECT \
                measure_key, \
                count() AS total_rows, \
                countIf(has(arrayMap(d -> d.key, dimensions), ?)) AS matching_rows \
             FROM {database}.{table} \
             WHERE source_key = ? \
               AND entity_type = ? \
               AND metric_date >= today() - {PROBE_WINDOW_DAYS} \
               AND measure_key IN ({placeholders}) \
             GROUP BY measure_key"
        );
        let mut query = self
            .ch
            .query(&sql)
            .bind(dimension)
            .bind(source_key)
            .bind(entity_type);
        for measure_key in measure_keys {
            query = query.bind(*measure_key);
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
struct CountProbeRow {
    rows: u64,
}

#[derive(Row, Deserialize)]
struct MeasureKeyProbeRow {
    measure_key: String,
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
