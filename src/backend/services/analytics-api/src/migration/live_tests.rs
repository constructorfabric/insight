//! Live MariaDB integration tests for the metric-catalog schema (Refs #519).
//!
//! All tests in this module are `#[ignore]`d by default and skip silently
//! when the `INTEGRATION_TESTS_MARIADB_URL` env var is unset, so `cargo test` and even
//! `cargo test -- --ignored` are green on a stock dev machine. Set
//! `INTEGRATION_TESTS_MARIADB_URL=mysql://root:pass@127.0.0.1:3306/insight_test` against a
//! throwaway MariaDB 11+ to exercise them.
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

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement, Value};
use sea_orm_migration::MigratorTrait;

use super::{Migrator, REQUIRED_CHECKS_BY_TABLE};
use crate::infra::db::{check_probe, product_default_probe};

const ENV_VAR: &str = "INTEGRATION_TESTS_MARIADB_URL";
const TEST_METRIC_KEY: &str = "analytics_metrics.tasks_closed";

/// Expected catalog row count produced by every product-default catalog
/// seed migration combined. Pinned here so the live DB test catches drift
/// between the seed `SEEDS` arrays and the rendered state without
/// re-importing the consts. Breakdown:
/// - `m20260527_000001_seed_metric_catalog`               → 69 rows
/// - `m20260601_000002_seed_claude_team_metrics_catalog`  →  3 rows
/// - `m20260603_000001_seed_crm_metric_catalog`           →  8 rows
const EXPECTED_SEED_ROW_COUNT: i64 = 80;

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
    // Reverse-dependency order. `metric_query_catalog` (ADR-001, m20260529)
    // has FKs into both `metric_catalog` and `metrics` with `ON DELETE
    // CASCADE` — drop it FIRST or the parent drops fail with MariaDB
    // error 1451.
    for table in [
        "metric_query_catalog",
        "threshold_lock_audit",
        "metric_threshold",
        "metric_catalog",
    ] {
        db.execute_unprepared(&format!("DROP TABLE IF EXISTS {table}"))
            .await?;
    }
    // Strip the matching `seaql_migrations` rows so Migrator::up reruns the
    // catalog migrations cleanly without thinking they've already applied.
    // Propagate any error (a swallowed `.ok()` here used to mask
    // `seaql_migrations` cleanup failures and produce misleading downstream
    // assertion failures when the probe saw migration rows out of sync with
    // the actual schema).
    //
    // Covers the schema migrations (`m20260522_*`: catalog / threshold /
    // audit), the seed migrations (`m20260527_*` Refs #523, the Claude
    // Team catalog seed `m20260601_000002`, the CRM catalog seed +
    // link `m20260603_*`), and the junction-table migration
    // (`m20260529_*`: ADR-001 `metric_query_catalog`). Any future
    // catalog-domain migration must extend this OR pattern — otherwise
    // a partial replay leaves catalog tables empty (schema reapplied,
    // seed skipped) and downstream invariants break in confusing ways.
    //
    // `m20260601_000001` is intentionally NOT listed — it modifies
    // `analytics.metrics.query_ref` rather than catalog tables, and
    // catalog-tables cleanup doesn't undo that side-effect.
    db.execute_unprepared(
        "DELETE FROM seaql_migrations \
         WHERE version LIKE 'm20260522_%' \
            OR version LIKE 'm20260527_%' \
            OR version LIKE 'm20260529_%' \
            OR version = 'm20260601_000002_seed_claude_team_metrics_catalog' \
            OR version LIKE 'm20260603_%'",
    )
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
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
            "metric_key without a dot MUST violate chk_metric_catalog_metric_key_shape".to_owned(),
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

    // ── invariant 7: audit-scope CHECKs enforced ─────────────────────
    //
    // The two scope columns on `threshold_lock_audit` are domain-constrained
    // by CHECK so the forensic record cannot collect garbage strings (a
    // direct SQL insert, emitter bug, or ORM regression would otherwise
    // poison `GROUP BY attempted_scope` histograms). A `bypass_attempt`
    // event with `attempted_scope = 'not-a-scope'` MUST be rejected; a
    // canonical scope value MUST succeed.
    let bad_attempted_scope = db
        .execute_unprepared(&format!(
            "INSERT INTO threshold_lock_audit \
             (id, event_type, actor_subject, tenant_id, metric_key, attempted_scope, event_at) \
             VALUES (UNHEX(REPLACE(UUID(),'-','')), 'bypass_attempt', 'svc-test', \
                     UNHEX(REPLACE(UUID(),'-','')), '{TEST_METRIC_KEY}', 'not-a-scope', NOW())"
        ))
        .await;
    if bad_attempted_scope.is_ok() {
        failures.push(
            "non-canonical attempted_scope MUST violate \
             chk_threshold_lock_audit_attempted_scope"
                .to_owned(),
        );
    }
    let bad_blocking_scope = db
        .execute_unprepared(&format!(
            "INSERT INTO threshold_lock_audit \
             (id, event_type, actor_subject, tenant_id, metric_key, blocking_scope, event_at) \
             VALUES (UNHEX(REPLACE(UUID(),'-','')), 'bypass_attempt', 'svc-test', \
                     UNHEX(REPLACE(UUID(),'-','')), '{TEST_METRIC_KEY}', 'wrong-value', NOW())"
        ))
        .await;
    if bad_blocking_scope.is_ok() {
        failures.push(
            "non-canonical blocking_scope MUST violate \
             chk_threshold_lock_audit_blocking_scope"
                .to_owned(),
        );
    }
    // Positive case: a canonical scope succeeds; NULL also succeeds.
    let good_audit_row = db
        .execute_unprepared(&format!(
            "INSERT INTO threshold_lock_audit \
             (id, event_type, actor_subject, tenant_id, metric_key, attempted_scope, blocking_scope, event_at) \
             VALUES (UNHEX(REPLACE(UUID(),'-','')), 'bypass_attempt', 'svc-test', \
                     UNHEX(REPLACE(UUID(),'-','')), '{TEST_METRIC_KEY}', 'tenant', 'product-default', NOW())"
        ))
        .await;
    if let Err(e) = good_audit_row {
        failures.push(format!(
            "canonical attempted_scope='tenant' / blocking_scope='product-default' \
             MUST be accepted by the audit-scope CHECKs; got: {e}"
        ));
    }

    // ── invariant 8: probe rejects a DB where a CHECK has been dropped ──
    db.execute_unprepared(
        "ALTER TABLE metric_catalog DROP CONSTRAINT chk_metric_catalog_tenant_id_null",
    )
    .await?;
    match check_probe::assert_required_checks(&db).await {
        Ok(()) => {
            failures.push("probe MUST refuse to start when a required CHECK is missing".to_owned());
        }
        Err(e) => {
            let msg = format!("{e:#}");
            if !msg.contains("chk_metric_catalog_tenant_id_null") {
                failures.push(format!(
                    "probe error must name the missing CHECK; got: {msg}"
                ));
            }
        }
    }

    // ── invariant 9: drop-and-re-up round-trip ───────────────────────
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

