//! Live MariaDB integration tests for the custom-metric repository.
//!
//! `#[ignore]`d and skip silently when `INTEGRATION_TESTS_MARIADB_URL` is
//! unset, so `cargo test` stays green on a stock dev machine (same convention
//! as `metric_definitions/live_tests.rs`). CI runs them with `--include-ignored`
//! against a migrated MariaDB, which is where the raw-SQL write/read paths
//! (`insert_graph`, `delete_graph`, `fetch_*`, `export`/`import`) earn their
//! coverage — the pure graph validation is unit-tested separately. No
//! ClickHouse is touched: the repository is service-DB only (the observation
//! probe lives in the handler, not here).

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement};
use uuid::Uuid;

use super::{
    CustomMetric, CustomMetricInput, ReplaceOutcome, WriteOutcome, create_custom_metric,
    delete_custom_metric, export_custom_metrics, fetch_custom_metric, import_custom_metrics,
    list_custom_metrics, replace_custom_metric,
};
use crate::domain::metric_definitions::definition::{
    MetricComputation, MetricDirection, MetricFormat, MetricInputRole,
};

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

/// A key suffix unique per call, so sibling metrics within a test and parallel
/// `#[ignore]`d tests never collide on the per-tenant unique keys. The full
/// simple form is used deliberately: a v7 UUID's leading hex is the millisecond
/// timestamp, so truncating it would make two calls in the same millisecond
/// collide — the random tail is what guarantees uniqueness.
fn suffix() -> String {
    Uuid::now_v7().simple().to_string()
}

fn sum_graph(suffix: &str) -> CustomMetric {
    CustomMetric {
        metric_key: format!("custom.k{suffix}"),
        label: "Example".to_owned(),
        short_label: Some("Ex".to_owned()),
        subject: Some("activity".to_owned()),
        description: Some("desc".to_owned()),
        explanation: None,
        entity_type: "person".to_owned(),
        unit: Some("lines".to_owned()),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        computation: MetricComputation::Sum,
        scale: None,
        peer_cohort_key: Some("org_unit".to_owned()),
        transform: None,
        source_key: format!("custom_s{suffix}"),
        observation_sql: "SELECT tenant_id, source_key, entity_type, entity_id, metric_date, \
            measure_key, observed_at, value, subject_key, dimensions FROM system.one"
            .to_owned(),
        measures: vec!["events".to_owned()],
        dimensions: vec!["tool".to_owned(), "repository".to_owned()],
        tags: vec!["rate".to_owned(), "duration".to_owned()],
        inputs: vec![CustomMetricInput {
            role: MetricInputRole::Value,
            measure_key: "events".to_owned(),
        }],
        origin: None,
    }
}

fn ratio_graph(suffix: &str) -> CustomMetric {
    let mut graph = sum_graph(suffix);
    graph.computation = MetricComputation::Ratio;
    graph.scale = Some(100.0);
    graph.measures = vec!["num".to_owned(), "den".to_owned()];
    graph.inputs = vec![
        CustomMetricInput {
            role: MetricInputRole::Numerator,
            measure_key: "num".to_owned(),
        },
        CustomMetricInput {
            role: MetricInputRole::Denominator,
            measure_key: "den".to_owned(),
        },
    ];
    graph
}

