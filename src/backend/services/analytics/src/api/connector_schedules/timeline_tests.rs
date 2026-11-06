use axum::http::StatusCode;
use serde_json::json;

use super::support::*;
use crate::domain::connector_schedules::test_support::{acknowledgement, schedule_response};
use crate::infra::db::entities::connector_schedules::Model;

#[tokio::test]
async fn the_api_serves_acknowledged_schedule_deployment_and_attempt_error() -> R {
    for (revision, failed, expected) in [
        (2, false, "applied"),
        (2, true, "error"),
        (3, true, "pending"),
    ] {
        let mut ack = acknowledgement();
        ack.last_error = failed.then(|| "synthetic readback failure".to_owned());
        let request = Model {
            connector: "example-tracker".to_owned(),
            source_id: "main".to_owned(),
            crons: json!(["0 3 * * *"]),
            revision,
            updated_by: CALLER,
            updated_at: chrono::DateTime::UNIX_EPOCH,
        };
        let response = super::reply(schedule_response(request, &ack)?);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["etag"], format!("\"{revision}\""));
        let response = body(response).await?;
        assert_eq!(response["status"], expected);
        assert_eq!(
            response["applied"],
            json!({
                "crons": ["0 3 * * *"], "timezone": "UTC", "origin": "api", "suspended": false,
                "revision": 2, "observed_at": "1970-01-01T00:00:00Z"
            })
        );
        assert_eq!(
            response["inherited"],
            json!({"crons": ["0 5 * * *"], "origin": "descriptor"})
        );
        if failed {
            assert_eq!(
                response["error"],
                json!({"attempted_revision": 2, "attempted_at": "1970-01-01T00:00:00Z", "message": "synthetic readback failure"})
            );
        } else {
            assert!(response["error"].is_null());
        }
        assert!(response.get("not_applied_since").is_none());
    }
    Ok(())
}
