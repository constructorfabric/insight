//! Live ClickHouse test for the cache write path: the migration's own DDL, the
//! compiled build for each row kind, and the partition swap, all executed for
//! real in a scratch database that the test drops when it is done.
//!
//! `#[ignore]`d and skips silently when `INTEGRATION_TESTS_CLICKHOUSE_URL` is
//! unset; optional auth via `INTEGRATION_TESTS_CLICKHOUSE_USER` /
//! `INTEGRATION_TESTS_CLICKHOUSE_PASSWORD`.

use chrono::NaiveDate;
use clickhouse::Row;
use serde::Deserialize;
use uuid::Uuid;

use std::collections::BTreeMap;

use super::plan;
use crate::domain::compiler::cache_build::{CacheBuild, CacheRowKind, compile_cache_build};
use crate::domain::compiler::cache_read::{CachedInput, compile_cached_metric_query};
use crate::domain::compiler::metric::compile_metric_query;
use crate::domain::compiler::request::{Bucket, EntityScope, MetricQuery, ViewKind};
use crate::domain::compiler::sql::{CompiledMeasureQuery, QueryParam};
use crate::domain::definitions::definition::{
    Aggregation, Computation, DimensionBinding, Direction, Format, MeasureDefinition,
    MetricDefinition,
};
use crate::domain::field_catalog::model::{
    CatalogDataset, CatalogField, EntityType, FieldCatalog, FieldRole, FieldType, ReadDiscipline,
};

const URL_VAR: &str = "INTEGRATION_TESTS_CLICKHOUSE_URL";

/// The relations the migration writes are named for the app database; a scratch
/// run rewrites that one prefix and changes nothing else.
const SHIPPED_DATABASE: &str = "insight.";

const MIGRATION: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../ingestion/scripts/migrations/20260828000000_semantic-measure-cache.sql"
);

const WINDOW_FROM: (i32, u32, u32) = (2026, 3, 1);
const WINDOW_TO: (i32, u32, u32) = (2026, 4, 30);
const REBUILD_FROM: (i32, u32, u32) = (2026, 4, 1);
const VERSION: u32 = 4;

#[derive(Debug, Row, Deserialize)]
struct Counted {
    rows: u64,
}

#[derive(Debug, Row, Deserialize)]
struct Summed {
    total: f64,
}

/// One read the reconciliation pins: the metric to compute, the measure the
/// cache answers it from, and what the fixture rows say the answer is.
struct Reconciled {
    measure_key: &'static str,
    computation: Computation,
    expected: &'static [(&'static str, f64)],
}

/// One partition of the served relation, as the version read names it.
#[derive(Debug, Row, Deserialize)]
struct Partition {
    version: u32,
    month: u32,
}

/// One entity's answer, as every value view keys it.
#[derive(Debug, Row, Deserialize, PartialEq)]
struct EntityValue {
    entity_id: String,
    value: f64,
}

// Empty counts as unset: the CI matrix passes '' to entries without a
// provisioned ClickHouse, and set-but-empty must skip exactly like absent.
fn client_or_skip() -> Option<insight_clickhouse::Client> {
    let url = std::env::var(URL_VAR).unwrap_or_default();
    if url.is_empty() {
        eprintln!("skipping: {URL_VAR} not set");
        return None;
    }
    let mut config = insight_clickhouse::Config::new(url, "default");
    if let (Ok(user), Ok(password)) = (
        std::env::var("INTEGRATION_TESTS_CLICKHOUSE_USER"),
        std::env::var("INTEGRATION_TESTS_CLICKHOUSE_PASSWORD"),
    ) && !user.is_empty()
    {
        config = config.with_auth(user, password);
    }
    Some(insight_clickhouse::Client::new(config))
}

fn scoped(sql: &str, database: &str) -> String {
    sql.replace(SHIPPED_DATABASE, &format!("{database}."))
}

async fn run(ch: &insight_clickhouse::Client, sql: &str) -> Result<(), clickhouse::error::Error> {
    ch.query(sql).execute().await
}

async fn bind_and_run(
    ch: &insight_clickhouse::Client,
    compiled: &CompiledMeasureQuery,
    database: &str,
) -> Result<(), clickhouse::error::Error> {
    let mut request = ch.query(&scoped(&compiled.sql, database));
    for param in &compiled.params {
        request = match param {
            QueryParam::Text(value) => request.bind(value.as_str()),
            QueryParam::Int(value) => request.bind(*value),
            QueryParam::UInt(value) => request.bind(*value),
            QueryParam::Float(value) => request.bind(*value),
            QueryParam::Bool(value) => request.bind(*value),
        };
    }
    request.execute().await
}

