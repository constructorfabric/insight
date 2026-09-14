use std::error::Error;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{HeaderValue, Request, StatusCode};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt as _;

use super::*;
use crate::api::AppState;
use crate::catalog::Catalog;
use crate::chat::ChatClient;
use crate::config::McpConfig;
use crate::definitions::memory::MemoryDefinitions;
use crate::identity::IdentityClient;
use crate::mcp::test_support::Issuer;
use crate::metric_query::{MetricRunner, People};
use crate::raw_data::RawDataStore;
use crate::tables::TableStore;

type R = Result<(), Box<dyn Error>>;

fn surfaces() -> tools::CustomSurfaces {
    let client = || {
        insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            "http://clickhouse.invalid",
            "insight",
        ))
    };

    let Ok(identity) = IdentityClient::new("http://identity.invalid") else {
        panic!("a plain http base URL builds an identity client");
    };

    let state = Arc::new(AppState::new(
        RawDataStore::new(client()),
        TableStore::new(client()),
        Arc::new(MemoryDefinitions::new()),
        MetricRunner::new(client(), People::new("identity")),
        ChatClient::keyless(),
        identity,
        Catalog::new(client(), "insight".to_owned()),
    ));

    tools::CustomSurfaces::new(state)
}

fn enabled() -> McpConfig {
    McpConfig {
        enabled: true,
        bind_addr: "127.0.0.1:0".to_owned(),
        public_url: "http://localhost:3000".to_owned(),
        jwks_url: String::new(),
        allow_insecure_private_network: false,
    }
}

fn call(authorization: Option<&str>) -> Result<Request<Body>, Box<dyn Error>> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(auth::MCP_PATH)
        .header("host", "localhost")
        .header("content-type", "application/json");

    if let Some(value) = authorization {
        builder = builder.header(AUTHORIZATION, HeaderValue::from_str(value)?);
    }

    Ok(builder.body(Body::from(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
    ))?)
}

fn mcp_request(
    token: &str,
    session: Option<&str>,
    payload: &str,
) -> Result<Request<Body>, Box<dyn Error>> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(auth::MCP_PATH)
        .header("host", "localhost")
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json, text/event-stream")
        .header(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))?,
        );

    if let Some(session) = session {
        builder = builder.header("mcp-session-id", HeaderValue::from_str(session)?);
    }

    Ok(builder.body(Body::from(payload.to_owned()))?)
}

#[tokio::test]
async fn a_disabled_server_binds_nothing_and_reports_success() -> R {
    start(&McpConfig::default(), surfaces(), CancellationToken::new()).await?;

    Ok(())
}

#[tokio::test]
async fn an_enabled_server_with_no_public_url_refuses_to_start() {
    let config = McpConfig {
        enabled: true,
        ..McpConfig::default()
    };

    assert!(
        start(&config, surfaces(), CancellationToken::new())
            .await
            .is_err(),
        "a server with no origin cannot verify a token and must not listen"
    );
}

#[tokio::test]
async fn an_enabled_server_binds_its_address_and_stops_when_cancelled() -> R {
    let cancellation = CancellationToken::new();

    start(&enabled(), surfaces(), cancellation.clone()).await?;
    cancellation.cancel();

    Ok(())
}

#[tokio::test]
async fn a_request_carrying_no_bearer_token_is_challenged() -> R {
    let router = router(&enabled(), surfaces(), CancellationToken::new())?;

    let response = router.oneshot(call(None)?).await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let Some(challenge) = response
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
    else {
        panic!("a challenge tells the client where to authorize");
    };
    assert!(
        challenge.contains("oauth-protected-resource/mcp/v3"),
        "{challenge}"
    );
    assert!(challenge.contains(auth::MCP_SCOPE), "{challenge}");

    Ok(())
}

