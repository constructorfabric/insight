use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::put;
use serde_json::json;
use toolkit_canonical_errors::Problem;
use tower::ServiceExt;

use super::*;

type R = Result<(), Box<dyn std::error::Error>>;

#[test]
fn body_shape_errors_identify_the_request_field() -> R {
    for (value, field, detail) in [
        (json!({}), "crons", "missing required"),
        (json!({"crons": "0 6 * * *"}), "crons", "expected a list"),
        (
            json!({"crons": [1]}),
            "crons[0]",
            "expected a cron expression string",
        ),
        (json!({"crons": null}), "crons", "expected a list"),
        (json!([]), "body", "expected a JSON object"),
        (
            json!({"crons": ["0 6 * * *"], "timezone": "Europe/Sofia"}),
            "timezone",
            "schedules are always UTC",
        ),
        (json!({"other": true}), "other", "unknown field"),
    ] {
        let Err(error) = parse_value(&value) else {
            panic!("must reject {value}");
        };
        let problem = Problem::from(error);
        assert_eq!(problem.status, Some(400), "{value}");
        let violation = &problem.context["field_violations"][0];
        assert_eq!(violation["field"], field, "{value}: {problem:?}");
        assert!(
            violation["description"]
                .as_str()
                .is_some_and(|text| text.contains(detail)),
            "{value}: {problem:?}"
        );
    }
    assert_eq!(
        parse_value(&json!({"crons": ["0 6 * * *"]}))?.crons,
        ["0 6 * * *"]
    );
    Ok(())
}

#[tokio::test]
async fn json_extraction_errors_use_canonical_details() -> R {
    let app = Router::new().route(
        "/",
        put(|request| async move {
            parse(request).map(|request| Json(json!({"crons": request.crons})))
        }),
    );
    for (content_type, payload, status, field, detail) in [
        (
            Some("application/json"),
            "not JSON",
            StatusCode::BAD_REQUEST,
            "body",
            "invalid JSON",
        ),
        (
            Some("application/json"),
            "{",
            StatusCode::BAD_REQUEST,
            "body",
            "invalid JSON",
        ),
        (
            None,
            "{}",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type",
            "application/json is required",
        ),
        (
            Some("text/plain"),
            "{}",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type",
            "application/json is required",
        ),
    ] {
        let mut request = Request::builder().method("PUT").uri("/");
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(payload))?)
            .await?;
        assert_eq!(response.status(), status, "{content_type:?}: {payload}");
        assert_eq!(
            response.headers()["content-type"],
            "application/problem+json"
        );
        let problem = crate::api::connector_schedules::support::body(response).await?;
        let violation = &problem["context"]["field_violations"][0];
        assert_eq!(violation["field"], field, "{problem}");
        assert!(
            violation["description"]
                .as_str()
                .is_some_and(|text| text.contains(detail)),
            "{problem}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn request_parsing_refuses_cron_count_boundaries_and_unknown_fields() -> R {
    let app = Router::new().route(
        "/",
        put(|request| async move {
            let request = parse(request)?;
            let crons =
                crate::domain::connector_schedules::validation::Crons::parse(&request.crons)
                    .map_err(|problem| {
                        super::super::error::domain_error(
                            crate::domain::connector_schedules::Error::Invalid(problem),
                        )
                    })?;
            Ok::<_, super::super::error::ScheduleError>(Json(json!({"crons": crons.into_vec()})))
        }),
    );
    for (payload, field, detail) in [
        (json!({"crons": []}), "crons", "1 to 10"),
        (
            json!({"crons": (0..11).map(|minute| format!("{minute} 3 * * *")).collect::<Vec<_>>()}),
            "crons",
            "1 to 10",
        ),
        (
            json!({"crons": ["0 3 * * *"], "extra": true}),
            "extra",
            "unknown field",
        ),
    ] {
        let request = Request::builder()
            .method("PUT")
            .uri("/")
            .header("content-type", "application/json")
            .body(Body::from(payload.to_string()))?;
        let response = app.clone().oneshot(request).await?;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "should reject: {payload}"
        );
        let problem = crate::api::connector_schedules::support::body(response).await?;
        let violation = &problem["context"]["field_violations"][0];
        assert_eq!(violation["field"], field, "{payload}: {problem}");
        assert!(
            violation["description"]
                .as_str()
                .is_some_and(|text| text.contains(detail)),
            "{payload}: {problem}"
        );
    }
    Ok(())
}