async fn count(
    ch: &insight_clickhouse::Client,
    sql: &str,
) -> Result<u64, clickhouse::error::Error> {
    ch.query(sql)
        .fetch_one::<Counted>()
        .await
        .map(|row| row.rows)
}

/// Six rows in two months: a repeated ticket on one day so a subject build
/// deduplicates, three rows on one day so an aggregate build folds, and one row
/// with no tenant that no build may keep.
async fn create_source(
    ch: &insight_clickhouse::Client,
    database: &str,
) -> Result<(), clickhouse::error::Error> {
    run(
        ch,
        &format!(
            "CREATE TABLE {database}.work_items (
                 tenant_id Nullable(String),
                 actor String,
                 happened_at DateTime,
                 amount Int64,
                 ticket String,
                 repo String,
                 repo_label String
             ) ENGINE = MergeTree ORDER BY (actor, happened_at)"
        ),
    )
    .await?;
    run(
        ch,
        &format!(
            "INSERT INTO {database}.work_items VALUES
                 ('acme', 'alice@example.com', '2026-03-02 09:00:00', 10, 'T-1', 'r1', 'Repo One'),
                 ('acme', 'alice@example.com', '2026-03-02 10:00:00', 20, 'T-1', 'r1', 'Repo One'),
                 ('acme', 'alice@example.com', '2026-03-02 11:00:00', 30, 'T-2', 'r1', 'Repo One'),
                 ('acme', 'bob@example.com',   '2026-03-05 09:00:00', 40, 'T-3', 'r2', 'Repo Two'),
                 (NULL,   'carol@example.com', '2026-03-06 09:00:00', 50, 'T-4', 'r1', 'Repo One'),
                 ('acme', 'alice@example.com', '2026-04-01 09:00:00', 60, 'T-5', 'r1', 'Repo One')"
        ),
    )
    .await
}

fn dataset(database: &str) -> CatalogDataset {
    let field = |name: &str, raw: &str, role: Option<FieldRole>| CatalogField {
        name: name.to_owned(),
        field_type: FieldType::parse(raw),
        role,
        display: Vec::new(),
        label_field: None,
    };

    CatalogDataset {
        key: "work_items".to_owned(),
        database: database.to_owned(),
        relation: "work_items".to_owned(),
        entity_type: EntityType::Person,
        read_discipline: ReadDiscipline::Direct,
        sorting_key: vec!["actor".to_owned(), "happened_at".to_owned()],
        row_identity: vec!["ticket".to_owned()],
        fields: vec![
            field("tenant_id", "Nullable(String)", Some(FieldRole::Tenant)),
            field("actor", "String", Some(FieldRole::Entity)),
            field("happened_at", "DateTime", Some(FieldRole::EventTime)),
            field("amount", "Int64", Some(FieldRole::Measurable)),
            field("ticket", "String", Some(FieldRole::Dimension)),
            field("repo", "String", Some(FieldRole::Dimension)),
            field("repo_label", "String", None),
        ],
    }
}

fn measure(key: &str, aggregation: Aggregation) -> MeasureDefinition {
    MeasureDefinition {
        key: key.to_owned(),
        dataset: "work_items".to_owned(),
        description: None,
        filter: None,
        aggregation,
        value_expr: matches!(aggregation, Aggregation::Sum | Aggregation::Avg)
            .then(|| "amount".to_owned()),
        subject_expr: (aggregation == Aggregation::CountDistinct).then(|| "ticket".to_owned()),
        event_time: "happened_at".to_owned(),
        entity: "actor".to_owned(),
        dimensions: vec![DimensionBinding {
            key: "repository".to_owned(),
            value_field: "repo".to_owned(),
            label_field: Some("repo_label".to_owned()),
        }],
    }
}

fn window_date(parts: (i32, u32, u32)) -> NaiveDate {
    NaiveDate::from_ymd_opt(parts.0, parts.1, parts.2).expect("valid date")
}

/// One measure's build over the whole fixture span.
async fn build_and_swap(
    ch: &insight_clickhouse::Client,
    database: &str,
    measure: &MeasureDefinition,
    kind: CacheRowKind,
) -> Result<(), clickhouse::error::Error> {
    build_window(
        ch,
        database,
        measure,
        kind,
        plan::HotWindow {
            from: window_date(WINDOW_FROM),
            to: window_date(WINDOW_TO),
        },
    )
    .await
}

