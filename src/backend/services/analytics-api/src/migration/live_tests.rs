//! Live MariaDB integration tests for the metric-catalog schema (Refs #519).
//!
//! All tests in this module are `#[ignore]`d by default and skip silently
//! when the `MARIADB_URL` env var is unset, so `cargo test` and even
//! `cargo test -- --ignored` are green on a stock dev machine. Set
//! `MARIADB_URL=mysql://root:pass@127.0.0.1:3306/insight_test` against a
//! throwaway MariaDB 10.3+ to exercise them.
//!
//! Coverage map vs the Definition of Done:
//! - `DoD` #1 (round-trip up + down): [`catalog_schema_end_to_end`].
//! - `DoD` #3 (probe rejects DB where a CHECK is missing): same.
//! - Sentinel-vs-NULL uniqueness behaviour: same.
//! - CHECK enforcement on `lock_reason` / scope-shape / `metric_key` shape: same.
//!
//! The full surface lives in a single test fn to keep the live-DB test serial
//! and self-cleaning without pulling in `serial_test` or a `testcontainers`
//! dependency. Within that one fn, each invariant is checked independently and
//! failures accumulate into a single report — so one bad assertion doesn't
//! mask the next.

use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement, Value,
};
use sea_orm_migration::MigratorTrait;

use super::{Migrator, REQUIRED_CHECKS_BY_TABLE};
use crate::infra::db::check_probe;

const ENV_VAR: &str = "MARIADB_URL";
const TEST_METRIC_KEY: &str = "analytics_metrics.tasks_closed";

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