type R = Result<(), sea_orm::DbErr>;

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn create_fetch_list_delete_roundtrip() -> R {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();
    let graph = sum_graph(&suffix());

    assert!(matches!(
        create_custom_metric(&db, tenant, &graph).await?,
        WriteOutcome::Created
    ));

    let fetched = fetch_custom_metric(&db, tenant, &graph.metric_key)
        .await?
        .unwrap_or_else(|| panic!("created metric must be fetchable"));
    assert_eq!(fetched.metric_key, graph.metric_key);
    assert_eq!(fetched.source_key, graph.source_key);
    assert_eq!(fetched.observation_sql, graph.observation_sql);
    assert_eq!(fetched.measures, graph.measures);
    assert_eq!(fetched.dimensions, graph.dimensions);
    assert_eq!(fetched.tags, graph.tags);
    assert_eq!(fetched.inputs.len(), 1);
    assert_eq!(fetched.origin.as_deref(), Some("custom"));
    assert_eq!(fetched.short_label.as_deref(), Some("Ex"));
    assert_eq!(fetched.subject.as_deref(), Some("activity"));

    let summaries = list_custom_metrics(&db, tenant).await?;
    let summary = summaries
        .iter()
        .find(|s| s.metric_key == graph.metric_key)
        .unwrap_or_else(|| panic!("created metric must be listed"));
    assert_eq!(summary.subject.as_deref(), Some("activity"));

    assert!(delete_custom_metric(&db, tenant, &graph.metric_key).await?);
    assert!(
        fetch_custom_metric(&db, tenant, &graph.metric_key)
            .await?
            .is_none()
    );
    // A second delete finds nothing.
    assert!(!delete_custom_metric(&db, tenant, &graph.metric_key).await?);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn ratio_roundtrip_preserves_scale_and_both_inputs() -> R {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();
    let graph = ratio_graph(&suffix());

    assert!(matches!(
        create_custom_metric(&db, tenant, &graph).await?,
        WriteOutcome::Created
    ));

    let fetched = fetch_custom_metric(&db, tenant, &graph.metric_key)
        .await?
        .unwrap_or_else(|| panic!("ratio metric must be fetchable"));
    assert_eq!(fetched.computation, MetricComputation::Ratio);
    assert_eq!(fetched.scale, Some(100.0));
    assert_eq!(fetched.inputs.len(), 2);
    assert!(
        fetched
            .inputs
            .iter()
            .any(|i| i.role == MetricInputRole::Numerator)
    );
    assert!(
        fetched
            .inputs
            .iter()
            .any(|i| i.role == MetricInputRole::Denominator)
    );

    delete_custom_metric(&db, tenant, &graph.metric_key).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn create_conflicts_on_existing_key_or_source() -> R {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();
    let sfx = suffix();
    let graph = sum_graph(&sfx);
    create_custom_metric(&db, tenant, &graph).await?;

    // Same metric_key -> conflict.
    assert!(matches!(
        create_custom_metric(&db, tenant, &graph).await?,
        WriteOutcome::AlreadyExists
    ));

    // Different metric_key, same source_key -> conflict.
    let mut other = sum_graph(&suffix());
    other.source_key = graph.source_key.clone();
    assert!(matches!(
        create_custom_metric(&db, tenant, &other).await?,
        WriteOutcome::AlreadyExists
    ));

    delete_custom_metric(&db, tenant, &graph.metric_key).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn replace_updates_and_rejects_source_conflict() -> R {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();
    let first = sum_graph(&suffix());
    let second = sum_graph(&suffix());
    create_custom_metric(&db, tenant, &first).await?;
    create_custom_metric(&db, tenant, &second).await?;

    // Replace `first` with a new label + new source_key.
    let mut updated = first.clone();
    updated.label = "Renamed".to_owned();
    updated.source_key = format!("custom_r{}", suffix());
    assert_eq!(
        replace_custom_metric(&db, tenant, &first.metric_key, &updated).await?,
        ReplaceOutcome::Replaced
    );
    let fetched = fetch_custom_metric(&db, tenant, &first.metric_key)
        .await?
        .unwrap_or_else(|| panic!("replaced metric must be fetchable"));
    assert_eq!(fetched.label, "Renamed");
    assert_eq!(fetched.source_key, updated.source_key);

    // Replacing `first` with `second`'s source_key is a conflict.
    let mut clash = fetched.clone();
    clash.source_key = second.source_key.clone();
    assert_eq!(
        replace_custom_metric(&db, tenant, &first.metric_key, &clash).await?,
        ReplaceOutcome::SourceConflict
    );

    // Replacing a key that does not exist reports NotFound.
    assert_eq!(
        replace_custom_metric(&db, tenant, "custom.absent_key", &clash).await?,
        ReplaceOutcome::NotFound
    );

    delete_custom_metric(&db, tenant, &first.metric_key).await?;
    delete_custom_metric(&db, tenant, &second.metric_key).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn export_then_import_rehomes_tenant_and_is_idempotent() -> R {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let source_tenant = Uuid::now_v7();
    let target_tenant = Uuid::now_v7();
    let a = sum_graph(&suffix());
    let b = ratio_graph(&suffix());
    create_custom_metric(&db, source_tenant, &a).await?;
    create_custom_metric(&db, source_tenant, &b).await?;

    let exported = export_custom_metrics(&db, source_tenant).await?;
    assert!(
        exported.iter().all(|g| g.origin.is_none()),
        "export is portable"
    );
    assert!(exported.iter().any(|g| g.metric_key == a.metric_key));
    assert!(exported.iter().any(|g| g.metric_key == b.metric_key));

    // Import onto another tenant: both land, re-homed under the same keys.
    let skipped = import_custom_metrics(&db, target_tenant, &exported).await?;
    assert!(skipped.is_empty(), "first import skips nothing");
    let landed = fetch_custom_metric(&db, target_tenant, &a.metric_key)
        .await?
        .unwrap_or_else(|| panic!("imported metric must be fetchable on the target tenant"));
    assert_eq!(landed.origin.as_deref(), Some("custom"));

    // Re-import is idempotent: every key already exists, so all are skipped.
    let skipped_again = import_custom_metrics(&db, target_tenant, &exported).await?;
    assert_eq!(skipped_again.len(), exported.len());

    for key in [&a.metric_key, &b.metric_key] {
        delete_custom_metric(&db, source_tenant, key).await?;
        delete_custom_metric(&db, target_tenant, key).await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn builtin_keys_are_invisible_to_the_custom_repository() -> R {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();

    // A builtin metric_key is owned by the reconcile (origin='builtin',
    // tenant_id NULL); the custom repository must never see or mutate it.
    let builtin_key = a_builtin_metric_key(&db).await?;
    assert!(
        fetch_custom_metric(&db, tenant, &builtin_key)
            .await?
            .is_none()
    );
    assert!(!delete_custom_metric(&db, tenant, &builtin_key).await?);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB 11+; set INTEGRATION_TESTS_MARIADB_URL to enable"]
async fn definition_with_no_inputs_is_still_deletable() -> R {
    let Some(db) = connect_or_skip().await else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();
    let graph = sum_graph(&suffix());
    create_custom_metric(&db, tenant, &graph).await?;

    // Simulate the recovery case the source-via-inputs join cannot serve: drop
    // the definition's input rows out of band. The definition is now
    // unreachable through the inputs join, but it must not become a listed,
    // unremovable ghost.
    db.execute_raw(Statement::from_sql_and_values(
        db.get_database_backend(),
        "DELETE i FROM metric_definition_inputs i \
         INNER JOIN metric_definitions d ON d.id = i.metric_definition_id \
         WHERE d.origin = 'custom' AND d.metric_key = ?",
        [sea_orm::Value::from(graph.metric_key.as_str())],
    ))
    .await?;

    // The delete resolves the definition directly, so it succeeds and the row
    // leaves the listing.
    assert!(delete_custom_metric(&db, tenant, &graph.metric_key).await?);
    let summaries = list_custom_metrics(&db, tenant).await?;
    assert!(!summaries.iter().any(|s| s.metric_key == graph.metric_key));
    Ok(())
}

/// Any seeded builtin `metric_key` — the reconcile runs during `migrate`, so at
/// least one is guaranteed to exist.
async fn a_builtin_metric_key(db: &DatabaseConnection) -> Result<String, sea_orm::DbErr> {
    let row = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            "SELECT metric_key FROM metric_definitions \
             WHERE origin = 'builtin' AND tenant_id IS NULL LIMIT 1",
        ))
        .await?
        .ok_or_else(|| sea_orm::DbErr::Custom("no seeded builtin definitions".to_owned()))?;
    row.try_get("", "metric_key")
}