#[tokio::test]
async fn a_request_whose_authorization_is_not_a_bearer_is_challenged() -> R {
    let router = router(&enabled(), surfaces(), CancellationToken::new())?;

    let response = router.oneshot(call(Some("Basic abc"))?).await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

#[tokio::test]
async fn a_bearer_this_server_cannot_verify_does_not_reach_the_tools() -> R {
    let router = router(&enabled(), surfaces(), CancellationToken::new())?;

    let response = router.oneshot(call(Some("Bearer not-a-token"))?).await?;

    assert!(
        matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::SERVICE_UNAVAILABLE
        ),
        "unexpected status: {}",
        response.status()
    );

    Ok(())
}

#[test]
fn a_bearer_header_yields_its_token_and_anything_else_yields_none() {
    let mut headers = axum::http::HeaderMap::new();
    assert_eq!(auth::bearer_token(&headers), None);

    for value in ["Basic abc", "Bearer ", "Bearer two words", "bearer-abc"] {
        let Ok(header) = HeaderValue::from_str(value) else {
            panic!("the fixture is a valid header value: {value}");
        };
        headers.insert(AUTHORIZATION, header);
        assert_eq!(
            auth::bearer_token(&headers),
            None,
            "should reject: {value:?}"
        );
    }

    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer abc"));
    assert_eq!(auth::bearer_token(&headers), Some("abc"));
}

#[tokio::test]
async fn an_authorized_client_initializes_and_lists_the_tools_over_http() -> R {
    let issuer = Issuer::start().await;
    let config = McpConfig {
        enabled: true,
        bind_addr: "127.0.0.1:0".to_owned(),
        public_url: issuer.origin.clone(),
        jwks_url: String::new(),
        allow_insecure_private_network: true,
    };
    let router = router(&config, surfaces(), CancellationToken::new())?;
    let token = issuer.sign(&issuer.claims());

    let initialize = router
        .clone()
        .oneshot(mcp_request(
            &token,
            None,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test-client","version":"0.0.0"}}}"#,
        )?)
        .await?;

    assert_eq!(
        initialize.status(),
        StatusCode::OK,
        "initialize should be accepted for a valid token"
    );
    // No session id: legacy_session_mode is off, so the transport is stateless
    // and every request stands alone.
    assert!(
        !initialize.headers().contains_key("mcp-session-id"),
        "a stateless transport hands out no session"
    );

    let body = to_bytes(initialize.into_body(), 64 * 1024).await?;
    let initialized: Value = serde_json::from_slice(&body)?;
    assert_eq!(
        initialized["result"]["serverInfo"]["name"], "insight-custom-surfaces",
        "{initialized}"
    );

    let listed = router
        .oneshot(mcp_request(
            &token,
            None,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        )?)
        .await?;

    assert_eq!(listed.status(), StatusCode::OK);
    let body = to_bytes(listed.into_body(), 256 * 1024).await?;
    let listed: Value = serde_json::from_slice(&body)?;

    let Some(tools) = listed["result"]["tools"].as_array() else {
        panic!("tools/list returns an array: {listed}");
    };
    let mut names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    names.sort_unstable();

    assert_eq!(
        names,
        [
            "arrange_dashboard",
            "delete_definition",
            "get_definition",
            "list_definitions",
            "list_tables",
            "put_dashboard",
            "put_metric",
            "put_widget",
            "run_metric",
            "search_definitions",
        ],
        "{listed}"
    );

    issuer.stop();
    Ok(())
}

#[tokio::test]
async fn a_token_for_the_read_only_server_does_not_open_this_one_over_http() -> R {
    let issuer = Issuer::start().await;
    let config = McpConfig {
        enabled: true,
        bind_addr: "127.0.0.1:0".to_owned(),
        public_url: issuer.origin.clone(),
        jwks_url: String::new(),
        allow_insecure_private_network: true,
    };
    let router = router(&config, surfaces(), CancellationToken::new())?;

    let mut claims = issuer.claims();
    claims["aud"] = serde_json::json!(format!("{}/mcp", issuer.origin));
    claims["scope"] = serde_json::json!("openid mcp:query");
    let token = issuer.sign(&claims);

    let response = router
        .oneshot(mcp_request(
            &token,
            None,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )?)
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    issuer.stop();
    Ok(())
}
