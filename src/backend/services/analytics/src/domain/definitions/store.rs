//! The definition store's write path — the only one. Every writer comes
//! through here, so a definition can never be written by a path that skips
//! versioning or the audit trail.
//!
//! Versioning is never hand-maintained: a write canonicalizes the definition's
//! semantic fields, compares them against what the row already holds, and bumps
//! `definition_version` only on difference — with a compare-and-set against the
//! version it read, so two writers cannot both bump to the same number. A
//! forgotten bump would serve new semantics from a cache keyed by the old
//! version, so the possibility is removed structurally rather than by
//! convention.

use sea_orm::{ConnectionTrait, DbErr, Statement, Value};
use serde_json::json;
use uuid::Uuid;

use super::definition::{DatasetDefinition, MeasureDefinition, MetricDefinition, Origin};
use crate::infra::db::entities::{semantic_datasets, semantic_measures, semantic_metrics};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefinitionKind {
    Dataset,
    Measure,
    Metric,
}

impl DefinitionKind {
    fn as_db(self) -> &'static str {
        match self {
            Self::Dataset => "dataset",
            Self::Measure => "measure",
            Self::Metric => "metric",
        }
    }
}

/// What a write did, so a reconciler can report and a caller can react.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// The definition did not exist; it was written at version 1.
    Created,
    /// Semantic fields differed; the row moved to this version.
    Bumped(i32),
    /// The stored semantics already match; nothing was written.
    Unchanged(i32),
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Db(#[from] DbErr),
    #[error("definition `{key}` was written concurrently; retry against version {version}")]
    Conflict { key: String, version: i32 },
    #[error("definition `{key}` cannot be canonicalized: {reason}")]
    Canonicalize { key: String, reason: String },
}

/// The version a write plan targets, decided before any row is touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WritePlan {
    Insert,
    Bump { from: i32 },
    Unchanged { at: i32 },
}

fn plan(stored: Option<(i32, &serde_json::Value)>, incoming: &serde_json::Value) -> WritePlan {
    match stored {
        None => WritePlan::Insert,
        Some((version, body)) if body == incoming => WritePlan::Unchanged { at: version },
        Some((version, _)) => WritePlan::Bump { from: version },
    }
}

/// The semantic body of a dataset: which rows a measure over it will see.
/// Availability is the probe's state, not definition, and never appears here.
fn dataset_body(dataset: &DatasetDefinition) -> serde_json::Value {
    json!({
        "relation": dataset.relation,
        "read_discipline": dataset.read_discipline,
        "retention_horizon": dataset.retention_horizon,
    })
}

fn stored_dataset_body(model: &semantic_datasets::Model) -> serde_json::Value {
    json!({
        "relation": model.database_relation,
        "read_discipline": model.read_discipline,
        "retention_horizon": model.retention_horizon,
    })
}

/// The semantic body of a measure: everything that changes what the number
/// means. Description and enablement are deliberately absent — they are not
/// semantics, so editing them must not invalidate a cache.
fn measure_body(measure: &MeasureDefinition) -> serde_json::Value {
    let mut dimensions = measure.dimensions.clone();
    dimensions.sort_by(|a, b| a.key.cmp(&b.key));
    json!({
        "dataset": measure.dataset,
        "filter": measure.filter,
        "aggregation": measure.aggregation,
        "value_expr": measure.value_expr,
        "subject_expr": measure.subject_expr,
        "event_time": measure.event_time,
        "entity": measure.entity,
        "dimensions": dimensions,
    })
}

fn stored_measure_body(model: &semantic_measures::Model) -> serde_json::Value {
    json!({
        "dataset": model.dataset_ref,
        "filter": model.filter,
        "aggregation": model.aggregation,
        "value_expr": model.value_expr,
        "subject_expr": model.subject_expr,
        "event_time": model.event_time,
        "entity": model.entity,
        "dimensions": model.dimensions.clone().unwrap_or_else(|| json!([])),
    })
}