/// One measure's build, staged and swapped exactly as a refresh tick does it,
/// over the window that tick covers.
async fn build_window(
    ch: &insight_clickhouse::Client,
    database: &str,
    measure: &MeasureDefinition,
    kind: CacheRowKind,
    window: plan::HotWindow,
) -> Result<(), clickhouse::error::Error> {
    let compiled = compile_cache_build(
        &dataset(database),
        &CacheBuild {
            measure,
            definition_version: VERSION,
            kind,
            from: window.from,
            to: window.to,
        },
    )
    .expect("a synthetic measure over a synthetic dataset compiles");
    let months = plan::months(window);

    for month in &months {
        partition(
            ch,
            database,
            &plan::clear_staging_partition_sql(),
            *month,
            measure,
        )
        .await?;
    }
    bind_and_run(ch, &compiled, database).await?;
    for month in &months {
        partition(ch, database, &plan::swap_partition_sql(), *month, measure).await?;
    }
    for month in &months {
        partition(
            ch,
            database,
            &plan::clear_staging_partition_sql(),
            *month,
            measure,
        )
        .await?;
    }
    Ok(())
}

async fn partition(
    ch: &insight_clickhouse::Client,
    database: &str,
    sql: &str,
    month: u32,
    measure: &MeasureDefinition,
) -> Result<(), clickhouse::error::Error> {
    ch.query(&scoped(sql, database))
        .bind(measure.key.as_str())
        .bind(VERSION)
        .bind(month)
        .execute()
        .await
}

#[tokio::test]
#[ignore = "requires live ClickHouse; set INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
async fn a_build_of_each_row_kind_lands_through_the_shipped_ddl_and_replaces_on_a_rerun()
-> anyhow::Result<()> {
    let Some(ch) = client_or_skip() else {
        return Ok(());
    };
    let database = format!("semantic_cache_live_{}", Uuid::now_v7().simple());

    let outcome = exercise(&ch, &database).await;

    let _ = run(&ch, &format!("DROP DATABASE IF EXISTS {database}")).await;
    outcome
}

async fn exercise(ch: &insight_clickhouse::Client, database: &str) -> anyhow::Result<()> {
    run(ch, &format!("CREATE DATABASE {database}")).await?;
    let migration = std::fs::read_to_string(MIGRATION)?;
    for statement in migration.split(';') {
        if statement.trim().is_empty() {
            continue;
        }
        run(ch, &scoped(statement, database)).await?;
    }
    create_source(ch, database).await?;

    let cases = [
        (
            measure("folded", Aggregation::Count),
            CacheRowKind::Aggregate,
            3_u64,
        ),
        (measure("kept", Aggregation::Sum), CacheRowKind::Event, 5),
        (
            measure("counted", Aggregation::CountDistinct),
            CacheRowKind::Subject,
            4,
        ),
    ];

    for (measure, kind, expected) in &cases {
        build_and_swap(ch, database, measure, *kind).await?;

        let landed = count(
            ch,
            &format!(
                "SELECT count() AS rows FROM {database}.semantic_measure_cache \
                 WHERE measure_key = '{}'",
                measure.key
            ),
        )
        .await?;
        anyhow::ensure!(
            landed == *expected,
            "{kind:?} landed {landed} rows, expected {expected}"
        );

        build_and_swap(ch, database, measure, *kind).await?;

        let after_rerun = count(
            ch,
            &format!(
                "SELECT count() AS rows FROM {database}.semantic_measure_cache \
                 WHERE measure_key = '{}'",
                measure.key
            ),
        )
        .await?;
        anyhow::ensure!(
            after_rerun == *expected,
            "{kind:?} appended on the second run: {after_rerun} rows"
        );
    }

    let staging_left = count(
        ch,
        &format!("SELECT count() AS rows FROM {database}.semantic_measure_cache_staging"),
    )
    .await?;
    anyhow::ensure!(staging_left == 0, "staging kept {staging_left} rows");

    let untenanted = count(
        ch,
        &format!(
            "SELECT count() AS rows FROM {database}.semantic_measure_cache \
             WHERE entity = 'carol@example.com'"
        ),
    )
    .await?;
    anyhow::ensure!(untenanted == 0, "a row naming no tenant was cached");

    let unlabelled = count(
        ch,
        &format!(
            "SELECT count() AS rows FROM {database}.semantic_measure_cache \
             WHERE dimensions[1].1 != 'repository' OR dimensions[1].3 IS NULL"
        ),
    )
    .await?;
    anyhow::ensure!(unlabelled == 0, "a dimension tuple lost its key or label");

    let summed: f64 = ch
        .query(&format!(
            "SELECT sum(value) AS total FROM {database}.semantic_measure_cache \
             WHERE measure_key = 'kept'"
        ))
        .fetch_one::<Summed>()
        .await?
        .total;
    anyhow::ensure!(
        (summed - 160.0).abs() < f64::EPSILON,
        "kept rows summed to {summed}, expected 160"
    );

    reconcile(ch, database, &cases).await?;
    heal_a_reshaped_measure(ch, database).await
}

