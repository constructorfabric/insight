//! Live MariaDB tests for the cache policy and coverage store. `#[ignore]`d and
//! skipped when `INTEGRATION_TESTS_MARIADB_URL` is unset; CI runs them with
//! `--include-ignored` against a migrated MariaDB.

use chrono::NaiveDate;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement, Value};

use super::policy::{
    CoverageWrite, coverage_row_kind, enabled_policies, record_coverage, seed_cache_policies,
};
use crate::domain::compiler::cache_build::CacheRowKind;
use crate::domain::definitions::seeds::{product_definitions, reconcile_product_definitions};

const ENV_VAR: &str = "INTEGRATION_TESTS_MARIADB_URL";

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

async fn cleanup(db: &DatabaseConnection, key: &str) {
    for table in ["semantic_cache_policies", "semantic_cache_coverage"] {
        let _ = db
            .execute_raw(Statement::from_sql_and_values(
                db.get_database_backend(),
                format!("DELETE FROM {table} WHERE measure_key = ?"),
                [Value::from(key)],
            ))
            .await;
    }
}

async fn scalar(db: &DatabaseConnection, sql: &str, key: &str) -> String {
    let row = db
        .query_one_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            sql,
            [Value::from(key)],
        ))
        .await
        .expect("query runs")
        .expect("row exists");
    row.try_get::<String>("", "answer").expect("column reads")
}

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn seeding_a_policy_twice_leaves_what_an_operator_set() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    let key = "live_cache_policy_seed";
    cleanup(&db, key).await;

    seed_cache_policies(&db, [key]).await.expect("first seed");
    db.execute_raw(Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE semantic_cache_policies SET enabled = FALSE, hot_window_days = 7 \
         WHERE measure_key = ?",
        [Value::from(key)],
    ))
    .await
    .expect("an operator narrows the policy");

    seed_cache_policies(&db, [key]).await.expect("second seed");

    let settings = scalar(
        &db,
        "SELECT CONCAT(enabled, ':', hot_window_days) AS answer \
         FROM semantic_cache_policies WHERE measure_key = ?",
        key,
    )
    .await;

    assert_eq!(settings, "0:7", "a reseed must not overwrite the policy");
    cleanup(&db, key).await;
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn coverage_widens_with_every_successful_build_and_never_narrows() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    let key = "live_cache_coverage";
    cleanup(&db, key).await;

    record_coverage(
        &db,
        key,
        1,
        CacheRowKind::Aggregate,
        date(2026, 2, 1),
        date(2026, 3, 8),
        CoverageWrite::Widen,
    )
    .await
    .expect("first build lands");
    record_coverage(
        &db,
        key,
        1,
        CacheRowKind::Aggregate,
        date(2026, 2, 5),
        date(2026, 3, 12),
        CoverageWrite::Widen,
    )
    .await
    .expect("second build lands");

    let covered = scalar(
        &db,
        "SELECT CONCAT(covered_from, '..', covered_to, ' ', row_kind) AS answer \
         FROM semantic_cache_coverage WHERE measure_key = ?",
        key,
    )
    .await;

    assert_eq!(covered, "2026-02-01..2026-03-12 aggregate");
    cleanup(&db, key).await;
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn a_rebuild_at_a_new_row_shape_replaces_the_window_the_old_shape_covered() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    let key = "live_cache_reshaped";
    cleanup(&db, key).await;

    record_coverage(
        &db,
        key,
        1,
        CacheRowKind::Aggregate,
        date(2026, 1, 1),
        date(2026, 3, 8),
        CoverageWrite::Widen,
    )
    .await
    .expect("the first shape lands");

    assert_eq!(
        coverage_row_kind(&db, key, 1)
            .await
            .expect("coverage reads"),
        Some(CacheRowKind::Aggregate)
    );

    record_coverage(
        &db,
        key,
        1,
        CacheRowKind::Event,
        date(2026, 3, 1),
        date(2026, 3, 12),
        CoverageWrite::Replace,
    )
    .await
    .expect("the rebuild lands");

    let covered = scalar(
        &db,
        "SELECT CONCAT(covered_from, '..', covered_to, ' ', row_kind) AS answer \
         FROM semantic_cache_coverage WHERE measure_key = ?",
        key,
    )
    .await;

    assert_eq!(
        covered, "2026-03-01..2026-03-12 event",
        "a reshaped rebuild claims only what it built"
    );
    cleanup(&db, key).await;
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn every_shipped_measure_reconciles_with_a_policy_and_no_second_pass_churns_it() {
    let Some(db) = connect_or_skip().await else {
        return;
    };

    reconcile_product_definitions(&db)
        .await
        .expect("first reconcile");
    let after_first = enabled_policies(&db).await.expect("policies read");

    reconcile_product_definitions(&db)
        .await
        .expect("second reconcile");
    let after_second = enabled_policies(&db).await.expect("policies read");

    let shipped = product_definitions().expect("definitions are valid");
    for measure in &shipped.measures {
        assert!(
            after_first
                .iter()
                .any(|policy| policy.measure_key == measure.key),
            "measure `{}` has no cache policy",
            measure.key
        );
    }
    assert_eq!(
        after_first, after_second,
        "a second reconcile churns policy"
    );
}