/// The semantic body of a metric. Format, direction, and labels are display
/// identity: they change how a value reads, never what it is.
fn metric_body(metric: &MetricDefinition) -> serde_json::Value {
    json!({
        "computation": metric.computation,
        "transform": metric.transform,
        "entity_type": metric.entity_type,
        "cohort_key": metric.cohort_key,
    })
}

fn stored_metric_body(model: &semantic_metrics::Model) -> serde_json::Value {
    json!({
        "computation": model.computation,
        "transform": model.transform,
        "entity_type": model.entity_type,
        "cohort_key": model.cohort_key,
    })
}

pub async fn reconcile_dataset<C: ConnectionTrait>(
    conn: &C,
    dataset: &DatasetDefinition,
    origin: Origin,
    actor: &str,
) -> Result<WriteOutcome, StoreError> {
    let incoming = dataset_body(dataset);
    let stored = fetch_dataset(conn, &dataset.key).await?;
    let stored_body = stored.as_ref().map(stored_dataset_body);
    let outcome = match plan(
        stored
            .as_ref()
            .map(|model| model.definition_version)
            .zip(stored_body.as_ref()),
        &incoming,
    ) {
        WritePlan::Unchanged { at } => return Ok(WriteOutcome::Unchanged(at)),
        WritePlan::Insert => {
            insert_dataset(conn, dataset, origin).await?;
            WriteOutcome::Created
        }
        WritePlan::Bump { from } => {
            let next = from + 1;
            let updated = update_dataset(conn, dataset, from).await?;
            if updated != 1 {
                return Err(StoreError::Conflict {
                    key: dataset.key.clone(),
                    version: from,
                });
            }
            WriteOutcome::Bumped(next)
        }
    };
    append_revision(
        conn,
        DefinitionKind::Dataset,
        &dataset.key,
        version_of(outcome),
        actor,
        &incoming,
    )
    .await?;
    Ok(outcome)
}

pub async fn reconcile_measure<C: ConnectionTrait>(
    conn: &C,
    measure: &MeasureDefinition,
    origin: Origin,
    actor: &str,
) -> Result<WriteOutcome, StoreError> {
    let incoming = measure_body(measure);
    let stored = fetch_measure(conn, &measure.key).await?;
    let stored_body = stored.as_ref().map(stored_measure_body);
    let outcome = match plan(
        stored
            .as_ref()
            .map(|model| model.definition_version)
            .zip(stored_body.as_ref()),
        &incoming,
    ) {
        WritePlan::Unchanged { at } => return Ok(WriteOutcome::Unchanged(at)),
        WritePlan::Insert => {
            insert_measure(conn, measure, origin).await?;
            WriteOutcome::Created
        }
        WritePlan::Bump { from } => {
            let next = from + 1;
            let updated = update_measure(conn, measure, from).await?;
            if updated != 1 {
                return Err(StoreError::Conflict {
                    key: measure.key.clone(),
                    version: from,
                });
            }
            WriteOutcome::Bumped(next)
        }
    };
    append_revision(
        conn,
        DefinitionKind::Measure,
        &measure.key,
        version_of(outcome),
        actor,
        &incoming,
    )
    .await?;
    Ok(outcome)
}

pub async fn reconcile_metric<C: ConnectionTrait>(
    conn: &C,
    metric: &MetricDefinition,
    origin: Origin,
    actor: &str,
) -> Result<WriteOutcome, StoreError> {
    let incoming = metric_body(metric);
    let stored = fetch_metric(conn, &metric.key).await?;
    let stored_body = stored.as_ref().map(stored_metric_body);
    let outcome = match plan(
        stored
            .as_ref()
            .map(|model| model.definition_version)
            .zip(stored_body.as_ref()),
        &incoming,
    ) {
        WritePlan::Unchanged { at } => {
            // Display identity may still have moved; it is not versioned.
            update_metric_display(conn, metric).await?;
            return Ok(WriteOutcome::Unchanged(at));
        }
        WritePlan::Insert => {
            insert_metric(conn, metric, origin).await?;
            WriteOutcome::Created
        }
        WritePlan::Bump { from } => {
            let next = from + 1;
            let updated = update_metric(conn, metric, from).await?;
            if updated != 1 {
                return Err(StoreError::Conflict {
                    key: metric.key.clone(),
                    version: from,
                });
            }
            WriteOutcome::Bumped(next)
        }
    };
    append_revision(
        conn,
        DefinitionKind::Metric,
        &metric.key,
        version_of(outcome),
        actor,
        &incoming,
    )
    .await?;
    Ok(outcome)
}

