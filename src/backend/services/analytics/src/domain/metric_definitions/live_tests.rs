//! Live MariaDB integration tests for the metric-definitions listing read
//! path and the status writer.
//!
//! `#[ignore]`d and skip silently when `INTEGRATION_TESTS_MARIADB_URL` is
//! unset, so `cargo test` stays green on a stock dev machine (same convention
//! as `domain/catalog/live_tests.rs`). CI runs them with `--include-ignored`
//! against a migrated MariaDB, which is where the SQL query paths
//! (`fetch_listing_rows`, `update_definition_status`) earn their coverage —
//! the pure grouping/mapping/monotonic logic is unit-tested separately.

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement, Value};
use uuid::Uuid;

use crate::domain::metric_definitions::error_code::SchemaStatus;
use crate::domain::metric_definitions::listing::list_definition_views;
use crate::domain::metric_definitions::repository::{
    update_definition_status, update_evidence_status, update_source_status,
};
use crate::domain::metric_definitions::test_fixture::DrilldownFixture;
use crate::domain::metric_definitions::validator::MetricDefinitionValidator;
use crate::domain::metric_drilldown::load_capabilities;

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

/// Any seeded product `metric_key` — the listing seeds are migration-owned, so
/// one is guaranteed to exist.
async fn a_product_metric_key(db: &DatabaseConnection) -> Result<String, sea_orm::DbErr> {
    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "SELECT metric_key FROM metric_definitions WHERE tenant_id IS NULL LIMIT 1",
        ))
        .await?
        .ok_or_else(|| sea_orm::DbErr::Custom("no seeded product definitions".to_owned()))?;
    row.try_get("", "metric_key")
}

/// Insert a minimal definition row and return its id. `tenant` isolates the
/// row from seeded data and from sibling tests running in parallel.
async fn insert_definition(
    db: &DatabaseConnection,
    tenant: Uuid,
    metric_key: &str,
    label: &str,
) -> Result<Uuid, sea_orm::DbErr> {
    let id = Uuid::now_v7();
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO metric_definitions \
            (id, tenant_id, metric_key, label, format, direction, entity_type, computation_type, origin) \
         VALUES (?, ?, ?, ?, 'integer', 'higher_is_better', 'person', 'sum', 'custom')",
        [
            Value::Bytes(Some(Box::new(id.as_bytes().to_vec()))),
            Value::Bytes(Some(Box::new(tenant.as_bytes().to_vec()))),
            Value::from(metric_key),
            Value::from(label),
        ],
    ))
    .await?;
    Ok(id)
}