async fn drop_catalog_tables(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    // Reverse-dependency order. None of the v1 catalog tables have FKs, but
    // keeping the order obvious helps when future FK migrations land.
    for table in ["threshold_lock_audit", "metric_threshold", "metric_catalog"] {
        db.execute_unprepared(&format!("DROP TABLE IF EXISTS {table}"))
            .await?;
    }
    // Strip the matching `seaql_migrations` rows so Migrator::up reruns the
    // catalog migrations cleanly without thinking they've already applied.
    // Propagate any error (a swallowed `.ok()` here used to mask
    // `seaql_migrations` cleanup failures and produce misleading downstream
    // assertion failures when the probe saw migration rows out of sync with
    // the actual schema).
    db.execute_unprepared("DELETE FROM seaql_migrations WHERE version LIKE 'm20260522_%'")
        .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 10.3+; set MARIADB_URL to enable"]
#[allow(clippy::too_many_lines)] // one big test fn by design — see module docs
async fn catalog_schema_end_to_end() -> anyhow::Result<()> {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };

    // Setup errors are hard failures — no point continuing if migrations
    // can't even apply. Per-assertion failures further down accumulate so a
    // single live-DB run surfaces every signal.
    //
    // `None` (apply every pending migration) is robust to a future fourth
    // catalog migration landing: `drop_catalog_tables` deletes only the
    // `m20260522_*` rows from `seaql_migrations`, so the catalog migrations
    // are the only ones pending — but if we hardcoded `Some(3)` it would
    // either skip the new one or step into a later non-catalog migration
    // depending on registration order. `None` removes that landmine.
    drop_catalog_tables(&db).await?;
    Migrator::up(&db, None).await?;

    let mut failures: Vec<String> = Vec::new();

    // ── invariant 1: probe sees every required CHECK ─────────────────
    if let Err(e) = check_probe::assert_required_checks(&db).await {
        failures.push(format!("probe rejected fresh schema: {e:#}"));
    }

    // ── invariant 2: INFORMATION_SCHEMA lists each expected CHECK ────
    for (table, required) in REQUIRED_CHECKS_BY_TABLE {
        for name in *required {
            match count_check_constraint(&db, table, name).await {
                Ok(1) => {}
                Ok(n) => failures.push(format!(
                    "expected exactly one CHECK {table}.{name}, found {n}"
                )),
                Err(e) => failures.push(format!(
                    "CHECK_CONSTRAINTS lookup for {table}.{name} failed: {e}"
                )),
            }
        }
    }

    // Seed a single catalog row so threshold inserts have something to point at.
    if let Err(e) = insert_metric_catalog(&db, TEST_METRIC_KEY).await {
        failures.push(format!("seed metric_catalog insert failed: {e}"));
    }

    // ── invariant 3: sentinel-not-NULL makes the UNIQUE composite work ──
    //
    // Two `product-default` rows with the same (NULL tenant_id, metric_key,
    // scope, '', '') MUST collide. If `role_slug` / `team_id` were NULL instead
    // of '', SQL's NULL-distinct rule would let the duplicate through and we'd
    // get two `product-default` rows for the same metric — the exact failure
    // the empty-string sentinel pattern is designed to prevent.
    if insert_product_default_threshold(&db, TEST_METRIC_KEY)
        .await
        .is_err()
    {
        failures.push("first product-default insert must succeed".to_owned());
    }
    if insert_product_default_threshold(&db, TEST_METRIC_KEY)
        .await
        .is_ok()
    {
        failures.push(
            "duplicate product-default insert MUST violate \
             uq_metric_threshold_scope_target"
                .to_owned(),
        );
    }

    // ── invariant 4: lock_reason CHECK enforced ──────────────────────
    let locked_no_reason = db
        .execute_unprepared(&format!(
            "INSERT INTO metric_threshold \
             (id, tenant_id, metric_key, scope, role_slug, team_id, good, warn, is_locked) \
             VALUES (UNHEX(REPLACE(UUID(),'-','')), NULL, '{TEST_METRIC_KEY}', 'product-default', '', '', 1, 2, TRUE)"
        ))
        .await;
    if locked_no_reason.is_ok() {
        failures.push(
            "is_locked=true with NULL lock_reason MUST violate \
             chk_metric_threshold_lock_reason_when_locked"
                .to_owned(),
        );
    }

    // ── invariant 5: scope-shape CHECK enforced ──────────────────────
    let bad_role_shape = db
        .execute_unprepared(&format!(
            "INSERT INTO metric_threshold \
             (id, tenant_id, metric_key, scope, role_slug, team_id, good, warn) \
             VALUES (UNHEX(REPLACE(UUID(),'-','')), UNHEX(REPLACE(UUID(),'-','')), '{TEST_METRIC_KEY}', 'role', '', '', 1, 2)"
        ))
        .await;
    if bad_role_shape.is_ok() {
        failures.push(
            "scope=role with empty role_slug MUST violate \
             chk_metric_threshold_role_slug_shape"
                .to_owned(),
        );
    }

    // ── invariant 6: metric_key shape CHECK enforced ─────────────────
    //
    // The previous version of this test never proved the regex was doing
    // anything — only that it parsed. These two inserts close that gap:
    // (a) lowercase + dot but missing the dot at all → must fail;
    // (b) uppercase characters (regex requires `[a-z]`) → must fail.
    let bad_shape_no_dot = db
        .execute_unprepared(
            "INSERT INTO metric_catalog \
             (id, tenant_id, metric_key, label, higher_is_better, is_member_scale, source_tags, is_enabled) \
             VALUES (UNHEX(REPLACE(UUID(),'-','')), NULL, 'no_dot_here', 'X', TRUE, FALSE, JSON_ARRAY('jira'), TRUE)",
        )
        .await;
    if bad_shape_no_dot.is_ok() {
        failures.push(
            "metric_key without a dot MUST violate chk_metric_catalog_metric_key_shape"
                .to_owned(),
        );
    }
    let bad_shape_uppercase = db
        .execute_unprepared(
            "INSERT INTO metric_catalog \
             (id, tenant_id, metric_key, label, higher_is_better, is_member_scale, source_tags, is_enabled) \
             VALUES (UNHEX(REPLACE(UUID(),'-','')), NULL, 'Analytics.Tasks', 'X', TRUE, FALSE, JSON_ARRAY('jira'), TRUE)",
        )
        .await;
    if bad_shape_uppercase.is_ok() {
        failures.push(
            "uppercase metric_key MUST violate chk_metric_catalog_metric_key_shape \
             (regex requires lowercase snake_case both sides)"
                .to_owned(),
        );
    }
    // Pin the positive case for a single-character segment. The regex
    // `^[a-z][a-z0-9_]*[.][a-z][a-z0-9_]*$` accepts segments of length ≥ 1
    // (the `*` quantifier), so `a.b` is valid. This is intentional —
    // ClickHouse permits single-character identifiers and DESIGN §3.7 line
    // 206 only specifies `table_name.column_name` form, not a minimum
    // segment length. If a future revision tightens this to ≥ 2 chars
    // (`*` → `+`), this test will start failing — flip it then.
    let good_short_segments = db
        .execute_unprepared(
            "INSERT INTO metric_catalog \
             (id, tenant_id, metric_key, label, higher_is_better, is_member_scale, source_tags, is_enabled) \
             VALUES (UNHEX(REPLACE(UUID(),'-','')), NULL, 'a.b', 'Single-char segments', TRUE, FALSE, JSON_ARRAY('jira'), TRUE)",
        )
        .await;
    if let Err(e) = good_short_segments {
        failures.push(format!(
            "single-char segments like `a.b` MUST be accepted by \
             chk_metric_catalog_metric_key_shape; got: {e}"
        ));
    }

    // ── invariant 7: probe rejects a DB where a CHECK has been dropped ──
    db.execute_unprepared(
        "ALTER TABLE metric_catalog DROP CONSTRAINT chk_metric_catalog_tenant_id_null",
    )
    .await?;
    match check_probe::assert_required_checks(&db).await {
        Ok(()) => failures
            .push("probe MUST refuse to start when a required CHECK is missing".to_owned()),
        Err(e) => {
            let msg = format!("{e:#}");
            if !msg.contains("chk_metric_catalog_tenant_id_null") {
                failures.push(format!(
                    "probe error must name the missing CHECK; got: {msg}"
                ));
            }
        }
    }

    // ── invariant 8: drop-and-re-up round-trip ───────────────────────
    //
    // Migrations are forward-only (`down()` returns an error in every
    // catalog migration on purpose), so this isn't a `Migrator::down →
    // Migrator::up` round-trip. We drop the catalog tables via raw DDL,
    // strip the matching `seaql_migrations` rows, and re-run `up`. Catches
    // non-idempotency in the `up()` bodies (e.g., a CHECK whose name no
    // longer drops cleanly with the table, leaving a phantom entry that
    // makes the second up() fail).
    if let Err(e) = drop_catalog_tables(&db).await {
        failures.push(format!("teardown after probe test failed: {e}"));
    }
    if let Err(e) = Migrator::up(&db, None).await {
        failures.push(format!("second-time migration up (round-trip) failed: {e}"));
    }
    if let Err(e) = check_probe::assert_required_checks(&db).await {
        failures.push(format!("probe rejected schema after round-trip: {e:#}"));
    }

    // Final cleanup is best-effort — if it fails we still want to surface the
    // accumulated assertion failures.
    let cleanup = drop_catalog_tables(&db).await;

    anyhow::ensure!(
        failures.is_empty(),
        "live-DB invariants failed:\n  - {}",
        failures.join("\n  - ")
    );
    cleanup?;
    Ok(())
}

