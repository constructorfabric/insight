use axum::http::StatusCode;
use clickhouse::test::{Mock, handlers};
use sea_orm::{DatabaseBackend, MockDatabase};
use serde_json::json;
use tower::ServiceExt;

use super::support::*;
use crate::infra::db::entities::connector_schedules::Model;

#[tokio::test]
async fn writes_enforce_the_configured_minimum() -> R {
    let sync_history = Mock::new();
    sync_history.add(handlers::provide([configured_instance_record()]));
    let db = MockDatabase::new(DatabaseBackend::MySql).into_connection();
    let mut config = schedule_config(true);
    config.min_interval_minutes = 60;
    let h =
        harness_with_config(db, sync_history.url(), config, context("user", &[])?, true).await?;
    let response = h
        .app
        .clone()
        .oneshot(request(
            "PUT",
            URI,
            Some("\"0\""),
            &json!({"crons": ["0,30 * * * *"]}),
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        body(response)
            .await?
            .to_string()
            .contains("at most once every 60 minutes")
    );
    Ok(())
}

#[tokio::test]
async fn rejected_requests_identify_the_invalid_field_and_reason() -> R {
    for (uri, payload, field, reason, reads_sync_history) in [
        (
            URI,
            json!({"crons": ["*/15 * * * *"]}),
            "crons[0]",
            "has starts 15 minutes apart",
            true,
        ),
        (
            URI,
            json!({"crons": ["0 6 31 2 *"]}),
            "crons[0]",
            "never runs",
            true,
        ),
        (
            URI,
            json!({"crons": ["0 6 * * *", "0 25 * * *"]}),
            "crons[1]",
            "hour",
            true,
        ),
        (
            URI,
            json!({"crons": ["0 6 * * *", " 0\t6 * * * "]}),
            "crons[1]",
            "duplicates crons[0]",
            true,
        ),
        (
            URI,
            json!({"crons": "0 6 * * *"}),
            "crons",
            "expected a list",
            true,
        ),
        (URI, json!({}), "crons", "missing required", true),
        (
            URI,
            json!({"crons": ["0 6 * * *"], "timezone": "Europe/Sofia"}),
            "timezone",
            "always UTC",
            true,
        ),
        (
            "/v1/connectors/INVALID/main/schedule",
            json!({"crons": ["0 6 * * *"]}),
            "connector",
            "invalid connector",
            false,
        ),
        (
            "/v1/connectors/example-tracker/INVALID/schedule",
            json!({"crons": ["0 6 * * *"]}),
            "source_id",
            "invalid source_id",
            false,
        ),
    ] {
        let sync_history = Mock::new();
        if reads_sync_history {
            sync_history.add(handlers::provide([configured_instance_record()]));
        }
        let db = MockDatabase::new(DatabaseBackend::MySql).into_connection();
        let h = harness(db, sync_history.url(), true, context("user", &[])?, true).await?;
        let response = h
            .app
            .clone()
            .oneshot(request("PUT", uri, Some("\"0\""), &payload)?)
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{payload}");
        let problem = body(response).await?;
        let violation = &problem["context"]["field_violations"][0];
        assert_eq!(violation["field"], field, "{payload}: {problem}");
        assert!(
            violation["description"]
                .as_str()
                .is_some_and(|detail| detail.contains(reason)),
            "{payload}: {problem}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn unchanged_puts_and_deletes_return_the_current_revision() -> R {
    for method in ["PUT", "DELETE"] {
        let sync_history = Mock::new();
        sync_history.add(handlers::provide([configured_instance_record()]));
        let crons = if method == "PUT" {
            json!(["0 3 * * *"])
        } else {
            json!([])
        };
        let row = Model {
            connector: "example-tracker".to_owned(),
            source_id: "main".to_owned(),
            crons,
            revision: 7,
            updated_by: CALLER,
            updated_at: chrono::DateTime::UNIX_EPOCH,
        };
        let db = MockDatabase::new(DatabaseBackend::MySql)
            .append_query_results([vec![row]])
            .into_connection();
        let h = harness(db, sync_history.url(), true, context("user", &[])?, true).await?;
        let response = h
            .app
            .clone()
            .oneshot(request(
                method,
                URI,
                Some("\"7\""),
                &json!({"crons": [" 0\t3 * * * "]}),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK, "{method}");
        assert_eq!(response.headers()["etag"], "\"7\"");
        let json = body(response).await?;
        assert_eq!(json["revision"], 7);
        assert_eq!(json["min_interval_minutes"], 30, "{method}");
        if method == "PUT" {
            assert_eq!(json["requested"]["updated_at"], "1970-01-01T00:00:00Z");
        } else {
            assert_eq!(json["requested"]["crons"], json!([]));
            assert_eq!(json["requested"]["updated_at"], "1970-01-01T00:00:00Z");
        }
    }
    Ok(())
}

#[tokio::test]
async fn malformed_json_and_media_types_have_actionable_handler_responses() -> R {
    for (content_type, payload, status, field) in [
        (
            Some("application/json"),
            "not JSON",
            StatusCode::BAD_REQUEST,
            "body",
        ),
        (
            None,
            "{}",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type",
        ),
        (
            Some("text/plain"),
            "{}",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type",
        ),
    ] {
        let sync_history = Mock::new();
        sync_history.add(handlers::provide([configured_instance_record()]));
        let db = MockDatabase::new(DatabaseBackend::MySql).into_connection();
        let h = harness(db, sync_history.url(), true, context("user", &[])?, true).await?;
        let mut request = request("PUT", URI, Some("\"0\""), &json!({}))?;
        request.headers_mut().remove("content-type");
        if let Some(content_type) = content_type {
            request
                .headers_mut()
                .insert("content-type", content_type.parse()?);
        }
        *request.body_mut() = axum::body::Body::from(payload);
        let response = h.app.clone().oneshot(request).await?;
        assert_eq!(response.status(), status, "{content_type:?}: {payload}");
        let problem = body(response).await?;
        assert_eq!(
            problem["context"]["field_violations"][0]["field"], field,
            "{problem}"
        );
        if status == StatusCode::UNSUPPORTED_MEDIA_TYPE {
            assert!(
                problem["detail"]
                    .as_str()
                    .is_some_and(|text| text.contains("application/json is required")),
                "{problem}"
            );
        }
    }
    Ok(())
}
