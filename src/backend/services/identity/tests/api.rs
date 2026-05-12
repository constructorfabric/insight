//! HTTP integration tests for the Identity Resolution stub.

use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use identity_resolution::people::PeopleStore;
use tower::ServiceExt;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn test_data() -> &'static [u8] {
    br#"{"id":"1","status":"Active","firstName":"Alice","lastName":"Smith","displayName":"Alice Smith","workEmail":"alice@example.com","department":"Engineering","division":"R&D","jobTitle":"Staff Engineer","supervisorEmail":"bob@example.com","supervisor":"Jones, Bob"}
{"id":"2","status":"Active","firstName":"Bob","lastName":"Jones","displayName":"Bob Jones","workEmail":"bob@example.com","department":"Engineering","division":"R&D","jobTitle":"Engineering Manager","supervisorEmail":null,"supervisor":null}
{"id":"3","status":"Active","firstName":"Dave","lastName":"Ng","displayName":"Dave Ng","workEmail":"dave@example.com","department":"Engineering","division":"R&D","jobTitle":"Senior Engineer","supervisorEmail":"bob@example.com","supervisor":"Jones, Bob"}"#
}

fn app() -> Result<axum::Router, Box<dyn std::error::Error>> {
    let store = PeopleStore::from_json_lines(test_data())?;
    Ok(identity_resolution::build_router(store))
}

async fn get(uri: &str) -> Result<(StatusCode, serde_json::Value), Box<dyn std::error::Error>> {
    let resp = app()?
        .oneshot(Request::get(uri).body(axum::body::Body::empty())?)
        .await?;
    let status = resp.status();
    let body = resp.into_body().collect().await?.to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    Ok((status, json))
}

#[tokio::test]
async fn health_returns_healthy() -> TestResult {
    let (status, body) = get("/health").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "healthy");
    Ok(())
}

#[tokio::test]
async fn healthz_returns_ok() -> TestResult {
    let resp = app()?
        .oneshot(Request::get("/healthz").body(axum::body::Body::empty())?)
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await?.to_bytes();
    assert_eq!(&body[..], b"ok");
    Ok(())
}

#[tokio::test]
async fn get_person_returns_data() -> TestResult {
    let (status, body) = get("/v1/persons/alice@example.com").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], "alice@example.com");
    assert_eq!(body["display_name"], "Alice Smith");
    assert_eq!(body["department"], "Engineering");
    assert_eq!(body["job_title"], "Staff Engineer");
    assert_eq!(body["supervisor_email"], "bob@example.com");
    Ok(())
}

#[tokio::test]
async fn get_person_case_insensitive() -> TestResult {
    let (status, body) = get("/v1/persons/Alice@Example.COM").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], "alice@example.com");
    Ok(())
}

#[tokio::test]
async fn get_person_not_found() -> TestResult {
    let (status, body) = get("/v1/persons/nobody@example.com").await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["status"], 404);
    let detail = body["detail"].as_str().ok_or("missing detail field")?;
    assert!(detail.contains("nobody@example.com"));
    Ok(())
}

#[tokio::test]
async fn supervisor_includes_subordinates() -> TestResult {
    let (status, body) = get("/v1/persons/bob@example.com").await?;
    assert_eq!(status, StatusCode::OK);

    let subs = body["subordinates"]
        .as_array()
        .ok_or("missing subordinates array")?;
    assert_eq!(subs.len(), 2);

    let emails: Vec<&str> = subs.iter().filter_map(|s| s["email"].as_str()).collect();
    assert!(emails.contains(&"alice@example.com"));
    assert!(emails.contains(&"dave@example.com"));
    Ok(())
}

#[tokio::test]
async fn leaf_has_no_subordinates() -> TestResult {
    let (status, body) = get("/v1/persons/alice@example.com").await?;
    assert_eq!(status, StatusCode::OK);
    let subs = body["subordinates"]
        .as_array()
        .ok_or("missing subordinates array")?;
    assert!(subs.is_empty());
    Ok(())
}
