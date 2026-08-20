use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

const SERVICE: &str = "identity-resolution";

#[derive(Serialize, Clone)]
struct Report {
    service: &'static str,
    version: String,
}

pub(crate) fn router(stamped: Option<String>) -> Router {
    let report = Report {
        service: SERVICE,
        version: stamped
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unknown".to_owned()),
    };

    Router::new().route(
        "/version",
        get(move || {
            let report = report.clone();
            async move { Json(report) }
        }),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn reported(stamped: Option<&str>) -> serde_json::Value {
        let response = router(stamped.map(str::to_owned))
            .oneshot(
                Request::builder()
                    .uri("/version")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn names_the_service_so_a_reply_cannot_be_mistaken_for_another_gear() {
        assert_eq!(
            reported(Some("2026.08.20.10.11-f8db7bc")).await["service"],
            "identity-resolution"
        );
    }

    #[tokio::test]
    async fn answers_with_the_version_the_deploy_stamped() {
        assert_eq!(
            reported(Some("2026.08.20.10.11-f8db7bc")).await["version"],
            "2026.08.20.10.11-f8db7bc"
        );
    }

    #[tokio::test]
    async fn answers_unknown_when_the_deploy_stamped_nothing() {
        assert_eq!(reported(None).await["version"], "unknown");
    }

    #[tokio::test]
    async fn treats_an_empty_stamp_as_nothing_stamped() {
        assert_eq!(reported(Some("")).await["version"], "unknown");
    }
}
