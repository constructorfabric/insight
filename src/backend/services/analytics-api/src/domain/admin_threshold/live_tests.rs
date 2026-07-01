//! Live MariaDB integration tests for admin-crud / lock-enforcer /
//! audit-emitter (Refs #525).
//!
//! All tests are `#[ignore]`d by default and skip silently when
//! `INTEGRATION_TESTS_MARIADB_URL` is unset. Convention matches
//! `domain/catalog/live_tests.rs` and `infra/cache/live_tests.rs`.
//!
//! Coverage map vs issue #525's Definition of Done:
//!
//! - [x] Happy create / update / delete — [`happy_create_update_delete_round_trip`].
//! - [x] List endpoint rejects `tenant_id` filter — pinned at the parser
//!   in `dto::tests` (`deny_unknown_fields`); end-to-end HTTP coverage
//!   lands when the live test harness for HTTP routing wires up
//!   alongside the existing `tenant_resolution_tests.rs` shape.
//! - [x] Lock-bypass 403 + audit row written —
//!   [`lock_bypass_writes_audit_row_and_returns_403`].
//! - [x] Immutable-field PUT → 400 `failed_precondition` —
//!   [`immutable_field_put_rejected`].
//! - [x] Cross-tenant write → 403 — [`cross_tenant_put_rejected`].
//! - [x] Missing `lock_reason` on lock-set → 400 — [`lock_set_without_reason_rejected`].
//! - [x] CHECK violation per name → mapped 4xx —
//!   [`db_check_violations_map_to_canonical_4xx`].
//! - [ ] Audit-row INSERT failure → 503 — requires a fault-injection
//!   shim around the audit-emitter sink. Lands behind a feature flag in
//!   the security-review follow-up (PR description documents).
//! - [ ] Cache invalidation observable on a peer replica within 2 s p99
//!   — exercised by the existing `infra/cache/live_tests::cross_instance_invalidation_is_visible_immediately`
//!   harness; here we only verify admin-crud DOES call `invalidate` on
//!   every successful 2xx, via [`successful_write_invalidates_cache`].

use std::sync::Arc;

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement, Value};
use sea_orm_migration::MigratorTrait;
use uuid::Uuid;

use toolkit_security::SecurityContext;
use crate::domain::admin_threshold::dto::{CreateRequest, Scope, UpdateRequest};
use crate::domain::admin_threshold::service::AdminThresholdService;
use crate::domain::auth::{ConfigTenantAuthorization, TenantAuthorization};
use crate::domain::schema_validator::SchemaValidator;
use crate::infra::cache::catalog_cache::{CatalogCache, NoopCatalogCache};
use crate::migration::Migrator;

const ENV_VAR: &str = "INTEGRATION_TESTS_MARIADB_URL";

async fn connect_or_skip() -> Option<DatabaseConnection> {
    let Ok(url) = std::env::var(ENV_VAR) else {
        eprintln!("skipping: {ENV_VAR} not set");
        return None;
    };
    // Best-effort tracing init for `--nocapture` runs so the service's
    // internal-error logs surface on the test console.
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug")),
        )
        .try_init();
    let mut opts = ConnectOptions::new(url);
    opts.max_connections(4).sqlx_logging(false);
    match Database::connect(opts).await {
        Ok(db) => Some(db),
        Err(e) => {
            eprintln!("skipping: cannot connect to {ENV_VAR}: {e}");
            None
        }
    }
}

