use axum::http::{HeaderMap, HeaderValue, StatusCode};
use clickhouse::test::{Mock, handlers, status};
use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};
use serde_json::json;
use tower::ServiceExt;

use super::support::*;
use crate::infra::db::entities::connector_schedules::Model;

fn empty_db() -> sea_orm::DatabaseConnection {
    MockDatabase::new(DatabaseBackend::MySql).into_connection()
}

#[tokio::test]
async fn every_admin_verb_refuses_a_pair_without_configuration_or_storage() -> R {
    for method in ["GET", "PUT", "DELETE"] {
        let sync_history = Mock::new();
        sync_history.add(handlers::provide(Vec::<ConfiguredInstanceRecord>::new()));
        let db = MockDatabase::new(DatabaseBackend::MySql)
            .append_query_results([Vec::<Model>::new()])
            .into_connection();
        let h = harness(db, sync_history.url(), context("user", &[])?, true).await?;
        let response = h
            .app
            .clone()
            .oneshot(request(
                method,
                URI,
                Some("\"0\""),
                &json!({"crons": ["0 3 * * *"]}),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method}");
        let problem = body(response).await?;
        for text in [
            "example-tracker/main",
            "configured on this install",
            "Connector health",
            "connector and source id",
        ] {
            assert!(
                problem["detail"]
                    .as_str()
                    .is_some_and(|detail| detail.contains(text)),
                "{method}: {problem}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn every_admin_verb_returns_unavailable_when_the_sync_history_read_fails() -> R {
    for method in ["GET", "PUT", "DELETE"] {
        let sync_history = Mock::new();
        sync_history.add(handlers::failure(status::INTERNAL_SERVER_ERROR));
        let h = harness(empty_db(), sync_history.url(), context("user", &[])?, true).await?;
        let response = h
            .app
            .clone()
            .oneshot(request(
                method,
                URI,
                Some("\"0\""),
                &json!({"crons": ["0 3 * * *"]}),
            )?)
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{method}"
        );
        assert_eq!(body(response).await?["status"], 503);
    }
    Ok(())
}

#[tokio::test]
async fn schedule_routes_require_the_admin_role() -> R {
    let h = harness(
        empty_db(),
        "http://127.0.0.1:1",
        context("user", &[])?,
        false,
    )
    .await?;
    for method in ["GET", "PUT", "DELETE"] {
        let response = h
            .app
            .clone()
            .oneshot(request(
                method,
                URI,
                Some("\"0\""),
                &json!({"crons": ["0 3 * * *"]}),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method}");
    }
    Ok(())
}

#[tokio::test]
async fn get_without_a_request_or_acknowledgement_returns_revision_zero_and_inheritance() -> R {
    let sync_history = Mock::new();
    sync_history.add(handlers::provide([configured_instance_record()]));
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([Vec::<Model>::new()])
        .append_query_results([Vec::<
            crate::infra::db::entities::connector_schedule_acknowledgements::Model,
        >::new()])
        .into_connection();
    let h = harness(db, sync_history.url(), context("user", &[])?, true).await?;
    let response = h
        .app
        .clone()
        .oneshot(request("GET", URI, None, &json!(null))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["etag"], "\"0\"");
    let json = body(response).await?;
    assert_eq!(json["revision"], 0);
    assert!(json["requested"].is_null());
    assert!(json["applied"].is_null());
    assert!(json["inherited"].is_null());
    assert_eq!(json["status"], "inherited");
    assert_eq!(json["reconcile_schedules"], json!(["*/15 * * * *"]));
    assert_eq!(json["min_interval_minutes"], 30);
    Ok(())
}

#[tokio::test]
async fn writes_require_one_well_formed_if_match_header() -> R {
    for method in ["PUT", "DELETE"] {
        for (header, expected) in [
            (None, StatusCode::PRECONDITION_REQUIRED),
            (Some("W/\"0\""), StatusCode::BAD_REQUEST),
        ] {
            let sync_history = Mock::new();
            sync_history.add(handlers::provide([configured_instance_record()]));
            let h = harness(empty_db(), sync_history.url(), context("user", &[])?, true).await?;
            let response = h
                .app
                .clone()
                .oneshot(request(
                    method,
                    URI,
                    header,
                    &json!({"crons": ["0 3 * * *"]}),
                )?)
                .await?;
            assert_eq!(response.status(), expected, "{method}, {header:?}");
            assert_eq!(body(response).await?["status"], expected.as_u16());
        }
    }
    Ok(())
}

#[tokio::test]
async fn stale_update_returns_precondition_failed() -> R {
    let sync_history = Mock::new();
    sync_history.add(handlers::provide([configured_instance_record()]));
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([
            vec![Model {
                connector: "example-tracker".to_owned(),
                source_id: "main".to_owned(),
                crons: json!(["0 6 * * *"]),
                revision: 3,
                updated_by: CALLER,
                updated_at: chrono::DateTime::UNIX_EPOCH,
            }],
            vec![Model {
                connector: "example-tracker".to_owned(),
                source_id: "main".to_owned(),
                crons: json!(["0 9 * * *"]),
                revision: 4,
                updated_by: CALLER,
                updated_at: chrono::DateTime::UNIX_EPOCH,
            }],
        ])
        .append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 0,
        }])
        .into_connection();
    let h = harness(db, sync_history.url(), context("user", &[])?, true).await?;
    let response = h
        .app
        .clone()
        .oneshot(request(
            "PUT",
            URI,
            Some("\"3\""),
            &json!({"crons": ["0 3 * * *"]}),
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
    assert_eq!(response.headers()["etag"], "\"4\"");
    assert_eq!(body(response).await?["context"]["revision"], 4);
    Ok(())
}

#[test]
fn repeated_if_match_headers_are_invalid() {
    let mut headers = HeaderMap::new();
    headers.append("if-match", HeaderValue::from_static("\"0\""));
    headers.append("if-match", HeaderValue::from_static("\"0\""));
    assert!(matches!(
        super::if_match(&headers),
        Err(super::domain::Error::InvalidMatch)
    ));
}

#[test]
fn schedule_config_defaults_to_empty_cadence() -> R {
    let config: crate::config::GearConfig = serde_json::from_value(json!({}))?;
    assert!(config.connector_schedules.reconcile_schedules.is_empty());
    Ok(())
}

#[test]
fn schedule_contract_documents_routes_preconditions_and_etags() -> R {
    let json = serde_json::to_value(crate::api::openapi_document()?)?;
    let path = &json["paths"]["/v1/connectors/{connector}/{source_id}/schedule"];
    for method in ["get", "put", "delete"] {
        let op = &path[method];
        assert!(
            op["responses"]["200"]["headers"]["ETag"].is_object(),
            "{method}"
        );
        assert!(op["responses"]["503"].is_object(), "{method}");
    }
    for method in ["put", "delete"] {
        assert!(path[method]["responses"]["412"]["headers"]["ETag"].is_object());
        for code in ["409", "412", "428"] {
            assert!(
                path[method]["responses"][code].is_object(),
                "{method} {code}"
            );
        }
        let parameters = path[method]["parameters"]
            .as_array()
            .ok_or("parameters missing")?;
        assert!(
            parameters
                .iter()
                .any(|p| p["name"] == "If-Match" && p["in"] == "header" && p["required"] == true)
        );
    }
    assert!(path["put"]["responses"]["415"].is_object());
    assert!(path["put"]["responses"].get("422").is_none());
    assert!(json["paths"]["/v1/connector-schedules"].is_null());
    Ok(())
}
