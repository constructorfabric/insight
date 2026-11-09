use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::{Json, Router, routing::get};
use sea_orm::{DatabaseBackend, MockDatabase};
use serde_json::json;
use toolkit_canonical_errors::CanonicalError;

use super::{AppState, require_admin};
use crate::infra::identity::IdentityClient;

type R = Result<(), Box<dyn std::error::Error>>;

fn state(url: &str) -> Result<AppState, reqwest::Error> {
    Ok(AppState {
        db: MockDatabase::new(DatabaseBackend::MySql).into_connection(),
        ch: insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            "http://localhost:8123",
            "insight",
        )),
        identity: IdentityClient::new(url)?,
    })
}

#[derive(Debug)]
#[toolkit_canonical_errors::resource_error("gts.cf.insight.connectors_api.test.v1~")]
struct TestError;

fn denied() -> CanonicalError {
    TestError::permission_denied()
        .with_reason("admin role required")
        .create()
}

#[tokio::test]
async fn an_unconfigured_identity_fails_closed() -> R {
    for url in ["", "file:///tmp/example-identity"] {
        let result = require_admin(&state(url)?, &HeaderMap::new(), denied).await;
        assert_eq!(
            result
                .err()
                .ok_or("expected permission failure")?
                .into_response()
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    Ok(())
}

#[tokio::test]
async fn only_the_active_identity_admin_role_allows_access() -> R {
    for (body, expected) in [
        (
            json!({"roles": [{"role_id": "a4d11000-0000-4000-8000-000000000001"}]}),
            StatusCode::OK,
        ),
        (
            json!({"roles": [{"role_id": "00000000-0000-4000-8000-000000000001"}]}),
            StatusCode::FORBIDDEN,
        ),
        (json!({"roles": []}), StatusCode::FORBIDDEN),
        (json!({}), StatusCode::INTERNAL_SERVER_ERROR),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let app = Router::new().route(
            "/v1/me",
            get(move |headers: HeaderMap| async move {
                assert_eq!(
                    headers
                        .get(header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok()),
                    Some("Bearer example-token")
                );
                Json(body)
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer example-token".parse()?);
        let result = require_admin(&state(&format!("http://{address}"))?, &headers, denied).await;
        server.abort();
        let status =
            result.map_or_else(|error| error.into_response().status(), |()| StatusCode::OK);
        assert_eq!(status, expected, "identity response must yield {expected}");
    }
    Ok(())
}

#[tokio::test]
async fn identity_failures_are_server_errors() -> R {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let app = Router::new().route("/v1/me", get(|| async { StatusCode::SERVICE_UNAVAILABLE }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let result = require_admin(
        &state(&format!("http://{address}"))?,
        &HeaderMap::new(),
        denied,
    )
    .await;
    server.abort();
    assert_eq!(
        result
            .err()
            .ok_or("expected permission failure")?
            .into_response()
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    Ok(())
}