/// End-to-end live test for the seed migration + product-default probe
/// (Refs #523). Mirrors the `catalog_schema_end_to_end` skip-when-unset
/// pattern. Asserts:
///
/// 1. Fresh `Migrator::up` lands `EXPECTED_SEED_ROW_COUNT` catalog rows
///    (all `tenant_id IS NULL`).
/// 2. Same count of `product-default` `metric_threshold` rows (all
///    `tenant_id IS NULL`, `role_slug = ''`, `team_id = ''`).
/// 3. Every catalog row's `metric_key` matches a threshold row's
///    `metric_key` (1:1 pairing — no orphans either direction).
/// 4. `product_default_probe::assert_product_default_present` returns
///    `Ok` against the freshly seeded DB.
/// 5. Deleting one `product-default` row → probe returns `Err` whose
///    message names the orphaned `metric_key` (so an operator can find
///    and fix it from the log line).
#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
#[allow(clippy::too_many_lines)]
async fn seed_migration_and_product_default_probe_end_to_end() -> anyhow::Result<()> {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };

    drop_catalog_tables(&db).await?;
    Migrator::up(&db, None).await?;

    let mut failures: Vec<String> = Vec::new();

    // ── invariant 1: catalog row count + tenant_id IS NULL ───────────
    let catalog_count = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT COUNT(*) AS c FROM metric_catalog WHERE tenant_id IS NULL",
            [],
        ))
        .await?
        .ok_or_else(|| anyhow::anyhow!("no row from metric_catalog COUNT"))?
        .try_get::<i64>("", "c")?;
    if catalog_count != EXPECTED_SEED_ROW_COUNT {
        failures.push(format!(
            "metric_catalog expected {EXPECTED_SEED_ROW_COUNT} NULL-tenant rows, got {catalog_count}"
        ));
    }

    // Defense in depth: also confirm no non-null tenant rows landed
    // (the v1 CHECK should prevent it, but pin behaviour).
    let nonnull_tenant_count = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT COUNT(*) AS c FROM metric_catalog WHERE tenant_id IS NOT NULL",
            [],
        ))
        .await?
        .ok_or_else(|| anyhow::anyhow!("no row from non-null tenant COUNT"))?
        .try_get::<i64>("", "c")?;
    if nonnull_tenant_count != 0 {
        failures.push(format!(
            "seed must only insert tenant_id IS NULL rows; \
             found {nonnull_tenant_count} with non-null tenant"
        ));
    }

    // ── invariant 2: product-default threshold row count ─────────────
    let threshold_count = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT COUNT(*) AS c FROM metric_threshold \
             WHERE scope = 'product-default' AND tenant_id IS NULL \
               AND role_slug = '' AND team_id = ''",
            [],
        ))
        .await?
        .ok_or_else(|| anyhow::anyhow!("no row from threshold COUNT"))?
        .try_get::<i64>("", "c")?;
    if threshold_count != EXPECTED_SEED_ROW_COUNT {
        failures.push(format!(
            "metric_threshold product-default expected {EXPECTED_SEED_ROW_COUNT} rows, \
             got {threshold_count}"
        ));
    }

    // ── invariant 3: 1:1 pairing between catalog and product-default
    // thresholds — no orphans either direction.
    let orphan_catalog = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT COUNT(*) AS c FROM metric_catalog mc \
             LEFT JOIN metric_threshold mt \
               ON mt.metric_key = mc.metric_key \
              AND mt.scope = 'product-default' \
              AND mt.tenant_id IS NULL \
             WHERE mt.id IS NULL",
            [],
        ))
        .await?
        .ok_or_else(|| anyhow::anyhow!("no row from orphan-catalog COUNT"))?
        .try_get::<i64>("", "c")?;
    if orphan_catalog != 0 {
        failures.push(format!(
            "catalog rows missing matching product-default threshold: {orphan_catalog}"
        ));
    }

    let orphan_threshold = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT COUNT(*) AS c FROM metric_threshold mt \
             LEFT JOIN metric_catalog mc ON mc.metric_key = mt.metric_key \
             WHERE mt.scope = 'product-default' \
               AND mt.tenant_id IS NULL \
               AND mc.id IS NULL",
            [],
        ))
        .await?
        .ok_or_else(|| anyhow::anyhow!("no row from orphan-threshold COUNT"))?
        .try_get::<i64>("", "c")?;
    if orphan_threshold != 0 {
        failures.push(format!(
            "product-default thresholds with no matching catalog row: {orphan_threshold}"
        ));
    }

    // ── invariant 4: probe is green against the freshly seeded DB ────
    if let Err(e) = product_default_probe::assert_product_default_present(&db).await {
        failures.push(format!(
            "product_default_probe MUST pass against freshly seeded DB; got: {e:#}"
        ));
    }

    // ── invariant 5: probe fails with the offending metric_key in
    // the message after we DELETE one product-default row. Pick an
    // `ic_kpis.*` row — those are the ones the FE will hit first via
    // the IC KPI strip, so we want the probe to catch a manual
    // recovery DELETE there too.
    let removed_metric_key = "ic_kpis.tasks_closed";
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "DELETE FROM metric_threshold \
         WHERE scope = 'product-default' AND tenant_id IS NULL AND metric_key = ?",
        [Value::from(removed_metric_key)],
    ))
    .await?;
    match product_default_probe::assert_product_default_present(&db).await {
        Ok(()) => failures.push(
            "probe MUST refuse to start when a product-default row is missing \
             for an enabled metric"
                .to_owned(),
        ),
        Err(e) => {
            let msg = format!("{e:#}");
            if !msg.contains(removed_metric_key) {
                failures.push(format!(
                    "probe error must name the missing metric_key {removed_metric_key:?}; \
                     got: {msg}"
                ));
            }
        }
    }

    let cleanup = drop_catalog_tables(&db).await.map_err(anyhow::Error::from);

    anyhow::ensure!(
        failures.is_empty(),
        "live test failures:\n  - {}",
        failures.join("\n  - ")
    );
    cleanup?;
    Ok(())
}
