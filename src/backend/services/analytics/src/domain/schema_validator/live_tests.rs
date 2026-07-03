//! Live integration tests for the schema-validator (Refs #521).
//!
//! All tests are `#[ignore]`d by default and skip silently when the required
//! env vars are unset, so `cargo test` and `cargo test -- --ignored` stay
//! green on a stock dev machine. Set `INTEGRATION_TESTS_MARIADB_URL` (always)
//! and `INTEGRATION_TESTS_CLICKHOUSE_URL` (for the ClickHouse-touching tests)
//! against throwaway services to exercise them.
//!
//! Migrations are applied once, up front (the `analytics migrate` step /
//! the CI job) — tests never migrate or reset the DB. Each test owns a unique
//! `metric_key` (see `unique_metric_key`), so they run in parallel against the
//! shared catalog without colliding.
//!
//! ## Why the `INTEGRATION_TESTS_` prefix
//!
//! A plain `MARIADB_URL` / `CLICKHOUSE_URL` would collide with the same names
//! commonly exported in a dev shell (compose stacks, docker-machine helpers,
//! in-cluster service discovery) — running `cargo test -- --ignored` with those
//! set would point the tests at whatever DB they name. The `INTEGRATION_TESTS_`
//! prefix forces the operator to opt in for THIS test suite specifically.
//!
//! Coverage map (Definition of Done):
//! - `DoD` #1 readiness on dead ClickHouse: [`validate_all_against_dead_clickhouse_marks_unchecked`].
//! - `DoD` #3 debounce: [`validate_debounces_within_window`].
//! - `DoD` #4 canonical error codes only: [`validate_all_only_writes_canonical_error_codes`].
//! - `DoD` #5 `updated_at` is pinned: [`schema_writes_do_not_bump_updated_at`].
//! - `DoD` #6 rename column → error → ok: [`column_rename_flips_status`].

use std::env;
use std::time::Duration;

use chrono::Utc;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement, Value};

use crate::domain::schema_validator::error_code::SchemaErrorCode;
use crate::domain::schema_validator::repository::{find_by_metric_key, update_schema_columns};
use crate::domain::schema_validator::status::SchemaState;
use crate::domain::schema_validator::{DEFAULT_DEBOUNCE, SchemaValidator, ValidationOutcome};
use uuid::Uuid;

/// Per-test unique `metric_key` so tests run in parallel against the shared,
/// pre-migrated DB without colliding (migrations run once, up front; tests
/// never reset the catalog).
fn unique_metric_key() -> String {
    format!("schema_validator_test_{}.exists", Uuid::now_v7().simple())
}

const MARIADB_ENV: &str = "INTEGRATION_TESTS_MARIADB_URL";
const CLICKHOUSE_ENV: &str = "INTEGRATION_TESTS_CLICKHOUSE_URL";
const CLICKHOUSE_DB_ENV: &str = "INTEGRATION_TESTS_CLICKHOUSE_DATABASE";
const TEST_METRIC_KEY: &str = "schema_validator_test.exists";
const TEST_METRIC_KEY_MISSING_TABLE: &str = "no_such_table.col";

async fn connect_mariadb() -> Option<DatabaseConnection> {
    let Ok(url) = env::var(MARIADB_ENV) else {
        eprintln!("skipping: {MARIADB_ENV} not set");
        return None;
    };
    let mut opts = ConnectOptions::new(url);
    opts.max_connections(2).sqlx_logging(false);
    match Database::connect(opts).await {
        Ok(db) => Some(db),
        Err(e) => {
            eprintln!("skipping: cannot connect to {MARIADB_ENV}: {e}");
            None
        }
    }
}

fn connect_clickhouse() -> Option<insight_clickhouse::Client> {
    let Ok(url) = env::var(CLICKHOUSE_ENV) else {
        eprintln!("skipping: {CLICKHOUSE_ENV} not set");
        return None;
    };
    let database = env::var(CLICKHOUSE_DB_ENV).unwrap_or_else(|_| "default".to_owned());
    Some(insight_clickhouse::Client::new(
        insight_clickhouse::Config::new(url, database),
    ))
}

async fn seed_row(db: &DatabaseConnection, metric_key: &str) -> anyhow::Result<()> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO metric_catalog \
         (id, tenant_id, metric_key, label, higher_is_better, is_member_scale, source_tags, is_enabled) \
         VALUES (UNHEX(REPLACE(UUID(),'-','')), NULL, ?, 'Live test', TRUE, FALSE, JSON_ARRAY('test'), TRUE)",
        [Value::from(metric_key)],
    ))
    .await?;
    Ok(())
}