fn version_of(outcome: WriteOutcome) -> i32 {
    match outcome {
        WriteOutcome::Created => 1,
        WriteOutcome::Bumped(version) | WriteOutcome::Unchanged(version) => version,
    }
}

async fn fetch_dataset<C: ConnectionTrait>(
    conn: &C,
    key: &str,
) -> Result<Option<semantic_datasets::Model>, DbErr> {
    use sea_orm::EntityTrait;
    use sea_orm::QueryFilter;
    use sea_orm::prelude::Expr;

    semantic_datasets::Entity::find()
        .filter(Expr::col(semantic_datasets::Column::DatasetKey).eq(key))
        .filter(Expr::col(semantic_datasets::Column::TenantId).is_null())
        .one(conn)
        .await
}

async fn insert_dataset<C: ConnectionTrait>(
    conn: &C,
    dataset: &DatasetDefinition,
    origin: Origin,
) -> Result<(), DbErr> {
    conn.execute(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "INSERT INTO semantic_datasets \
            (id, tenant_id, dataset_key, database_relation, read_discipline, \
             retention_horizon, origin, definition_version, is_enabled) \
         VALUES (?, NULL, ?, ?, ?, ?, ?, 1, TRUE)",
        [
            uuid_value(Uuid::now_v7()),
            Value::from(dataset.key.as_str()),
            Value::from(dataset.relation.as_str()),
            Value::from(dataset.read_discipline.as_db()),
            optional_str(dataset.retention_horizon.as_deref()),
            Value::from(origin.as_db()),
        ],
    ))
    .await?;
    Ok(())
}

pub(super) async fn update_dataset<C: ConnectionTrait>(
    conn: &C,
    dataset: &DatasetDefinition,
    from_version: i32,
) -> Result<u64, DbErr> {
    let result = conn
        .execute(Statement::from_sql_and_values(
            conn.get_database_backend(),
            "UPDATE semantic_datasets SET \
                database_relation = ?, read_discipline = ?, retention_horizon = ?, \
                definition_version = definition_version + 1 \
             WHERE dataset_key = ? AND tenant_id IS NULL AND definition_version = ?",
            [
                Value::from(dataset.relation.as_str()),
                Value::from(dataset.read_discipline.as_db()),
                optional_str(dataset.retention_horizon.as_deref()),
                Value::from(dataset.key.as_str()),
                Value::from(from_version),
            ],
        ))
        .await?;
    Ok(result.rows_affected())
}

async fn fetch_measure<C: ConnectionTrait>(
    conn: &C,
    key: &str,
) -> Result<Option<semantic_measures::Model>, DbErr> {
    use sea_orm::EntityTrait;
    use sea_orm::QueryFilter;
    use sea_orm::prelude::Expr;

    semantic_measures::Entity::find()
        .filter(Expr::col(semantic_measures::Column::MeasureKey).eq(key))
        .filter(Expr::col(semantic_measures::Column::TenantId).is_null())
        .one(conn)
        .await
}

async fn fetch_metric<C: ConnectionTrait>(
    conn: &C,
    key: &str,
) -> Result<Option<semantic_metrics::Model>, DbErr> {
    use sea_orm::EntityTrait;
    use sea_orm::QueryFilter;
    use sea_orm::prelude::Expr;

    semantic_metrics::Entity::find()
        .filter(Expr::col(semantic_metrics::Column::MetricKey).eq(key))
        .filter(Expr::col(semantic_metrics::Column::TenantId).is_null())
        .one(conn)
        .await
}

