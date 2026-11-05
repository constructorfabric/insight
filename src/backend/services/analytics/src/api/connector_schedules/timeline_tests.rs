use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use clickhouse::test::{Mock, handlers};
use sea_orm::{DatabaseBackend, MockDatabase};
use serde::Serialize;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::support::*;
use crate::infra::db::entities::connector_schedules::Model;

#[derive(Debug, Serialize, clickhouse::Row)]
struct CompletedPassRecord {
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    pass_completed_at: DateTime<Utc>,
}

async fn at_pass(
    crons: &[&str],
    origin: &str,
    previous_secs: i64,
) -> Result<Value, Box<dyn std::error::Error>> {
    let saved = DateTime::UNIX_EPOCH + Duration::hours(1);
    let latest = saved + Duration::minutes(10);
    let sync_history = Mock::new();
    sync_history.add(handlers::provide([ConfiguredInstanceRecord {
        schedules: vec!["0 3 * * *".to_owned()],
        schedule_origin: origin.to_owned(),
        pass_completed_at: latest,
        ..configured_instance_record()
    }]));
    if !crons.is_empty() || origin == "api" {
        sync_history.add(handlers::provide([
            CompletedPassRecord {
                pass_completed_at: latest,
            },
            CompletedPassRecord {
                pass_completed_at: saved + Duration::seconds(previous_secs),
            },
        ]));
    }
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([vec![Model {
            connector: "example-tracker".to_owned(),
            source_id: "main".to_owned(),
            crons: json!(crons),
            revision: 2,
            updated_by: CALLER,
            updated_at: saved,
        }]])
        .into_connection();
    let h = harness(db, sync_history.url(), true, context("user", &[])?, true).await?;
    let response = h
        .app
        .clone()
        .oneshot(request("GET", URI, None, &json!(null))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    body(response).await
}

#[tokio::test]
async fn a_save_during_a_pass_waits_for_the_following_complete_pass() -> R {
    for (previous_secs, status) in [(-600, "pending"), (0, "pending"), (1, "not_applied")] {
        let response = at_pass(&["0 6 * * *"], "api", previous_secs).await?;
        assert_eq!(
            response["status"], status,
            "previous pass completion offset {previous_secs}"
        );
        assert_eq!(response["not_applied_since"].is_null(), status == "pending");
    }
    Ok(())
}

#[tokio::test]
async fn get_pass_completion_reads_stop_at_the_applied_snapshot_in_milliseconds() -> R {
    let sync_history = Mock::new();
    let through = DateTime::UNIX_EPOCH + Duration::hours(2) + Duration::milliseconds(123);
    sync_history.add(handlers::provide([ConfiguredInstanceRecord {
        pass_completed_at: through,
        ..configured_instance_record()
    }]));
    let bound = sync_history.add(handlers::record_ddl());
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([vec![Model {
            connector: "example-tracker".to_owned(),
            source_id: "main".to_owned(),
            crons: json!(["0 6 * * *"]),
            revision: 2,
            updated_by: CALLER,
            updated_at: DateTime::UNIX_EPOCH,
        }]])
        .into_connection();
    let h = harness(db, sync_history.url(), true, context("user", &[])?, true).await?;
    let response = h
        .app
        .clone()
        .oneshot(request("GET", URI, None, &json!(null))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let query = bound.query().await;
    assert!(
        query.contains(&format!(
            "ts <= fromUnixTimestamp64Milli({})",
            through.timestamp_millis()
        )),
        "{query}"
    );
    Ok(())
}

#[tokio::test]
async fn pass_completion_failure_makes_get_unavailable() -> R {
    let sync_history = Mock::new();
    sync_history.add(handlers::provide([ConfiguredInstanceRecord {
        pass_completed_at: DateTime::UNIX_EPOCH + Duration::hours(2),
        ..configured_instance_record()
    }]));
    sync_history.add(handlers::failure(
        clickhouse::test::status::INTERNAL_SERVER_ERROR,
    ));
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([vec![Model {
            connector: "example-tracker".to_owned(),
            source_id: "main".to_owned(),
            crons: json!(["0 6 * * *"]),
            revision: 2,
            updated_by: CALLER,
            updated_at: DateTime::UNIX_EPOCH,
        }]])
        .into_connection();
    let h = harness(db, sync_history.url(), true, context("user", &[])?, true).await?;
    let response = h
        .app
        .clone()
        .oneshot(request("GET", URI, None, &json!(null))?)
        .await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body(response).await?["status"], 503);
    Ok(())
}

#[tokio::test]
async fn matching_annotation_crons_never_apply_an_api_request() -> R {
    let response = at_pass(&["0 3 * * *"], "annotation", 1).await?;
    assert_eq!(response["status"], "not_applied");
    Ok(())
}

#[tokio::test]
async fn clearing_a_pending_request_waits_until_the_sync_history_records_inheritance() -> R {
    let pending = at_pass(&[], "api", -600).await?;
    assert_eq!(pending["status"], "pending");
    assert_eq!(pending["requested"]["crons"], json!([]));
    for origin in ["annotation", "descriptor", "default"] {
        let inherited = at_pass(&[], origin, 1).await?;
        assert_eq!(inherited["status"], "inherited", "{origin}");
        assert_eq!(inherited["revision"], 2);
    }
    Ok(())
}