async fn reset_catalog(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    // `metric_query_catalog` (ADR-001, m20260529) FKs into `metric_catalog`
    // with `ON DELETE CASCADE` — drop it first to avoid MariaDB error 1451.
    for table in [
        "metric_query_catalog",
        "threshold_lock_audit",
        "metric_threshold",
        "metric_catalog",
    ] {
        db.execute_unprepared(&format!("DROP TABLE IF EXISTS {table}"))
            .await?;
    }
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

/// Stand up the service against a real DB with a no-op cache + a
/// disconnected validator stub. The validator path is best-effort and
/// returns `ValidatorError` on any DB call against the disconnected
/// connection — admin-crud treats that as informational, so it doesn't
/// affect the assertions here.
fn service_against(db: DatabaseConnection) -> AdminThresholdService {
    let tenant_auth: Arc<dyn TenantAuthorization> = Arc::new(ConfigTenantAuthorization::new(None));
    let cache: Arc<dyn CatalogCache> = Arc::new(NoopCatalogCache::default());
    // Validator stub: connect ClickHouse to an unreachable address; the
    // service only calls `validate(metric_key)` best-effort and ignores
    // the outcome.
    let validator = SchemaValidator::new(
        db.clone(),
        insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            "http://127.0.0.1:1",
            "test",
        )),
    );
    AdminThresholdService::new(db, tenant_auth, cache, validator)
}

fn ctx_for(tenant: Uuid) -> SecurityContext {
    let Ok(ctx) = SecurityContext::builder()
        .subject_id(Uuid::nil())
        .subject_tenant_id(tenant)
        .build()
    else {
        unreachable!("subject_id + subject_tenant_id are set")
    };
    ctx
}

