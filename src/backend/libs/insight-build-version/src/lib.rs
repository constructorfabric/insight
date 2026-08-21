use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

const ENV: &str = "INSIGHT_BUILD_VERSION";
const UNKNOWN: &str = "unknown";

#[derive(Serialize, Clone)]
struct Report {
    service: String,
    version: String,
}

pub fn router(service: &str) -> Router {
    router_with(service, std::env::var(ENV).ok())
}

fn router_with(service: &str, stamped: Option<String>) -> Router {
    let report = Report {
        service: service.to_owned(),
        version: stamped
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| UNKNOWN.to_owned()),
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::router_with;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn reported(service: &str, stamped: Option<&str>) -> serde_json::Value {
        let response = router_with(service, stamped.map(str::to_owned))
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
        let report = reported("analytics", Some("1970.01.01.00.00-abc1234")).await;

        assert_eq!(report["service"], "analytics");
    }

    #[tokio::test]
    async fn answers_with_the_tag_the_deploy_stamped() {
        let report = reported("analytics", Some("1970.01.01.00.00-abc1234")).await;

        assert_eq!(report["version"], "1970.01.01.00.00-abc1234");
    }

    #[tokio::test]
    async fn answers_unknown_when_the_deploy_stamped_nothing() {
        let report = reported("analytics", None).await;

        assert_eq!(report["version"], "unknown");
    }

    #[tokio::test]
    async fn treats_an_empty_stamp_as_nothing_stamped() {
        let report = reported("analytics", Some("")).await;

        assert_eq!(report["version"], "unknown");
    }
}