/// A measure whose row shape changed at the same version: the refresher drops
/// the partitions the old shape occupies before it swaps the new build in, so
/// no month is left holding rows of a shape the coverage no longer attests.
async fn heal_a_reshaped_measure(
    ch: &insight_clickhouse::Client,
    database: &str,
) -> anyhow::Result<()> {
    let reshaped = measure("reshaped", Aggregation::Sum);
    build_and_swap(ch, database, &reshaped, CacheRowKind::Aggregate).await?;

    let folded = count(
        ch,
        &format!(
            "SELECT count() AS rows FROM {database}.semantic_measure_cache \
             WHERE measure_key = 'reshaped'"
        ),
    )
    .await?;
    anyhow::ensure!(folded == 3, "the folded build landed {folded} rows");

    // The rebuild's hot window reaches back only into April, so March is a
    // settled month the swap alone would leave holding the replaced shape.
    drop_version(ch, database, &reshaped).await?;
    let dropped = count(
        ch,
        &format!(
            "SELECT count() AS rows FROM {database}.semantic_measure_cache \
             WHERE measure_key = 'reshaped'"
        ),
    )
    .await?;
    anyhow::ensure!(dropped == 0, "the old shape left {dropped} rows behind");

    build_window(
        ch,
        database,
        &reshaped,
        CacheRowKind::Event,
        plan::HotWindow {
            from: window_date(REBUILD_FROM),
            to: window_date(WINDOW_TO),
        },
    )
    .await?;

    let mixed = count(
        ch,
        &format!(
            "SELECT count() AS rows FROM {database}.semantic_measure_cache \
             WHERE measure_key = 'reshaped' AND kind != 'event'"
        ),
    )
    .await?;
    anyhow::ensure!(mixed == 0, "{mixed} rows of the replaced shape survived");

    let median = entity_values(
        ch,
        &compile_cached_metric_query(
            &metric(
                "reconciliation",
                Computation::Percentile {
                    measure: "reshaped".to_owned(),
                    quantile: 0.5,
                },
            ),
            &BTreeMap::from([(reshaped.key.clone(), reshaped.clone())]),
            &BTreeMap::from([(
                reshaped.key.clone(),
                CachedInput {
                    kind: CacheRowKind::Event,
                    definition_version: VERSION,
                },
            )]),
            &window_query(),
        )?,
        database,
    )
    .await?;

    anyhow::ensure!(
        median
            == vec![EntityValue {
                entity_id: "alice@example.com".to_owned(),
                value: 60.0,
            }],
        "the rebuilt shape answered {median:?}"
    );
    Ok(())
}

/// The partitions one version occupies, dropped exactly as a reshaped refresh
/// drops them.
async fn drop_version(
    ch: &insight_clickhouse::Client,
    database: &str,
    measure: &MeasureDefinition,
) -> Result<(), clickhouse::error::Error> {
    let occupied = ch
        .query(&scoped(&plan::version_partitions_sql(), database))
        .bind(measure.key.as_str())
        .bind(VERSION)
        .fetch_all::<Partition>()
        .await?;

    let sql = plan::drop_cache_partition_sql();
    for found in occupied {
        ch.query(&scoped(&sql, database))
            .bind(measure.key.as_str())
            .bind(found.version)
            .bind(found.month)
            .execute()
            .await?;
    }
    Ok(())
}

fn catalog(database: &str) -> FieldCatalog {
    FieldCatalog {
        datasets: vec![dataset(database)],
    }
}

