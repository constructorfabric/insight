//! Live MariaDB tests for the definition store's write path.
//!
//! `#[ignore]`d and skipped when `INTEGRATION_TESTS_MARIADB_URL` is unset, so
//! `cargo test` stays green on a stock dev machine (the convention of
//! `metric_definitions/live_tests.rs`). CI runs them with `--include-ignored`
//! against a migrated MariaDB, which is where the SQL earns its coverage: the
//! version decision is unit-tested, but idempotence, the compare-and-set, and
//! the CHECK constraints only exist against a real server.

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement, Value};

use super::definition::{
    Aggregation, Computation, Direction, Format, MeasureDefinition, MetricDefinition, Origin,
    Transform,
};
use super::store::{WriteOutcome, reconcile_measure, reconcile_metric, update_measure};

const ENV_VAR: &str = "INTEGRATION_TESTS_MARIADB_URL";
const ACTOR: &str = "live-tests";

async fn connect_or_skip() -> Option<DatabaseConnection> {
    let Ok(url) = std::env::var(ENV_VAR) else {
        eprintln!("skipping: {ENV_VAR} not set");
        return None;
    };
    let mut opts = ConnectOptions::new(url);
    opts.max_connections(2).sqlx_logging(false);
    match Database::connect(opts).await {
        Ok(db) => Some(db),
        Err(e) => {
            eprintln!("skipping: cannot connect to {ENV_VAR}: {e}");
            None
        }
    }
}

/// Product rows are keyed by `(tenant sentinel, key)`, so a test key must be
/// unique per test to keep parallel runs independent.
fn measure(key: &str) -> MeasureDefinition {
    MeasureDefinition {
        key: key.to_owned(),
        dataset: "git_pull_requests".to_owned(),
        description: None,
        filter: serde_yaml::from_str("{ field: state, op: eq, value: merged }").ok(),
        aggregation: Aggregation::Count,
        value_expr: None,
        subject_expr: None,
        event_time: "closed_on".to_owned(),
        entity: "author_email".to_owned(),
        dimensions: Vec::new(),
    }
}

