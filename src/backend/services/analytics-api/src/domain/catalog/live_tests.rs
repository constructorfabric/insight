//! Live MariaDB integration tests for the threshold-resolver (Refs #524).
//!
//! All tests are `#[ignore]`d by default and skip silently when
//! `INTEGRATION_TESTS_MARIADB_URL` is unset, so `cargo test` and `cargo test
//! -- --ignored` stay green on a stock dev machine. Set
//! `INTEGRATION_TESTS_MARIADB_URL=mysql://root:pass@127.0.0.1:3306/insight_test`
//! against a throwaway MariaDB 11+ to exercise them.
//!
//! ## Why the `INTEGRATION_TESTS_` prefix
//!
//! The tests INSERT into `metric_threshold` to set up tenant-scope overlays.
//! A plain `MARIADB_URL` would collide with the same name commonly exported
//! in a dev shell (compose stacks, docker-machine helpers, in-cluster
//! service discovery) — running `cargo test -- --ignored` with that set
//! would mutate whatever DB it pointed at. The `INTEGRATION_TESTS_` prefix
//! forces the operator to opt in for THIS test suite specifically, so the
//! mutating setup runs only with full knowledge of what it triggers. Same
//! convention as `domain/schema_validator/live_tests.rs`.
//!
//! Coverage map vs the issue's Definition of Done:
//! - `DoD` #4 (cache-hit short-circuit, 0 DB queries on hit) — unit tested in
//!   `reader.rs::cache_hit_short_circuits_resolver`. Counting in-memory cache
//!   makes the assertion air-tight; reaching for a real DB here would only
//!   re-test SeaORM.
//! - `DoD` #5 (locked broader-scope row halts walk; correct `resolved_from`;
//!   `bounded_by_lock = true`): [`tenant_lock_shadows_team_override`].
//! - `DoD` #6 (multi-replica invalidation NFR) — covered in `infra/cache/live_tests.rs`
//!   against a real Redis. The resolver doesn't span replicas; the cache does.

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement, Value};
use sea_orm_migration::MigratorTrait;
use uuid::Uuid;

use crate::domain::catalog::resolver::ThresholdResolver;
use crate::migration::Migrator;

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

/// Wipe the catalog tables + the matching `seaql_migrations` rows so
/// `Migrator::up` reruns the schema + seed migrations cleanly. Tolerates
/// the first-run case where `seaql_migrations` itself doesn't exist yet —
/// the table is created the first time `Migrator::up` runs.
async fn reset_catalog(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    // Drop children before parents — `metric_query_catalog` has FKs into
    // both `metric_catalog` and `metrics`, so dropping `metric_catalog`
    // first triggers MariaDB error 1451. Order also matters for
    // `threshold_lock_audit` / `metric_threshold` (audit references
    // threshold rows by id but with `ON DELETE CASCADE`, so the order
    // doesn't strictly matter there — kept for symmetry).
    for table in [
        "metric_query_catalog",
        "threshold_lock_audit",
        "metric_threshold",
        "metric_catalog",
    ] {
        db.execute_unprepared(&format!("DROP TABLE IF EXISTS {table}"))
            .await?;
    }
    // First-run-friendly: ignore "table doesn't exist" so a brand-new test
    // database doesn't fail the test before Migrator::up gets to create
    // seaql_migrations. Includes `m20260529_%` so the junction-table
    // migration re-runs on every reset (otherwise the table is dropped
    // above but the seaql_migrations row remains and Migrator::up skips
    // the create).
    let _ = db
        .execute_unprepared(
            "DELETE FROM seaql_migrations \
             WHERE version LIKE 'm20260522_%' \
                OR version LIKE 'm20260527_%' \
                OR version LIKE 'm20260529_%'",
        )
        .await;
    Ok(())
}