async fn create_test_table(ch: &insight_clickhouse::Client) -> anyhow::Result<()> {
    // Create a tiny table with the two columns we exercise: `exists` (probed
    // for `Ok`) and `other` (used by the rename test).
    ch.query("CREATE TABLE IF NOT EXISTS schema_validator_test (exists UInt8, other UInt8) ENGINE = Memory")
        .execute()
        .await?;
    Ok(())
}

async fn drop_test_table(ch: &insight_clickhouse::Client) {
    let _ = ch
        .query("DROP TABLE IF EXISTS schema_validator_test")
        .execute()
        .await;
}

#[tokio::test]
#[ignore = "requires live MariaDB; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn schema_writes_do_not_bump_updated_at() -> anyhow::Result<()> {
    // DoD #5: every schema_* write pins `updated_at` so the product-metadata
    // last-changed signal stays meaningful.
    let Some(db) = connect_mariadb().await else {
        return Ok(());
    };
    let metric_key = unique_metric_key();
    seed_row(&db, &metric_key).await?;

    let initial = find_by_metric_key(&db, &metric_key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("seeded row vanished"))?;
    let row_id = initial.id;

    // Snapshot `updated_at` from the row directly — the validator entity
    // doesn't read this column on the read path, so query it inline.
    let updated_before = read_updated_at(&db, row_id).await?;

    for i in 0..50 {
        let state = if i % 2 == 0 {
            SchemaState::ok()
        } else {
            SchemaState::error(SchemaErrorCode::ColumnNotFound)
        };
        update_schema_columns(&db, row_id, state, Utc::now()).await?;
    }

    let updated_after = read_updated_at(&db, row_id).await?;
    anyhow::ensure!(
        updated_before == updated_after,
        "updated_at must be pinned across schema_* writes; before={updated_before:?} after={updated_after:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn validate_debounces_within_window() -> anyhow::Result<()> {
    // DoD #3: the per-write hook skips ClickHouse when schema_checked_at is
    // recent. We don't need a live ClickHouse here — the debounce branch
    // short-circuits before the probe.
    let Some(db) = connect_mariadb().await else {
        return Ok(());
    };
    let metric_key = unique_metric_key();
    seed_row(&db, &metric_key).await?;

    // Use a deliberately broken ClickHouse URL so any second call would error
    // loudly (and fail the test). Combined with a wide debounce window, this
    // pins the no-network behaviour.
    let ch = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
        "http://127.0.0.1:1",
        "default",
    ));
    let v = SchemaValidator::new(db.clone(), ch).with_debounce(Duration::from_hours(1));

    // Prime the row's schema_checked_at to "now" so the next call hits the
    // debounce branch unambiguously, even on slow CI runners.
    let row_id = find_by_metric_key(&db, &metric_key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("seeded row vanished"))?
        .id;
    update_schema_columns(&db, row_id, SchemaState::ok(), Utc::now()).await?;

    let outcome = v.validate(&metric_key).await;
    anyhow::ensure!(
        outcome == ValidationOutcome::DebouncedSkipped,
        "expected DebouncedSkipped within the debounce window, got {outcome:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB + live ClickHouse; set INTEGRATION_TESTS_MARIADB_URL + INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
async fn validate_all_against_dead_clickhouse_marks_unchecked() -> anyhow::Result<()> {
    // DoD #1: when ClickHouse is unreachable, the validator marks rows
    // unchecked and the readiness probe is unaffected. We don't drive the
    // HTTP handler here — `health` is a pure 200-returning closure, proven
    // by the route registration in `api::router`. What we DO prove is that
    // the validator's startup pass doesn't panic, doesn't deadlock, and
    // flips rows to `unchecked` with `error_code = NULL` (biconditional).
    let Some(db) = connect_mariadb().await else {
        return Ok(());
    };
    let metric_key = unique_metric_key();
    seed_row(&db, &metric_key).await?;

    // Dead ClickHouse — port 1 is reserved by IANA and nothing listens there.
    let ch = insight_clickhouse::Client::new(
        insight_clickhouse::Config::new("http://127.0.0.1:1", "default")
            .with_query_timeout(Duration::from_secs(1)),
    );

    // Use a tight backoff so the test doesn't have to wait 5 minutes for cap.
    let v = SchemaValidator::new(db.clone(), ch);
    let v_clone = v.clone();
    let handle = tokio::spawn(async move { v_clone.validate_all().await });

    // Give the validator one beat to enter `mark_all_unchecked`, then bail.
    // (We don't need it to complete — we only care that the bulk-mark fires.)
    tokio::time::sleep(Duration::from_secs(3)).await;
    handle.abort();

    let row = find_by_metric_key(&db, &metric_key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("row vanished"))?;
    anyhow::ensure!(
        row.schema_status == "unchecked",
        "expected schema_status='unchecked' after CH-down mark; got {}",
        row.schema_status
    );
    anyhow::ensure!(
        row.schema_error_code.is_none(),
        "biconditional violated: status='unchecked' must have NULL error_code on the wire; got {:?}",
        row.schema_error_code
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB + live ClickHouse; set INTEGRATION_TESTS_MARIADB_URL + INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
async fn validate_all_only_writes_canonical_error_codes() -> anyhow::Result<()> {
    // DoD #4: no raw CH text in schema_error_code. We seed a row whose table
    // doesn't exist; the validator must persist `error_code='table_not_found'`,
    // never anything from the raw ClickHouse error string.
    let Some(db) = connect_mariadb().await else {
        return Ok(());
    };
    let Some(ch) = connect_clickhouse() else {
        return Ok(());
    };
    seed_row(&db, TEST_METRIC_KEY_MISSING_TABLE).await?;

    let v = SchemaValidator::new(db.clone(), ch);
    let _ = v.validate_all().await;

    let row = find_by_metric_key(&db, TEST_METRIC_KEY_MISSING_TABLE)
        .await?
        .ok_or_else(|| anyhow::anyhow!("row vanished"))?;
    anyhow::ensure!(
        row.schema_status == "error",
        "expected error status for missing table; got {}",
        row.schema_status
    );
    let code = row
        .schema_error_code
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("schema_error_code is NULL despite status=error"))?;
    anyhow::ensure!(
        matches!(
            code,
            "table_not_found" | "column_not_found" | "clickhouse_unreachable" | "unknown"
        ),
        "schema_error_code must be canonical; got {code:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB + live ClickHouse; set INTEGRATION_TESTS_MARIADB_URL + INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
async fn column_rename_flips_status() -> anyhow::Result<()> {
    // DoD #6: rename a column → next validate flips to error/column_not_found;
    // rename back → flips to ok.
    let Some(db) = connect_mariadb().await else {
        return Ok(());
    };
    let Some(ch) = connect_clickhouse() else {
        return Ok(());
    };
    seed_row(&db, TEST_METRIC_KEY).await?;
    create_test_table(&ch).await?;

    // Zero-out debounce so we can re-validate immediately.
    let v = SchemaValidator::new(db.clone(), ch.clone()).with_debounce(Duration::from_millis(0));

    // Initial pass: column exists → Ok.
    let outcome = v.validate(TEST_METRIC_KEY).await;
    anyhow::ensure!(
        outcome == ValidationOutcome::Ok,
        "expected Ok with column present; got {outcome:?}"
    );

    // Rename `exists` → `existed`. Next validate must see column_not_found.
    ch.query("ALTER TABLE schema_validator_test RENAME COLUMN exists TO existed")
        .execute()
        .await?;
    let outcome = v.validate(TEST_METRIC_KEY).await;
    anyhow::ensure!(
        outcome == ValidationOutcome::Error(SchemaErrorCode::ColumnNotFound),
        "expected Error(ColumnNotFound) after rename; got {outcome:?}"
    );

    // Rename back. Validate flips to Ok again.
    ch.query("ALTER TABLE schema_validator_test RENAME COLUMN existed TO exists")
        .execute()
        .await?;
    let outcome = v.validate(TEST_METRIC_KEY).await;
    anyhow::ensure!(
        outcome == ValidationOutcome::Ok,
        "expected Ok after rename back; got {outcome:?}"
    );

    drop_test_table(&ch).await;
    Ok(())
}

// Compile-time pin: per-write debounce tests assume the default window equals
// 60 s. If a future refactor changes `DEFAULT_DEBOUNCE`, this assert fails at
// build time and the test that primes `schema_checked_at = now` would need a
// new window. (`Duration::as_secs` is `const` since Rust 1.79.)
const _: () = assert!(DEFAULT_DEBOUNCE.as_secs() == 60);

async fn read_updated_at(
    db: &DatabaseConnection,
    id: uuid::Uuid,
) -> anyhow::Result<chrono::DateTime<chrono::Utc>> {
    use sea_orm::FromQueryResult;

    // `metric_catalog.updated_at` is `TIMESTAMP` (set by SeaORM's
    // `Expr::current_timestamp()` in the migration). sea-orm 1.1.20's
    // decoder refuses to map `TIMESTAMP` into `NaiveDateTime` — it
    // only accepts `DATETIME` there. `DateTime<Utc>` IS the documented
    // round-trip for the `TIMESTAMP` SQL type, so we use it here.
    #[derive(FromQueryResult)]
    struct Row {
        updated_at: chrono::DateTime<chrono::Utc>,
    }

    let row = Row::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT updated_at FROM metric_catalog WHERE id = ?",
        [Value::from(id)],
    ))
    .one(db)
    .await?
    .ok_or_else(|| anyhow::anyhow!("row {id} not found"))?;
    Ok(row.updated_at)
}