fn metric(key: &str, measure_key: &str) -> MetricDefinition {
    MetricDefinition {
        key: key.to_owned(),
        computation: Computation::Direct {
            measure: measure_key.to_owned(),
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

async fn cleanup(db: &DatabaseConnection, keys: &[&str]) {
    for (table, column) in [
        ("semantic_measures", "measure_key"),
        ("semantic_metrics", "metric_key"),
        ("semantic_definition_revisions", "definition_key"),
    ] {
        for key in keys {
            let _ = db
                .execute(Statement::from_sql_and_values(
                    db.get_database_backend(),
                    format!("DELETE FROM {table} WHERE {column} = ?"),
                    [Value::from(*key)],
                ))
                .await;
        }
    }
}

async fn stored_version(db: &DatabaseConnection, table: &str, column: &str, key: &str) -> i32 {
    let row = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            format!("SELECT definition_version FROM {table} WHERE {column} = ?"),
            [Value::from(key)],
        ))
        .await
        .expect("query runs")
        .expect("row exists");
    row.try_get::<i32>("", "definition_version")
        .expect("column reads")
}

async fn revision_versions(db: &DatabaseConnection, key: &str) -> Vec<i32> {
    let rows = db
        .query_all(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT version FROM semantic_definition_revisions \
             WHERE definition_key = ? ORDER BY version",
            [Value::from(key)],
        ))
        .await
        .expect("query runs");
    rows.into_iter()
        .map(|row| row.try_get::<i32>("", "version").expect("column reads"))
        .collect()
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn reconciling_the_same_measure_twice_writes_once() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    let key = "live_store_idempotent";
    cleanup(&db, &[key]).await;

    let definition = measure(key);
    assert_eq!(
        reconcile_measure(&db, &definition, Origin::Product, ACTOR)
            .await
            .expect("first write"),
        WriteOutcome::Created
    );
    assert_eq!(
        reconcile_measure(&db, &definition, Origin::Product, ACTOR)
            .await
            .expect("second write"),
        WriteOutcome::Unchanged(1)
    );

    assert_eq!(
        stored_version(&db, "semantic_measures", "measure_key", key).await,
        1
    );
    assert_eq!(
        revision_versions(&db, key).await,
        [1],
        "a reconcile that changes nothing writes no revision"
    );
    cleanup(&db, &[key]).await;
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn a_semantic_change_bumps_the_version_and_appends_a_revision() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    let key = "live_store_bump";
    cleanup(&db, &[key]).await;

    reconcile_measure(&db, &measure(key), Origin::Product, ACTOR)
        .await
        .expect("first write");

    let mut changed = measure(key);
    changed.aggregation = Aggregation::Sum;
    changed.value_expr = Some("lines_added".to_owned());
    assert_eq!(
        reconcile_measure(&db, &changed, Origin::Product, ACTOR)
            .await
            .expect("second write"),
        WriteOutcome::Bumped(2)
    );
    assert_eq!(
        stored_version(&db, "semantic_measures", "measure_key", key).await,
        2
    );
    assert_eq!(revision_versions(&db, key).await, [1, 2]);
    cleanup(&db, &[key]).await;
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn a_write_against_a_superseded_version_touches_nothing() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    let key = "live_store_conflict";
    cleanup(&db, &[key]).await;

    reconcile_measure(&db, &measure(key), Origin::Product, ACTOR)
        .await
        .expect("first write");

    // Another writer bumps the row after this one read version 1.
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE semantic_measures SET definition_version = definition_version + 1 \
         WHERE measure_key = ?",
        [Value::from(key)],
    ))
    .await
    .expect("concurrent bump");

    let mut changed = measure(key);
    changed.event_time = "created_on".to_owned();
    assert_eq!(
        update_measure(&db, &changed, 1).await.expect("update runs"),
        0,
        "the compare-and-set must match no row once the version has moved"
    );
    assert_eq!(
        stored_version(&db, "semantic_measures", "measure_key", key).await,
        2,
        "a losing writer leaves the row exactly as it found it"
    );

    // Re-reading is what resolves it: the next reconcile sees version 2.
    assert_eq!(
        reconcile_measure(&db, &changed, Origin::Product, ACTOR)
            .await
            .expect("write against the current version"),
        WriteOutcome::Bumped(3)
    );
    cleanup(&db, &[key]).await;
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn metric_display_moves_without_a_version_bump() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    let key = "live.store_display";
    cleanup(&db, &[key]).await;

    reconcile_metric(&db, &metric(key, "prs_merged"), Origin::Product, ACTOR)
        .await
        .expect("first write");

    let mut redisplayed = metric(key, "prs_merged");
    redisplayed.format = Format::Percent;
    redisplayed.direction = Direction::Neutral;
    assert_eq!(
        reconcile_metric(&db, &redisplayed, Origin::Product, ACTOR)
            .await
            .expect("display write"),
        WriteOutcome::Unchanged(1)
    );
    assert_eq!(
        stored_version(&db, "semantic_metrics", "metric_key", key).await,
        1
    );

    let mut transformed = metric(key, "prs_merged");
    transformed.transform = Some(Transform {
        multiplier: Some(100.0),
        ..Transform::default()
    });
    assert_eq!(
        reconcile_metric(&db, &transformed, Origin::Product, ACTOR)
            .await
            .expect("transform write"),
        WriteOutcome::Bumped(2)
    );
    cleanup(&db, &[key]).await;
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn the_store_rejects_an_aggregation_without_its_operand() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    let key = "live_store_check";
    cleanup(&db, &[key]).await;

    // `sum` without a value expression violates the store's biconditional. The
    // validators reject this before it reaches the store; the CHECK is the
    // backstop that makes the rejection structural.
    let mut invalid = measure(key);
    invalid.aggregation = Aggregation::Sum;
    assert!(
        reconcile_measure(&db, &invalid, Origin::Product, ACTOR)
            .await
            .is_err()
    );
    cleanup(&db, &[key]).await;
}