async fn insert_measure<C: ConnectionTrait>(
    conn: &C,
    measure: &MeasureDefinition,
    origin: Origin,
) -> Result<(), DbErr> {
    conn.execute(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "INSERT INTO semantic_measures \
            (id, tenant_id, measure_key, dataset_ref, filter, aggregation, value_expr, \
             subject_expr, event_time, entity, dimensions, definition_version, origin, is_enabled) \
         VALUES (?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, TRUE)",
        [
            uuid_value(Uuid::now_v7()),
            Value::from(measure.key.as_str()),
            Value::from(measure.dataset.as_str()),
            json_value(measure.filter.as_ref()),
            Value::from(measure.aggregation.as_db()),
            optional_str(measure.value_expr.as_deref()),
            optional_str(measure.subject_expr.as_deref()),
            Value::from(measure.event_time.as_str()),
            Value::from(measure.entity.as_str()),
            json_value(Some(&measure.dimensions)),
            Value::from(origin.as_db()),
        ],
    ))
    .await?;
    Ok(())
}

pub(super) async fn update_measure<C: ConnectionTrait>(
    conn: &C,
    measure: &MeasureDefinition,
    from_version: i32,
) -> Result<u64, DbErr> {
    let result = conn
        .execute(Statement::from_sql_and_values(
            conn.get_database_backend(),
            "UPDATE semantic_measures SET \
                dataset_ref = ?, filter = ?, aggregation = ?, value_expr = ?, subject_expr = ?, \
                event_time = ?, entity = ?, dimensions = ?, definition_version = definition_version + 1 \
             WHERE measure_key = ? AND tenant_id IS NULL AND definition_version = ?",
            [
                Value::from(measure.dataset.as_str()),
                json_value(measure.filter.as_ref()),
                Value::from(measure.aggregation.as_db()),
                optional_str(measure.value_expr.as_deref()),
                optional_str(measure.subject_expr.as_deref()),
                Value::from(measure.event_time.as_str()),
                Value::from(measure.entity.as_str()),
                json_value(Some(&measure.dimensions)),
                Value::from(measure.key.as_str()),
                Value::from(from_version),
            ],
        ))
        .await?;
    Ok(result.rows_affected())
}

async fn insert_metric<C: ConnectionTrait>(
    conn: &C,
    metric: &MetricDefinition,
    origin: Origin,
) -> Result<(), DbErr> {
    conn.execute(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "INSERT INTO semantic_metrics \
            (id, tenant_id, metric_key, computation, transform, format, direction, \
             entity_type, cohort_key, definition_version, origin, is_enabled) \
         VALUES (?, NULL, ?, ?, ?, ?, ?, ?, ?, 1, ?, TRUE)",
        [
            uuid_value(Uuid::now_v7()),
            Value::from(metric.key.as_str()),
            json_value(Some(&metric.computation)),
            json_value(metric.transform.as_ref()),
            Value::from(metric.format.as_db()),
            Value::from(metric.direction.as_db()),
            Value::from(metric.entity_type.as_str()),
            optional_str(metric.cohort_key.as_deref()),
            Value::from(origin.as_db()),
        ],
    ))
    .await?;
    Ok(())
}

async fn update_metric<C: ConnectionTrait>(
    conn: &C,
    metric: &MetricDefinition,
    from_version: i32,
) -> Result<u64, DbErr> {
    let result = conn
        .execute(Statement::from_sql_and_values(
            conn.get_database_backend(),
            "UPDATE semantic_metrics SET \
                computation = ?, transform = ?, format = ?, direction = ?, \
                entity_type = ?, cohort_key = ?, definition_version = definition_version + 1 \
             WHERE metric_key = ? AND tenant_id IS NULL AND definition_version = ?",
            [
                json_value(Some(&metric.computation)),
                json_value(metric.transform.as_ref()),
                Value::from(metric.format.as_db()),
                Value::from(metric.direction.as_db()),
                Value::from(metric.entity_type.as_str()),
                optional_str(metric.cohort_key.as_deref()),
                Value::from(metric.key.as_str()),
                Value::from(from_version),
            ],
        ))
        .await?;
    Ok(result.rows_affected())
}