async fn count_check_constraint(
    db: &DatabaseConnection,
    table: &str,
    name: &str,
) -> anyhow::Result<i64> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT COUNT(*) AS c FROM INFORMATION_SCHEMA.CHECK_CONSTRAINTS \
             WHERE CONSTRAINT_SCHEMA = DATABASE() \
               AND TABLE_NAME = ? AND CONSTRAINT_NAME = ?",
            [Value::from(table), Value::from(name)],
        ))
        .await?
        .ok_or_else(|| anyhow::anyhow!("no row from CHECK_CONSTRAINTS query"))?;
    Ok(row.try_get::<i64>("", "c")?)
}

async fn insert_metric_catalog(
    db: &DatabaseConnection,
    metric_key: &str,
) -> Result<(), sea_orm::DbErr> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO metric_catalog \
         (id, tenant_id, metric_key, label, higher_is_better, is_member_scale, source_tags, is_enabled) \
         VALUES (UNHEX(REPLACE(UUID(),'-','')), NULL, ?, 'Tasks Closed', TRUE, FALSE, JSON_ARRAY('jira'), TRUE)",
        [Value::from(metric_key)],
    ))
    .await?;
    Ok(())
}

async fn insert_product_default_threshold(
    db: &DatabaseConnection,
    metric_key: &str,
) -> Result<(), sea_orm::DbErr> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO metric_threshold \
         (id, tenant_id, metric_key, scope, role_slug, team_id, good, warn) \
         VALUES (UNHEX(REPLACE(UUID(),'-','')), NULL, ?, 'product-default', '', '', 20, 10)",
        [Value::from(metric_key)],
    ))
    .await?;
    Ok(())
}