async fn stored_last_observed(
    db: &DatabaseConnection,
    id: Uuid,
) -> Result<Option<chrono::NaiveDate>, sea_orm::DbErr> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT last_observed_date FROM metric_definitions WHERE id = ?",
            [Value::Bytes(Some(Box::new(id.as_bytes().to_vec())))],
        ))
        .await?
        .ok_or_else(|| sea_orm::DbErr::Custom("definition disappeared".to_owned()))?;
    row.try_get("", "last_observed_date")
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn listing_resolves_tenant_override_over_product() -> anyhow::Result<()> {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();
    let metric_key = a_product_metric_key(&db).await?;
    let label = format!("override-{}", Uuid::now_v7().simple());
    insert_definition(&db, tenant, &metric_key, &label).await?;

    let response = list_definition_views(&db, tenant).await?;

    let keys = response
        .metrics
        .iter()
        .map(|m| m.metric_key.clone())
        .collect::<Vec<_>>();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "listing must be sorted by metric_key");
    assert_eq!(
        keys.iter().filter(|k| **k == metric_key).count(),
        1,
        "override collapses onto the product key"
    );
    let row = response
        .metrics
        .iter()
        .find(|m| m.metric_key == metric_key)
        .ok_or_else(|| anyhow::anyhow!("overridden key present"))?;
    assert_eq!(row.label, label, "tenant override label wins");
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn update_definition_status_advances_but_never_regresses_freshness() -> anyhow::Result<()> {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();
    let id = insert_definition(&db, tenant, "git.commits", "freshness-probe").await?;

    let newer = "2026-07-20".parse::<chrono::NaiveDate>()?;
    let older = "2026-01-01".parse::<chrono::NaiveDate>()?;
    let newest = "2026-07-31".parse::<chrono::NaiveDate>()?;

    update_definition_status(&db, id, SchemaStatus::Ok, None, Some(newer)).await?;
    assert_eq!(stored_last_observed(&db, id).await?, Some(newer));

    // Older sweep result must not regress the stored date.
    update_definition_status(&db, id, SchemaStatus::Ok, None, Some(older)).await?;
    assert_eq!(stored_last_observed(&db, id).await?, Some(newer));

    // A NULL (no observation this sweep) preserves the stored date.
    update_definition_status(&db, id, SchemaStatus::Ok, None, None).await?;
    assert_eq!(stored_last_observed(&db, id).await?, Some(newer));

    // A strictly newer date advances it.
    update_definition_status(&db, id, SchemaStatus::Ok, None, Some(newest)).await?;
    assert_eq!(stored_last_observed(&db, id).await?, Some(newest));
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn drilldown_capabilities_follow_healthy_evidence_metadata() -> anyhow::Result<()> {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let fixture = DrilldownFixture::insert(&db, &["git.commits", "tasks.closed"], &[]).await?;
    let keys = vec![
        "git.commits".to_owned(),
        "tasks.closed".to_owned(),
        "missing.metric".to_owned(),
    ];
    let result = async {
        let capabilities = load_capabilities(&db, fixture.tenant_id, &keys).await?;
        anyhow::ensure!(capabilities.contains_key("git.commits"));
        anyhow::ensure!(capabilities.contains_key("tasks.closed"));
        anyhow::ensure!(!capabilities.contains_key("missing.metric"));
        anyhow::ensure!(
            load_capabilities(&db, fixture.tenant_id, &[])
                .await?
                .is_empty()
        );
        Ok(())
    }
    .await;
    fixture.delete(&db).await?;
    result
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn evidence_status_writer_is_revision_conditional() -> anyhow::Result<()> {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let fixture = DrilldownFixture::insert(&db, &["git.commits"], &[]).await?;
    let result = async {
        let revision = fixture.config_revision(&db).await?;
        update_evidence_status(
            &db,
            fixture.source_id,
            &revision,
            SchemaStatus::Error,
            Some(crate::domain::metric_definitions::error_code::MetricSchemaErrorCode::Unknown),
        )
        .await?;
        let current = fixture.statuses(&db).await?;
        anyhow::ensure!(
            current
                == (
                    "ok".to_owned(),
                    "error".to_owned(),
                    Some("unknown".to_owned())
                )
        );
        update_evidence_status(
            &db,
            fixture.source_id,
            "1970-01-01 00:00:00.000000",
            SchemaStatus::Ok,
            None,
        )
        .await?;
        let stale = fixture.statuses(&db).await?;
        anyhow::ensure!(
            stale == current,
            "stale evidence status write changed {current:?} to {stale:?}"
        );
        Ok(())
    }
    .await;
    fixture.delete(&db).await?;
    result
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn source_status_writer_is_revision_conditional() -> anyhow::Result<()> {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let fixture = DrilldownFixture::insert(&db, &["git.commits"], &["repository"]).await?;
    let result = async {
        let revision = fixture.config_revision(&db).await?;
        update_source_status(
            &db,
            fixture.source_id,
            &revision,
            SchemaStatus::Error,
            Some(
                crate::domain::metric_definitions::error_code::MetricSchemaErrorCode::TableNotFound,
            ),
        )
        .await?;
        let current = fixture.statuses(&db).await?;
        anyhow::ensure!(
            current.0 == "error",
            "revision-matched write did not land: {current:?}"
        );

        update_source_status(
            &db,
            fixture.source_id,
            "1970-01-01 00:00:00.000000",
            SchemaStatus::Ok,
            None,
        )
        .await?;
        let stale = fixture.statuses(&db).await?;
        anyhow::ensure!(
            stale == current,
            "stale source status write changed {current:?} to {stale:?}"
        );
        Ok(())
    }
    .await;
    fixture.delete(&db).await?;
    result
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn invalid_evidence_reference_is_an_error_not_unchecked() -> anyhow::Result<()> {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let fixture = DrilldownFixture::insert(&db, &["git.commits"], &[]).await?;
    let result = async {
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "UPDATE metric_sources SET evidence_ref = ? WHERE id = ?",
            [
                Value::from("Not A Relation"),
                Value::Bytes(Some(Box::new(fixture.source_id.as_bytes().to_vec()))),
            ],
        ))
        .await?;
        let invalidated = fixture.statuses(&db).await?;
        anyhow::ensure!(
            invalidated.1 == "unchecked",
            "changing evidence_ref must invalidate the probe: {invalidated:?}"
        );

        let ch = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            "http://127.0.0.1:1",
            "analytics",
        ));
        MetricDefinitionValidator::new(db.clone(), ch)
            .validate_all()
            .await;

        let after = fixture.statuses(&db).await?;
        anyhow::ensure!(
            after.1 == "error" && after.2.as_deref() == Some("unknown"),
            "an unparseable evidence_ref must fail closed as an error: {after:?}"
        );
        Ok(())
    }
    .await;
    fixture.delete(&db).await?;
    result
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn metric_definition_validator_handles_unavailable_clickhouse() -> anyhow::Result<()> {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let fixture = DrilldownFixture::insert(&db, &["git.commits"], &[]).await?;
    let result = async {
        let before = fixture.statuses(&db).await?;
        let ch = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            "http://127.0.0.1:1",
            "analytics",
        ));
        MetricDefinitionValidator::new(db.clone(), ch)
            .validate_all()
            .await;
        let after = fixture.statuses(&db).await?;
        anyhow::ensure!(
            after == before,
            "unavailable ClickHouse changed source status from {before:?} to {after:?}"
        );
        Ok(())
    }
    .await;
    fixture.delete(&db).await?;
    result
}