fn metric(key: &str, computation: Computation) -> MetricDefinition {
    MetricDefinition {
        key: key.to_owned(),
        computation,
        transform: None,
        format: Format::Decimal,
        direction: Direction::HigherIsBetter,
        entity_type: EntityType::Person,
        cohort_key: None,
        label: None,
        description: None,
    }
}

fn window_query() -> MetricQuery {
    MetricQuery {
        tenant_id: "acme".to_owned(),
        entity_scope: EntityScope::Tenant,
        from: window_date(WINDOW_FROM),
        to: window_date(WINDOW_TO),
        bucket: Bucket::Day,
        dimension_filters: Vec::new(),
        view: ViewKind::SubjectTotal,
        row_limit: 10_000,
    }
}

async fn entity_values(
    ch: &insight_clickhouse::Client,
    compiled: &CompiledMeasureQuery,
    database: &str,
) -> Result<Vec<EntityValue>, clickhouse::error::Error> {
    let mut request = ch.query(&scoped(&compiled.sql, database));
    for param in &compiled.params {
        request = match param {
            QueryParam::Text(value) => request.bind(value.as_str()),
            QueryParam::Int(value) => request.bind(*value),
            QueryParam::UInt(value) => request.bind(*value),
            QueryParam::Float(value) => request.bind(*value),
            QueryParam::Bool(value) => request.bind(*value),
        };
    }

    let mut rows = request.fetch_all::<EntityValue>().await?;
    rows.sort_by(|left, right| left.entity_id.cmp(&right.entity_id));
    Ok(rows)
}

/// The reconciliation in miniature: for each row kind, what the cached read
/// answers, what the dataset read answers, and what the fixture rows say — all
/// three must agree.
async fn reconcile(
    ch: &insight_clickhouse::Client,
    database: &str,
    cases: &[(MeasureDefinition, CacheRowKind, u64)],
) -> anyhow::Result<()> {
    // alice authored three rows in March and one in April; bob one in March;
    // the row naming no tenant belongs to no answer.
    let expectations = [
        Reconciled {
            measure_key: "folded",
            computation: Computation::Direct {
                measure: "folded".to_owned(),
            },
            expected: &[("alice@example.com", 4.0), ("bob@example.com", 1.0)],
        },
        Reconciled {
            measure_key: "kept",
            computation: Computation::Direct {
                measure: "kept".to_owned(),
            },
            expected: &[("alice@example.com", 120.0), ("bob@example.com", 40.0)],
        },
        Reconciled {
            measure_key: "counted",
            computation: Computation::Direct {
                measure: "counted".to_owned(),
            },
            expected: &[("alice@example.com", 3.0), ("bob@example.com", 1.0)],
        },
        Reconciled {
            measure_key: "kept",
            computation: Computation::Percentile {
                measure: "kept".to_owned(),
                quantile: 0.5,
            },
            expected: &[("alice@example.com", 30.0), ("bob@example.com", 40.0)],
        },
    ];

    let measures: BTreeMap<String, MeasureDefinition> = cases
        .iter()
        .map(|(measure, _, _)| (measure.key.clone(), measure.clone()))
        .collect();
    let kinds: BTreeMap<String, CacheRowKind> = cases
        .iter()
        .map(|(measure, kind, _)| (measure.key.clone(), *kind))
        .collect();

    for Reconciled {
        measure_key,
        computation,
        expected,
    } in expectations
    {
        let metric = metric("reconciliation", computation);
        let query = window_query();
        let cached = BTreeMap::from([(
            measure_key.to_owned(),
            CachedInput {
                kind: kinds[measure_key],
                definition_version: VERSION,
            },
        )]);

        let from_cache = entity_values(
            ch,
            &compile_cached_metric_query(&metric, &measures, &cached, &query)?,
            database,
        )
        .await?;
        let from_dataset = entity_values(
            ch,
            &compile_metric_query(&catalog(database), &metric, &measures, &query)?,
            database,
        )
        .await?;
        let hand_computed: Vec<EntityValue> = expected
            .iter()
            .map(|(entity_id, value)| EntityValue {
                entity_id: (*entity_id).to_owned(),
                value: *value,
            })
            .collect();

        anyhow::ensure!(
            from_cache == hand_computed,
            "{measure_key}: the cache answered {from_cache:?}, the fixture rows say {hand_computed:?}"
        );
        anyhow::ensure!(
            from_dataset == hand_computed,
            "{measure_key}: the dataset answered {from_dataset:?}, the fixture rows say {hand_computed:?}"
        );
    }

    Ok(())
}