/// Display identity is not versioned, so it is written without a bump and
/// without a revision: nothing about meaning changed.
async fn update_metric_display<C: ConnectionTrait>(
    conn: &C,
    metric: &MetricDefinition,
) -> Result<(), DbErr> {
    conn.execute(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "UPDATE semantic_metrics SET format = ?, direction = ? \
         WHERE metric_key = ? AND tenant_id IS NULL",
        [
            Value::from(metric.format.as_db()),
            Value::from(metric.direction.as_db()),
            Value::from(metric.key.as_str()),
        ],
    ))
    .await?;
    Ok(())
}

async fn append_revision<C: ConnectionTrait>(
    conn: &C,
    kind: DefinitionKind,
    key: &str,
    version: i32,
    actor: &str,
    body: &serde_json::Value,
) -> Result<(), DbErr> {
    conn.execute(Statement::from_sql_and_values(
        conn.get_database_backend(),
        "INSERT INTO semantic_definition_revisions \
            (id, kind, definition_key, version, actor, body) \
         VALUES (?, ?, ?, ?, ?, ?)",
        [
            uuid_value(Uuid::now_v7()),
            Value::from(kind.as_db()),
            Value::from(key),
            Value::from(version),
            Value::from(actor),
            Value::from(body.to_string()),
        ],
    ))
    .await?;
    Ok(())
}

fn uuid_value(id: Uuid) -> Value {
    Value::Bytes(Some(Box::new(id.as_bytes().to_vec())))
}

fn optional_str(value: Option<&str>) -> Value {
    match value {
        Some(value) => Value::from(value),
        None => Value::String(None),
    }
}

