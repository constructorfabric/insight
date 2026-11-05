use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use clickhouse::test::{Mock, handlers};
use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};
use serde::Serialize;
use serde_json::json;
use tower::ServiceExt;

use super::support::*;
use crate::infra::db::entities::connector_schedules::Model;

fn stored(crons: &[&str]) -> Model {
    Model {
        connector: "example-tracker".to_owned(),
        source_id: "main".to_owned(),
        crons: json!(crons),
        revision: 7,
        updated_by: CALLER,
        updated_at: DateTime::UNIX_EPOCH,
    }
}

fn removed() -> Mock {
    let sync_history = Mock::new();
    sync_history.add(handlers::provide(Vec::<ConfiguredInstanceRecord>::new()));
    sync_history
}

#[tokio::test]
async fn editing_off_refuses_to_clear_a_removed_instance() -> R {
    let sync_history = removed();
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([vec![stored(&["0 3 * * *"])], vec![stored(&["0 3 * * *"])]])
        .append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }])
        .into_connection();
    let h = harness(db, sync_history.url(), false, context("user", &[])?, true).await?;
    let response = h
        .app
        .clone()
        .oneshot(request("DELETE", URI, Some("\"7\""), &json!(null))?)
        .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    Ok(())
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct CompletedPassRecord {
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
    pass_completed_at: DateTime<Utc>,
}

#[tokio::test]
async fn removed_requests_remain_readable_with_the_existing_status_rules() -> R {
    for enabled in [false, true] {
        for (crons, completed_pass, expected) in [
            (vec!["0 3 * * *"], false, "pending"),
            (vec!["0 3 * * *"], true, "not_applied"),
            (vec![], false, "inherited"),
        ] {
            let sync_history = removed();
            if !crons.is_empty() {
                let completed_passes = if completed_pass {
                    vec![
                        CompletedPassRecord {
                            pass_completed_at: DateTime::UNIX_EPOCH + Duration::hours(2),
                        },
                        CompletedPassRecord {
                            pass_completed_at: DateTime::UNIX_EPOCH + Duration::hours(1),
                        },
                    ]
                } else {
                    vec![]
                };
                sync_history.add(handlers::provide(completed_passes));
            }
            let db = MockDatabase::new(DatabaseBackend::MySql)
                .append_query_results([vec![stored(&crons)]])
                .into_connection();
            let h = harness(db, sync_history.url(), enabled, context("user", &[])?, true).await?;
            let response = h
                .app
                .clone()
                .oneshot(request("GET", URI, None, &json!(null))?)
                .await?;
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "{expected}, enabled={enabled}"
            );
            assert_eq!(response.headers()["etag"], "\"7\"");
            let json = body(response).await?;
            assert_eq!(json["revision"], 7);
            assert_eq!(json["requested"]["crons"], json!(crons));
            assert_eq!(json["requested"]["updated_by"], CALLER.to_string());
            assert_eq!(json["requested"]["updated_at"], "1970-01-01T00:00:00Z");
            assert!(json["applied"].is_null());
            assert_eq!(json["status"], expected, "{crons:?}, enabled={enabled}");
            assert_eq!(json["editable"], enabled);
            assert_eq!(json["not_applied_since"].is_null(), !completed_pass);
            if completed_pass {
                assert_eq!(json["not_applied_since"], "1970-01-01T02:00:00Z");
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn clearing_a_removed_request_advances_the_revision_and_clears_reconcile() -> R {
    let sync_history = removed();
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([
            vec![stored(&["0 3 * * *"])],
            vec![stored(&["0 3 * * *"])],
            vec![],
        ])
        .append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }])
        .into_connection();
    let h = harness(
        db.clone(),
        sync_history.url(),
        true,
        context("user", &[])?,
        true,
    )
    .await?;
    let response = h
        .app
        .clone()
        .oneshot(request("DELETE", URI, Some("\"7\""), &json!(null))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["etag"], "\"8\"");
    let json = body(response).await?;
    assert_eq!(json["revision"], 8);
    assert_eq!(json["requested"]["crons"], json!([]));
    assert_eq!(json["requested"]["updated_by"], CALLER.to_string());
    assert!(json["applied"].is_null());
    assert_eq!(json["status"], "inherited");
    assert!(
        crate::domain::connector_schedules::list(&db)
            .await?
            .schedules
            .is_empty()
    );
    let log = format!("{:?}", db.into_transaction_log());
    assert!(log.contains("UPDATE `connector_schedules` SET"), "{log}");
    assert!(log.contains("JSON_LENGTH(crons) > 0"), "{log}");
    Ok(())
}

#[tokio::test]
async fn clearing_a_removed_request_requires_a_current_strong_if_match() -> R {
    for (header, expected) in [
        (None, StatusCode::PRECONDITION_REQUIRED),
        (Some("W/\"7\""), StatusCode::BAD_REQUEST),
        (Some("\"6\""), StatusCode::PRECONDITION_FAILED),
    ] {
        let sync_history = removed();
        let db = MockDatabase::new(DatabaseBackend::MySql)
            .append_query_results([vec![stored(&["0 3 * * *"])], vec![stored(&["0 3 * * *"])]])
            .into_connection();
        let h = harness(db, sync_history.url(), true, context("user", &[])?, true).await?;
        let response = h
            .app
            .clone()
            .oneshot(request("DELETE", URI, header, &json!(null))?)
            .await?;
        assert_eq!(response.status(), expected, "{header:?}");
        if expected == StatusCode::PRECONDITION_FAILED {
            assert_eq!(response.headers()["etag"], "\"7\"");
            assert_eq!(body(response).await?["context"]["revision"], 7);
        }
    }
    Ok(())
}

#[tokio::test]
async fn clearing_an_already_cleared_removed_request_preserves_revision_and_audit() -> R {
    let sync_history = removed();
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([vec![stored(&[])], vec![stored(&[])]])
        .into_connection();
    let h = harness(
        db.clone(),
        sync_history.url(),
        true,
        context("user", &[])?,
        true,
    )
    .await?;
    let response = h
        .app
        .clone()
        .oneshot(request("DELETE", URI, Some("\"7\""), &json!(null))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["etag"], "\"7\"");
    let json = body(response).await?;
    assert_eq!(json["revision"], 7);
    assert_eq!(json["requested"]["crons"], json!([]));
    assert_eq!(json["requested"]["updated_at"], "1970-01-01T00:00:00Z");
    assert_eq!(json["requested"]["updated_by"], CALLER.to_string());
    assert_eq!(json["status"], "inherited");
    assert!(json["applied"].is_null());
    assert!(!format!("{:?}", db.into_transaction_log()).contains("UPDATE"));
    Ok(())
}

#[tokio::test]
async fn removed_requests_cannot_be_replaced_even_when_stored() -> R {
    let sync_history = removed();
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([vec![stored(&["0 3 * * *"])], vec![stored(&["0 3 * * *"])]])
        .append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }])
        .into_connection();
    let h = harness(db, sync_history.url(), true, context("user", &[])?, true).await?;
    let response = h
        .app
        .clone()
        .oneshot(request(
            "PUT",
            URI,
            Some("\"7\""),
            &json!({"crons": ["0 6 * * *"]}),
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        body(response).await?["detail"]
            .as_str()
            .is_some_and(|text| text.contains("configured on this install"))
    );
    Ok(())
}
