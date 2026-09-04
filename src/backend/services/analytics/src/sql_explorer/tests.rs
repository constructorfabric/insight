use secrecy::SecretString;

use super::*;

#[test]
fn database_error_classification_requires_an_exact_leading_code() {
    use super::executor::QueryFailure;

    for message in [
        "",
        "TIMEOUT_EXCEEDED",
        "Code: not-a-number",
        "Code: 159x.",
        "Code: 65536.",
        "unexpected Code: 159.",
        "Code: 9999. nested Code: 159.",
    ] {
        let failure = QueryFailure::from(clickhouse::error::Error::BadResponse(message.to_owned()));
        assert!(
            matches!(failure, QueryFailure::ClickHouse(_)),
            "message: {message}"
        );
    }
    assert!(matches!(
        QueryFailure::from(clickhouse::error::Error::BadResponse(
            "Code: 159".to_owned()
        )),
        QueryFailure::Timeout
    ));
    assert!(matches!(
        QueryFailure::from(clickhouse::error::Error::TimedOut),
        QueryFailure::Timeout
    ));
}

#[test]
fn api_only_startup_requires_credentials_but_not_oauth_metadata() {
    let mut mcp = McpConfig::default();
    let mut api = SqlApiConfig::default();
    assert!(validate_config(&mcp, &api).is_ok());
    api.enabled = true;
    assert!(validate_config(&mcp, &api).is_err());
    mcp.clickhouse_password = SecretString::from("synthetic-db-password".to_owned());
    for token in ["", "short", "synthetic-token-with-a-space-in it"] {
        api.token = SecretString::from(token.to_owned());
        assert!(validate_config(&mcp, &api).is_err(), "token case: {token}");
    }
    api.token = SecretString::from("synthetic-test-token-not-a-real-secret".to_owned());
    assert!(validate_config(&mcp, &api).is_ok());
    mcp.enabled = true;
    assert!(validate_config(&mcp, &api).is_err());
    mcp.public_url = "https://insight.example.com".to_owned();
    assert!(validate_config(&mcp, &api).is_ok());
}

#[tokio::test]
async fn cloned_executors_share_capacity_and_release_it_after_completion()
-> Result<(), Box<dyn std::error::Error>> {
    use std::sync::Arc;

    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::json;
    use tokio::sync::Notify;

    use super::executor::{QueryExecutor, QueryFailure};

    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let handler_entered = entered.clone();
    let handler_release = release.clone();
    let upstream = Router::new().route(
        "/",
        post(move || {
            let entered = handler_entered.clone();
            let release = handler_release.clone();
            async move {
                entered.notify_one();
                release.notified().await;
                Json(json!({"meta":[],"data":[]}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await });
    let client = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
        format!("http://{address}"),
        "gold",
    ));
    let executor = QueryExecutor::new(client, 1);
    let other = executor.clone();
    let running = tokio::spawn(async move { other.query("SELECT 1".to_owned(), Ok).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified()).await?;
    let result = executor.query("SELECT 2".to_owned(), Ok).await;
    assert!(matches!(result, Err(QueryFailure::Busy)));
    release.notify_one();
    running.await??;
    let result = executor
        .query("DROP TABLE gold.example".to_owned(), Ok)
        .await;
    assert!(matches!(result, Err(QueryFailure::InvalidSql(_))));
    server.abort();
    Ok(())
}