/// Insert a fresh `metric_catalog` row + the matching `product-default`
/// floor; returns the `(metric_id, metric_key)` pair so a test can
/// drive admin writes against it.
async fn seed_metric(
    db: &DatabaseConnection,
    is_enabled: bool,
    higher_is_better: bool,
) -> Result<(Uuid, String), sea_orm::DbErr> {
    let metric_id = Uuid::now_v7();
    let metric_key = format!("test_table.col_{}", &metric_id.to_string()[..8]);
    let backend = db.get_database_backend();
    db.execute(Statement::from_sql_and_values(
        backend,
        "INSERT INTO metric_catalog \
         (id, tenant_id, metric_key, label, sublabel, description, unit, format, \
          higher_is_better, is_member_scale, source_tags, is_enabled, schema_status, \
          schema_checked_at, schema_error_code, created_at, updated_at) \
         VALUES (?, NULL, ?, 'Label', NULL, NULL, NULL, NULL, ?, FALSE, '[]', ?, \
          'unchecked', NULL, NULL, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
        [
            Value::Bytes(Some(Box::new(metric_id.as_bytes().to_vec()))),
            Value::from(metric_key.as_str()),
            Value::from(higher_is_better),
            Value::from(is_enabled),
        ],
    ))
    .await?;

    // Seed product-default floor so the resolver invariant holds.
    let pd_id = Uuid::now_v7();
    db.execute(Statement::from_sql_and_values(
        backend,
        "INSERT INTO metric_threshold \
         (id, tenant_id, metric_key, scope, role_slug, team_id, good, warn, \
          is_locked, created_at, updated_at) \
         VALUES (?, NULL, ?, 'product-default', '', '', 10, 5, FALSE, \
          CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
        [
            Value::Bytes(Some(Box::new(pd_id.as_bytes().to_vec()))),
            Value::from(metric_key.as_str()),
        ],
    ))
    .await?;

    Ok((metric_id, metric_key))
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn happy_create_update_delete_round_trip() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    reset_catalog(&db)
        .await
        .unwrap_or_else(|e| panic!("reset: {e}"));
    Migrator::up(&db, None)
        .await
        .unwrap_or_else(|e| panic!("migrate: {e}"));
    let (metric_id, _metric_key) = seed_metric(&db, true, true)
        .await
        .unwrap_or_else(|e| panic!("seed: {e}"));

    let tenant = Uuid::now_v7();
    let svc = service_against(db);
    let ctx = ctx_for(tenant);

    // Create
    let created = svc
        .create(
            &ctx,
            &CreateRequest {
                metric_id,
                scope: Scope::Tenant,
                role_slug: None,
                team_id: None,
                good: 25.0,
                warn: 12.0,
                alert_trigger: None,
                alert_bad: None,
                is_locked: false,
                lock_reason: None,
            },
        )
        .await
        .unwrap_or_else(|resp| panic!("create rejected: {}", resp.status()));
    assert_eq!(created.scope, Scope::Tenant);
    assert!(!created.is_locked);

    // Update — change `good` / `warn`, keep scope.
    let updated = svc
        .update(
            &ctx,
            created.id,
            &UpdateRequest {
                scope: Some(Scope::Tenant),
                role_slug: None,
                team_id: None,
                good: 30.0,
                warn: 15.0,
                alert_trigger: None,
                alert_bad: None,
                is_locked: false,
                lock_reason: None,
            },
        )
        .await
        .unwrap_or_else(|resp| panic!("update rejected: {}", resp.status()));
    assert!((updated.good - 30.0).abs() < f64::EPSILON);
    assert!((updated.warn - 15.0).abs() < f64::EPSILON);

    // Delete
    svc.delete(&ctx, created.id)
        .await
        .unwrap_or_else(|resp| panic!("delete rejected: {}", resp.status()));
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn lock_bypass_writes_audit_row_and_returns_403() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    reset_catalog(&db)
        .await
        .unwrap_or_else(|e| panic!("reset: {e}"));
    Migrator::up(&db, None)
        .await
        .unwrap_or_else(|e| panic!("migrate: {e}"));
    let (metric_id, metric_key) = seed_metric(&db, true, true)
        .await
        .unwrap_or_else(|e| panic!("seed: {e}"));

    let tenant = Uuid::now_v7();
    // Pre-seed a locked tenant-scope row; a narrower-scope write MUST be
    // shadowed by it.
    let blocking_id = Uuid::now_v7();
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO metric_threshold \
         (id, tenant_id, metric_key, scope, role_slug, team_id, good, warn, \
          is_locked, locked_by, locked_at, lock_reason, created_at, updated_at) \
         VALUES (?, ?, ?, 'tenant', '', '', 20, 10, TRUE, 'u-alice', \
          CURRENT_TIMESTAMP, 'TICKET-7421: HR SLA pin', \
          CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
        [
            Value::Bytes(Some(Box::new(blocking_id.as_bytes().to_vec()))),
            Value::Bytes(Some(Box::new(tenant.as_bytes().to_vec()))),
            Value::from(metric_key.as_str()),
        ],
    ))
    .await
    .unwrap_or_else(|e| panic!("seed blocking row: {e}"));

    let svc = service_against(db.clone());
    let ctx = ctx_for(tenant);

    let resp = svc
        .create(
            &ctx,
            &CreateRequest {
                metric_id,
                scope: Scope::Role,
                role_slug: Some("eng".to_owned()),
                team_id: None,
                good: 25.0,
                warn: 12.0,
                alert_trigger: None,
                alert_bad: None,
                is_locked: false,
                lock_reason: None,
            },
        )
        .await
        .err()
        .unwrap_or_else(|| panic!("create MUST be rejected by lock-enforcer"));
    assert_eq!(resp.status(), 403);

    // Audit row landed in `threshold_lock_audit`?
    let count: i64 = sea_orm::ConnectionTrait::query_one(
        &db,
        Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT COUNT(*) AS c FROM threshold_lock_audit \
             WHERE tenant_id = ? AND metric_key = ? AND event_type = 'bypass_attempt'",
            [
                Value::Bytes(Some(Box::new(tenant.as_bytes().to_vec()))),
                Value::from(metric_key.as_str()),
            ],
        ),
    )
    .await
    .unwrap_or_else(|e| panic!("audit count: {e}"))
    .and_then(|r| r.try_get::<i64>("", "c").ok())
    .unwrap_or(0);
    assert_eq!(
        count, 1,
        "bypass_attempt audit row MUST be persisted before the 403 returns"
    );
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn immutable_field_put_rejected() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    reset_catalog(&db)
        .await
        .unwrap_or_else(|e| panic!("reset: {e}"));
    Migrator::up(&db, None)
        .await
        .unwrap_or_else(|e| panic!("migrate: {e}"));
    let (metric_id, _) = seed_metric(&db, true, true)
        .await
        .unwrap_or_else(|e| panic!("seed: {e}"));

    let tenant = Uuid::now_v7();
    let svc = service_against(db);
    let ctx = ctx_for(tenant);

    let created = svc
        .create(
            &ctx,
            &CreateRequest {
                metric_id,
                scope: Scope::Tenant,
                role_slug: None,
                team_id: None,
                good: 25.0,
                warn: 12.0,
                alert_trigger: None,
                alert_bad: None,
                is_locked: false,
                lock_reason: None,
            },
        )
        .await
        .unwrap_or_else(|resp| panic!("create rejected: {}", resp.status()));

    // PUT trying to change `scope` from `tenant` → `role`.
    let resp = svc
        .update(
            &ctx,
            created.id,
            &UpdateRequest {
                scope: Some(Scope::Role),
                role_slug: Some("eng".to_owned()),
                team_id: None,
                good: 30.0,
                warn: 15.0,
                alert_trigger: None,
                alert_bad: None,
                is_locked: false,
                lock_reason: None,
            },
        )
        .await
        .err()
        .unwrap_or_else(|| panic!("PUT MUST be rejected for immutable scope change"));
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn cross_tenant_put_rejected() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    reset_catalog(&db)
        .await
        .unwrap_or_else(|e| panic!("reset: {e}"));
    Migrator::up(&db, None)
        .await
        .unwrap_or_else(|e| panic!("migrate: {e}"));
    let (metric_id, _) = seed_metric(&db, true, true)
        .await
        .unwrap_or_else(|e| panic!("seed: {e}"));

    let tenant_a = Uuid::now_v7();
    let tenant_b = Uuid::now_v7();
    let svc = service_against(db);

    let created = svc
        .create(
            &ctx_for(tenant_a),
            &CreateRequest {
                metric_id,
                scope: Scope::Tenant,
                role_slug: None,
                team_id: None,
                good: 25.0,
                warn: 12.0,
                alert_trigger: None,
                alert_bad: None,
                is_locked: false,
                lock_reason: None,
            },
        )
        .await
        .unwrap_or_else(|resp| panic!("create as A rejected: {}", resp.status()));

    // Now try to update tenant A's row from tenant B's session.
    let resp = svc
        .update(
            &ctx_for(tenant_b),
            created.id,
            &UpdateRequest {
                scope: None,
                role_slug: None,
                team_id: None,
                good: 30.0,
                warn: 15.0,
                alert_trigger: None,
                alert_bad: None,
                is_locked: false,
                lock_reason: None,
            },
        )
        .await
        .err()
        .unwrap_or_else(|| panic!("cross-tenant PUT MUST be rejected"));
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn lock_set_without_reason_rejected() {
    let Some(db) = connect_or_skip().await else {
        return;
    };
    reset_catalog(&db)
        .await
        .unwrap_or_else(|e| panic!("reset: {e}"));
    Migrator::up(&db, None)
        .await
        .unwrap_or_else(|e| panic!("migrate: {e}"));
    let (metric_id, _) = seed_metric(&db, true, true)
        .await
        .unwrap_or_else(|e| panic!("seed: {e}"));

    let tenant = Uuid::now_v7();
    let svc = service_against(db);

    let resp = svc
        .create(
            &ctx_for(tenant),
            &CreateRequest {
                metric_id,
                scope: Scope::Tenant,
                role_slug: None,
                team_id: None,
                good: 25.0,
                warn: 12.0,
                alert_trigger: None,
                alert_bad: None,
                is_locked: true,
                lock_reason: None,
            },
        )
        .await
        .err()
        .unwrap_or_else(|| panic!("is_locked=true without lock_reason MUST be rejected"));
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn db_check_violations_map_to_canonical_4xx() {
    // End-to-end pin that a CHECK violation surfaces through the mapper
    // rather than as a bare 500. The gauntlet catches most of these
    // BEFORE the DB write — to drive a real CHECK we bypass the
    // gauntlet and INSERT a row with `is_locked = TRUE` and
    // `lock_reason = NULL`, which trips
    // `chk_metric_threshold_lock_reason_when_locked`.
    //
    // The length CHECK `chk_metric_threshold_lock_reason_length` cannot
    // be provoked from outside the gauntlet here — MariaDB's
    // VARCHAR(512) column-length check (error 1406, "Data too long")
    // fires before the CHECK in the driver's evaluation order. The
    // gauntlet's `validate_lock_reason` is the canonical layer that
    // catches the 600-char case; the DB CHECK exists as a backstop for
    // direct-SQL writes that bypass the gauntlet entirely (e.g.,
    // migrations).
    let Some(db) = connect_or_skip().await else {
        return;
    };
    reset_catalog(&db)
        .await
        .unwrap_or_else(|e| panic!("reset: {e}"));
    Migrator::up(&db, None)
        .await
        .unwrap_or_else(|e| panic!("migrate: {e}"));

    let row_id = Uuid::now_v7();
    let result = db
        .execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO metric_threshold \
             (id, tenant_id, metric_key, scope, role_slug, team_id, good, warn, \
              is_locked, lock_reason, created_at, updated_at) \
             VALUES (?, NULL, 't.c', 'product-default', '', '', 10, 5, TRUE, NULL, \
              CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            [Value::Bytes(Some(Box::new(row_id.as_bytes().to_vec())))],
        ))
        .await;
    let Err(err) = result else {
        panic!(
            "DB MUST reject is_locked=TRUE without lock_reason via \
             chk_metric_threshold_lock_reason_when_locked"
        );
    };
    let mapped = crate::api::admin::error_map::map_db_err(&err, None);
    assert_eq!(
        mapped.status(),
        400,
        "CHECK violation MUST surface as a 4xx, not 500 — DESIGN §3.2 admin-crud invariant"
    );
}

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn successful_write_invalidates_cache() {
    use crate::infra::cache::catalog_cache::{CatalogCache, InvalidateMode};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counting cache stub — counts `invalidate` calls so the test can
    /// assert admin-crud DOES call it on every successful 2xx write.
    /// Inline rather than a top-level helper to avoid leaking test
    /// scaffolding into the prod-build surface.
    struct CountingCache {
        invalidate_count: AtomicUsize,
    }

    #[async_trait]
    impl CatalogCache for CountingCache {
        async fn get(
            &self,
            _: Uuid,
            _: Option<&str>,
            _: Option<&str>,
        ) -> anyhow::Result<Option<crate::domain::catalog::response::CatalogResponse>> {
            Ok(None)
        }
        async fn put(
            &self,
            _: Uuid,
            _: Option<&str>,
            _: Option<&str>,
            _: &crate::domain::catalog::response::CatalogResponse,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn invalidate(&self, _: Uuid, _: InvalidateMode) -> anyhow::Result<()> {
            self.invalidate_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn flush_all(&self) -> anyhow::Result<()> {
            Ok(())
        }
        fn should_skip(&self, _: Uuid) -> bool {
            false
        }
    }

    let Some(db) = connect_or_skip().await else {
        return;
    };
    reset_catalog(&db)
        .await
        .unwrap_or_else(|e| panic!("reset: {e}"));
    Migrator::up(&db, None)
        .await
        .unwrap_or_else(|e| panic!("migrate: {e}"));
    let (metric_id, _) = seed_metric(&db, true, true)
        .await
        .unwrap_or_else(|e| panic!("seed: {e}"));

    let tenant = Uuid::now_v7();
    let tenant_auth: Arc<dyn TenantAuthorization> = Arc::new(ConfigTenantAuthorization::new(None));
    let cache = Arc::new(CountingCache {
        invalidate_count: AtomicUsize::new(0),
    });
    let validator = SchemaValidator::new(
        db.clone(),
        insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            "http://127.0.0.1:1",
            "test",
        )),
    );
    let svc = AdminThresholdService::new(db, tenant_auth, cache.clone(), validator);

    svc.create(
        &ctx_for(tenant),
        &CreateRequest {
            metric_id,
            scope: Scope::Tenant,
            role_slug: None,
            team_id: None,
            good: 25.0,
            warn: 12.0,
            alert_trigger: None,
            alert_bad: None,
            is_locked: false,
            lock_reason: None,
        },
    )
    .await
    .unwrap_or_else(|resp| panic!("create rejected: {}", resp.status()));

    assert_eq!(
        cache.invalidate_count.load(Ordering::SeqCst),
        1,
        "successful create MUST trigger cache invalidate exactly once"
    );
}