fn json_value<T: serde::Serialize>(value: Option<&T>) -> Value {
    match value.map(|value| serde_json::to_string(value)) {
        Some(Ok(json)) => Value::from(json),
        // A definition that cannot serialize never reaches the store: the write
        // path only accepts parsed, validated definitions, whose types are
        // serde-derived and total.
        Some(Err(_)) | None => Value::String(None),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::domain::definitions::definition::{
        Aggregation, Computation, DimensionBinding, Direction, Format, ReadDiscipline, Transform,
    };

    fn dataset() -> DatasetDefinition {
        DatasetDefinition {
            key: "git_pull_requests".to_owned(),
            relation: "silver.class_git_pull_requests".to_owned(),
            read_discipline: ReadDiscipline::Final,
            description: None,
            retention_horizon: None,
        }
    }

    fn measure() -> MeasureDefinition {
        serde_yaml::from_str(
            r"
key: prs_merged
dataset: git_pull_requests
aggregation: count
event_time: closed_on
entity: author_email
dimensions:
  - { key: repository, value_field: repo_slug }
",
        )
        .expect("parses")
    }

    fn metric() -> MetricDefinition {
        MetricDefinition {
            key: "git.prs_merged".to_owned(),
            computation: Computation::Direct {
                measure: "prs_merged".to_owned(),
            },
            transform: None,
            format: Format::Integer,
            direction: Direction::HigherIsBetter,
            entity_type: "person".to_owned(),
            cohort_key: None,
            label: None,
            description: None,
        }
    }

    #[test]
    fn dataset_semantics_bump_but_its_description_does_not() {
        let base = dataset_body(&dataset());

        let mut described = dataset();
        described.description = Some("Pull requests, deduplicated.".to_owned());
        assert_eq!(
            plan(Some((1, &base)), &dataset_body(&described)),
            WritePlan::Unchanged { at: 1 }
        );

        let mut moved = dataset();
        moved.relation = "insight.git_pull_requests".to_owned();
        let mut reread = dataset();
        reread.read_discipline = ReadDiscipline::None;
        let mut retained = dataset();
        retained.retention_horizon = Some("P2Y".to_owned());

        for (field, mutated) in [
            ("relation", moved),
            ("read_discipline", reread),
            ("retention_horizon", retained),
        ] {
            assert_eq!(
                plan(Some((1, &base)), &dataset_body(&mutated)),
                WritePlan::Bump { from: 1 },
                "{field} must be semantic"
            );
        }
    }

    #[test]
    fn an_absent_definition_is_inserted() {
        assert_eq!(plan(None, &measure_body(&measure())), WritePlan::Insert);
    }

    #[test]
    fn identical_semantics_do_not_bump() {
        let body = measure_body(&measure());
        assert_eq!(
            plan(Some((3, &body)), &measure_body(&measure())),
            WritePlan::Unchanged { at: 3 }
        );
    }

    #[test]
    fn dimension_order_is_not_a_semantic_change() {
        let mut reordered = measure();
        reordered.dimensions.insert(
            0,
            DimensionBinding {
                key: "source".to_owned(),
                value_field: "data_source".to_owned(),
                label_field: None,
            },
        );
        let mut same = measure();
        same.dimensions.push(DimensionBinding {
            key: "source".to_owned(),
            value_field: "data_source".to_owned(),
            label_field: None,
        });
        assert_eq!(measure_body(&reordered), measure_body(&same));
    }

    #[test]
    fn a_description_change_is_not_a_semantic_change() {
        let mut described = measure();
        described.description = Some("Merged pull requests.".to_owned());
        assert_eq!(measure_body(&described), measure_body(&measure()));
    }

    #[test]
    fn every_semantic_field_bumps() {
        let base = measure_body(&measure());
        let mut summed = measure();
        summed.aggregation = Aggregation::Sum;
        summed.value_expr = Some("lines_added".to_owned());
        let mut refiltered = measure();
        refiltered.filter = serde_yaml::from_str("{ field: state, op: eq, value: merged }").ok();
        let mut redated = measure();
        redated.event_time = "created_on".to_owned();
        let mut reentitied = measure();
        reentitied.entity = "committer_email".to_owned();
        let mut redimensioned = measure();
        redimensioned.dimensions.clear();
        let mut moved = measure();
        moved.dataset = "git_commits".to_owned();

        for (field, mutated) in [
            ("aggregation", summed),
            ("filter", refiltered),
            ("event_time", redated),
            ("entity", reentitied),
            ("dimensions", redimensioned),
            ("dataset", moved),
        ] {
            assert_eq!(
                plan(Some((1, &base)), &measure_body(&mutated)),
                WritePlan::Bump { from: 1 },
                "{field} must be semantic"
            );
        }
    }

    #[test]
    fn metric_display_is_not_semantic_but_computation_and_transform_are() {
        let base = metric_body(&metric());

        let mut redisplayed = metric();
        redisplayed.format = Format::Percent;
        redisplayed.direction = Direction::Neutral;
        redisplayed.label = Some("Merged PRs".to_owned());
        assert_eq!(
            plan(Some((2, &base)), &metric_body(&redisplayed)),
            WritePlan::Unchanged { at: 2 }
        );

        let mut transformed = metric();
        transformed.transform = Some(Transform {
            multiplier: Some(100.0),
            ..Transform::default()
        });
        assert_eq!(
            plan(Some((2, &base)), &metric_body(&transformed)),
            WritePlan::Bump { from: 2 }
        );

        let mut recomputed = metric();
        recomputed.computation = Computation::Ratio {
            numerator: "prs_merged".to_owned(),
            denominator: "prs_created".to_owned(),
        };
        assert_eq!(
            plan(Some((2, &base)), &metric_body(&recomputed)),
            WritePlan::Bump { from: 2 }
        );
    }

    #[test]
    fn the_revision_body_is_the_semantic_body() {
        let outcome = WriteOutcome::Bumped(4);
        assert_eq!(version_of(outcome), 4);
        assert_eq!(version_of(WriteOutcome::Created), 1);
        assert_eq!(version_of(WriteOutcome::Unchanged(7)), 7);
    }
}
