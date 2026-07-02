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
use uuid::Uuid;

use crate::domain::catalog::resolver::ThresholdResolver;

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

/// Insert a GLOBAL (`tenant_id IS NULL`) catalog row with a unique key and
/// return its id. Catalog is global-only in v1 (the resolver reads
/// `WHERE c.tenant_id IS NULL`), so per-test isolation is by unique key, not
/// tenant — the row is inert for other tenants until one gets a threshold.
async fn insert_global_catalog_metric(
    db: &DatabaseConnection,
    metric_key: &str,
) -> Result<Uuid, sea_orm::DbErr> {
    let id = Uuid::now_v7();
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO metric_catalog \
            (id, tenant_id, metric_key, label, higher_is_better, source_tags, is_enabled) \
         VALUES (?, NULL, ?, 'walk-drop regression metric', TRUE, '[]', TRUE)",
        [
            Value::Bytes(Some(Box::new(id.as_bytes().to_vec()))),
            Value::from(metric_key),
        ],
    ))
    .await?;
    Ok(id)
}

/// Any existing `metrics` row id — the junction's `metrics_id` is a FK, but the
/// resolver only uses it as an opaque `query_id` grouping key.
async fn any_metrics_row_id(db: &DatabaseConnection) -> Result<Uuid, sea_orm::DbErr> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT id FROM metrics LIMIT 1",
            [],
        ))
        .await?
        .ok_or_else(|| sea_orm::DbErr::Custom("seed has no metrics rows".into()))?;
    let bytes: Vec<u8> = row.try_get("", "id")?;
    Uuid::from_slice(&bytes).map_err(|e| sea_orm::DbErr::Custom(format!("id decode: {e}")))
}

/// Link a catalog row into `metric_query_catalog` under an existing query.
async fn insert_query_catalog_link(
    db: &DatabaseConnection,
    metrics_id: Uuid,
    catalog_id: Uuid,
) -> Result<(), sea_orm::DbErr> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO metric_query_catalog (id, metrics_id, metric_catalog_id) VALUES (?, ?, ?)",
        [
            Value::Bytes(Some(Box::new(Uuid::now_v7().as_bytes().to_vec()))),
            Value::Bytes(Some(Box::new(metrics_id.as_bytes().to_vec()))),
            Value::Bytes(Some(Box::new(catalog_id.as_bytes().to_vec()))),
        ],
    ))
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn response_link_map_omits_metrics_dropped_by_walk_all() -> anyhow::Result<()> {
    // Regression (ADR-003 `surfaced_ids` filter): a metric that is enabled in
    // the catalog but has NO threshold candidate for the caller MUST be dropped
    // by `walk_all` AND omitted from `response.links`. A naive global
    // `is_enabled = TRUE` link query would leave a phantom reference.
    //
    // We prove this with a fully ISOLATED, purely-ADDITIVE fixture — no shared
    // seed row is read or mutated, so the test is parallel-safe and never
    // resets the DB. A GLOBAL catalog row (v1 requires `tenant_id IS NULL`)
    // with a UNIQUE key is linked into the junction and given a threshold for
    // ONE tenant only. Resolving as THAT tenant surfaces it (+ its link);
    // resolving as ANY OTHER tenant has no candidate, so the metric drops and
    // the surfaced-ids filter keeps its link out — the exact invariant, driven
    // entirely by tenant scoping rather than by deleting anything.
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };

    let metric_key = format!("ztest_{}.walkdrop", Uuid::now_v7().simple());
    let catalog_id = insert_global_catalog_metric(&db, &metric_key).await?;
    let query_id = any_metrics_row_id(&db).await?;
    insert_query_catalog_link(&db, query_id, catalog_id).await?;

    // Only `tenant_with` gets a threshold; `tenant_without` is a distinct,
    // never-seen tenant, so the two resolves cannot intersect.
    let tenant_with = Uuid::now_v7();
    let tenant_without = Uuid::now_v7();
    insert_tenant_threshold(&db, tenant_with, &metric_key, 25.0, 12.0, false, None).await?;

    let resolver = ThresholdResolver::new(db.clone());

    // Tenant WITH a threshold: the metric surfaces AND appears in the link map.
    let present = resolver.resolve(tenant_with, "", "").await?;
    assert!(
        present.metrics.iter().any(|m| m.id == catalog_id),
        "metric with a tenant-scope threshold MUST surface"
    );
    assert!(
        present
            .links
            .iter()
            .any(|l| l.catalog_metric_ids.contains(&catalog_id)),
        "a surfaced metric MUST appear in at least one link entry"
    );

    // Tenant WITHOUT any threshold: no candidate -> `walk_all` drops the metric,
    // and the surfaced-ids filter drops its junction link (no phantom).
    let absent = resolver.resolve(tenant_without, "", "").await?;
    assert!(
        !absent.metrics.iter().any(|m| m.id == catalog_id),
        "walk_all MUST drop a metric with no threshold candidate"
    );
    for link in &absent.links {
        assert!(
            !link.catalog_metric_ids.contains(&catalog_id),
            "link map MUST NOT reference a catalog_id that walk_all dropped; \
             query_id={} ids={:?}",
            link.query_id,
            link.catalog_metric_ids,
        );
    }
    Ok(())
}
