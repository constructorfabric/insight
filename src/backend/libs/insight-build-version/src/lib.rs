use axum::Router;
use axum::body::Bytes;
use axum::http::header;
use axum::routing::get;
use serde::Serialize;

const ENV: &str = "INSIGHT_BUILD_VERSION";
const UNKNOWN: &str = "unknown";

#[derive(Serialize)]
struct Report<'a> {
    service: &'a str,
    version: &'a str,
}

pub fn router(service: &'static str) -> Router {
    let body = Bytes::from(document(service, std::env::var(ENV).ok()));

    Router::new().route(
        "/version",
        get(move || {
            let body = body.clone();
            async move { ([(header::CONTENT_TYPE, "application/json")], body) }
        }),
    )
}

fn document(service: &str, stamped: Option<String>) -> Vec<u8> {
    let version = stamped.filter(|value| !value.is_empty());
    let report = Report {
        service,
        version: version.as_deref().unwrap_or(UNKNOWN),
    };

    serde_json::to_vec(&report).unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{document, router};
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt;

    async fn answered(service: &'static str) -> (String, serde_json::Value) {
        let response = router(service)
            .oneshot(
                Request::builder()
                    .uri("/version")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();

        (content_type, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn names_the_service_so_a_reply_cannot_be_mistaken_for_another_gear() {
        let (_, report) = answered("analytics").await;

        assert_eq!(report["service"], "analytics");
    }

    #[tokio::test]
    async fn answers_json_so_a_caller_can_parse_it_without_sniffing() {
        let (content_type, _) = answered("analytics").await;

        assert!(
            content_type.starts_with("application/json"),
            "got {content_type:?}"
        );
    }

    #[tokio::test]
    async fn answers_unknown_when_the_deploy_stamped_nothing() {
        let (_, report) = answered("analytics").await;

        assert_eq!(report["version"], "unknown");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod stamps {
    use super::document;

    fn version_of(stamped: Option<&str>) -> String {
        let body = document("analytics", stamped.map(str::to_owned));
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        parsed["version"].as_str().unwrap().to_owned()
    }

    #[test]
    fn a_stamped_tag_is_reported_verbatim() {
        assert_eq!(
            version_of(Some("2026.08.20.10.11-f8db7bc")),
            "2026.08.20.10.11-f8db7bc"
        );
    }

    #[test]
    fn nothing_stamped_reads_unknown() {
        assert_eq!(version_of(None), "unknown");
    }

    #[test]
    fn an_empty_stamp_is_nothing_stamped() {
        assert_eq!(version_of(Some("")), "unknown");
    }
}