/// Insert a tenant-scope threshold row for an existing seeded metric.
async fn insert_tenant_threshold(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    metric_key: &str,
    good: f64,
    warn: f64,
    is_locked: bool,
    lock_reason: Option<&str>,
) -> Result<(), sea_orm::DbErr> {
    let id = Uuid::now_v7();
    let sql = "\
        INSERT INTO metric_threshold \
            (id, tenant_id, metric_key, scope, role_slug, team_id, good, warn, is_locked, lock_reason) \
        VALUES (?, ?, ?, 'tenant', '', '', ?, ?, ?, ?)";
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [
            Value::Bytes(Some(Box::new(id.as_bytes().to_vec()))),
            Value::Bytes(Some(Box::new(tenant_id.as_bytes().to_vec()))),
            Value::from(metric_key),
            Value::from(good),
            Value::from(warn),
            Value::from(is_locked),
            match lock_reason {
                Some(r) => Value::from(r),
                None => Value::String(None),
            },
        ],
    ))
    .await?;
    Ok(())
}

/// Look up the catalog `id` for a `metric_key`. Used by tests to pin
/// assertions on a specific metric: `metric_key` is now surfaced on the wire
/// per ADR-002, but tests historically pinned by `id` and that contract is
/// load-bearing — `id` is the stable lookup key consumers MUST use.
async fn metric_id_for_key(
    db: &DatabaseConnection,
    metric_key: &str,
) -> Result<Uuid, sea_orm::DbErr> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT id FROM metric_catalog WHERE metric_key = ?",
            [Value::from(metric_key)],
        ))
        .await?
        .ok_or_else(|| {
            sea_orm::DbErr::Custom(format!("metric_key {metric_key} not found in seed"))
        })?;
    let bytes: Vec<u8> = row.try_get("", "id")?;
    Uuid::from_slice(&bytes).map_err(|e| sea_orm::DbErr::Custom(format!("id decode: {e}")))
}

/// Insert a `team+role`-scope threshold (the most-specific narrower row).
async fn insert_team_role_threshold(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    metric_key: &str,
    role_slug: &str,
    team_id: &str,
    good: f64,
    warn: f64,
) -> Result<(), sea_orm::DbErr> {
    let id = Uuid::now_v7();
    let sql = "\
        INSERT INTO metric_threshold \
            (id, tenant_id, metric_key, scope, role_slug, team_id, good, warn, is_locked) \
        VALUES (?, ?, ?, 'team+role', ?, ?, ?, ?, FALSE)";
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [
            Value::Bytes(Some(Box::new(id.as_bytes().to_vec()))),
            Value::Bytes(Some(Box::new(tenant_id.as_bytes().to_vec()))),
            Value::from(metric_key),
            Value::from(role_slug),
            Value::from(team_id),
            Value::from(good),
            Value::from(warn),
        ],
    ))
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn product_default_wins_when_no_tenant_overlay() -> anyhow::Result<()> {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    reset_catalog(&db).await?;
    Migrator::up(&db, None).await?;

    let resolver = ThresholdResolver::new(db.clone());
    let tenant_id = Uuid::now_v7();
    let response = resolver.resolve(tenant_id, "", "").await?;

    assert!(
        !response.metrics.is_empty(),
        "seed migration must produce at least one enabled metric"
    );
    for m in &response.metrics {
        assert_eq!(
            m.thresholds.resolved_from, "product-default",
            "no tenant overlay → every metric must resolve at product-default"
        );
        assert!(
            !m.thresholds.bounded_by_lock,
            "no locks present → bounded_by_lock must be false"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn tenant_overlay_wins_when_no_lock() -> anyhow::Result<()> {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    reset_catalog(&db).await?;
    Migrator::up(&db, None).await?;

    let tenant_id = Uuid::now_v7();
    let metric_key = "ic_kpis.tasks_closed"; // present in the seed
    // Use values nowhere in the seed so a `.find` cannot match a sibling
    // metric's product-default row. Both `good` and `warn` are intentionally
    // far from any seeded value; the assertion below pins the resolved row
    // by metric `id`, not by these values.
    insert_tenant_threshold(&db, tenant_id, metric_key, 12_345.0, 6_789.0, false, None).await?;
    let target_id = metric_id_for_key(&db, metric_key).await?;

    let resolver = ThresholdResolver::new(db.clone());
    let response = resolver.resolve(tenant_id, "", "").await?;

    let m = response
        .metrics
        .iter()
        .find(|m| m.id == target_id)
        .unwrap_or_else(|| panic!("must find metric {metric_key} in response"));
    assert_eq!(
        m.thresholds.resolved_from, "tenant",
        "tenant overlay MUST win when no lock"
    );
    assert!(!m.thresholds.bounded_by_lock);
    assert!((m.thresholds.good - 12_345.0).abs() < f64::EPSILON);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn tenant_lock_shadows_team_override() -> anyhow::Result<()> {
    // `DoD` #5: a tenant-scope locked row MUST shadow a narrower team+role
    // override. The walk halts on the lock; `resolved_from = "tenant"`;
    // `bounded_by_lock = true`.
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    reset_catalog(&db).await?;
    Migrator::up(&db, None).await?;

    let tenant_id = Uuid::now_v7();
    let metric_key = "ic_kpis.tasks_closed";
    let role_slug = "eng_ic";
    let team_id_str = "alpha";

    // tenant-scope row, locked. Values chosen far from any seed so the
    // assertion can also pin the exact winning numbers (the row identity
    // is verified by `id`, not by `good`).
    insert_tenant_threshold(
        &db,
        tenant_id,
        metric_key,
        11_111.0,
        2_222.0,
        true,
        Some("TICKET-7421: compliance pin"),
    )
    .await?;
    // team+role row that would win without the lock.
    insert_team_role_threshold(
        &db,
        tenant_id,
        metric_key,
        role_slug,
        team_id_str,
        99_999.0,
        88_888.0,
    )
    .await?;
    let target_id = metric_id_for_key(&db, metric_key).await?;

    let resolver = ThresholdResolver::new(db.clone());
    let response = resolver.resolve(tenant_id, role_slug, team_id_str).await?;

    let m = response
        .metrics
        .iter()
        .find(|m| m.id == target_id)
        .unwrap_or_else(|| panic!("must find metric {metric_key} in response"));
    assert_eq!(
        m.thresholds.resolved_from, "tenant",
        "locked tenant row MUST win over narrower team+role"
    );
    assert!(
        m.thresholds.bounded_by_lock,
        "bounded_by_lock MUST be true when a broader lock shadows a narrower candidate"
    );
    assert!(
        (m.thresholds.good - 11_111.0).abs() < f64::EPSILON,
        "winning row MUST be the locked tenant row, not the team+role override"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn response_includes_metric_key_for_fe_bridge() -> anyhow::Result<()> {
    // ADR-002: `metric_key` IS on the wire as the transitional FE-bridge
    // identifier. Every metric in the response must carry a non-empty key
    // so the FE can align its compile-in `BULLET_DEFS` constants to wire
    // rows during the catalog-hydration release.
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    reset_catalog(&db).await?;
    Migrator::up(&db, None).await?;

    let resolver = ThresholdResolver::new(db.clone());
    let response = resolver.resolve(Uuid::now_v7(), "", "").await?;
    assert!(
        !response.metrics.is_empty(),
        "seed migration must produce at least one enabled metric"
    );
    for m in &response.metrics {
        assert!(
            !m.metric_key.is_empty(),
            "every metric row must carry a metric_key per ADR-002; id={}",
            m.id
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn response_includes_link_map_from_metric_query_catalog() -> anyhow::Result<()> {
    // ADR-003: the `metric_query_catalog` M:N mapping is surfaced on the
    // top-level `links` field. The seed migration backfills 9 query→prefix
    // entries; we assert the link map is non-empty and well-formed, and
    // that every `catalog_metric_ids` UUID corresponds to a real catalog
    // row in the same response (no phantom references).
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    reset_catalog(&db).await?;
    Migrator::up(&db, None).await?;

    let resolver = ThresholdResolver::new(db.clone());
    let response = resolver.resolve(Uuid::now_v7(), "", "").await?;

    assert!(
        !response.links.is_empty(),
        "metric_query_catalog seed expects at least one (query, catalog) link"
    );
    let known_ids: std::collections::HashSet<Uuid> =
        response.metrics.iter().map(|m| m.id).collect();
    for link in &response.links {
        assert!(
            !link.catalog_metric_ids.is_empty(),
            "every link row groups at least one catalog id; query_id={}",
            link.query_id
        );
        // `fetch_links` filters by the SURFACED metric ids
        // (`resolve::surfaced_ids`), not by a global `is_enabled = TRUE`
        // join — so every link id MUST resolve back to a row in
        // `response.metrics` by construction. The
        // `response_link_map_omits_metrics_dropped_by_walk_all` test
        // below exercises the failure mode this guarantee closes.
        for cid in &link.catalog_metric_ids {
            assert!(
                known_ids.contains(cid),
                "link references catalog_id={cid} not present in metrics[]; \
                 surfaced-ids filter regression"
            );
        }
        // The grouping logic sorts catalog ids ascending at the DB layer.
        // A wire-stable order makes the response byte-stable for caches
        // and diff tooling.
        let mut sorted = link.catalog_metric_ids.clone();
        sorted.sort();
        assert_eq!(
            sorted, link.catalog_metric_ids,
            "catalog_metric_ids must be ascending for byte-stable wire"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn response_link_map_omits_metrics_dropped_by_walk_all() -> anyhow::Result<()> {
    // Regression: in a healthy v1 seed every metric has a `product-default`
    // threshold so `walk_one` never returns None and the surfacing /
    // link-map consistency check is trivially satisfied. This test
    // engineers the pathological case explicitly: delete the
    // `product-default` threshold for one catalog row, then assert the
    // resolver drops the metric AND its junction links are absent from
    // `response.links`.
    //
    // Without the `surfaced_ids` filter in `fetch_links` (the bug that
    // shipped in the first cut of ADR-003 and was fixed in the same
    // branch), the dropped metric's catalog_id would still appear in
    // some link entry — a phantom reference.
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    reset_catalog(&db).await?;
    Migrator::up(&db, None).await?;

    // Pick a seeded metric whose storage prefix is known to be in the
    // junction map (`ic_kpis.tasks_closed` is wired to METRIC_REGISTRY.IC_KPIS
    // by the link migration).
    let target_metric_key = "ic_kpis.tasks_closed";
    let target_id = metric_id_for_key(&db, target_metric_key).await?;

    // Sanity: before mutation, the resolver surfaces this metric AND its
    // link entry references its id.
    let resolver = ThresholdResolver::new(db.clone());
    let before = resolver.resolve(Uuid::now_v7(), "", "").await?;
    assert!(
        before.metrics.iter().any(|m| m.id == target_id),
        "pre-condition: target metric must be present in the healthy response"
    );
    assert!(
        before
            .links
            .iter()
            .any(|l| l.catalog_metric_ids.contains(&target_id)),
        "pre-condition: target metric must appear in at least one link entry"
    );

    // Delete the `product-default` threshold for the target metric_key so
    // `walk_one` will see no threshold candidate and skip the row. The
    // catalog row itself stays `is_enabled = TRUE` — that's what makes
    // this a regression test for the global-JOIN bug: a global
    // `is_enabled = TRUE` link query would still surface the row.
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "DELETE FROM metric_threshold \
         WHERE scope = 'product-default' AND metric_key = ?",
        [Value::from(target_metric_key)],
    ))
    .await?;

    let after = resolver.resolve(Uuid::now_v7(), "", "").await?;

    assert!(
        !after.metrics.iter().any(|m| m.id == target_id),
        "walk_all MUST drop the metric whose product-default row was deleted"
    );
    for link in &after.links {
        assert!(
            !link.catalog_metric_ids.contains(&target_id),
            "link map MUST NOT reference a catalog_id that walk_all dropped; \
             query_id={} ids={:?}",
            link.query_id,
            link.catalog_metric_ids,
        );
    }
    Ok(())
}
